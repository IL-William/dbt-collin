//! Coverage, and every gap with enough detail to act on it.
//!
//! The report is a deliverable, not a log. When a column cannot be resolved the
//! useful output is not silence, it is "this relation was read, here is what was
//! known about it, here is what was missing", so the fix in YAML is obvious.
//!
//! It carries the three way comparison the whole design rests on: computed
//! against the warehouse says whether this compile is representative, and the
//! warehouse against the YAML says whether the documentation is stale.

use serde::Serialize;

/// One issue the engine raised, as the report carries it.
///
/// Mirrors `engine::Issue` rather than reusing it, because the report is a file
/// format and the engine's type is an interface: the two are free to drift, and
/// the boundary is the point of `engine.rs`.
#[derive(Serialize)]
pub struct ReportIssue {
    pub code: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub message: String,
    pub severity: &'static str,
    /// Byte range in the compiled SQL, so a reader can go straight there: the
    /// SQL dbt compiled, not `file`, which is the model before Jinja. For a parse
    /// error, the token the parser stopped at. For a column, the first place the
    /// statement spells its name, which need not be the reference at fault.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<[usize; 2]>,
    /// The table the engine could not find is the model's own. dbt compiles an
    /// incremental model's filter as a read of itself, and a model is never its
    /// own parent, so the engine is never handed it. Such an issue alone does not
    /// put a model here, unless an output column comes from that table alone:
    /// see 0012.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub self_reference: bool,
}

#[derive(Serialize, Default)]
pub struct Report {
    pub generated_at: String,
    pub project: String,
    pub dbt_version: String,
    pub adapter: String,
    /// Where the compiled files were looked for, beside the manifest, when it
    /// had no `compiled_code` for some model (0033).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub compiled_files: String,
    pub totals: Totals,
    /// Warehouse objects more than one dbt node claims. Not a model finding, so
    /// it sits at the top. An edge naming one of these relations goes to the
    /// claimant its model depends on, and to the first by name only when the
    /// model depends on both or on neither (0014).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub relation_collisions: Vec<RelationCollision>,
    /// Catalog entries describing another table than the one the manifest
    /// gives their node, usually one built under another target, and not used:
    /// their models are checked against nothing (0037). A finding about the
    /// inputs rather than a model, so it sits at the top.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub catalog_elsewhere: Vec<CatalogElsewhere>,
    /// Columns the SQL reads to decide which rows exist, by model: join keys,
    /// filters, dedup keys, each saying whether the model also carries it
    /// through to an output.
    ///
    /// Its own section rather than a field on `ModelReport`, because it is not a
    /// fault and `models` is a list of faults. Mixed in, it outnumbered the
    /// faults three to one and cost that list the property that makes it worth
    /// reading: that everything in it wants doing something about.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub row_deciding_reads: Vec<ModelReads>,
    /// Only the models with something to say. A clean model is not news.
    pub models: Vec<ModelReport>,
}

#[derive(Serialize)]
pub struct ModelReads {
    pub name: String,
    pub unique_id: String,
    pub columns: Vec<IndirectReport>,
    /// Names a `QUALIFY` reads that no single source of its `SELECT` has, so
    /// they are read by nobody rather than by every candidate.
    #[serde(skip_serializing_if = "is_zero")]
    pub unplaced_qualify: usize,
    /// The same for a `JOIN ... ON`.
    #[serde(skip_serializing_if = "is_zero")]
    pub unplaced_join: usize,
    /// The same for a `WHERE` or `HAVING`.
    #[serde(skip_serializing_if = "is_zero")]
    pub unplaced_filter: usize,
}

#[derive(Serialize)]
pub struct CatalogElsewhere {
    pub unique_id: String,
    /// The table the manifest names.
    pub relation: String,
    /// The table the catalog entry describes.
    pub catalog_relation: String,
}

#[derive(Serialize)]
pub struct RelationCollision {
    pub relation: String,
    /// Every dbt node claiming it, sorted. The first is the one a model that
    /// depends on both, or on neither, reads.
    pub nodes: Vec<String>,
}

#[derive(Serialize, Default)]
pub struct Totals {
    pub models: usize,
    pub parsed: usize,
    pub parse_failed: usize,
    /// Models the manifest has no `compiled_code` for: a `parse` wrote it, or a
    /// `run` that did not select them (0033).
    pub without_compiled_code: usize,
    /// Of those, the models whose SQL is the file a compile left in the
    /// `compiled/` directory beside the manifest. Such a file can be older than
    /// the manifest, and compiled for another target.
    pub sql_from_files: usize,
    /// Of those, the ones whose file reads a relation the manifest does not
    /// give the model, compiled against another graph, so set aside: their
    /// edges are inferred. See `ModelReport::sql_file_set_aside`.
    pub sql_from_files_set_aside: usize,
    /// Computed columns match the warehouse: the compiled SQL is representative.
    pub confirmed: usize,
    /// Computed columns disagree with the warehouse: this compile is degraded.
    pub degraded: usize,
    /// No catalog entry, so nothing could be checked.
    pub unchecked: usize,
    /// Catalog entries set aside for describing another table, whose models
    /// count as unchecked. See `Report::catalog_elsewhere`.
    pub catalog_elsewhere: usize,
    /// Degraded models whose compile kept its parsed edges on the columns the
    /// warehouse has too. See `Plan::PerColumn` in the pass.
    pub per_column: usize,
    /// Columns those compiles produce and the warehouse lacks, emitted on no
    /// rung. Each model names its own under `unexpected_columns`.
    pub columns_withheld: usize,
    pub edges_parsed: usize,
    pub edges_inferred: usize,
    /// Coverage against whichever list a downstream model would see, which for
    /// a model with no catalog entry is the one this run computed. Kept because
    /// it is the figure earlier runs printed, not because it is the honest one.
    pub columns_covered: usize,
    pub columns_total: usize,
    /// Coverage against a list collin did not produce: the warehouse where there
    /// is one, the YAML otherwise. On a project where 2756 of 3341 models have
    /// no catalog entry, the pair above measures the compile against itself.
    pub columns_covered_independent: usize,
    pub columns_total_independent: usize,
    /// Columns built from literals, which have no parent to find. Held out of
    /// the independent denominator above, because a root cannot be covered, and
    /// published here so the raw count is still recoverable.
    pub columns_root: usize,
    /// Columns of models that read no relation but their own: rows the model
    /// carries over, a table something outside dbt writes, and the literals
    /// beside them. No parent in dbt to find either, so held out of the
    /// independent denominator as the roots are (0039). See
    /// `ModelReport::reads_only_itself`.
    #[serde(skip_serializing_if = "is_zero")]
    pub columns_self: usize,
    /// YAML disagrees with the warehouse. A documentation finding, not a
    /// lineage one, reported because nothing else in the project looks.
    pub yaml_stale: usize,
    /// The compile produced far fewer columns than the project documents, with
    /// no catalog to arbitrate. See `ModelReport::thin`.
    pub thin: usize,
    /// Edges for a column read to decide which rows exist. Counted apart from
    /// `edges_parsed` because they claim something different: which rows exist,
    /// not what a value is. Zero unless the run asked for them.
    pub edges_indirect: usize,
    /// Columns read to decide which rows exist, counted once per model, column
    /// and role rather than once per output column they were fanned out to.
    /// This is the number of facts; `edges_indirect` is what it costs to draw
    /// them.
    pub row_deciding_reads: usize,
    /// Of those, the ones the model never carries through to an output: the
    /// lineage no direct edge holds.
    pub read_not_projected: usize,
    /// Models whose own relation the engine said it could not find. Counted here
    /// because such an issue alone keeps a model out of `models`, so this is the
    /// only place the count survives. See `ReportIssue::self_reference`.
    pub self_reads: usize,
    /// Models whose SQL reads a relation dbt was not told of. See
    /// `ModelReport::undeclared_relations`.
    pub undeclared: usize,
    /// Output columns of trusted models that came out with no edge and are not
    /// built from literals. See `ModelReport::lost_columns`.
    pub columns_lost: usize,
    /// Models whose SQL gives one name to two scopes. See
    /// `ModelReport::merged_scopes`.
    pub scope_merged: usize,
    /// Names read from a CTE that does not project them, once per model and
    /// name. See `ModelReport::unbacked`.
    pub unbacked_reads: usize,
    /// Columns trusted children read from a model whose list lacked them, once
    /// per model and column. See `ModelReport::read_downstream_but_absent`.
    pub read_downstream_but_absent: usize,
    /// `QUALIFY` names read by nobody. See `ModelReads::unplaced_qualify`.
    pub qualify_unplaced: usize,
    /// Columns a trusted compile reads through a `select *` over a parent whose
    /// list lacks them, and which another list of that parent has: resolved
    /// again with the column, the edge goes out and the read is filed on the
    /// parent under `read_downstream_but_absent`. One per child and column.
    pub read_through_star: usize,
    /// The same reads where no list of the parent has the column, left lost.
    pub read_through_star_unwitnessed: usize,
    /// Inferred columns several parents could have given, by how each was
    /// settled. See `ModelReport::ambiguous_inferred`.
    pub ambiguous_by_compile: usize,
    pub ambiguous_by_every: usize,
    pub ambiguous_by_order: usize,
    /// Join condition names read by nobody. See `ModelReads::unplaced_join`.
    pub join_unplaced: usize,
    /// Filter names read by nobody. See `ModelReads::unplaced_filter`.
    pub filter_unplaced: usize,
}

#[derive(Serialize)]
pub struct ModelReport {
    pub name: String,
    pub unique_id: String,
    pub file: String,
    /// The compiled file the SQL was read from, under the target directory,
    /// when the manifest had no `compiled_code` for the model (0033).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql_file: Option<String>,
    /// That file reads a relation the manifest does not give the model, listed
    /// under `undeclared_relations`: it was compiled for another target, or
    /// before a `ref` moved, and its edges would name tables no node owns. The
    /// compile is set aside, and the edges are inferred.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub sql_file_set_aside: bool,
    /// The SQL reads no relation but the model's own: an incremental model
    /// that only adds to itself, or a table something outside dbt writes and
    /// dbt only creates. Its columns have no parent in dbt, and none is listed
    /// as lost (0039). Declaring such a table as a source says so to dbt too.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub reads_only_itself: bool,
    /// parsed, per_column, inferred or unresolved.
    pub provenance: &'static str,
    pub agreement: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<ReportIssue>,
    /// Parents with no column list known for them, so the engine was never told
    /// of them. The actionable list: give each one its columns.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unknown_relations: Vec<String>,
    /// Relations the SQL reads that are neither the model nor anything dbt says
    /// it depends on: a raw table no source declares, or a node read without
    /// `ref`. Apart from `unknown_relations` because the remedy differs: here it
    /// is to declare the dependency, not to document a parent.
    ///
    /// Not exhaustive. SQL that did not parse reads nothing here, and a table
    /// named in one part cannot be told apart from a dependency. Nor exact the
    /// other way: a dependency or the model itself named in two parts where dbt
    /// wrote three is compared as written and listed here, since the database
    /// that would complete it is the session's and supplying it would be a guess.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub undeclared_relations: Vec<String>,
    /// Columns the warehouse has that this compile did not produce.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_columns: Vec<String>,
    /// Columns this compile produced that the warehouse does not have.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unexpected_columns: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub yaml_stale: bool,
    /// Set when the compile produced far fewer columns than the YAML documents
    /// and no catalog exists to say which of the two is right.
    ///
    /// Not an `Agreement` rung on purpose: the edges such a compile gives are
    /// still the ones its SQL supports. Either the YAML is stale, or a macro
    /// that reads the warehouse at compile time found its source missing and
    /// the compile came out short, which is what the cases read by hand turned
    /// out to be (0019).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thin: Option<Thin>,
    /// Output columns of a trusted compile that came out with no edge, and are
    /// not built from literals: lineage the SQL has and the engine lost. Each
    /// says where the walk back from it stopped.
    ///
    /// Only for a compile the plan trusted. A compile set aside is already in
    /// this list for that, and its edges come from inference, so what its SQL
    /// lost says nothing about what the cache holds.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lost_columns: Vec<LostColumn>,
    /// Names the SQL gives to two derived tables, or two CTEs, of one
    /// statement. The engine reads them apart since the fork's seventh patch,
    /// so this is a fact about the SQL, not a fault (0018).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub merged_scopes: Vec<MergedScopeReport>,
    /// Names the SQL reads from a CTE whose select list does not have them, so
    /// it cannot run as compiled: usually a macro that reads the warehouse at
    /// compile time and found the relation missing from it. Each with the
    /// relations under that CTE that do have the name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unbacked: Vec<UnbackedReport>,
    /// Inferred edges in a model whose other edges are parsed: an unbacked read
    /// with exactly one relation having the name, or a column only the table of
    /// a `per_column` model has.
    #[serde(skip_serializing_if = "is_zero")]
    pub edges_inferred: usize,
    /// Columns a trusted child reads from this model, with an edge, that the
    /// list collin had for this model lacks. The child's SQL is evidence the
    /// column exists; `against` says which list was short and `in_compile`
    /// whether this model's own compile has the column, so a reader can tell a
    /// short compile from an older table or a YAML behind the code.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub read_downstream_but_absent: Vec<ReadDownstream>,
    /// Inferred columns more than one parent has, each with the parents it was
    /// taken from and how: `compile`, the set aside compile names them;
    /// `every`, there was no reading and each candidate is taken; `order`, the
    /// first in the manifest. The `every` rows are the ones to read by hand: a
    /// union is right to take them all, a lookup join is not.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ambiguous_inferred: Vec<AmbiguousInferred>,
    pub edges: usize,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct AmbiguousInferred {
    pub column: String,
    pub candidates: Vec<String>,
    pub chosen: Vec<String>,
    pub by: &'static str,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ReadDownstream {
    pub column: String,
    /// `warehouse`, `computed` or `declared`: the list the child was compiled
    /// against, as collin had it.
    pub against: &'static str,
    pub in_compile: bool,
    /// The children reading it, by unique_id, sorted.
    pub read_by: Vec<String>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

#[derive(Serialize, Debug, PartialEq)]
pub struct UnbackedReport {
    pub cte: String,
    pub column: String,
    pub owners: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct MergedScopeReport {
    /// `derived` or `cte`.
    pub kind: &'static str,
    pub name: String,
    pub count: usize,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct LostColumn {
    pub column: String,
    pub ends: Vec<LostEnd>,
}

/// Where the walk back from a lost column stopped.
#[derive(Serialize, Debug, PartialEq)]
pub struct LostEnd {
    /// `phantom`: copied from a CTE that does not have the column, the shape of
    /// a `select *` over a parent whose column list lacks it. `names_unreached`:
    /// an expression reads these names and the engine never placed them, inside
    /// a form it does not read or unresolved. `unresolved`: anything else.
    pub reason: &'static str,
    /// The CTE, for `phantom`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// Every relation that CTE reads, for `phantom`: never one picked out, and
    /// each with where the list of columns handed to the engine for it came
    /// from, since a short list is the usual cause.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<LostRelation>,
    /// For `names_unreached`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct LostRelation {
    pub relation: String,
    /// `warehouse`, `computed`, `declared`, or `unknown` for a relation the
    /// model does not depend on.
    pub columns_from: &'static str,
}

#[derive(Serialize)]
pub struct IndirectReport {
    pub relation: String,
    pub column: String,
    /// `join_key`, `dedup_key` or `filter`.
    pub role: &'static str,
    /// The model also has a parsed edge from this column: it decides which
    /// rows exist and is carried through as well.
    pub projected: bool,
}

/// What the two column lists actually said, so the reader can judge rather than
/// take the verdict on trust.
#[derive(Serialize)]
pub struct Thin {
    pub declared: usize,
    pub computed: usize,
    /// Names present in both.
    pub shared: usize,
}

impl Report {
    /// Kept apart from the pass, which never sees these entries: the catalog
    /// is read, and set aside, before it starts.
    pub fn set_catalog_elsewhere(&mut self, elsewhere: Vec<crate::catalog::Elsewhere>) {
        self.totals.catalog_elsewhere = elsewhere.len();
        self.catalog_elsewhere = elsewhere
            .into_iter()
            .map(|e| CatalogElsewhere {
                unique_id: e.unique_id,
                relation: e.relation,
                catalog_relation: e.catalog_relation,
            })
            .collect();
    }

    pub fn write(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }
}

/// Columns in `want` that are absent from `have`, both assumed lower case.
pub fn missing(want: &[String], have: &[String]) -> Vec<String> {
    let set: std::collections::HashSet<&str> = have.iter().map(String::as_str).collect();
    want.iter().filter(|c| !set.contains(c.as_str())).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn missing_is_a_plain_set_difference() {
        assert_eq!(missing(&v(&["a", "b", "c"]), &v(&["b"])), v(&["a", "c"]));
        assert!(missing(&v(&["a"]), &v(&["a", "b"])).is_empty());
        assert!(missing(&[], &v(&["a"])).is_empty());
    }

    #[test]
    fn a_clean_model_is_left_out_of_the_json() {
        let r = Report::default();
        let text = serde_json::to_string(&r).unwrap();
        assert!(text.contains("\"models\":[]"), "{text}");
    }

    #[test]
    fn only_a_self_reference_says_so() {
        let issue = |self_reference| ReportIssue {
            code: "UNRESOLVED_REFERENCE".into(),
            message: String::new(),
            severity: "warning",
            span: None,
            self_reference,
        };
        let plain = serde_json::to_string(&issue(false)).unwrap();
        assert!(!plain.contains("self_reference"), "{plain}");
        let own = serde_json::to_string(&issue(true)).unwrap();
        assert!(own.contains("\"self_reference\":true"), "{own}");
    }

    #[test]
    fn an_undeclared_relation_is_named_only_when_there_is_one() {
        let model = |undeclared_relations: Vec<String>| ModelReport {
            name: "m".into(),
            unique_id: "model.p.m".into(),
            file: String::new(),
            sql_file: None,
            sql_file_set_aside: false,
            reads_only_itself: false,
            provenance: "parsed",
            agreement: "unchecked",
            parse_error: None,
            issues: Vec::new(),
            unknown_relations: Vec::new(),
            undeclared_relations,
            missing_columns: Vec::new(),
            unexpected_columns: Vec::new(),
            yaml_stale: false,
            thin: None,
            lost_columns: Vec::new(),
            merged_scopes: Vec::new(),
            unbacked: Vec::new(),
            edges_inferred: 0,
            read_downstream_but_absent: Vec::new(),
            ambiguous_inferred: Vec::new(),
            edges: 0,
        };
        let none = serde_json::to_string(&model(Vec::new())).unwrap();
        assert!(!none.contains("lost_columns"), "an entry with nothing lost reads as before: {none}");
        assert!(!none.contains("undeclared_relations"), "{none}");
        assert!(!none.contains("sql_file"), "SQL from the manifest reads as before: {none}");
        let one = serde_json::to_string(&model(v(&["RAW_DB.DBO.ORDERS_BASE"]))).unwrap();
        assert!(one.contains("\"undeclared_relations\":[\"RAW_DB.DBO.ORDERS_BASE\"]"), "{one}");
    }

    #[test]
    fn a_column_read_but_not_projected_is_not_listed_as_a_fault() {
        // `models` is the list of things to go and fix. Reading a join key is
        // not one of them, so it lives in its own section and a model that only
        // does that stays out of the fault list.
        let r = Report {
            row_deciding_reads: vec![ModelReads {
                name: "m".into(),
                unique_id: "model.p.m".into(),
                columns: vec![IndirectReport {
                    relation: "DB.SCH.T".into(),
                    column: "id".into(),
                    role: "join_key",
                    projected: false,
                }],
                unplaced_qualify: 0,
                unplaced_join: 0,
                unplaced_filter: 0,
            }],
            ..Default::default()
        };
        let text = serde_json::to_string(&r).unwrap();
        assert!(text.contains("\"models\":[]"), "{text}");
        assert!(text.contains("\"row_deciding_reads\":[{"), "{text}");
    }
}
