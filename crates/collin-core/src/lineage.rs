//! The pass that turns a dbt project into a lineage cache.
//!
//! Models are walked parent first, so by the time a model is resolved every
//! relation it reads already has a column list. That ordering is what makes
//! `select *` tractable: it propagates from the leaves instead of needing a
//! warehouse round trip.
//!
//! This module owns the one decision the engine must not make: what to do when
//! the SQL does not answer. The rule is the provenance ladder, and it never
//! silently mixes rungs.
//!
//! 1. `parsed`     the edge was read out of the SQL, and carries its role.
//! 2. `inferred`   the compile was degraded, so the edge comes from matching
//!                 known output columns against known parent columns. The role
//!                 is not claimed, because it was not observed.
//! 3. unresolved   nothing could be said. The report names what was missing.
//!
//! One model's columns can stand on different rungs, one edge never on two: a
//! compile whose table was built from older code keeps its parsed edges on the
//! columns both have, and the table's others are inferred (0029).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::cache::{self, Cache};
use crate::catalog::{Elsewhere, RawCatalog};
use crate::engine::{self, Visible};
use crate::manifest::{norm_relation, Node, Project, RawManifest, SqlSource};
use crate::report::{
    self, IndirectReport, LostColumn, LostEnd, LostRelation, MergedScopeReport, ModelReads,
    AmbiguousInferred, ModelReport, ReadDownstream, RelationCollision, Report, ReportIssue, Thin,
    Totals, UnbackedReport,
};
use crate::role;
use crate::schema::{Agreement, Provenance, Store};

pub struct Options {
    pub project: PathBuf,
    pub manifest: Option<PathBuf>,
    pub catalog: Option<PathBuf>,
    pub out: PathBuf,
    pub report: PathBuf,
    /// Fill degraded models by name matching. On by default: a labelled
    /// inference beats a blank graph, and the label is what keeps it honest.
    pub infer: bool,
    /// Emit a cache edge for a column the SQL reads to decide which rows exist.
    ///
    /// Off by default, on the measurement. Such a column decides which rows
    /// exist rather than what any one value is, so it bears on every output
    /// column and on none in particular, and an edge per output column is the
    /// only shape the cache format has for it. On a 3341 model project that
    /// turned 3327 facts into 114008 edges and took the cache from 18.8 MB to
    /// 39.6 MB, 5217 edges on the largest model alone.
    ///
    /// The report carries the 3327 facts either way, so this flag buys seeing
    /// them in dbt-lens, not knowing them.
    pub indirect: bool,
    pub target: String,
}

impl Options {
    pub fn new(project: PathBuf) -> Options {
        let out = cache::default_path(&project.join("target"));
        let rep = project.join("target").join("column_lineage.report.json");
        Options {
            project,
            manifest: None,
            catalog: None,
            out,
            report: rep,
            infer: true,
            indirect: false,
            target: String::new(),
        }
    }
    fn manifest_path(&self) -> PathBuf {
        self.manifest.clone().unwrap_or_else(|| self.project.join("target").join("manifest.json"))
    }
    /// The manifest's directory. dbt writes `compiled/` beside the manifest, so
    /// that is where the compiled files are looked for, wherever `--manifest`
    /// points (0033).
    pub fn target_dir(&self) -> PathBuf {
        self.manifest_path().parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    }
    fn catalog_path(&self) -> PathBuf {
        self.catalog.clone().unwrap_or_else(|| self.project.join("target").join("catalog.json"))
    }
}

pub struct Outcome {
    pub totals: Totals,
    pub out: PathBuf,
    pub report: PathBuf,
}

/// A project read from disk and not yet walked.
pub struct Loaded {
    pub project: Project,
    /// unique_id to column names, from `catalog.json`. Empty without one.
    pub warehouse: HashMap<String, Vec<String>>,
    /// Catalog entries describing another table than the manifest's, left out
    /// of `warehouse` (0037).
    pub catalog_elsewhere: Vec<Elsewhere>,
}

/// What one pass produced, before any of it is written.
pub struct Run {
    /// In cache order: models parent first, and within a model the direct
    /// edges before the indirect ones.
    pub edges: Vec<cache::Edge>,
    pub report: Report,
}

/// A relation a model reads, as the pass found it before asking the engine.
pub struct Parent<'a> {
    pub node: &'a Node,
    /// The best list the store had, which is the list the engine was handed.
    /// Empty when nothing was known, and the engine was then not told of the
    /// relation at all: the report names it in `unknown_relations` instead.
    pub columns: Vec<String>,
    pub provenance: Provenance,
}

/// What the pass saw of one model, handed to an observer once its edges are
/// published.
///
/// The cache and the report keep what the plan let through, so a model whose
/// compile was set aside shows name matches there and nothing of what its SQL
/// said. Reading why takes the engine's own view, and taking it from the pass
/// rather than from a replay of it is what stops the two drifting apart.
pub struct Seen<'a> {
    pub model: &'a Node,
    /// Every node the model reads, in manifest order, an ephemeral parent
    /// replaced by what it reads. See `read_nodes`.
    pub parents: &'a [Parent<'a>],
    pub resolved: &'a engine::Resolved,
    pub agreement: Agreement,
    /// Known parents and not one edge, which condemns the compile on its own.
    /// Given rather than left to be worked out, so that an observer cannot
    /// disagree with the plan about it.
    pub silent: bool,
    /// A compiled file that reads a relation the manifest does not give the
    /// model, which sets it aside on its own (0033). Given for the same reason.
    pub foreign: bool,
    pub plan: Plan,
    /// As the pass left it, so this model's computed list is there only when
    /// the plan kept it.
    pub store: &'a Store,
    /// The direct edges this model put in the cache, as written. Where an edge
    /// comes from and what role it plays are the pass's to say, so an observer
    /// is handed them rather than working them out from `resolved` again.
    pub published: &'a [cache::Edge],
    /// The indirect edges written after them, empty unless the run asked.
    pub published_indirect: &'a [cache::Edge],
}

/// The dbt unique_id behind a relation a model reads, or the `rel:` escape
/// hatch the cache format reserves for objects dbt does not own.
///
/// `read` is `claimants` for that model. A relation two nodes claim is settled
/// by what the model depends on before it is settled by name: the SQL names the
/// relation, and the dependency says which node the model meant.
fn node_for(project: &Project, read: &HashMap<String, Option<String>>, relation: &str) -> String {
    if let Some(Some(uid)) = read.get(relation) {
        return uid.clone();
    }
    match project.by_relation.get(relation) {
        Some(uid) => uid.clone(),
        None => format!("rel:{}", relation.to_lowercase()),
    }
}

pub fn generate(opts: &Options) -> Result<Outcome, String> {
    let Loaded { project, warehouse, catalog_elsewhere } = load(opts)?;
    let Run { edges, mut report } = run(&project, warehouse, opts);
    report.set_catalog_elsewhere(catalog_elsewhere);
    Cache::new(opts.target.clone(), edges).write(&opts.out)?;
    report.write(&opts.report)?;
    Ok(Outcome { totals: report.totals, out: opts.out.clone(), report: opts.report.clone() })
}

/// Everything the pass reads from disk. The catalog is optional: without one
/// the pass still runs, and no compile can be confirmed.
pub fn load(opts: &Options) -> Result<Loaded, String> {
    let manifest = RawManifest::load(&opts.manifest_path())?;
    let mut project = Project::from_manifest(manifest);
    read_compiled_files(&mut project, &opts.target_dir());

    let catalog_path = opts.catalog_path();
    let witness = if catalog_path.exists() {
        let relation_of = |uid: &str| project.nodes.get(uid).map(|n| n.relation.as_str()).filter(|r| !r.is_empty());
        RawCatalog::load(&catalog_path)?.columns(relation_of)
    } else {
        Default::default()
    };
    Ok(Loaded { project, warehouse: witness.columns, catalog_elsewhere: witness.elsewhere })
}

/// The pass itself, over a project already in memory.
///
/// Apart from `generate` so that it reads and writes no file: a test can hand
/// it an invented project and read back the edges and the report, where
/// otherwise the pass is only observable through a manifest on disk.
pub fn run(project: &Project, warehouse: HashMap<String, Vec<String>>, opts: &Options) -> Run {
    run_observed(project, warehouse, opts, |_| {})
}

/// The pass, showing `observe` each model as it goes.
///
/// For a tool that has to show more than the cache and the report keep. What
/// it is shown is what the pass acted on and wrote, not a recomputation, so it
/// describes the pass `generate` runs and no older one.
pub fn run_observed(
    project: &Project,
    warehouse: HashMap<String, Vec<String>>,
    opts: &Options,
    mut observe: impl FnMut(&Seen),
) -> Run {
    let declared: HashMap<String, Vec<String>> =
        project.nodes.values().map(|n| (n.uid.clone(), n.declared.clone())).collect();
    let mut store = Store::new(declared, warehouse);

    let mut edges: Vec<cache::Edge> = Vec::new();
    // What each model's compile produced, kept past a retraction: a child's
    // read of a column its parent's list lacks is explained if the parent's own
    // compile has it.
    let mut outputs_of: HashMap<String, Vec<String>> = HashMap::new();
    // Columns trusted children read with an edge that their parent's list
    // lacks, filed on the parent once every child has been read (0020).
    let mut downstream: std::collections::BTreeMap<String, std::collections::BTreeMap<String, ReadDownstream>> =
        Default::default();
    // Entries of models with nothing to say, kept in case a child gives them
    // something.
    let mut quiet: HashMap<String, ModelReport> = HashMap::new();
    let mut rep = Report {
        generated_at: cache::now(),
        project: project.project_name.clone(),
        dbt_version: project.dbt_version.clone(),
        adapter: project.adapter.clone(),
        relation_collisions: project
            .collisions
            .iter()
            .map(|(relation, nodes)| RelationCollision {
                relation: relation.clone(),
                nodes: nodes.clone(),
            })
            .collect(),
        ..Default::default()
    };

    for model in project.topological_models() {
        rep.totals.models += 1;

        // Everything this model reads, with the best column list available. An
        // ephemeral parent is never read under its own name: dbt inlines it, so
        // the child reads what the ephemeral reads, and that is what the engine
        // has to be handed for a `select *` through it to expand.
        let mut parents: Vec<Parent> = Vec::new();
        for parent in read_nodes(project, model) {
            let (cols, provenance) = store.visible(&parent.uid);
            parents.push(Parent { node: parent, columns: cols.to_vec(), provenance });
        }
        let mut visible: Vec<Visible> = Vec::new();
        let mut unknown_relations: Vec<String> = Vec::new();
        for parent in &parents {
            if parent.columns.is_empty() {
                // Normalised here because this list is read by a person, and one
                // spelling is easier to scan than dbt's mixture of quoted and not.
                unknown_relations.push(norm_relation(&parent.node.relation));
                continue;
            }
            // Raw, quotes and all: the engine has to match this against the SQL.
            let relation = parent.node.relation.clone();
            visible.push(Visible { relation, columns: parent.columns.clone() });
        }
        unknown_relations.sort();
        unknown_relations.dedup();

        let mut resolved = match model.no_sql() {
            Some(why) => engine::Resolved { parse_error: Some(why), ..Default::default() },
            None => engine::resolve(&model.sql, &project.adapter, &visible),
        };

        outputs_of.insert(model.uid.clone(), resolved.outputs.clone());
        if model.sql_source != SqlSource::Manifest {
            rep.totals.without_compiled_code += 1;
        }
        if model.sql_source.file().is_some() {
            rep.totals.sql_from_files += 1;
        }
        let parse_failed = resolved.parse_error.is_some();
        if parse_failed {
            rep.totals.parse_failed += 1;
        } else {
            rep.totals.parsed += 1;
            store.set_computed(&model.uid, resolved.outputs.clone());
        }

        let agreement = store.agreement(&model.uid);
        match agreement {
            Agreement::Confirmed => rep.totals.confirmed += 1,
            Agreement::Degraded => rep.totals.degraded += 1,
            Agreement::Unchecked => rep.totals.unchecked += 1,
        }
        let stale = store.yaml_is_stale(&model.uid);
        if stale {
            rep.totals.yaml_stale += 1;
        }

        let parents_known = !visible.is_empty();
        // The columns this compile and the model's table both have, in the
        // compile's order. None without a table to ask.
        let shared: Vec<String> = match store.warehouse(&model.uid) {
            Some(w) => resolved.outputs.iter().filter(|c| w.contains(c)).cloned().collect(),
            None => Vec::new(),
        };
        let own = norm_relation(&model.relation);
        let undeclared_relations = undeclared(&own, &dependencies(project, model), &resolved.reads);
        // A compiled file reading a relation the manifest does not give its
        // model was compiled against another graph: another target, or code
        // from before a `ref` moved. Its edges would leave the graph for tables
        // no node owns, where the same read in the manifest's own SQL is the
        // project's to fix (0033).
        let foreign = model.sql_source.file().is_some() && !undeclared_relations.is_empty();
        if foreign {
            rep.totals.sql_from_files_set_aside += 1;
        }
        let plan = plan(&resolved, agreement, parents_known, !shared.is_empty(), foreign);
        // Only where the SQL is trusted: a set aside compile's reads are no
        // evidence about its parents.
        let through_star = match plan {
            Plan::Inferred => Vec::new(),
            _ => read_through_star(project, &store, &outputs_of, model, &visible, &mut resolved),
        };
        // A compile reading a name its own CTE does not project cannot run, so
        // the column list it would give describes no object, whatever edges of
        // it the SQL does support, and the warehouse or the YAML stands in for
        // it downstream. With neither, the list stays: the reads say the SQL
        // cannot run, not that the names it projects are wrong, and taking it
        // away would leave nothing at all (0019).
        let replaceable =
            store.warehouse(&model.uid).is_some() || store.declared(&model.uid).is_some();
        if plan != Plan::Parsed || (!resolved.unbacked.is_empty() && replaceable) {
            store.retract_computed(&model.uid);
        }
        let published = publish(project, &store, model, &resolved, plan, &shared, opts);
        observe(&Seen {
            model,
            parents: &parents,
            resolved: &resolved,
            agreement,
            silent: silent(&resolved, parents_known),
            foreign,
            plan,
            store: &store,
            published: &published.edges[..published.direct],
            published_indirect: &published.edges[published.direct..],
        });
        for a in &published.ambiguous {
            match a.by {
                "compile" => rep.totals.ambiguous_by_compile += 1,
                "every" => rep.totals.ambiguous_by_every += 1,
                _ => rep.totals.ambiguous_by_order += 1,
            }
        }
        rep.totals.edges_parsed += published.direct - published.inferred;
        rep.totals.edges_inferred += published.inferred;
        rep.totals.edges_indirect += published.edges.len() - published.direct;

        // Coverage and the per model edge count are both measured over the
        // direct edges alone: an indirect edge reaches every output column, so
        // counting it would report every model fully covered.
        let produced: HashSet<&str> =
            published.edges[..published.direct].iter().map(|e| e.to_col.as_str()).collect();

        // What a downstream model would see. This is the figure earlier runs
        // printed, kept so the two are comparable, but for a model with no
        // catalog entry it is the list this compile produced, so it measures the
        // compile against itself.
        let (reference, _) = store.visible(&model.uid);
        rep.totals.columns_total += reference.len();
        rep.totals.columns_covered +=
            reference.iter().filter(|c| produced.contains(c.as_str())).count();

        // The same sum against a list collin did not produce. Roots come out of
        // the denominator: a column built from a literal has no parent, so
        // counting it as a miss would understate coverage and send a reader
        // looking for something the SQL says is not there.
        let roots: HashSet<&str> = resolved.roots.iter().map(String::as_str).collect();
        if let Some(independent) = store.warehouse(&model.uid).or_else(|| store.declared(&model.uid))
        {
            let judged = independent.iter().filter(|c| !roots.contains(c.as_str()));
            rep.totals.columns_total_independent += judged.clone().count();
            rep.totals.columns_covered_independent +=
                judged.filter(|c| produced.contains(c.as_str())).count();
            rep.totals.columns_root +=
                independent.iter().filter(|c| roots.contains(c.as_str())).count();
        }

        let missing_columns = match store.warehouse(&model.uid) {
            Some(w) => report::missing(w, &resolved.outputs),
            None => Vec::new(),
        };
        let unexpected_columns = match store.warehouse(&model.uid) {
            Some(w) => report::missing(&resolved.outputs, w),
            None => Vec::new(),
        };
        if plan == Plan::PerColumn {
            rep.totals.per_column += 1;
            rep.totals.columns_withheld += unexpected_columns.len();
        }

        let provenance = published.provenance();

        // Only a trusted compile's: one set aside is listed for that already, and
        // its edges come from inference, so what its SQL lost is not what the
        // cache lacks.
        let lost = match plan {
            Plan::Parsed => lost_columns(&resolved, &parents, &published.bridged),
            // The columns both have. The table's others went out by name or not
            // at all, and the compile's others on no rung.
            Plan::PerColumn => lost_columns(&resolved, &parents, &published.bridged)
                .into_iter()
                .filter(|l| shared.contains(&l.column))
                .collect(),
            Plan::Inferred => Vec::new(),
        };
        rep.totals.columns_lost += lost.len();
        let unbacked = unbacked_report(&resolved);
        rep.totals.unbacked_reads += unbacked.len();
        if !resolved.merged_scopes.is_empty() {
            rep.totals.scope_merged += 1;
        }

        // A compile that accounts for a small part of what the project documents,
        // with no catalog to say which side is wrong. Without this the model is
        // absent from the report, and absence there means clean.
        let thin = thin_finding(&store, &model.uid);
        if thin.is_some() {
            rep.totals.thin += 1;
        }

        // A column read to decide which rows exist goes in its own section, not
        // in the fault list: it is lineage to be had rather than a thing to fix,
        // and three quarters of the models here read one, which would have made
        // the fault list unreadable.
        let unplaced = resolved.unplaced_qualify + resolved.unplaced_join + resolved.unplaced_filter;
        if plan != Plan::Inferred && (!resolved.indirect.is_empty() || unplaced > 0) {
            let reads = row_reads(&resolved, &norm_relation(&model.relation));
            rep.totals.row_deciding_reads += reads.len();
            rep.totals.read_not_projected += reads.iter().filter(|r| !r.projected).count();
            rep.totals.qualify_unplaced += resolved.unplaced_qualify;
            rep.totals.join_unplaced += resolved.unplaced_join;
            rep.totals.filter_unplaced += resolved.unplaced_filter;
            rep.row_deciding_reads.push(ModelReads {
                name: model.name.clone(),
                unique_id: model.uid.clone(),
                columns: reads,
                unplaced_qualify: resolved.unplaced_qualify,
                unplaced_join: resolved.unplaced_join,
                unplaced_filter: resolved.unplaced_filter,
            });
        }

        let (gap, self_read) = engine_gap(&resolved, &own);
        if self_read {
            rep.totals.self_reads += 1;
        }

        if !undeclared_relations.is_empty() {
            rep.totals.undeclared += 1;
        }

        // A column read through a star over a parent whose list lacks it, once
        // it has its edge, is the same finding as the direct read below, and is
        // filed the same way (0030). The parent's side explains it or nothing
        // does, as for a direct read, but a read through a star raises no issue
        // on the child, so it lists the child only through its lost columns.
        for s in through_star.iter().filter(|s| s.published) {
            let parent = node_for(project, &claimants(project, model), &s.relation);
            if !project.nodes.get(&parent).is_some_and(Node::is_model) {
                continue;
            }
            rep.totals.read_through_star += 1;
            let against = parents.iter().find(|p| p.node.uid == parent).map_or("unknown", |p| p.provenance.as_str());
            let in_compile = outputs_of.get(&parent).is_some_and(|o| o.contains(&s.column));
            let entry = downstream.entry(parent).or_default().entry(s.column.clone()).or_insert_with(|| {
                ReadDownstream { column: s.column.clone(), against, in_compile, read_by: Vec::new() }
            });
            if !entry.read_by.contains(&model.uid) {
                entry.read_by.push(model.uid.clone());
            }
        }
        rep.totals.read_through_star_unwitnessed += through_star.iter().filter(|s| s.witness == "none").count();

        // A column read from a parent whose list lacks it. Filed on the parent when
        // the child's compile is not set aside, the read drew an edge, and the parent
        // is a model; it leaves the child when the parent's side explains it: the
        // list was the parent's own short compile, or the parent's compile does
        // have the column and the list was an older table or the YAML. Otherwise
        // nothing but this child says the column exists, and it stays here. So
        // it does when that child is a compiled file and the parent's SQL is the
        // manifest's: the file can be older than the parent, and then reads what
        // the parent no longer has, so its read alone indicts nothing (0033).
        let mut unexplained = false;
        if plan != Plan::Inferred {
            let read = claimants(project, model);
            for u in &resolved.unknown_columns {
                if u.relation == own {
                    continue;
                }
                let parent = node_for(project, &read, &u.relation);
                let backed = resolved
                    .edges
                    .iter()
                    .any(|e| e.from_relation == u.relation && e.from_column == u.column);
                if u.relation.is_empty() || !backed || !project.nodes.get(&parent).is_some_and(Node::is_model) {
                    unexplained = true;
                    continue;
                }
                let against = parents
                    .iter()
                    .find(|p| p.node.uid == parent)
                    .map_or("unknown", |p| p.provenance.as_str());
                let in_compile = outputs_of.get(&parent).is_some_and(|o| o.contains(&u.column));
                let parent_compiled = project.nodes.get(&parent).is_some_and(|p| p.sql_source == SqlSource::Manifest);
                if model.sql_source.file().is_some() && parent_compiled && !in_compile {
                    unexplained = true;
                    continue;
                }
                let entry = downstream.entry(parent).or_default().entry(u.column.clone()).or_insert_with(|| {
                    ReadDownstream { column: u.column.clone(), against, in_compile, read_by: Vec::new() }
                });
                if !entry.read_by.contains(&model.uid) {
                    entry.read_by.push(model.uid.clone());
                }
                if !(against == "computed" || in_compile) {
                    unexplained = true;
                }
            }
        }

        let clean = provenance == "parsed"
            && unknown_relations.is_empty()
            && undeclared_relations.is_empty()
            && missing_columns.is_empty()
            && unexpected_columns.is_empty()
            && thin.is_none()
            && !stale
            && !gap
            && lost.is_empty()
            && unbacked.is_empty()
            && !unexplained;
        {
            let entry = ModelReport {
                name: model.name.clone(),
                unique_id: model.uid.clone(),
                file: model.file.clone(),
                sql_file: model.sql_source.file().map(str::to_string),
                sql_file_set_aside: foreign,
                provenance,
                agreement: agreement.as_str(),
                parse_error: resolved.parse_error.clone(),
                issues: resolved
                    .issues
                    .iter()
                    .map(|i| ReportIssue {
                        code: i.code.clone(),
                        message: i.message.clone(),
                        severity: i.severity,
                        span: i.span.map(|(a, b)| [a, b]),
                        self_reference: i.relation.as_deref() == Some(own.as_str()),
                    })
                    .collect(),
                unknown_relations,
                undeclared_relations,
                missing_columns,
                unexpected_columns,
                yaml_stale: stale,
                thin,
                lost_columns: lost,
                unbacked,
                edges_inferred: match plan {
                    Plan::Parsed | Plan::PerColumn => published.inferred,
                    Plan::Inferred => 0,
                },
                merged_scopes: resolved
                    .merged_scopes
                    .iter()
                    .map(|m| MergedScopeReport { kind: m.kind, name: m.name.clone(), count: m.count })
                    .collect(),
                read_downstream_but_absent: Vec::new(),
                ambiguous_inferred: published
                    .ambiguous
                    .iter()
                    .map(|a| AmbiguousInferred {
                        column: a.column.clone(),
                        candidates: a.candidates.clone(),
                        chosen: a.chosen.clone(),
                        by: a.by,
                    })
                    .collect(),
                edges: published.direct,
            };
            if clean {
                quiet.insert(model.uid.clone(), entry);
            } else {
                rep.models.push(entry);
            }
        }

        edges.extend(published.edges);
    }

    for (parent, reads) in downstream {
        let at = match rep.models.iter().position(|m| m.unique_id == parent) {
            Some(at) => at,
            None => match quiet.remove(&parent) {
                Some(entry) => {
                    rep.models.push(entry);
                    rep.models.len() - 1
                }
                None => continue,
            },
        };
        rep.totals.read_downstream_but_absent += reads.len();
        rep.models[at].read_downstream_but_absent = reads
            .into_values()
            .map(|mut r| {
                r.read_by.sort();
                r
            })
            .collect();
    }

    if rep.totals.without_compiled_code > 0 {
        rep.compiled_files = opts.target_dir().join("compiled").display().to_string();
    }
    rep.models.sort_by(|a, b| a.name.cmp(&b.name));
    rep.row_deciding_reads.sort_by(|a, b| a.name.cmp(&b.name));
    Run { edges, report: rep }
}

/// A name a compile reads through a `select *` over one relation whose list
/// lacks it, and what the pass made of it.
struct StarRead {
    /// Normalised, as the engine gives a relation.
    relation: String,
    column: String,
    /// The list of the relation's node that has the column: `warehouse`,
    /// `computed` for its own compile, set aside or not, or `declared`; `none`
    /// when no list has it.
    witness: &'static str,
    /// Whether an edge out of it went into the reading.
    published: bool,
}

/// Reads of a column through a `select *` over a relation whose list lacks it.
///
/// The engine expands the star from the list it was handed, so a later read of
/// the missing name is a phantom of the CTE, and raises no issue, where the
/// same read written straight off the relation keeps its edge. The SQL alone
/// says where the name comes from when the star is over exactly one relation,
/// but not that the relation has it: another list of that relation's node, its
/// table, its own compile even set aside, or its YAML, must name the column.
/// The list only vetoes; it never says where a name comes from.
///
/// Then the statement is resolved again, those columns added to that relation
/// for this one call, and the edges out of them are kept if nothing else of the
/// reading moved: the same outputs, and every edge it had. They go out parsed,
/// read by the same engine from the same SQL. The outputs they reach are no
/// longer lost or roots. The store never hears of the columns, so they define
/// nothing downstream (0030).
fn read_through_star(
    project: &Project,
    store: &Store,
    outputs_of: &HashMap<String, Vec<String>>,
    model: &Node,
    visible: &[Visible],
    resolved: &mut engine::Resolved,
) -> Vec<StarRead> {
    let mut found: Vec<StarRead> = Vec::new();
    for lost in &resolved.lost {
        for end in &lost.ends {
            let engine::DeadEnd::Phantom { column, reads, star: true, .. } = end else { continue };
            let [relation] = reads.as_slice() else { continue };
            if !found.iter().any(|f| f.relation == *relation && f.column == *column) {
                found.push(StarRead { relation: relation.clone(), column: column.clone(), witness: "none", published: false });
            }
        }
    }
    if found.is_empty() {
        return found;
    }
    let read = claimants(project, model);
    let has = |list: Option<&Vec<String>>, column: &str| list.is_some_and(|l| l.iter().any(|c| c == column));
    for f in &mut found {
        let uid = node_for(project, &read, &f.relation);
        f.witness = if has(store.warehouse(&uid), &f.column) {
            "warehouse"
        } else if has(outputs_of.get(&uid), &f.column) {
            "computed"
        } else if has(store.declared(&uid), &f.column) {
            "declared"
        } else {
            "none"
        };
    }
    let mut extended: Vec<Visible> =
        visible.iter().map(|v| Visible { relation: v.relation.clone(), columns: v.columns.clone() }).collect();
    let mut added: HashSet<(&str, &str)> = HashSet::new();
    for f in found.iter().filter(|f| f.witness != "none") {
        if let Some(v) = extended.iter_mut().find(|v| norm_relation(&v.relation) == f.relation) {
            v.columns.push(f.column.clone());
            added.insert((f.relation.as_str(), f.column.as_str()));
        }
    }
    if added.is_empty() {
        return found;
    }
    let again = engine::resolve(&model.sql, &project.adapter, &extended);
    let key = |e: &engine::RawEdge| (e.from_relation.clone(), e.from_column.clone(), e.to_column.clone());
    let had: HashSet<(String, String, String)> = resolved.edges.iter().map(key).collect();
    let now: HashSet<(String, String, String)> = again.edges.iter().map(key).collect();
    if again.outputs != resolved.outputs || !had.is_subset(&now) {
        return found;
    }
    let new: Vec<engine::RawEdge> = again
        .edges
        .into_iter()
        .filter(|e| added.contains(&(e.from_relation.as_str(), e.from_column.as_str())) && !had.contains(&key(e)))
        .collect();
    for f in &mut found {
        f.published = new.iter().any(|e| e.from_relation == f.relation && e.from_column == f.column);
    }
    let fed: HashSet<String> = new.iter().map(|e| e.to_column.clone()).collect();
    resolved.roots.retain(|r| !fed.contains(r));
    resolved.lost.retain(|l| !fed.contains(&l.column));
    resolved.edges.extend(new);
    found
}

/// A model's unbacked reads as the report lists them: each name read from a
/// CTE once, whatever outputs it reached.
fn unbacked_report(resolved: &engine::Resolved) -> Vec<UnbackedReport> {
    let mut out: Vec<UnbackedReport> = Vec::new();
    for u in &resolved.unbacked {
        if !out.iter().any(|r| r.cte == u.cte && r.column == u.column) {
            out.push(UnbackedReport { cte: u.cte.clone(), column: u.column.clone(), owners: u.owners.clone() });
        }
    }
    out.sort_by(|a, b| (&a.cte, &a.column).cmp(&(&b.cte, &b.column)));
    out
}

/// A trusted compile's lost columns, as the report names them. Each relation a
/// phantom reads is labelled with where the column list the engine was handed
/// for it came from: that is the pass's knowledge, not the engine's, and a list
/// short of the column is the usual cause.
fn lost_columns(
    resolved: &engine::Resolved,
    parents: &[Parent],
    bridged: &HashSet<String>,
) -> Vec<LostColumn> {
    let from: HashMap<String, &'static str> =
        parents.iter().map(|p| (norm_relation(&p.node.relation), p.provenance.as_str())).collect();
    let end = |d: &engine::DeadEnd| match d {
        engine::DeadEnd::Phantom { via, reads, .. } => LostEnd {
            reason: "phantom",
            via: Some(via.clone()),
            relations: reads
                .iter()
                .map(|r| LostRelation {
                    relation: r.clone(),
                    columns_from: from.get(r).copied().unwrap_or("unknown"),
                })
                .collect(),
            names: Vec::new(),
        },
        engine::DeadEnd::NamesUnreached { names } => LostEnd {
            reason: "names_unreached",
            via: None,
            relations: Vec::new(),
            names: names.clone(),
        },
        engine::DeadEnd::Unresolved => {
            LostEnd { reason: "unresolved", via: None, relations: Vec::new(), names: Vec::new() }
        }
    };
    resolved
        .lost
        .iter()
        // An unbacked read already speaks for it, with the edge it bridged.
        .filter(|l| !bridged.contains(&l.column))
        .map(|l| LostColumn { column: l.column.clone(), ends: l.ends.iter().map(end).collect() })
        .collect()
}

/// Which rung of the ladder a model's edges come from.
///
/// Decided once, because four things follow from it and must agree: whether
/// the computed list survives for downstream `select *`, which edges go out,
/// the provenance the report gives, and whether the model's reads are listed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Plan {
    /// The compile describes the model, and its edges go out as `parsed`.
    Parsed,
    /// The warehouse contradicts the compile, and the compile gives no reason
    /// of its own to doubt it: it parsed, spoke, read no name its CTEs lack and
    /// kept its scopes apart. A table built from older code than the SQL looks
    /// exactly so. The columns both have go out as `parsed`, the table's others
    /// as `inferred`, and the ones only the compile has on no rung, since
    /// nothing says the object has them (0029).
    PerColumn,
    /// It does not. Name matches go out as `inferred`, or nothing does when
    /// inference is off.
    Inferred,
}

impl Plan {
    pub fn as_str(self) -> &'static str {
        match self {
            Plan::Parsed => "parsed",
            Plan::PerColumn => "per_column",
            Plan::Inferred => "inferred",
        }
    }
}

/// The ladder, for one model. `parents_known` says whether any relation it
/// reads came with a column list, which is what makes silence a symptom,
/// `shared` whether its table has any column the compile produces, and
/// `foreign` whether its SQL is a compiled file compiled against another graph
/// than the manifest's.
fn plan(resolved: &engine::Resolved, agreement: Agreement, parents_known: bool, shared: bool, foreign: bool) -> Plan {
    // Nor may the edges of a statement the engine read two scopes of as one go
    // out: they cross the two, and nothing in them says which (0018). Two CTEs
    // of one name in nested `WITH` blocks still merge; two derived tables of
    // one name no longer do since the fork keys them by occurrence (0028), and
    // are only named.
    let sound = resolved.parse_error.is_none()
        && !silent(resolved, parents_known)
        && !resolved.merged_scopes.iter().any(|m| m.kind == "cte")
        && !foreign;
    match agreement {
        _ if !sound => Plan::Inferred,
        // A degraded compile has been shown not to describe the object that
        // exists, so its edges may not go out as `parsed` wholesale. Where the
        // table has the column too, both say it exists and the SQL is the one
        // account of it (0029). A read the SQL cannot back says the compile
        // cannot run, and a table sharing no column with it leaves nothing to
        // keep.
        Agreement::Degraded if shared && resolved.unbacked.is_empty() => Plan::PerColumn,
        Agreement::Degraded => Plan::Inferred,
        Agreement::Confirmed | Agreement::Unchecked => Plan::Parsed,
    }
}

/// A model that parsed but yielded no edge at all was read, not understood:
/// the compiled SQL lost its column list somewhere the parser cannot see,
/// typically an introspecting macro that returned nothing. Having known
/// parents and still nothing to say is the tell.
fn silent(resolved: &engine::Resolved, parents_known: bool) -> bool {
    resolved.edges.is_empty() && parents_known
}

/// Whether the engine admitted a gap that counts against the model, and whether
/// it said it could not find the model's own relation. `own` is normalised, as
/// `engine::Issue::relation` is.
///
/// dbt compiles an incremental model's filter as a read of the model itself,
/// and a model is never its own parent, so the engine is never handed that
/// relation and says so. That admission is set aside, and only that one about
/// that relation: what a model reads of itself decides which rows it adds, and
/// the cache drops a self edge anyway, so there is nothing to fix.
///
/// Unless some output column is fed by the model's own relation and by nothing
/// else. The read is then no filter: it is a column carried over from the
/// model's earlier rows, whose one edge the cache drops, and with no catalog to
/// contradict the compile the admission is the only sign of it.
fn engine_gap(resolved: &engine::Resolved, own: &str) -> (bool, bool) {
    let is_self = |i: &engine::Issue| i.relation.as_deref() == Some(own);
    let self_read = resolved.issues.iter().any(is_self);
    let mut only_own: HashMap<&str, bool> = HashMap::new();
    for e in &resolved.edges {
        *only_own.entry(e.to_column.as_str()).or_insert(true) &= e.from_relation == own;
    }
    let reaches_output = only_own.values().any(|&o| o);
    let gap = resolved.issues.iter().any(|i| i.degrading && (reaches_output || !is_self(i)));
    (gap, self_read)
}

/// The nodes dbt says a model reads: its parents, and in place of an ephemeral
/// parent that parent's own. dbt inlines an ephemeral model into the SQL of its
/// child, so the tables the ephemeral reads are read there under their own
/// names, and dbt was told of every one of them.
///
/// In manifest order, an ephemeral's parents taking its place in the list, and
/// each node once. Inference takes the first parent carrying a name, so the
/// order is part of the answer and must not move with the walk.
fn read_nodes<'a>(project: &'a Project, model: &'a Node) -> Vec<&'a Node> {
    fn walk<'a>(
        project: &'a Project,
        uids: &'a [String],
        seen: &mut HashSet<&'a str>,
        out: &mut Vec<&'a Node>,
    ) {
        for uid in uids {
            if !seen.insert(uid.as_str()) {
                continue;
            }
            let Some(node) = project.nodes.get(uid) else { continue };
            if node.is_ephemeral() {
                walk(project, &node.parents, seen, out);
            } else if !node.relation.is_empty() {
                out.push(node);
            }
        }
    }
    let mut out: Vec<&Node> = Vec::new();
    walk(project, &model.parents, &mut HashSet::new(), &mut out);
    out
}

/// The relations dbt says a model reads, normalised. See `read_nodes`.
fn dependencies(project: &Project, model: &Node) -> Vec<String> {
    read_nodes(project, model).iter().map(|n| norm_relation(&n.relation)).collect()
}

/// Each relation a model reads, with the node the model depends on for it, or
/// `None` when it depends on two nodes claiming the one relation.
///
/// Across the project a contested relation goes to the lowest unique_id, which
/// is the only choice that does not depend on the model. For one model there is
/// a better witness: dbt records which of the claimants it was built from, and
/// an edge pointing at the other one names a node the model never read. Only a
/// model that depends on both, or on neither, falls back to the name.
fn claimants(project: &Project, model: &Node) -> HashMap<String, Option<String>> {
    let mut out: HashMap<String, Option<String>> = HashMap::new();
    for node in read_nodes(project, model) {
        let held = out.entry(norm_relation(&node.relation)).or_insert_with(|| Some(node.uid.clone()));
        if held.as_deref() != Some(node.uid.as_str()) {
            *held = None;
        }
    }
    out
}

/// What the SQL reads that dbt was never told of: `reads` less the model's own
/// relation and its `dependencies`, all normalised.
///
/// The model's own relation is its incremental self read, which 0012 already
/// labels. Compared by relation and never by the node `node_for` would name: a
/// relation two nodes claim goes to one of them, not necessarily the parent,
/// and the parent's read would then pass for one nobody declared.
fn undeclared(own: &str, dependencies: &[String], reads: &[String]) -> Vec<String> {
    reads.iter().filter(|r| r.as_str() != own && !dependencies.contains(r)).cloned().collect()
}

/// What one model puts in the cache.
struct Published {
    plan: Plan,
    /// The direct edges, then the indirect fan-out when the run asked for it.
    edges: Vec<cache::Edge>,
    /// Where the direct edges end in `edges`.
    direct: usize,
    /// How many of the direct edges are inferred: all of them for a compile
    /// set aside, and the ones `unbacked_edges` bridged for a trusted one.
    inferred: usize,
    /// The output columns an unbacked read gave an inferred edge.
    bridged: HashSet<String>,
    /// Inferred columns several parents could have given.
    ambiguous: Vec<Ambiguity>,
}

impl Published {
    fn provenance(&self) -> &'static str {
        match self.plan {
            Plan::Parsed => "parsed",
            Plan::PerColumn => "per_column",
            Plan::Inferred if self.direct > 0 => "inferred",
            // Inference was off, or no name was known on both sides.
            Plan::Inferred => "unresolved",
        }
    }
}

fn publish(
    project: &Project,
    store: &Store,
    model: &Node,
    resolved: &engine::Resolved,
    plan: Plan,
    shared: &[String],
    opts: &Options,
) -> Published {
    let read = claimants(project, model);
    let mut ambiguous = Vec::new();
    let (mut edges, inferred) = match plan {
        Plan::Parsed => {
            let mut edges = parsed_edges(project, &read, model, resolved);
            let bridged = if opts.infer { unbacked_edges(project, &read, model, resolved, &edges) } else { Vec::new() };
            let n = bridged.len();
            edges.extend(bridged);
            (edges, n)
        }
        Plan::PerColumn => {
            let mut edges = parsed_edges(project, &read, model, resolved);
            edges.retain(|e| shared.contains(&e.to_col));
            let rest: Vec<String> = store
                .warehouse(&model.uid)
                .into_iter()
                .flatten()
                .filter(|c| !shared.contains(c))
                .cloned()
                .collect();
            let (inferred, found) =
                if opts.infer { infer_edges(project, store, model, resolved, &rest) } else { Default::default() };
            ambiguous = found;
            let n = inferred.len();
            edges.extend(inferred);
            (edges, n)
        }
        Plan::Inferred if opts.infer => {
            // Not `visible`, which prefers the computed list: that list is the
            // one just rejected, so inferring against it would inherit the
            // failure it replaces. The warehouse if there is one, the YAML
            // otherwise.
            let outputs = store.warehouse(&model.uid).or_else(|| store.declared(&model.uid));
            let (edges, found) = match outputs {
                Some(outputs) => infer_edges(project, store, model, resolved, outputs),
                None => Default::default(),
            };
            ambiguous = found;
            let n = edges.len();
            (edges, n)
        }
        Plan::Inferred => (Vec::new(), 0),
    };
    let direct = edges.len();
    let bridged: HashSet<String> = match plan {
        Plan::Parsed => edges[direct - inferred..direct].iter().map(|e| e.to_col.clone()).collect(),
        Plan::PerColumn | Plan::Inferred => HashSet::new(),
    };
    // Only where the SQL is trusted, and onto the columns it is trusted for. If
    // the compile is not describing this model, what it reads is no more
    // believable than what it writes.
    if plan != Plan::Inferred && opts.indirect {
        let outputs: Vec<String> = match plan {
            Plan::PerColumn => shared.to_vec(),
            _ => resolved.outputs.clone(),
        };
        let fanned = fan_out(project, &read, model, &resolved.indirect, &outputs, &edges);
        edges.extend(fanned);
    }
    Published { plan, edges, direct, inferred, bridged, ambiguous }
}

/// Edges for the reads a trusted compile makes of names its own CTE does not
/// project, where exactly one relation under that CTE has the name.
///
/// That relation's column is the one the SQL was written to read, but no SQL
/// that runs says so, and nothing says what the expression did with it: the
/// edge is inferred and claims no role. With no relation having the name, or
/// several, nothing is chosen, and the report lists the read. `parsed` is the
/// model's parsed edges, which an inferred one never doubles.
fn unbacked_edges(
    project: &Project,
    read: &HashMap<String, Option<String>>,
    model: &Node,
    resolved: &engine::Resolved,
    parsed: &[cache::Edge],
) -> Vec<cache::Edge> {
    let own = norm_relation(&model.relation);
    let have: HashSet<(&str, &str, &str)> =
        parsed.iter().map(|e| (e.from.as_str(), e.from_col.as_str(), e.to_col.as_str())).collect();
    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    let mut out = Vec::new();
    for u in &resolved.unbacked {
        let [owner] = u.owners.as_slice() else { continue };
        if *owner == own {
            continue;
        }
        let from = node_for(project, read, owner);
        if from == model.uid || have.contains(&(from.as_str(), u.column.as_str(), u.output.as_str())) {
            continue;
        }
        if seen.insert((from.clone(), u.column.clone(), u.output.clone())) {
            out.push(cache::Edge {
                from,
                from_col: u.column.clone(),
                to: model.uid.clone(),
                to_col: u.output.clone(),
                kind: "inferred".into(),
            });
        }
    }
    out
}

/// The edges the SQL states, each with the role its expression shows. `read`
/// is `claimants` for the model.
fn parsed_edges(
    project: &Project,
    read: &HashMap<String, Option<String>>,
    model: &Node,
    resolved: &engine::Resolved,
) -> Vec<cache::Edge> {
    let own = norm_relation(&model.relation);
    let mut out = Vec::new();
    for e in &resolved.edges {
        let from = node_for(project, read, &e.from_relation);
        // A model never feeds its own column. dbt compiles an incremental
        // model's filter as a read of the model itself, and the engine reads
        // that as a data flow into the projection. Tested on the relation too,
        // because a relation another node also claims can be settled on that
        // node, and the self read would then pass for a parent's column.
        if from == model.uid || e.from_relation == own {
            continue;
        }
        let kind = role::classify(&e.expression, &e.from_column, &e.to_column);
        out.push(cache::Edge {
            from,
            from_col: e.from_column.clone(),
            to: model.uid.clone(),
            to_col: e.to_column.clone(),
            kind: kind.as_str().to_string(),
        });
    }
    out
}

/// A column read to decide which rows exist has, read that way, no output
/// column of its own to point at: it bears on all of them and on none in
/// particular. Fanning out to every output column is the same shape
/// OpenLineage gives an INDIRECT transformation, and the role is what stops it
/// being read as a value flowing through.
///
/// `direct` is the model's direct edges, which a fanned edge never doubles, and
/// `read` is `claimants` for the model.
///
/// A column read for two reasons, a `WHERE` and a `QUALIFY` say, still makes
/// one column pair per output column, and the cache holds one edge per pair
/// (0009). Which role that edge carries is a choice, so it is made here and not
/// in the engine: the one `indirect_precedence` puts first. The report still
/// lists every read (0013).
fn fan_out(
    project: &Project,
    read: &HashMap<String, Option<String>>,
    model: &Node,
    reads: &[engine::IndirectRead],
    outputs: &[String],
    direct: &[cache::Edge],
) -> Vec<cache::Edge> {
    let direct: HashSet<(&str, &str, &str)> =
        direct.iter().map(|e| (e.from.as_str(), e.from_col.as_str(), e.to_col.as_str())).collect();
    let own = norm_relation(&model.relation);
    // Keyed by what the edge will name rather than by the relation read, and
    // kept in the order first seen, so the cache comes out in the same order
    // from run to run.
    let mut columns: Vec<(String, &str, role::Role)> = Vec::new();
    let mut at: HashMap<(String, &str), usize> = HashMap::new();
    for r in reads {
        let from = node_for(project, read, &r.relation);
        if from == model.uid || r.relation == own {
            continue;
        }
        let key = (from.clone(), r.column.as_str());
        match at.get(&key) {
            Some(&i) => {
                let held = &mut columns[i].2;
                if r.role.indirect_precedence() > held.indirect_precedence() {
                    *held = r.role;
                }
            }
            None => {
                at.insert(key, columns.len());
                columns.push((from, r.column.as_str(), r.role));
            }
        }
    }
    let mut fanned: Vec<cache::Edge> = Vec::new();
    for (from, column, role) in &columns {
        for to_col in outputs {
            // The direct edge says more, so it stands rather than being
            // doubled by a weaker claim about the same pair.
            if direct.contains(&(from.as_str(), *column, to_col.as_str())) {
                continue;
            }
            fanned.push(cache::Edge {
                from: from.clone(),
                from_col: column.to_string(),
                to: model.uid.clone(),
                to_col: to_col.clone(),
                kind: role.as_str().to_string(),
            });
        }
    }
    fanned
}

/// A model's indirect reads, as the report lists them, less any of its own
/// relation: what a model reads of itself decides which rows it adds, and 0012
/// already says so.
fn row_reads(resolved: &engine::Resolved, own: &str) -> Vec<IndirectReport> {
    resolved
        .indirect
        .iter()
        .filter(|r| r.relation != own)
        .map(|r| IndirectReport {
            projected: resolved
                .edges
                .iter()
                .any(|e| e.from_relation == r.relation && e.from_column == r.column),
            relation: r.relation.clone(),
            column: r.column.clone(),
            role: r.role.as_str(),
        })
        .collect()
}

/// The compile accounts for less than half of what the YAML documents, and no
/// catalog exists to say which side is wrong.
///
/// Held to two conditions. There must be no warehouse entry, because when there
/// is one `Agreement` already answers this and answers it better. And the model
/// must document enough columns for the ratio to mean anything: on a four column
/// model, missing two is a rounding error rather than a finding.
///
/// Deliberately not an `Agreement` rung. A short compile is either a stale YAML
/// or a macro that read the warehouse at compile time and found its source
/// missing, and the two cases 0010 read by hand were the second: one a stage
/// whose SQL cannot run, now caught by its unbacked reads (0019), one a
/// satellite that runs and is short for the same reason one step earlier. The
/// edges such a compile gives are still the ones its SQL supports, so it is a
/// finding and not a verdict.
fn thin_finding(store: &Store, uid: &str) -> Option<Thin> {
    if store.warehouse(uid).is_some() {
        return None;
    }
    let (declared, computed, shared) = store.overlap(uid)?;
    if declared < THIN_FLOOR || shared * 2 >= declared {
        return None;
    }
    Some(Thin { declared, computed, shared })
}

/// Below this many documented columns a shortfall says nothing worth reporting.
const THIN_FLOOR: usize = 8;

/// How an inferred column with several candidate parents was settled.
pub struct Ambiguity {
    pub column: String,
    pub candidates: Vec<String>,
    pub chosen: Vec<String>,
    /// `compile`, `every` or `order`. See `infer_edges`.
    pub by: &'static str,
}

/// Name matching, used only when the SQL has been shown not to describe the
/// object that exists. Every edge it makes is labelled `inferred`, because a
/// name match is evidence of a link, not of what the link does.
///
/// A column several parents have is settled, and the settling reported, in one
/// of three ways. `compile`: the set aside compile still names, for this very
/// column, the parents it reads it from, and they are taken: the compile is not
/// trusted to describe the object, but it is the only witness to which of two
/// same named columns the model was written against. `every`: there is no
/// reading at all, a compile that did not parse, and every candidate is taken,
/// less any known only by its YAML when another is known better; a union of
/// its parents is exactly that. `order`: otherwise the first parent in the
/// manifest, as before.
///
/// `outputs` is the model's columns to find a parent for.
fn infer_edges(
    project: &Project,
    store: &Store,
    model: &Node,
    reading: &engine::Resolved,
    outputs: &[String],
) -> (Vec<cache::Edge>, Vec<Ambiguity>) {
    // What the model reads, an ephemeral replaced by what it reads. The SQL
    // never names an ephemeral, so an edge inferred for its child must not
    // either: it would draw a path the parsed edges of every sibling skip.
    let parents: Vec<(&Node, HashSet<&str>, Provenance)> = read_nodes(project, model)
        .into_iter()
        .map(|p| {
            let (cols, provenance) = store.visible(&p.uid);
            (p, cols.iter().map(String::as_str).collect(), provenance)
        })
        .collect();
    let read = claimants(project, model);
    let mut chosen: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut ambiguous = Vec::new();
    for col in outputs {
        let candidates: Vec<&(&Node, HashSet<&str>, Provenance)> =
            parents.iter().filter(|(_, cols, _)| cols.contains(col.as_str())).collect();
        let pick: Vec<&str> = match candidates.as_slice() {
            [] => continue,
            [only] => vec![only.0.uid.as_str()],
            _ => {
                let named: HashSet<String> = reading
                    .edges
                    .iter()
                    .filter(|e| e.to_column == *col && e.from_column == *col)
                    .map(|e| node_for(project, &read, &e.from_relation))
                    .collect();
                let by_compile: Vec<&str> =
                    candidates.iter().map(|c| c.0.uid.as_str()).filter(|u| named.contains(*u)).collect();
                let known_better = candidates.iter().any(|c| c.2 != Provenance::Declared);
                let (pick, by): (Vec<&str>, &'static str) = if !by_compile.is_empty() {
                    (by_compile, "compile")
                } else if reading.edges.is_empty() {
                    let every = candidates
                        .iter()
                        .filter(|c| !(known_better && c.2 == Provenance::Declared))
                        .map(|c| c.0.uid.as_str())
                        .collect();
                    (every, "every")
                } else {
                    (vec![candidates[0].0.uid.as_str()], "order")
                };
                ambiguous.push(Ambiguity {
                    column: col.clone(),
                    candidates: candidates.iter().map(|c| c.0.uid.clone()).collect(),
                    chosen: pick.iter().map(|u| u.to_string()).collect(),
                    by,
                });
                pick
            }
        };
        chosen.insert(col.as_str(), pick);
    }
    // In the order the pass always wrote them, parent by parent.
    let mut out = Vec::new();
    for (parent, _, _) in &parents {
        for col in outputs {
            if chosen.get(col.as_str()).is_some_and(|c| c.contains(&parent.uid.as_str())) {
                out.push(cache::Edge {
                    from: parent.uid.clone(),
                    from_col: col.clone(),
                    to: model.uid.clone(),
                    to_col: col.clone(),
                    kind: "inferred".into(),
                });
            }
        }
    }
    (out, ambiguous)
}

/// Where dbt writes a model's compiled SQL, relative to the target directory
/// and with `/` between its parts, or None when the manifest gives no path
/// inside that directory.
///
/// dbt-core writes `compiled/<package>/<original_file_path>`, and dbt Fusion
/// does the same for the root project. For an installed package Fusion writes
/// the file's path from the project root instead,
/// `compiled/<package>/dbt_packages/<package>/...`: a compile records each
/// layout in `compiled_path`, but a `parse` records none, so the layout is the
/// one of the dbt that wrote the manifest, and only that one. A file in the
/// other is a leftover of another tool's compile.
///
/// dbt writes `original_file_path` with the separator of the machine that
/// parsed, so the path is split on both. The manifest is not trusted, so every
/// part must be a plain name: a `..`, a root or a drive would read outside the
/// target. Never `run/`, where dbt wraps the select in the statement that
/// builds the table, a `create ... as` or a `merge` (0033).
fn compiled_path(node: &Node, root: &str, fusion: bool) -> Option<String> {
    let plain = |name: &str| !matches!(name, "" | "." | "..") && !name.contains(['/', '\\', ':']);
    if !plain(&node.package) || node.file.starts_with(['/', '\\']) {
        return None;
    }
    let mut parts = vec!["compiled", node.package.as_str()];
    if fusion && !root.is_empty() && node.package != root {
        parts.extend(["dbt_packages", node.package.as_str()]);
    }
    let before = parts.len();
    for part in node.file.split(['/', '\\']).filter(|p| !p.is_empty() && *p != ".") {
        if !plain(part) {
            return None;
        }
        parts.push(part);
    }
    (parts.len() > before).then(|| parts.join("/"))
}

/// dbt Fusion is dbt 2.0 and later, dbt-core 1.x and before.
fn fusion(dbt_version: &str) -> bool {
    dbt_version.split('.').next().and_then(|major| major.parse::<u32>().ok()).is_some_and(|major| major >= 2)
}

/// Gives each SQL model the manifest has no `compiled_code` for the file dbt
/// compiled for it, the only copy a `parse` leaves, or the reason it has none
/// (0033).
///
/// The manifest wins wherever it has `compiled_code`, even empty: it was
/// written with the graph the pass walks, and a file under `target/` may be
/// older than both, or compiled for another target. A Python model is never
/// read: its compiled file is Python.
fn read_compiled_files(project: &mut Project, target: &Path) {
    let fusion = fusion(&project.dbt_version);
    let root = project.project_name.clone();
    for node in project.nodes.values_mut() {
        if !node.is_model() || !node.is_sql() || node.sql_source != SqlSource::Missing || node.file.is_empty() {
            continue;
        }
        let Some(rel) = compiled_path(node, &root, fusion) else {
            node.sql_source = SqlSource::Unusable("the manifest gives it no path inside the target directory".into());
            continue;
        };
        let path = target.join(&rel);
        // Only a file: a FIFO would hang the run, and a device never ends.
        let read = match std::fs::metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => Err(e.to_string()),
            Ok(meta) if !meta.is_file() => Err("not a file".to_string()),
            Ok(_) => std::fs::read_to_string(&path).map_err(|e| e.to_string()),
        };
        node.sql_source = match read {
            Ok(sql) if sql.trim().is_empty() => SqlSource::Unusable("its compiled file is empty".into()),
            Ok(sql) => {
                node.sql = sql;
                SqlSource::File(rel)
            }
            Err(e) => SqlSource::Unusable(format!("its compiled file cannot be read: {e}")),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(uid: &str, rel: &str, parents: &[&str], declared: &[&str]) -> (String, Node) {
        (
            uid.to_string(),
            Node {
                uid: uid.into(),
                name: uid.into(),
                kind: "model".into(),
                package: uid.split('.').nth(1).unwrap_or_default().into(),
                file: String::new(),
                relation: rel.into(),
                materialized: "table".into(),
                language: "sql".into(),
                sql: String::new(),
                sql_source: SqlSource::Manifest,
                declared: declared.iter().map(|s| s.to_string()).collect(),
                parents: parents.iter().map(|s| s.to_string()).collect(),
            },
        )
    }

    fn project_of(nodes: Vec<(String, Node)>) -> Project {
        let nodes: HashMap<String, Node> = nodes.into_iter().collect();
        let by_relation = nodes
            .values()
            .map(|n| (norm_relation(&n.relation), n.uid.clone()))
            .collect();
        Project {
            nodes,
            by_relation,
            collisions: Vec::new(),
            dbt_version: String::new(),
            project_name: String::new(),
            adapter: "snowflake".into(),
        }
    }

    fn project() -> Project {
        project_of(vec![
            node("model.p.child", "db.sch.child", &["model.p.parent"], &[]),
            node("model.p.parent", "db.sch.parent", &[], &["a", "b"]),
        ])
    }

    fn cols(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn shape(edges: &[cache::Edge]) -> Vec<(&str, &str, &str, &str, &str)> {
        edges
            .iter()
            .map(|e| {
                let (from, to) = (e.from.as_str(), e.to.as_str());
                (from, e.from_col.as_str(), to, e.to_col.as_str(), e.kind.as_str())
            })
            .collect()
    }

    fn opts(infer: bool, indirect: bool) -> Options {
        let mut o = Options::new(PathBuf::new());
        o.infer = infer;
        o.indirect = indirect;
        o
    }

    fn raw(from: &str, col: &str) -> engine::RawEdge {
        engine::RawEdge {
            from_relation: from.into(),
            from_column: col.into(),
            to_column: col.into(),
            expression: String::new(),
        }
    }

    /// A source, a model whose table matches its compile, and a model whose
    /// table was built from older code than the compile describes, every column
    /// since renamed, so that the two share none and the compile is set aside
    /// whole.
    ///
    /// Both models filter, so a distrusted compile has reads on hand to be
    /// wrongly drawn or listed. `channel` comes from a literal, so the coverage
    /// sums have an output no direct edge reaches and an indirect one would.
    /// The YAML of `orders` lags its table, which puts a trusted model with
    /// indirect reads in the report's model list.
    fn shop() -> (Project, HashMap<String, Vec<String>>) {
        let mut raw = node("source.p.raw", "db.sch.raw", &[], &[]);
        raw.1.kind = "source".into();
        let mut orders =
            node("model.p.orders", "db.sch.orders", &["source.p.raw"], &["doubled", "id"]);
        orders.1.sql = "select id, amount * 2 as doubled, 'web' as channel from db.sch.raw \
                        where status = 'open'"
            .into();
        let mut behind = node("model.p.behind", "db.sch.behind", &["source.p.raw"], &[]);
        behind.1.sql =
            "select id as order_id, amount as total from db.sch.raw where status = 'void'".into();
        let warehouse = HashMap::from([
            ("source.p.raw".to_string(), cols(&["amount", "id", "status"])),
            ("model.p.orders".to_string(), cols(&["channel", "doubled", "id"])),
            ("model.p.behind".to_string(), cols(&["amount", "id"])),
        ]);
        (project_of(vec![raw, orders, behind]), warehouse)
    }

    #[test]
    fn a_known_relation_becomes_its_dbt_id() {
        let p = project();
        assert_eq!(node_for(&p, &HashMap::new(), "DB.SCH.PARENT"), "model.p.parent");
    }

    #[test]
    fn a_relation_in_backticks_is_read_from_its_dbt_node() {
        // dbt-bigquery quotes relation_name in backticks. Read as a name of its
        // own, every parent was a relation dbt was not told of, and every edge
        // left the graph through `rel:`.
        let mut p = project_of(vec![
            node("model.p.child", "`my-proj`.`ds`.`child`", &["model.p.parent"], &[]),
            node("model.p.parent", "`my-proj`.`ds`.`parent`", &[], &["a", "b"]),
        ]);
        p.adapter = "bigquery".into();
        p.nodes.get_mut("model.p.child").unwrap().sql = "select a, b as c from `my-proj`.`ds`.`parent`".into();
        let r = run(&p, HashMap::new(), &opts(true, false));
        let edges: Vec<_> = r.edges.iter().map(|e| (e.from.as_str(), e.from_col.as_str(), e.to_col.as_str())).collect();
        assert_eq!(edges, vec![("model.p.parent", "a", "a"), ("model.p.parent", "b", "c")]);
        assert_eq!(r.report.totals.undeclared, 0);
        assert!(r.report.models.iter().all(|m| m.unknown_relations.is_empty()));
    }

    #[test]
    fn an_unknown_relation_keeps_the_rel_escape_hatch() {
        let p = project();
        assert_eq!(node_for(&p, &HashMap::new(), "DB.SCH.OUTSIDE"), "rel:db.sch.outside");
    }

    #[test]
    fn inference_matches_names_and_labels_itself() {
        let p = project();
        let mut store = Store::new(
            HashMap::from([("model.p.parent".into(), vec!["a".into(), "b".into()])]),
            HashMap::from([("model.p.child".into(), vec!["a".into(), "zz".into()])]),
        );
        store.set_computed("model.p.parent", vec!["a".into(), "b".into()]);
        let child = &p.nodes["model.p.child"];
        let (edges, _) = infer_edges(&p, &store, child, &engine::Resolved::default(), &cols(&["a", "zz"]));
        assert_eq!(edges.len(), 1, "only the column both sides know");
        assert_eq!(edges[0].from_col, "a");
        assert_eq!(edges[0].kind, "inferred", "an inference must never look parsed");
    }

    #[test]
    fn inference_says_nothing_when_the_output_is_unknown() {
        let p = project();
        let store = Store::new(HashMap::new(), HashMap::new());
        let child = &p.nodes["model.p.child"];
        let none = engine::Resolved::default();
        let published = publish(&p, &store, child, &none, Plan::Inferred, &[], &opts(true, false));
        assert!(published.edges.is_empty());
    }

    /// A target directory of one test's own, gone when the test ends, passing
    /// or not.
    struct Target(PathBuf);

    impl Target {
        fn new(test: &str) -> Target {
            let dir = std::env::temp_dir().join(format!("collin-{test}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Target(dir)
        }
        fn write(&self, path: &str, content: impl AsRef<[u8]>) {
            let path = self.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
    }

    impl Drop for Target {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A model of `shop`, the root project, at `file`, with `sql` as the
    /// manifest's `compiled_code`, or none at all.
    fn at(name: &str, file: &str, sql: Option<&str>) -> (String, Node) {
        let mut n = node(&format!("model.shop.{name}"), &format!("db.sch.{name}"), &[], &[]);
        n.1.file = file.into();
        match sql {
            Some(sql) => n.1.sql = sql.into(),
            None => n.1.sql_source = SqlSource::Missing,
        }
        n
    }

    /// `nodes`, read against `target` as the manifest dbt `version` wrote for
    /// the project `shop` gives them.
    fn read_with(target: &Target, version: &str, nodes: Vec<(String, Node)>) -> Project {
        let mut p = project_of(nodes);
        p.project_name = "shop".into();
        p.dbt_version = version.into();
        read_compiled_files(&mut p, &target.0);
        p
    }

    #[test]
    fn a_model_without_compiled_code_reads_the_file_dbt_wrote_for_it() {
        let t = Target::new("reads");
        t.write("compiled/shop/models/orders.sql", "select id from raw.orders");
        t.write("compiled/shop/models/empty.sql", "select 3");
        t.write("run/shop/models/payments.sql", "create table x as (select 1)");
        // No dbt writes here: the file of a package named `models`, if anyone's.
        t.write("compiled/models/refunds.sql", "select 9");
        let p = read_with(
            &t,
            "1.11.8",
            vec![
                at("orders", "models/orders.sql", None),
                at("has_sql", "models/orders.sql", Some("select 2")),
                at("empty", "models/empty.sql", Some("")),
                at("payments", "models/payments.sql", None),
                at("refunds", "models/refunds.sql", None),
            ],
        );
        let got = |name: &str| {
            let n = &p.nodes[&format!("model.shop.{name}")];
            (n.sql.as_str(), n.sql_source.clone(), n.no_sql())
        };
        let file = SqlSource::File("compiled/shop/models/orders.sql".into());
        assert_eq!(got("orders"), ("select id from raw.orders", file, None));
        assert_eq!(got("has_sql"), ("select 2", SqlSource::Manifest, None), "the manifest wins");
        // An empty compile is the manifest's answer, and an older file is not.
        assert_eq!(got("empty"), ("", SqlSource::Manifest, Some("compiled_code is empty".into())));
        let missing = ("", SqlSource::Missing, Some("no compiled_code and no compiled file".into()));
        assert_eq!(got("payments"), missing, "never run/");
        assert_eq!(got("refunds"), missing, "never a path without the package");
    }

    #[test]
    fn a_path_written_on_windows_is_read_on_any_machine() {
        // dbt writes `original_file_path` with the separator of the machine
        // that parsed the project.
        let t = Target::new("windows");
        t.write("compiled/shop/models/staging/orders.sql", "select id from raw.orders");
        let p = read_with(&t, "2.0.0-preview.196", vec![at("orders", "models\\staging\\orders.sql", None)]);
        let n = &p.nodes["model.shop.orders"];
        assert_eq!(n.sql_source, SqlSource::File("compiled/shop/models/staging/orders.sql".into()));
        assert_eq!(n.sql, "select id from raw.orders");
    }

    #[test]
    fn an_installed_package_is_read_where_the_dbt_that_wrote_the_manifest_puts_it() {
        let t = Target::new("packages");
        t.write("compiled/util/dbt_packages/util/models/u.sql", "select 'fusion'");
        t.write("compiled/util/models/u.sql", "select 'core'");
        t.write("compiled/shop/models/a.sql", "select 'root'");
        let package = || {
            let mut u = node("model.util.u", "db.sch.u", &[], &[]);
            u.1.file = "models/u.sql".into();
            u.1.sql_source = SqlSource::Missing;
            u
        };
        let read = |version: &str| {
            let p = read_with(&t, version, vec![package(), at("a", "models/a.sql", None)]);
            (p.nodes["model.util.u"].sql.clone(), p.nodes["model.shop.a"].sql.clone())
        };
        // Each the layout that dbt's own compile records as `compiled_path`.
        assert_eq!(read("2.0.0-preview.196"), ("select 'fusion'".to_string(), "select 'root'".to_string()));
        assert_eq!(read("1.11.8"), ("select 'core'".to_string(), "select 'root'".to_string()));
    }

    #[test]
    fn a_path_from_the_manifest_never_leaves_the_target() {
        let t = Target::new("escape");
        // Where `..` twice from `compiled/shop/` would land.
        t.write("outside.sql", "select 'outside'");
        let mut climbing = at("climbing", "models/a.sql", None);
        climbing.1.package = "..".into();
        let p = read_with(
            &t,
            "1.11.8",
            vec![
                at("up", "../../outside.sql", None),
                at("root", "/etc/hosts", None),
                at("drive", "C:\\Windows\\win.ini", None),
                climbing,
            ],
        );
        for name in ["up", "root", "drive", "climbing"] {
            let n = &p.nodes[&format!("model.shop.{name}")];
            assert_eq!(n.sql, "", "{name}");
            let why = "no compiled_code, and the manifest gives it no path inside the target directory";
            assert_eq!(n.no_sql().as_deref(), Some(why), "{name}");
        }
    }

    #[test]
    fn a_compiled_file_that_cannot_be_used_says_why() {
        let t = Target::new("unusable");
        t.write("compiled/shop/models/blank.sql", "\n  \n");
        t.write("compiled/shop/models/latin.sql", [b's', b'e', 0xe9]);
        std::fs::create_dir_all(t.0.join("compiled/shop/models/dir.sql")).unwrap();
        let p = read_with(
            &t,
            "1.11.8",
            vec![
                at("blank", "models/blank.sql", None),
                at("latin", "models/latin.sql", None),
                at("dir", "models/dir.sql", None),
            ],
        );
        let why = |name: &str| p.nodes[&format!("model.shop.{name}")].no_sql().unwrap();
        assert_eq!(why("blank"), "no compiled_code, and its compiled file is empty");
        let unreadable = "no compiled_code, and its compiled file cannot be read: ";
        assert!(why("latin").starts_with(unreadable), "{}", why("latin"));
        assert_eq!(why("dir"), format!("{unreadable}not a file"));
        assert!(p.nodes.values().all(|n| n.sql_source.file().is_none()), "none of them was read");
    }

    #[test]
    fn a_python_model_is_never_handed_to_the_sql_engine() {
        let t = Target::new("python");
        t.write("compiled/shop/models/fresh.py", "def model(dbt, session):\n    return dbt.ref('a')\n");
        let mut fresh = at("fresh", "models/fresh.py", None);
        fresh.1.language = "python".into();
        let mut compiled = at("compiled", "models/compiled.py", Some("def model(dbt, session): ..."));
        compiled.1.language = "python".into();
        let p = read_with(&t, "1.11.8", vec![fresh, compiled]);
        for name in ["fresh", "compiled"] {
            let why = p.nodes[&format!("model.shop.{name}")].no_sql();
            assert_eq!(why.as_deref(), Some("a python model, not SQL"), "{name}");
        }
        assert_eq!(p.nodes["model.shop.fresh"].sql_source, SqlSource::Missing, "its compiled file is Python");
    }

    /// The SQL of `n` as the compiled file at `path`.
    fn from_file(n: &mut (String, Node), path: &str, sql: &str) {
        n.1.sql = sql.into();
        n.1.sql_source = SqlSource::File(path.into());
    }

    #[test]
    fn a_compiled_file_for_another_target_is_set_aside_and_one_reading_sources_is_kept() {
        // Both files were compiled for a target whose schema is `prod`, the
        // manifest for one whose schema is `dev`. A source is named alike under
        // both, so `stg` reads what the manifest gives it and `fct` does not.
        let mut raw = node("source.p.raw", "db.raw.orders", &[], &[]);
        raw.1.kind = "source".into();
        let mut stg = node("model.p.stg", "db.dev.stg", &["source.p.raw"], &[]);
        from_file(&mut stg, "compiled/p/models/stg.sql", "select id, amount from db.raw.orders");
        let mut fct = node("model.p.fct", "db.dev.fct", &["model.p.stg"], &["amount", "id"]);
        from_file(&mut fct, "compiled/p/models/fct.sql", "select id, amount from db.prod.stg");
        let warehouse = HashMap::from([("source.p.raw".to_string(), cols(&["amount", "id"]))]);
        let r = run(&project_of(vec![raw, stg, fct]), warehouse, &opts(true, false));

        let mut got = shape(&r.edges);
        got.sort();
        assert_eq!(
            got,
            vec![
                ("model.p.stg", "amount", "model.p.fct", "amount", "inferred"),
                ("model.p.stg", "id", "model.p.fct", "id", "inferred"),
                ("source.p.raw", "amount", "model.p.stg", "amount", "passthrough"),
                ("source.p.raw", "id", "model.p.stg", "id", "passthrough"),
            ],
            "no edge leaves the graph for a table of the other target"
        );
        let t = &r.report.totals;
        assert_eq!((t.without_compiled_code, t.sql_from_files, t.sql_from_files_set_aside), (2, 2, 1));
        let listed: Vec<&str> = r.report.models.iter().map(|m| m.unique_id.as_str()).collect();
        assert_eq!(listed, vec!["model.p.fct"]);
        let fct = &r.report.models[0];
        assert_eq!((fct.provenance, fct.sql_file_set_aside), ("inferred", true));
        assert_eq!(fct.sql_file.as_deref(), Some("compiled/p/models/fct.sql"));
        assert_eq!(fct.undeclared_relations, cols(&["DB.PROD.STG"]));
    }

    #[test]
    fn a_compiled_file_reading_what_a_compiled_parent_lacks_indicts_only_itself() {
        // `stg` renamed `amount` and the manifest has its compile; `fct` still
        // reads the old name. With the manifest's SQL that read is the parent's
        // finding (0020). A compiled file may be the older of the two, and its
        // read alone says nothing of the parent.
        let with = |source: SqlSource| {
            let mut raw = node("source.p.raw", "db.raw.orders", &[], &[]);
            raw.1.kind = "source".into();
            let mut stg = node("model.p.stg", "db.sch.stg", &["source.p.raw"], &[]);
            stg.1.sql = "select id, amount as amount_usd from db.raw.orders".into();
            let mut fct = node("model.p.fct", "db.sch.fct", &["model.p.stg"], &[]);
            fct.1.sql = "select s.id, s.amount from db.sch.stg as s".into();
            fct.1.sql_source = source;
            let warehouse = HashMap::from([("source.p.raw".to_string(), cols(&["amount", "id"]))]);
            run(&project_of(vec![raw, stg, fct]), warehouse, &opts(true, false))
        };
        let listed = |r: &Run| -> Vec<(String, Vec<String>)> {
            r.report
                .models
                .iter()
                .map(|m| (m.unique_id.clone(), m.read_downstream_but_absent.iter().map(|d| d.column.clone()).collect()))
                .collect()
        };
        let manifest = with(SqlSource::Manifest);
        assert_eq!(listed(&manifest), vec![("model.p.stg".to_string(), cols(&["amount"]))]);
        let file = with(SqlSource::File("compiled/p/models/fct.sql".into()));
        assert_eq!(listed(&file), vec![("model.p.fct".to_string(), Vec::new())]);
        assert_eq!(file.report.totals.read_downstream_but_absent, 0);
    }

    #[test]
    fn a_model_never_feeds_its_own_column() {
        let p = project();
        let resolved = engine::Resolved {
            edges: vec![raw("DB.SCH.CHILD", "a"), raw("DB.SCH.PARENT", "b")],
            ..Default::default()
        };
        let child = &p.nodes["model.p.child"];
        let edges = parsed_edges(&p, &claimants(&p, child), child, &resolved);
        let kept = vec![("model.p.parent", "b", "model.p.child", "b", "passthrough")];
        assert_eq!(shape(&edges), kept);
    }

    #[test]
    fn only_a_compile_that_parsed_and_was_not_contradicted_is_planned_as_parsed() {
        let spoke =
            engine::Resolved { edges: vec![raw("DB.SCH.PARENT", "a")], ..Default::default() };
        assert_eq!(plan(&spoke, Agreement::Confirmed, true, true, false), Plan::Parsed);
        assert_eq!(plan(&spoke, Agreement::Unchecked, true, false, false), Plan::Parsed);
        assert_eq!(plan(&spoke, Agreement::Degraded, true, false, false), Plan::Inferred);
        assert_eq!(plan(&spoke, Agreement::Degraded, true, true, false), Plan::PerColumn);
        let failed = engine::Resolved { parse_error: Some("x".into()), ..Default::default() };
        assert_eq!(plan(&failed, Agreement::Unchecked, false, false, false), Plan::Inferred);
        // A compiled file read against another graph, whatever else it agrees on.
        assert_eq!(plan(&spoke, Agreement::Confirmed, true, true, true), Plan::Inferred);
        // Its own reasons still set a contradicted compile aside whole.
        let merged = engine::Resolved {
            edges: vec![raw("DB.SCH.PARENT", "a")],
            merged_scopes: vec![engine::MergedScope { kind: "cte", name: "t".into(), count: 2 }],
            ..Default::default()
        };
        assert_eq!(plan(&merged, Agreement::Degraded, true, true, false), Plan::Inferred);
    }

    #[test]
    fn silence_counts_against_a_compile_only_when_it_had_something_to_read() {
        let silent = engine::Resolved::default();
        assert_eq!(plan(&silent, Agreement::Unchecked, true, false, false), Plan::Inferred);
        // A model built from literals reads nothing, so saying nothing about
        // its parents is the truth rather than a symptom.
        assert_eq!(plan(&silent, Agreement::Unchecked, false, false, false), Plan::Parsed);
    }

    /// The engine's view of `sql` as model `db.sch.m`, whose one parent is
    /// `db.sch.src`.
    fn as_m(sql: &str) -> engine::Resolved {
        let src = Visible { relation: "db.sch.src".into(), columns: cols(&["id", "ts"]) };
        engine::resolve(sql, "snowflake", &[src])
    }

    #[test]
    fn a_model_reading_itself_to_filter_is_no_gap() {
        let r = as_m("select id, ts from db.sch.src where ts > (select max(ts) from db.sch.m)");
        assert!(r.approximate, "the engine still says it; the report keeps the count");
        assert_eq!(engine_gap(&r, "DB.SCH.M"), (false, true));
        // Read as any other model, the same issue is a gap and no self read:
        // only the model's own relation is set aside.
        assert_eq!(engine_gap(&r, "DB.SCH.N"), (true, false));
    }

    #[test]
    fn an_anti_join_over_the_model_itself_is_no_gap() {
        // A NOT IN over the model itself reads it only to filter, and the
        // engine keeps what the subquery projects out of the outputs (0028).
        let r = as_m("select id, ts from db.sch.src where id not in (select id from db.sch.m)");
        let mut fed: Vec<(&str, &str)> = r
            .edges
            .iter()
            .map(|e| (e.from_relation.as_str(), e.to_column.as_str()))
            .collect();
        fed.sort();
        assert_eq!(fed, vec![("DB.SCH.SRC", "id"), ("DB.SCH.SRC", "ts")]);
        assert_eq!(engine_gap(&r, "DB.SCH.M"), (false, true));
    }

    #[test]
    fn a_self_read_does_not_excuse_another_table_the_engine_cannot_find() {
        let r = as_m(
            "select id, ts from db.sch.src where ts > (select max(ts) from db.sch.m) \
             and id in (select id from db.sch.other)",
        );
        assert_eq!(engine_gap(&r, "DB.SCH.M"), (true, true));
        let mut named: Vec<&str> = r.issues.iter().filter_map(|i| i.relation.as_deref()).collect();
        named.sort();
        assert_eq!(named, vec!["DB.SCH.M", "DB.SCH.OTHER"]);
    }

    #[test]
    fn a_self_read_that_reaches_an_output_column_is_still_a_gap() {
        // A column carried over from the model's earlier rows, whose only edge
        // is the one the cache drops.
        let carried = as_m(
            "select s.id, t.first_seen from db.sch.src s left join db.sch.m t on s.id = t.id",
        );
        assert_eq!(engine_gap(&carried, "DB.SCH.M"), (true, true));
    }

    #[test]
    fn a_star_over_the_model_itself_is_still_a_gap() {
        // The engine cannot expand it, and says so under another code.
        let r = as_m("select * from db.sch.src union all select * from db.sch.m");
        assert!(r.issues.iter().any(|i| i.code == "APPROXIMATE_LINEAGE"));
        assert!(engine_gap(&r, "DB.SCH.M").0);
    }

    #[test]
    fn a_model_reading_itself_is_counted_and_labelled_but_not_listed_for_it() {
        // `m` reads itself and nothing else is wrong with it. `n` reads itself
        // too, and its YAML lags its table, so it is listed for that.
        let mut src = node("source.p.src", "db.sch.src", &[], &[]);
        src.1.kind = "source".into();
        let mut m = node("model.p.m", "db.sch.m", &["source.p.src"], &[]);
        m.1.sql = "select id, ts from db.sch.src where ts > (select max(ts) from db.sch.m)".into();
        let mut n = node("model.p.n", "db.sch.n", &["source.p.src"], &["id"]);
        n.1.sql = "select s.id, s.ts from db.sch.src s \
                   left join db.sch.n t on s.id = t.id where t.id is null"
            .into();
        let warehouse = HashMap::from([
            ("source.p.src".to_string(), cols(&["id", "ts"])),
            ("model.p.m".to_string(), cols(&["id", "ts"])),
            ("model.p.n".to_string(), cols(&["id", "ts"])),
        ]);
        let r = run(&project_of(vec![src, m, n]), warehouse, &opts(true, false));

        assert_eq!(r.report.totals.self_reads, 2);
        let listed: Vec<&str> = r.report.models.iter().map(|m| m.unique_id.as_str()).collect();
        assert_eq!(listed, vec!["model.p.n"]);
        let n = &r.report.models[0];
        assert!(n.yaml_stale);
        let own: Vec<(&str, bool)> =
            n.issues.iter().map(|i| (i.code.as_str(), i.self_reference)).collect();
        assert_eq!(own, vec![("UNRESOLVED_REFERENCE", true)]);
        // What goes in the cache is exactly what it was.
        assert!(r.edges.iter().all(|e| e.from == "source.p.src"));
        assert_eq!(r.edges.len(), 4);
    }

    #[test]
    fn what_is_undeclared_is_what_is_read_less_the_model_and_its_dependencies() {
        let reads = cols(&["DB.SCH.CHILD", "DB.SCH.PARENT", "RAW_DB.DBO.ORDERS_BASE"]);
        let got = undeclared("DB.SCH.CHILD", &cols(&["DB.SCH.PARENT"]), &reads);
        assert_eq!(got, vec!["RAW_DB.DBO.ORDERS_BASE"]);
    }

    /// A source `orders` with its warehouse columns, an ephemeral model reading
    /// it, and whatever children are given.
    fn through_an_ephemeral(children: Vec<(String, Node)>) -> (Project, HashMap<String, Vec<String>>) {
        let mut src = node("source.p.raw.orders", "db.raw.orders", &[], &[]);
        src.1.kind = "source".into();
        let mut eph = node("model.p.eph", "", &["source.p.raw.orders"], &[]);
        eph.1.materialized = "ephemeral".into();
        eph.1.sql = "select * from db.raw.orders".into();
        let mut nodes = vec![src, eph];
        nodes.extend(children);
        let warehouse =
            HashMap::from([("source.p.raw.orders".to_string(), cols(&["amount", "id"]))]);
        (project_of(nodes), warehouse)
    }

    fn child_of_eph(uid: &str, sql: &str) -> (String, Node) {
        let mut c = node(uid, &format!("db.sch.{}", uid.rsplit('.').next().unwrap()), &["model.p.eph"], &[]);
        c.1.sql = sql.into();
        c
    }

    #[test]
    fn an_ephemeral_gives_its_place_to_what_it_reads_in_manifest_order() {
        let mut eph = node("model.p.eph", "", &["source.p.a", "source.p.b"], &[]);
        eph.1.materialized = "ephemeral".into();
        let p = project_of(vec![
            eph,
            node("source.p.a", "db.raw.a", &[], &[]),
            node("source.p.b", "db.raw.b", &[], &[]),
            node("source.p.c", "db.raw.c", &[], &[]),
            node("model.p.m", "db.sch.m", &["source.p.c", "model.p.eph", "source.p.b"], &[]),
        ]);
        let read: Vec<&str> =
            read_nodes(&p, &p.nodes["model.p.m"]).iter().map(|n| n.uid.as_str()).collect();
        assert_eq!(read, vec!["source.p.c", "source.p.a", "source.p.b"]);
    }

    #[test]
    fn a_star_through_an_ephemeral_expands_from_what_the_ephemeral_reads() {
        // dbt inlines the ephemeral as a CTE. Handed nothing for it, the engine
        // had no column list for the star to expand into, and the child came
        // out with no columns at all.
        let (p, warehouse) = through_an_ephemeral(vec![
            child_of_eph(
                "model.p.star",
                "with __dbt__cte__eph as (select * from db.raw.orders) select * from __dbt__cte__eph",
            ),
            child_of_eph(
                "model.p.listed",
                "with __dbt__cte__eph as (select id, amount from db.raw.orders) \
                 select id, amount from __dbt__cte__eph",
            ),
        ]);
        let r = run(&p, warehouse, &opts(true, false));
        let into = |to: &str| -> Vec<(&str, &str, &str, &str, &str)> {
            shape(&r.edges).into_iter().filter(|e| e.2 == to).collect()
        };
        let o = "source.p.raw.orders";
        for m in ["model.p.star", "model.p.listed"] {
            assert_eq!(
                into(m),
                vec![(o, "amount", m, "amount", "passthrough"), (o, "id", m, "id", "passthrough")],
                "the edges name the table the SQL reads, never the ephemeral"
            );
        }
    }

    #[test]
    fn a_broken_child_of_an_ephemeral_infers_from_what_the_ephemeral_reads() {
        let (p, mut warehouse) = through_an_ephemeral(vec![child_of_eph("model.p.broken", "select from")]);
        warehouse.insert("model.p.broken".into(), cols(&["amount", "id"]));
        let r = run(&p, warehouse, &opts(true, false));
        let (o, m) = ("source.p.raw.orders", "model.p.broken");
        let into: Vec<_> = shape(&r.edges).into_iter().filter(|e| e.2 == m).collect();
        assert_eq!(into, vec![(o, "amount", m, "amount", "inferred"), (o, "id", m, "id", "inferred")]);
    }

    #[test]
    fn two_ctes_of_one_name_set_a_compile_aside_and_two_derived_tables_do_not() {
        let source = |uid: &str, rel: &str, c: &[&str]| {
            let mut s = node(uid, rel, &[], &[]);
            s.1.kind = "source".into();
            (s, (uid.to_string(), cols(c)))
        };
        let (pay, pay_cols) = source("source.p.s.payments", "db.s.payments", &["acct_id", "seen_at"]);
        let (inv, inv_cols) = source("source.p.s.invoices", "db.s.invoices", &["acct_id", "seen_at"]);
        let parents = ["source.p.s.payments", "source.p.s.invoices"];
        let model = |uid: &str, rel: &str, sql: &str| {
            let mut m = node(uid, rel, &parents, &[]);
            m.1.sql = sql.into();
            m
        };
        let nested = "select p.seen_at as paid_seen_at, q.seen_at as open_seen_at \
                      from (with z as (select seen_at from db.s.payments) select seen_at from z) as p \
                      cross join (with z as (select seen_at from db.s.invoices) select seen_at from z) as q";
        let derived = "with paid as (select acct_id, seen_at from \
                       (select distinct acct_id, seen_at from db.s.payments) as t), \
                       open as (select acct_id, seen_at from \
                       (select distinct acct_id, seen_at from db.s.invoices) as t) \
                       select p.acct_id, p.seen_at as paid_seen_at, o.seen_at as open_seen_at \
                       from paid as p join open as o on p.acct_id = o.acct_id";
        let p = project_of(vec![
            pay,
            inv,
            model("model.p.nested", "db.sch.nested", nested),
            model("model.p.derived", "db.sch.derived", derived),
        ]);
        let r = run(&p, HashMap::from([pay_cols, inv_cols]), &opts(false, false));
        let into = |to: &str, col: &str| -> Vec<&str> {
            r.edges.iter().filter(|e| e.to == to && e.to_col == col).map(|e| e.from.as_str()).collect()
        };
        // The engine still reads two CTEs of one name as one, so that compile
        // is set aside, and with no inference in this run publishes nothing.
        assert!(r.edges.iter().all(|e| e.to != "model.p.nested"), "{:?}", shape(&r.edges));
        let nested = r.report.models.iter().find(|m| m.unique_id == "model.p.nested").expect("listed");
        assert_eq!(nested.merged_scopes, vec![MergedScopeReport { kind: "cte", name: "z".into(), count: 2 }]);
        // Two derived tables of one name it keys apart, so that one is read.
        assert_eq!(into("model.p.derived", "paid_seen_at"), vec!["source.p.s.payments"]);
        assert_eq!(into("model.p.derived", "open_seen_at"), vec!["source.p.s.invoices"]);
        assert_eq!(r.report.totals.scope_merged, 2, "both named");
    }

    /// A source `p`, a stage over it whose macro found `p` missing so its
    /// `d` CTE lists only a derived column, and a child reading the stage.
    fn stage_project(stage_declared: &[&str]) -> (Project, HashMap<String, Vec<String>>) {
        let mut src = node("source.p.s.p", "db.s.p", &[], &[]);
        src.1.kind = "source".into();
        let mut stage = node("model.p.stage", "db.sch.stage", &["source.p.s.p"], stage_declared);
        stage.1.sql = "with d as (select a, 'x' as rs from db.s.p), \
                       h as (select a, rs, sha1_binary(upper(trim(cast(b as varchar)))) as hk from d) \
                       select * from h"
            .into();
        let mut child = node("model.p.child", "db.sch.child", &["model.p.stage"], &[]);
        child.1.sql = "select s.b, s.hk from db.sch.stage as s".into();
        let warehouse = HashMap::from([("source.p.s.p".to_string(), cols(&["a", "b", "c"]))]);
        (project_of(vec![src, stage, child]), warehouse)
    }

    #[test]
    fn an_unbacked_read_with_one_owner_is_inferred_and_its_compile_list_retracted() {
        let (p, warehouse) = stage_project(&["a", "b", "hk", "rs"]);
        let r = run(&p, warehouse, &opts(true, false));
        let (src, stage) = ("source.p.s.p", "model.p.stage");
        let into = |to: &str| -> Vec<(&str, &str, &str, &str, &str)> {
            shape(&r.edges).into_iter().filter(|e| e.2 == to).collect()
        };
        assert_eq!(
            into(stage),
            vec![(src, "a", stage, "a", "passthrough"), (src, "b", stage, "hk", "inferred")],
            "the parsed edge stays parsed and the unbacked read is bridged, labelled"
        );
        // The compile's list is withdrawn, so the child sees the YAML, which
        // has `b`, where the compile's list did not.
        let child: Vec<_> = into("model.p.child");
        assert!(child.contains(&(stage, "b", "model.p.child", "b", "passthrough")), "{child:?}");
        let child_entry = r.report.models.iter().find(|m| m.unique_id == "model.p.child");
        assert!(
            child_entry.is_none_or(|m| m.issues.iter().all(|i| i.code != "UNKNOWN_COLUMN")),
            "read against the YAML, `b` is a column the stage has"
        );
        let entry = r.report.models.iter().find(|m| m.unique_id == stage).expect("listed");
        assert_eq!(entry.provenance, "parsed");
        assert_eq!(entry.edges_inferred, 1);
        assert_eq!(entry.unbacked.len(), 1);
        assert!(entry.lost_columns.is_empty(), "the bridged column is not also lost");
        assert_eq!(r.report.totals.edges_inferred, 1);
    }

    #[test]
    fn with_nothing_to_stand_in_the_compile_list_stays() {
        // No catalog entry and no YAML: withdrawing the list would leave the
        // child nothing, where the reads only say the SQL cannot run.
        let (p, warehouse) = stage_project(&[]);
        let r = run(&p, warehouse, &opts(true, false));
        let child: Vec<_> =
            shape(&r.edges).into_iter().filter(|e| e.2 == "model.p.child").map(|e| e.3).collect();
        assert!(child.contains(&"hk"), "{child:?}");
        let child_entry = r.report.models.iter().find(|m| m.unique_id == "model.p.child");
        assert!(
            child_entry.is_none_or(|m| m.unknown_relations.is_empty()),
            "the child is still handed the stage's columns"
        );
    }

    #[test]
    fn a_column_a_child_reads_and_its_parent_s_list_lacks_is_the_parent_s_finding() {
        let mut feed = node("source.p.raw.feed", "db.raw.feed", &[], &[]);
        feed.1.kind = "source".into();
        let model = |uid: &str, parents: &[&str], declared: &[&str], sql: &str| {
            let mut m = node(uid, &format!("db.sch.{}", uid.rsplit('.').next().unwrap()), parents, declared);
            m.1.sql = sql.into();
            m
        };
        let raw = ["source.p.raw.feed"];
        let p = project_of(vec![
            feed,
            // Its own compile is short.
            model("model.p.short", &raw, &[], "select id from db.raw.feed"),
            model("model.p.short_kid", &["model.p.short"], &[], "select s.id, s.amount from db.sch.short as s"),
            // Its dev table is older than its code.
            model("model.p.older", &raw, &[], "select id, amount from db.raw.feed"),
            model("model.p.older_kid", &["model.p.older"], &[], "select o.id, o.amount from db.sch.older as o"),
            // Nothing but the child says the column exists.
            model("model.p.documented", &raw, &["id"], "select from"),
            model("model.p.documented_kid", &["model.p.documented"], &[], "select d.id, d.amount from db.sch.documented as d"),
            // The engine's own false alarm: a CTE named like the alias inside it.
            model("model.p.named", &raw, &[], "select id, amount as diff from db.raw.feed"),
            model(
                "model.p.named_kid",
                &["model.p.named"],
                &[],
                "with named as (select named.id, max(named.diff) as last_diff from db.sch.named as named group by 1) \
                 select named.id, named.last_diff from named",
            ),
        ]);
        let warehouse = HashMap::from([
            ("source.p.raw.feed".to_string(), cols(&["amount", "id"])),
            ("model.p.older".to_string(), cols(&["id"])),
        ]);
        let r = run(&p, warehouse, &opts(true, false));
        let entry = |uid: &str| r.report.models.iter().find(|m| m.unique_id == uid);
        let reads = |uid: &str| -> Vec<(&str, &str, bool, Vec<&str>)> {
            entry(uid)
                .map(|m| {
                    m.read_downstream_but_absent
                        .iter()
                        .map(|d| (d.column.as_str(), d.against, d.in_compile, d.read_by.iter().map(String::as_str).collect()))
                        .collect()
                })
                .unwrap_or_default()
        };
        assert_eq!(reads("model.p.short"), vec![("amount", "computed", false, vec!["model.p.short_kid"])]);
        assert!(entry("model.p.short_kid").is_none(), "explained by the parent's short compile");
        assert_eq!(reads("model.p.older"), vec![("amount", "warehouse", true, vec!["model.p.older_kid"])]);
        assert!(entry("model.p.older_kid").is_none(), "explained by the parent's own compile");
        assert_eq!(reads("model.p.documented"), vec![("amount", "declared", false, vec!["model.p.documented_kid"])]);
        assert!(entry("model.p.documented_kid").is_some(), "nothing but the child says it exists");
        assert!(reads("model.p.named").is_empty());
        assert!(entry("model.p.named_kid").is_some(), "no edge was drawn from the column named");
        assert_eq!(r.report.totals.read_downstream_but_absent, 3);
    }

    /// Two parents sharing column names, `model.p.hub` sorting first, and a
    /// child whose compile each test sets aside whole, so it is inferred.
    fn two_parents(child_sql: &str, child_warehouse: &[&str], extra: Vec<(String, Node)>) -> Run {
        let mut hub = node("model.p.hub", "db.sch.hub", &[], &[]);
        hub.1.sql = "select 1 as k, 2 as ldts".into();
        let mut lnk = node("model.p.lnk", "db.sch.lnk", &[], &[]);
        lnk.1.sql = "select 1 as k, 2 as ldts, 3 as v".into();
        let mut parents: Vec<&str> = vec!["model.p.hub", "model.p.lnk"];
        let extra_uids: Vec<String> = extra.iter().map(|(u, _)| u.clone()).collect();
        parents.extend(extra_uids.iter().map(String::as_str));
        let mut child = node("model.p.child", "db.sch.child", &parents, &[]);
        child.1.sql = child_sql.into();
        let mut nodes = vec![hub, lnk, child];
        nodes.extend(extra);
        let warehouse = HashMap::from([
            ("model.p.hub".to_string(), cols(&["k", "ldts"])),
            ("model.p.lnk".to_string(), cols(&["k", "ldts", "v"])),
            ("model.p.child".to_string(), cols(child_warehouse)),
        ]);
        run(&project_of(nodes), warehouse, &opts(true, false))
    }

    fn inferred_from(r: &Run) -> Vec<(&str, &str)> {
        let mut out: Vec<(&str, &str)> = r
            .edges
            .iter()
            .filter(|e| e.to == "model.p.child")
            .map(|e| (e.to_col.as_str(), e.from.as_str()))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn a_set_aside_compile_still_says_which_parent_a_shared_name_comes_from() {
        // The compile reads the link and is set aside for reading a name its
        // CTE does not give; by order the hub, first by name, would have had
        // `k`.
        let sql = "with l as (select k, ldts, v from db.sch.lnk) select l.k, l.ldts, l.v, l.w as extra from l";
        let r = two_parents(sql, &["k", "ldts", "v"], vec![]);
        assert_eq!(
            inferred_from(&r),
            vec![("k", "model.p.lnk"), ("ldts", "model.p.lnk"), ("v", "model.p.lnk")]
        );
        let entry = r.report.models.iter().find(|m| m.unique_id == "model.p.child").unwrap();
        assert!(entry.ambiguous_inferred.iter().all(|a| a.by == "compile"));
        assert_eq!(r.report.totals.ambiguous_by_compile, 2);
    }

    #[test]
    fn with_no_reading_every_parent_carrying_the_name_is_taken() {
        // A union that did not parse: each branch gives each column. A parent
        // known only by its YAML stands aside for ones known better.
        let mut yaml_only = node("model.p.yaml_only", "db.sch.yaml_only", &[], &["k"]);
        yaml_only.1.sql = "select from".into();
        let r = two_parents("select from", &["k", "ldts"], vec![yaml_only]);
        assert_eq!(
            inferred_from(&r),
            vec![("k", "model.p.hub"), ("k", "model.p.lnk"), ("ldts", "model.p.hub"), ("ldts", "model.p.lnk")]
        );
        assert_eq!(r.report.totals.ambiguous_by_every, 2);
    }

    #[test]
    fn a_reading_that_renames_does_not_choose() {
        // The compile reads `k` into another name, so it names no parent for
        // the column `k`, and the order decides as it did.
        let sql = "with l as (select v from db.sch.lnk) select l.v as k, l.w as extra from l";
        let r = two_parents(sql, &["k"], vec![]);
        assert_eq!(inferred_from(&r), vec![("k", "model.p.hub")]);
        assert_eq!(r.report.totals.ambiguous_by_order, 1);
    }

    /// A table built from older code than its compile: `p` gives `a`, `b`, `c`
    /// and `x`; the table of `m` has `a`, `bb` and `x`, and the SQL of `m` is
    /// whatever the test says it now is.
    fn behind_the_code(sql: &str, indirect: bool) -> Run {
        let mut p = node("model.p.p", "db.sch.p", &[], &[]);
        p.1.sql = "select 1 as a, 2 as b, 3 as c, 4 as x".into();
        let mut m = node("model.p.m", "db.sch.m", &["model.p.p"], &[]);
        m.1.sql = sql.into();
        let warehouse = HashMap::from([
            ("model.p.p".to_string(), cols(&["a", "b", "c", "x"])),
            ("model.p.m".to_string(), cols(&["a", "bb", "x"])),
        ]);
        run(&project_of(vec![p, m]), warehouse, &opts(true, indirect))
    }

    fn into_m(r: &Run) -> Vec<(&str, &str, &str, &str, &str)> {
        shape(&r.edges).into_iter().filter(|e| e.2 == "model.p.m").collect()
    }

    #[test]
    fn a_compile_its_table_is_behind_keeps_the_columns_both_have() {
        // The code renamed `b`, added `c` and dropped `x` since the table was
        // built: `a` and `bb` are read, `x` is matched, and `c` goes nowhere.
        let r = behind_the_code("select a, b as bb, c from db.sch.p where a > 0", false);
        assert_eq!(
            into_m(&r),
            vec![
                ("model.p.p", "a", "model.p.m", "a", "passthrough"),
                ("model.p.p", "b", "model.p.m", "bb", "rename"),
                ("model.p.p", "x", "model.p.m", "x", "inferred"),
            ]
        );
        let m = r.report.models.iter().find(|m| m.unique_id == "model.p.m").unwrap();
        assert_eq!((m.provenance, m.agreement, m.edges_inferred), ("per_column", "degraded", 1));
        assert_eq!(m.unexpected_columns, vec!["c"], "named, and in the cache on no rung");
        let t = &r.report.totals;
        assert_eq!((t.per_column, t.columns_withheld), (1, 1));
        assert_eq!((t.edges_parsed, t.edges_inferred), (2, 1));
        // What it reads to choose its rows is the same SQL's, and is read.
        assert_eq!(r.report.row_deciding_reads.len(), 1);
    }

    #[test]
    fn a_kept_compile_fans_out_onto_the_columns_it_is_kept_for() {
        let r = behind_the_code("select a, b as bb, c from db.sch.p where a > 0", true);
        let fanned: Vec<&str> =
            into_m(&r).into_iter().filter(|e| e.4 == "filter").map(|e| e.3).collect();
        assert_eq!(fanned, vec!["bb"], "`a` has its direct edge, `c` is withheld, `x` is not the SQL's");
    }

    #[test]
    fn a_kept_compile_still_tells_its_parent_what_it_reads() {
        // `y` is new in the code of `m` and read from `p`, whose table and
        // compile both lack it: the read is filed on `p`, whatever `m` keeps.
        let r = behind_the_code("select a, b as bb, y from db.sch.p", false);
        let p = r.report.models.iter().find(|m| m.unique_id == "model.p.p").expect("listed for the read");
        let reads: Vec<(&str, &[String])> =
            p.read_downstream_but_absent.iter().map(|d| (d.column.as_str(), d.read_by.as_slice())).collect();
        assert_eq!(reads, vec![("y", &["model.p.m".to_string()][..])]);
    }

    #[test]
    fn a_contradicted_compile_is_set_aside_whole_on_its_own_evidence() {
        // Every column renamed, so the two share nothing to keep; and a read
        // of a name the CTE does not give, so the SQL cannot run as it stands.
        for sql in [
            "select a as aa, b as b2 from db.sch.p",
            "with s as (select a from db.sch.p) select s.a, s.b as bb from s",
        ] {
            let r = behind_the_code(sql, false);
            let m = r.report.models.iter().find(|m| m.unique_id == "model.p.m").unwrap();
            assert_eq!(m.provenance, "inferred", "{sql}");
            assert!(into_m(&r).iter().all(|e| e.4 == "inferred"), "{sql}: {:?}", into_m(&r));
            assert_eq!(r.report.totals.per_column, 0);
        }
    }

    /// `thin`'s table has `id` alone, as a table older than its code would, and
    /// its YAML says `amount` and `status` too; `codes` has `id` alone and its
    /// YAML `amount`. `m` reads whatever the test says, and `g` takes `*`
    /// straight from `thin`.
    /// Each model as the observer saw it: its outputs and its roots.
    type Outputs = Vec<(String, Vec<String>, Vec<String>)>;

    fn over_thin(sql: &str) -> (Run, Outputs) {
        let mut thin = node("model.p.thin", "db.sch.thin", &[], &["id", "amount", "status"]);
        thin.1.sql = "select 1 as id".into();
        let mut codes = node("model.p.codes", "db.sch.codes", &[], &["id", "amount"]);
        codes.1.sql = "select 1 as id".into();
        let mut m = node("model.p.m", "db.sch.m", &["model.p.thin", "model.p.codes"], &[]);
        m.1.sql = sql.into();
        let mut g = node("model.p.g", "db.sch.g", &["model.p.thin"], &[]);
        g.1.sql = "select * from db.sch.thin".into();
        let warehouse = HashMap::from([
            ("model.p.thin".to_string(), cols(&["id"])),
            ("model.p.codes".to_string(), cols(&["id"])),
        ]);
        let mut seen = Vec::new();
        let r = run_observed(&project_of(vec![thin, codes, m, g]), warehouse, &opts(true, false), |s| {
            seen.push((s.model.uid.clone(), s.resolved.outputs.clone(), s.resolved.roots.clone()))
        });
        (r, seen)
    }

    fn edges_into<'a>(r: &'a Run, to: &str) -> Vec<(&'a str, &'a str, &'a str, &'a str)> {
        r.edges
            .iter()
            .filter(|e| e.to == to)
            .map(|e| (e.from.as_str(), e.from_col.as_str(), e.to_col.as_str(), e.kind.as_str()))
            .collect()
    }

    #[test]
    fn a_column_read_through_a_star_over_a_short_list_keeps_its_edge_when_witnessed() {
        let (r, seen) =
            over_thin("with s as (select * from db.sch.thin) select s.id, s.amount, s.status as state, s.nowhere from s");
        assert_eq!(
            edges_into(&r, "model.p.m"),
            vec![
                ("model.p.thin", "id", "id", "passthrough"),
                ("model.p.thin", "amount", "amount", "passthrough"),
                ("model.p.thin", "status", "state", "rename"),
            ]
        );
        let m = r.report.models.iter().find(|m| m.unique_id == "model.p.m").expect("listed for its loss");
        let lost: Vec<&str> = m.lost_columns.iter().map(|l| l.column.as_str()).collect();
        assert_eq!(lost, vec!["nowhere"], "no list of `thin` has it");
        assert!(m.issues.is_empty(), "the reading's issues are the first call's");
        let thin = r.report.models.iter().find(|m| m.unique_id == "model.p.thin").expect("told of the reads");
        let filed: Vec<(&str, &str, bool)> = thin
            .read_downstream_but_absent
            .iter()
            .map(|d| (d.column.as_str(), d.against, d.in_compile))
            .collect();
        assert_eq!(filed, vec![("amount", "warehouse", false), ("status", "warehouse", false)]);
        assert_eq!((r.report.totals.read_through_star, r.report.totals.read_through_star_unwitnessed), (2, 1));
        // The columns lived for one call: the store never heard of them.
        let g = seen.iter().find(|s| s.0 == "model.p.g").unwrap();
        assert_eq!(g.1, vec!["id"]);
    }

    #[test]
    fn an_expression_over_a_name_the_star_lacks_is_left_alone() {
        // The engine makes no column of `amount` inside an expression, so there
        // is no phantom to anchor on and nothing is extended. A known limit.
        let (r, _) =
            over_thin("with s as (select * from db.sch.thin) select s.id, sum(s.amount) as total from s group by s.id");
        assert_eq!(edges_into(&r, "model.p.m"), vec![("model.p.thin", "id", "id", "passthrough")]);
        assert_eq!(r.report.totals.read_through_star, 0);
    }

    #[test]
    fn an_extension_that_costs_an_edge_is_refused() {
        // With `code` added to `thin`, the bare `code` has two sources and
        // loses the edge it had from `codes`, though the outputs stay.
        let mut thin = node("model.p.thin", "db.sch.thin", &[], &["id", "code"]);
        thin.1.sql = "select 1 as id".into();
        let mut codes = node("model.p.codes", "db.sch.codes", &[], &[]);
        codes.1.sql = "select 1 as id, 2 as code".into();
        let mut m = node("model.p.m", "db.sch.m", &["model.p.thin", "model.p.codes"], &[]);
        m.1.sql = "with s as (select * from db.sch.thin) select s.code as sc, upper(code) as cu \
                   from s cross join db.sch.codes"
            .into();
        let warehouse = HashMap::from([
            ("model.p.thin".to_string(), cols(&["id"])),
            ("model.p.codes".to_string(), cols(&["code", "id"])),
        ]);
        let r = run(&project_of(vec![thin, codes, m]), warehouse, &opts(true, false));
        assert_eq!(edges_into(&r, "model.p.m"), vec![("model.p.codes", "code", "cu", "transform")]);
        assert_eq!(r.report.totals.read_through_star, 0);
    }

    #[test]
    fn only_a_name_missing_from_a_star_over_one_relation_is_extended() {
        // Each also reads `id`, so that the compile is read rather than set
        // aside as silent.
        for sql in [
            // An alias the CTE defines itself is no phantom, whatever a list
            // says.
            "with s as (select * from db.sch.thin), u as (select *, upper(s.status) as amount from s) \
             select u.id, u.amount from u",
            // A star over two relations names neither, though both YAMLs have
            // the column.
            "with p as (select * from db.sch.thin a join db.sch.codes c on a.id = c.id) \
             select t.id, p.amount from p cross join db.sch.thin t",
            // A list of the CTE's own: the SQL left the name out, and cannot run.
            "with s as (select id from db.sch.thin) select s.id, s.amount from s",
        ] {
            let (r, _) = over_thin(sql);
            let into = edges_into(&r, "model.p.m");
            assert!(into.contains(&("model.p.thin", "id", "id", "passthrough")), "{sql}: {into:?}");
            assert!(!into.iter().any(|e| e.1 == "amount"), "{sql}: {into:?}");
        }
    }

    #[test]
    fn an_extension_that_moves_the_reading_is_refused() {
        // A second star over `thin` would take the added column into the
        // outputs, which only the first call may decide.
        let (r, seen) = over_thin(
            "with s as (select * from db.sch.thin), u as (select * from db.sch.thin) \
             select s.amount as amt, u.* from s cross join u",
        );
        assert!(!edges_into(&r, "model.p.m").iter().any(|e| e.2 == "amt"));
        assert_eq!(seen.iter().find(|s| s.0 == "model.p.m").unwrap().1, vec!["amt", "id"]);
        assert_eq!(r.report.totals.read_through_star, 0);
    }

    #[test]
    fn a_trusted_model_names_its_lost_columns_and_a_distrusted_one_does_not() {
        let mut src = node("source.p.shop.orders", "db.sch.shop_orders", &[], &[]);
        src.1.kind = "source".into();
        let sql = "with o as (select * from db.sch.shop_orders) \
                   select o.order_id, o.coupon_code as coupon from o";
        let model = |uid: &str, rel: &str| {
            let mut m = node(uid, rel, &["source.p.shop.orders"], &[]);
            m.1.sql = sql.into();
            m
        };
        let p = project_of(vec![
            src,
            model("model.p.trusted", "db.sch.trusted"),
            model("model.p.stale", "db.sch.stale"),
            model("model.p.kept", "db.sch.kept"),
            model("model.p.kept_less", "db.sch.kept_less"),
        ]);
        let warehouse = HashMap::from([
            ("source.p.shop.orders".to_string(), cols(&["amount", "order_id"])),
            // The dev table shares no column with the compile, so it is set
            // aside whole.
            ("model.p.stale".to_string(), cols(&["amount", "status"])),
            // This one has both columns and one more, so the compile is kept
            // for the two, the lost one included.
            ("model.p.kept".to_string(), cols(&["coupon", "order_id", "status"])),
            // This one lacks the lost column, which then goes out on no rung
            // and is no loss of the cache's.
            ("model.p.kept_less".to_string(), cols(&["order_id", "status"])),
        ]);
        let r = run(&p, warehouse, &opts(true, false));
        let entry = |uid: &str| r.report.models.iter().find(|m| m.unique_id == uid);
        let trusted = entry("model.p.trusted").expect("a model with a lost column is a fault");
        assert_eq!(trusted.edges, 1);
        assert_eq!(
            trusted.lost_columns,
            vec![LostColumn {
                column: "coupon".into(),
                ends: vec![LostEnd {
                    reason: "phantom",
                    via: Some("o".into()),
                    relations: vec![LostRelation {
                        relation: "DB.SCH.SHOP_ORDERS".into(),
                        columns_from: "warehouse",
                    }],
                    names: Vec::new(),
                }],
            }]
        );
        assert!(entry("model.p.stale").expect("degraded").lost_columns.is_empty());
        assert_eq!(entry("model.p.kept").expect("degraded").lost_columns, trusted.lost_columns);
        assert!(entry("model.p.kept_less").expect("degraded").lost_columns.is_empty());
        assert_eq!(r.report.totals.columns_lost, 2);
    }

    #[test]
    fn a_contested_relation_goes_to_the_claimant_the_model_depends_on() {
        // Two sources declare one table, spelt in two cases. Across the project
        // the name order settles it on the upper case one, and a model built
        // from the lower case one must still name that one.
        let source = |uid: &str| {
            let mut s = node(uid, "db.raw.orders", &[], &[]);
            s.1.kind = "source".into();
            s
        };
        let sql = "select id, amount from db.raw.orders";
        let model = |uid: &str, rel: &str, parents: &[&str]| {
            let mut m = node(uid, rel, parents, &[]);
            m.1.sql = sql.into();
            m
        };
        let (upper, lower) = ("source.p.raw.ORDERS", "source.p.raw.orders");
        let mut stale = model("model.p.stale", "db.sch.stale", &[lower]);
        stale.1.sql = "select id as order_id, amount as total from db.raw.orders".into();
        let mut p = project_of(vec![
            source(upper),
            source(lower),
            model("model.p.lower_only", "db.sch.lower_only", &[lower]),
            stale,
            model("model.p.both", "db.sch.both", &[lower, upper]),
            model("model.p.both_again", "db.sch.both_again", &[upper, lower]),
        ]);
        p.by_relation.insert("DB.RAW.ORDERS".into(), upper.into());
        let warehouse = HashMap::from([
            (upper.to_string(), cols(&["amount", "id"])),
            (lower.to_string(), cols(&["amount", "id"])),
            // Not one column the compile gives, so this compile is set aside
            // and its edges come from name matching instead.
            ("model.p.stale".to_string(), cols(&["amount", "id"])),
        ]);
        let r = run(&p, warehouse, &opts(true, false));
        let from = |to: &str| -> Vec<(&str, &str)> {
            r.edges.iter().filter(|e| e.to == to).map(|e| (e.from.as_str(), e.kind.as_str())).collect()
        };
        assert_eq!(from("model.p.lower_only"), vec![(lower, "passthrough"), (lower, "passthrough")]);
        assert_eq!(from("model.p.stale"), vec![(lower, "inferred"), (lower, "inferred")]);
        // Built from both, in either order, the model gives no reason to prefer
        // one, and the name order decides as it does for the project.
        assert_eq!(from("model.p.both"), vec![(upper, "passthrough"), (upper, "passthrough")]);
        assert_eq!(from("model.p.both_again"), vec![(upper, "passthrough"), (upper, "passthrough")]);
    }

    #[test]
    fn a_self_read_is_dropped_even_when_another_node_holds_the_relation() {
        // A second model claims this one's table and wins it by name, so the
        // self read would otherwise go out as an edge from that twin.
        let mut src = node("source.p.raw.orders", "db.raw.orders", &[], &[]);
        src.1.kind = "source".into();
        let mut m = node("model.p.m", "db.sch.m", &["source.p.raw.orders"], &[]);
        m.1.sql = "select id from db.raw.orders union all select id from db.sch.m".into();
        let twin = node("model.p.a_twin", "db.sch.m", &[], &[]);
        let mut p = project_of(vec![src, m, twin]);
        p.by_relation.insert("DB.SCH.M".into(), "model.p.a_twin".into());
        let warehouse = HashMap::from([("source.p.raw.orders".to_string(), cols(&["id"]))]);
        let r = run(&p, warehouse, &opts(true, true));
        let into_m: Vec<_> = r.edges.iter().filter(|e| e.to == "model.p.m").collect();
        assert!(!into_m.is_empty());
        assert!(into_m.iter().all(|e| e.from == "source.p.raw.orders"), "{:?}", shape(&r.edges));
    }

    #[test]
    fn a_parent_that_lost_its_relation_to_another_claimant_is_still_declared() {
        let mut p = project();
        let (uid, other) = node("model.p.a_other", "db.sch.parent", &[], &[]);
        p.nodes.insert(uid.clone(), other);
        p.by_relation.insert("DB.SCH.PARENT".into(), uid);
        // Across the project the other claimant holds the relation, and the
        // child, which depends on the parent, still reads the parent.
        assert_eq!(node_for(&p, &HashMap::new(), "DB.SCH.PARENT"), "model.p.a_other");
        let read = claimants(&p, &p.nodes["model.p.child"]);
        assert_eq!(node_for(&p, &read, "DB.SCH.PARENT"), "model.p.parent");
        let deps = dependencies(&p, &p.nodes["model.p.child"]);
        assert!(undeclared("DB.SCH.CHILD", &deps, &cols(&["DB.SCH.PARENT"])).is_empty());
    }

    #[test]
    fn an_ephemeral_parent_is_read_through_to_the_relations_it_inlines() {
        let mut outer = node("model.p.outer", "", &["model.p.inner", "model.p.parent"], &[]);
        outer.1.materialized = "ephemeral".into();
        let mut inner = node("model.p.inner", "", &["source.p.src"], &[]);
        inner.1.materialized = "ephemeral".into();
        let p = project_of(vec![
            node("model.p.child", "db.sch.child", &["model.p.outer", "model.p.parent"], &[]),
            node("model.p.parent", "db.sch.parent", &[], &[]),
            node("source.p.src", "\"DB\".\"sch\".\"src\"", &[], &[]),
            outer,
            inner,
        ]);
        let mut deps = dependencies(&p, &p.nodes["model.p.child"]);
        deps.sort();
        assert_eq!(deps, vec!["DB.SCH.PARENT", "DB.SCH.SRC"]);
    }

    #[test]
    fn a_relation_dbt_was_not_told_of_is_named_and_lists_the_model() {
        let mut src = node("source.p.src", "db.sch.src", &[], &["id"]);
        src.1.kind = "source".into();
        let mut bare = node("source.p.bare", "db.sch.bare", &[], &[]);
        bare.1.kind = "source".into();
        // A raw table named where a source belongs. With no parent the engine
        // knows no table and says nothing, so nothing else would list it.
        let mut hard = node("model.p.hard", "db.sch.hard", &[], &[]);
        hard.1.sql = "select id from \"RAW_DB\".\"dbo\".\"orders_base\"".into();
        // dbt inlines `eph` into `via`, which then reads `src` by name.
        let mut eph = node("model.p.eph", "", &["source.p.src"], &[]);
        eph.1.materialized = "ephemeral".into();
        eph.1.sql = "select id from db.sch.src".into();
        let mut via = node("model.p.via", "db.sch.via", &["model.p.eph"], &[]);
        via.1.sql = "with __dbt__cte__eph as (select id from db.sch.src) \
                     select id from __dbt__cte__eph"
            .into();
        // A parent with no columns belongs to the other list.
        let mut both = node("model.p.both", "db.sch.both", &["source.p.src", "source.p.bare"], &[]);
        both.1.sql = "select s.id from db.sch.src s join db.sch.bare b on s.id = b.id".into();
        let warehouse = HashMap::from([("source.p.src".to_string(), cols(&["id"]))]);
        let p = project_of(vec![src, bare, hard, eph, via, both]);
        let r = run(&p, warehouse, &opts(true, false));

        let listed: Vec<(&str, &str, &[String], &[String])> = r
            .report
            .models
            .iter()
            .map(|m| {
                let (unknown, undeclared) = (&m.unknown_relations, &m.undeclared_relations);
                (m.unique_id.as_str(), m.provenance, &unknown[..], &undeclared[..])
            })
            .collect();
        let (bare, raw) = (cols(&["DB.SCH.BARE"]), cols(&["RAW_DB.DBO.ORDERS_BASE"]));
        assert_eq!(
            listed,
            vec![
                ("model.p.both", "parsed", &bare[..], &[][..]),
                ("model.p.hard", "parsed", &[][..], &raw[..]),
            ]
        );
        let hard = &r.report.models[1];
        assert!(hard.issues.is_empty(), "the engine said nothing of it");
        assert_eq!(r.report.totals.undeclared, 1);
        // Naming it is all: the edge the compile states goes out as before.
        let into_hard: Vec<_> =
            shape(&r.edges).into_iter().filter(|e| e.2 == "model.p.hard").collect();
        let stated = ("rel:raw_db.dbo.orders_base", "id", "model.p.hard", "id", "passthrough");
        assert_eq!(into_hard, vec![stated]);
    }

    #[test]
    fn a_dependency_named_in_two_parts_is_compared_as_written() {
        // Snowflake completes `sch.src` with the session database. Completing it
        // here would be a guess, so the declared source is listed as undeclared.
        let mut src = node("source.p.src", "db.sch.src", &[], &["id"]);
        src.1.kind = "source".into();
        let mut m = node("model.p.m", "db.sch.m", &["source.p.src"], &[]);
        m.1.sql = "select id from sch.src".into();
        let warehouse = HashMap::from([("source.p.src".to_string(), cols(&["id"]))]);
        let r = run(&project_of(vec![src, m]), warehouse, &opts(true, false));

        let listed: Vec<(&str, &[String])> = r
            .report
            .models
            .iter()
            .map(|m| (m.unique_id.as_str(), &m.undeclared_relations[..]))
            .collect();
        assert_eq!(listed, vec![("model.p.m", &cols(&["SCH.SRC"])[..])]);
    }

    #[test]
    fn a_fanned_edge_neither_doubles_a_direct_one_nor_comes_from_the_model_itself() {
        let p = project();
        let reads = vec![
            engine::IndirectRead {
                relation: "DB.SCH.PARENT".into(),
                column: "a".into(),
                role: role::Role::Filter,
            },
            engine::IndirectRead {
                relation: "DB.SCH.CHILD".into(),
                column: "a".into(),
                role: role::Role::Filter,
            },
        ];
        let direct = vec![cache::Edge {
            from: "model.p.parent".into(),
            from_col: "a".into(),
            to: "model.p.child".into(),
            to_col: "a".into(),
            kind: "passthrough".into(),
        }];
        let child = &p.nodes["model.p.child"];
        let fanned = fan_out(&p, &claimants(&p, child), child, &reads, &cols(&["a", "b"]), &direct);
        assert_eq!(shape(&fanned), vec![("model.p.parent", "a", "model.p.child", "b", "filter")]);
    }

    #[test]
    fn a_column_read_for_two_reasons_still_draws_one_edge_per_pair() {
        use role::Role::{DedupKey, Filter, JoinKey};
        let p = project_of(vec![
            node("model.p.m", "db.sch.m", &["source.p.orders"], &[]),
            node("source.p.orders", "db.shop.orders", &[], &[]),
        ]);
        let read = |column: &str, role| engine::IndirectRead {
            relation: "DB.SHOP.ORDERS".into(),
            column: column.into(),
            role,
        };
        // The winner comes first for one column and second for the other, so
        // neither the read order nor the engine's sort can be what decides.
        let reads = vec![
            read("valid_from", DedupKey),
            read("valid_from", Filter),
            read("customer_id", JoinKey),
            read("customer_id", DedupKey),
            read("id", Filter),
        ];
        let direct = vec![cache::Edge {
            from: "source.p.orders".into(),
            from_col: "id".into(),
            to: "model.p.m".into(),
            to_col: "id".into(),
            kind: "passthrough".into(),
        }];
        let m = &p.nodes["model.p.m"];
        let fanned = fan_out(&p, &claimants(&p, m), m, &reads, &cols(&["amount", "id"]), &direct);
        let (o, m) = ("source.p.orders", "model.p.m");
        assert_eq!(
            shape(&fanned),
            vec![
                (o, "valid_from", m, "amount", "dedup_key"),
                (o, "valid_from", m, "id", "dedup_key"),
                (o, "customer_id", m, "amount", "dedup_key"),
                (o, "customer_id", m, "id", "dedup_key"),
                (o, "id", m, "amount", "filter"),
            ]
        );
    }

    #[test]
    fn a_where_and_a_qualify_on_one_column_draw_one_edge_and_report_both_reads() {
        let mut orders = node("source.p.orders", "db.shop.orders", &[], &[]);
        orders.1.kind = "source".into();
        let mut m = node("model.p.m", "db.sch.m", &["source.p.orders"], &[]);
        m.1.sql = "select o.id, o.amount from db.shop.orders o \
                   where o.valid_from <= current_date and o.id is not null \
                   qualify o.valid_from = max(o.valid_from) over (partition by o.id)"
            .into();
        let warehouse = HashMap::from([(
            "source.p.orders".to_string(),
            cols(&["amount", "id", "valid_from"]),
        )]);
        let r = run(&project_of(vec![orders, m]), warehouse, &opts(true, true));

        let (o, m) = ("source.p.orders", "model.p.m");
        assert_eq!(
            shape(&r.edges),
            vec![
                (o, "amount", m, "amount", "passthrough"),
                (o, "id", m, "id", "passthrough"),
                (o, "id", m, "amount", "dedup_key"),
                (o, "valid_from", m, "amount", "dedup_key"),
                (o, "valid_from", m, "id", "dedup_key"),
            ]
        );
        assert_eq!(r.report.totals.edges_indirect, 3);
        // The report lists reads, not edges, so the reason the cache did not
        // draw is still on record.
        assert_eq!(r.report.totals.row_deciding_reads, 4);
        // `id` is carried through as well; `valid_from` is read and dropped.
        assert_eq!(r.report.totals.read_not_projected, 2);
        let reads: Vec<(&str, &str, bool)> = r.report.row_deciding_reads[0]
            .columns
            .iter()
            .map(|c| (c.column.as_str(), c.role, c.projected))
            .collect();
        assert_eq!(
            reads,
            vec![
                ("id", "dedup_key", true),
                ("id", "filter", true),
                ("valid_from", "dedup_key", false),
                ("valid_from", "filter", false),
            ]
        );
    }

    #[test]
    fn the_pass_runs_over_a_project_held_in_memory() {
        let (p, w) = shop();
        let r = run(&p, w, &opts(true, false));
        assert_eq!(
            shape(&r.edges),
            vec![
                ("source.p.raw", "amount", "model.p.behind", "amount", "inferred"),
                ("source.p.raw", "id", "model.p.behind", "id", "inferred"),
                ("source.p.raw", "amount", "model.p.orders", "doubled", "transform"),
                ("source.p.raw", "id", "model.p.orders", "id", "passthrough"),
            ]
        );
        let t = &r.report.totals;
        assert_eq!((t.models, t.parsed, t.confirmed, t.degraded), (2, 2, 1, 1));
        assert_eq!((t.edges_parsed, t.edges_inferred, t.edges_indirect), (2, 2, 0));
        assert_eq!((t.columns_covered, t.columns_total), (4, 5), "channel has no parent");
        assert_eq!((t.columns_covered_independent, t.columns_total_independent), (4, 4));
        assert_eq!((t.columns_root, t.yaml_stale), (1, 1));

        let listed: Vec<(&str, &str, &str, usize)> = r
            .report
            .models
            .iter()
            .map(|m| (m.unique_id.as_str(), m.provenance, m.agreement, m.edges))
            .collect();
        assert_eq!(
            listed,
            vec![
                ("model.p.behind", "inferred", "degraded", 2),
                ("model.p.orders", "parsed", "confirmed", 2),
            ]
        );

        // The trusted filter is reported whether or not the run draws it, and
        // the distrusted one is not reported at all.
        assert_eq!(t.read_not_projected, 1);
        let reads = &r.report.row_deciding_reads;
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].unique_id, "model.p.orders");
        let c = &reads[0].columns[0];
        let read = (c.relation.as_str(), c.column.as_str(), c.role);
        assert_eq!(read, ("DB.SCH.RAW", "status", "filter"));
    }

    #[test]
    fn indirect_edges_follow_their_model_s_direct_ones_and_count_apart() {
        let (p, w) = shop();
        let plain = run(&p, w.clone(), &opts(true, false));
        let r = run(&p, w, &opts(true, true));
        assert_eq!(
            shape(&r.edges),
            vec![
                ("source.p.raw", "amount", "model.p.behind", "amount", "inferred"),
                ("source.p.raw", "id", "model.p.behind", "id", "inferred"),
                ("source.p.raw", "amount", "model.p.orders", "doubled", "transform"),
                ("source.p.raw", "id", "model.p.orders", "id", "passthrough"),
                ("source.p.raw", "status", "model.p.orders", "channel", "filter"),
                ("source.p.raw", "status", "model.p.orders", "doubled", "filter"),
                ("source.p.raw", "status", "model.p.orders", "id", "filter"),
            ]
        );
        let (a, b) = (&plain.report.totals, &r.report.totals);
        assert_eq!(b.edges_indirect, 3);
        assert_eq!((b.edges_parsed, b.edges_inferred), (a.edges_parsed, a.edges_inferred));
        assert_eq!((b.columns_covered, b.columns_total), (a.columns_covered, a.columns_total));
        assert_eq!(b.columns_covered_independent, a.columns_covered_independent);
        let orders = r.report.models.iter().find(|m| m.unique_id == "model.p.orders").unwrap();
        assert_eq!(orders.edges, 2, "a model's own count is its direct edges");
    }

    #[test]
    fn a_distrusted_compile_does_not_decide_what_a_star_below_it_sees() {
        // No table to check `lost` against, and its compile shares no column
        // with its YAML, so the YAML is the better account of what `after` read.
        let mut raw = node("source.p.raw", "db.sch.raw", &[], &[]);
        raw.1.kind = "source".into();
        let mut lost = node("model.p.lost", "db.sch.lost", &["source.p.raw"], &["amount", "id"]);
        lost.1.sql = "select id as order_no from db.sch.raw".into();
        let mut after = node("model.p.after", "db.sch.after", &["model.p.lost"], &[]);
        after.1.sql = "select * from db.sch.lost".into();
        let warehouse =
            HashMap::from([("source.p.raw".to_string(), cols(&["amount", "id", "status"]))]);
        let r = run(&project_of(vec![raw, lost, after]), warehouse, &opts(true, false));
        assert_eq!(
            shape(&r.edges),
            vec![
                ("source.p.raw", "amount", "model.p.lost", "amount", "inferred"),
                ("source.p.raw", "id", "model.p.lost", "id", "inferred"),
                ("model.p.lost", "amount", "model.p.after", "amount", "passthrough"),
                ("model.p.lost", "id", "model.p.after", "id", "passthrough"),
            ]
        );
    }

    /// What an observer is shown of one model, reduced to what the tests check.
    #[derive(Debug)]
    struct Glimpse {
        uid: String,
        plan: Plan,
        agreement: Agreement,
        silent: bool,
        parents: Vec<(String, Provenance, Vec<String>)>,
        engine_edges: usize,
        computed: Option<Vec<String>>,
    }

    fn watch(p: &Project, w: HashMap<String, Vec<String>>) -> (Run, Vec<Glimpse>) {
        let mut seen = Vec::new();
        let r = run_observed(p, w, &opts(true, false), |s| {
            seen.push(Glimpse {
                uid: s.model.uid.clone(),
                plan: s.plan,
                agreement: s.agreement,
                silent: s.silent,
                parents: s
                    .parents
                    .iter()
                    .map(|p| (p.node.uid.clone(), p.provenance, p.columns.clone()))
                    .collect(),
                engine_edges: s.resolved.edges.len(),
                computed: s.store.computed(&s.model.uid).cloned(),
            })
        });
        (r, seen)
    }

    #[test]
    fn an_observer_is_shown_the_engine_s_edges_on_a_compile_the_cache_sets_aside() {
        let (p, w) = shop();
        let (r, seen) = watch(&p, w.clone());
        let behind = seen.iter().find(|s| s.uid == "model.p.behind").unwrap();
        assert_eq!((behind.plan, behind.agreement), (Plan::Inferred, Agreement::Degraded));
        let raw = cols(&["amount", "id", "status"]);
        assert_eq!(behind.parents, vec![("source.p.raw".into(), Provenance::Warehouse, raw)]);
        assert_eq!(behind.engine_edges, 2, "what the SQL said, and the cache replaced");
        assert_eq!(behind.computed, None, "already retracted, as the next model will find it");
        let orders = seen.iter().find(|s| s.uid == "model.p.orders").unwrap();
        assert_eq!(orders.computed, Some(cols(&["channel", "doubled", "id"])));
        let unwatched = run(&p, w, &opts(true, false));
        assert_eq!(shape(&r.edges), shape(&unwatched.edges), "watching changes nothing");
    }

    #[test]
    fn an_observer_is_shown_each_parent_as_the_engine_was_handed_it() {
        // `after` is walked once the compile of `lost` has been set aside, so
        // the list its parent went to the engine with is the YAML's. `bare` has
        // no list anywhere, so the engine never heard of it and the report did.
        let mut raw = node("source.p.raw", "db.sch.raw", &[], &[]);
        raw.1.kind = "source".into();
        let mut bare = node("source.p.bare", "db.sch.bare", &[], &[]);
        bare.1.kind = "source".into();
        let mut lost = node("model.p.lost", "db.sch.lost", &["source.p.raw"], &["amount", "id"]);
        lost.1.sql = "select id as order_no from db.sch.raw".into();
        let mut after =
            node("model.p.after", "db.sch.after", &["model.p.lost", "source.p.bare"], &[]);
        after.1.sql = "select * from db.sch.lost".into();
        let warehouse =
            HashMap::from([("source.p.raw".to_string(), cols(&["amount", "id", "status"]))]);
        let (r, seen) = watch(&project_of(vec![raw, bare, lost, after]), warehouse);

        let order: Vec<(&str, Plan)> = seen.iter().map(|s| (s.uid.as_str(), s.plan)).collect();
        assert_eq!(order, vec![("model.p.lost", Plan::Inferred), ("model.p.after", Plan::Parsed)]);
        let handed = vec![
            ("model.p.lost".to_string(), Provenance::Declared, cols(&["amount", "id"])),
            ("source.p.bare".to_string(), Provenance::Unknown, Vec::new()),
        ];
        assert_eq!(seen[1].parents, handed);
        let listed = r.report.models.iter().find(|m| m.unique_id == "model.p.after").unwrap();
        assert_eq!(listed.unknown_relations, vec!["DB.SCH.BARE"]);
    }

    #[test]
    fn an_observer_is_told_when_silence_condemned_a_compile() {
        // The compile parsed and nothing contradicts it, so the flag is the only
        // reason on show for the plan it got.
        let mut raw = node("source.p.raw", "db.sch.raw", &[], &[]);
        raw.1.kind = "source".into();
        let mut stamp = node("model.p.stamp", "db.sch.stamp", &["source.p.raw"], &[]);
        stamp.1.sql = "select current_date as loaded_on from db.sch.raw".into();
        let warehouse = HashMap::from([("source.p.raw".to_string(), cols(&["id"]))]);
        let (_, seen) = watch(&project_of(vec![raw, stamp]), warehouse);
        let s = &seen[0];
        assert_eq!((s.silent, s.plan, s.agreement), (true, Plan::Inferred, Agreement::Unchecked));
        assert_eq!(s.engine_edges, 0);
    }

    #[test]
    fn an_observer_is_shown_the_edges_the_cache_got_as_they_were_written() {
        // `behind` is walked first. Its engine edges would read as two renames,
        // so being shown them in place of its name matches, or
        // the fan-out of `orders` counted as direct, cannot pass for the cache.
        let own = |e: &cache::Edge| {
            (e.from.clone(), e.from_col.clone(), e.to.clone(), e.to_col.clone(), e.kind.clone())
        };
        let runs = [
            (true, false, [(2, 0), (2, 0)]),
            (true, true, [(2, 0), (2, 3)]),
            (false, true, [(0, 0), (2, 3)]),
        ];
        for (infer, indirect, split) in runs {
            let (p, w) = shop();
            let (mut shown, mut counts) = (Vec::new(), Vec::new());
            let r = run_observed(&p, w, &opts(infer, indirect), |s| {
                shown.extend(s.published.iter().chain(s.published_indirect).map(own));
                counts.push((s.published.len(), s.published_indirect.len()));
            });
            let written: Vec<_> = r.edges.iter().map(own).collect();
            assert_eq!(shown, written, "infer {infer}, indirect {indirect}");
            assert_eq!(counts, split, "infer {infer}, indirect {indirect}");
        }
    }

    #[test]
    fn without_inference_a_distrusted_compile_is_unresolved() {
        let (p, w) = shop();
        let r = run(&p, w, &opts(false, false));
        assert!(r.edges.iter().all(|e| e.to != "model.p.behind"), "nothing may stand in for it");
        assert_eq!(r.report.totals.edges_inferred, 0);
        let m = &r.report.models[0];
        let listed = (m.unique_id.as_str(), m.provenance, m.edges);
        assert_eq!(listed, ("model.p.behind", "unresolved", 0));
    }

    #[test]
    fn a_catalog_of_another_target_witnesses_nothing() {
        // The model's entry describes the table a personal target built, which
        // still has a column the code dropped. Taken as the witness, it would
        // withhold `amount` and look for a parent of `legacy`; set aside, the
        // compile stands unchecked, and the report says why (0037).
        let t = Target::new("catalog-elsewhere");
        let manifest = serde_json::json!({
            "metadata": {"dbt_version": "1.11.8", "project_name": "shop", "adapter_type": "snowflake"},
            "sources": {"source.shop.raw.orders": {
                "name": "orders", "resource_type": "source", "relation_name": "db.raw.orders"}},
            "nodes": {"model.shop.orders": {
                "name": "orders", "resource_type": "model", "relation_name": "db.sales.orders",
                "compiled_code": "select id, amount from db.raw.orders",
                "depends_on": {"nodes": ["source.shop.raw.orders"]},
                "config": {"materialized": "table"}}},
            "parent_map": {"model.shop.orders": ["source.shop.raw.orders"]}
        });
        let entry = |db: &str, schema: &str, name: &str, cols: &[&str]| {
            let cols: serde_json::Map<String, serde_json::Value> =
                cols.iter().map(|c| (c.to_string(), serde_json::json!({"name": c}))).collect();
            serde_json::json!({"metadata": {"database": db, "schema": schema, "name": name}, "columns": cols})
        };
        let catalog = serde_json::json!({
            "sources": {"source.shop.raw.orders": entry("DB", "RAW", "ORDERS", &["ID", "AMOUNT"])},
            "nodes": {"model.shop.orders": entry("DB", "DBT_ME", "ORDERS", &["ID", "LEGACY"])}
        });
        t.write("target/manifest.json", manifest.to_string());
        t.write("target/catalog.json", catalog.to_string());

        let loaded = load(&Options::new(t.0.clone())).unwrap();
        assert!(loaded.warehouse.contains_key("source.shop.raw.orders"), "the source's entry is its own");
        assert!(!loaded.warehouse.contains_key("model.shop.orders"));

        let o = generate(&Options::new(t.0.clone())).unwrap();
        assert_eq!((o.totals.catalog_elsewhere, o.totals.unchecked, o.totals.degraded), (1, 1, 0));
        assert_eq!((o.totals.edges_parsed, o.totals.edges_inferred, o.totals.columns_withheld), (2, 0, 0));
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&o.report).unwrap()).unwrap();
        assert_eq!(
            report["catalog_elsewhere"],
            serde_json::json!([{
                "unique_id": "model.shop.orders",
                "relation": "db.sales.orders",
                "catalog_relation": "DB.DBT_ME.ORDERS"
            }])
        );
    }
}
