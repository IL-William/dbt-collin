//! Dev only: what the engine saw of each model, one JSON file per model.
//!
//! The cache and the report keep what the provenance ladder let through, so a
//! model whose compile was set aside shows name matches there and nothing of
//! what its SQL said. Finding out why takes the engine's own view: each parent
//! with the columns it was handed and where they came from, every relation the
//! SQL reads, the outputs and the roots, every edge with its expression, the
//! join, filter and dedup keys, the issues, the agreement and the plan the pass
//! chose, and the compiled file the SQL came from when the manifest had none.
//! Next to it, the edges the model put in the cache, as they were written.
//!
//! It watches `lineage::run_observed` rather than replaying the pass. Where an
//! edge comes from and what role it plays are the pass's decisions, so they are
//! shown only on the edges it published, as it published them, and never worked
//! out here again: a change to how the pass attributes an edge reaches this file
//! without a line of it changing.
//!
//! ```text
//! cargo run --release -p collin-core --example dump -- --project DIR --out DIR
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use collin_core::lineage::{self, Loaded, Options, Seen};
use collin_core::engine::DeadEnd;
use collin_core::manifest::norm_relation;
use serde_json::{json, Value};

const USAGE: &str = "\
USAGE
    cargo run --release -p collin-core --example dump -- --out DIR [options]

OPTIONS
    --out DIR         where to write <model>.json, absent or empty
    --project DIR     dbt project root (default: .)
    --manifest PATH   default: <project>/target/manifest.json
    --catalog PATH    default: <project>/target/catalog.json
    --no-infer        as for generate. Only `published` and
    --indirect        `published_indirect` depend on them.
";

fn main() -> ExitCode {
    match dump() {
        Ok((models, out)) => {
            println!("{models} models in {}", out.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("dump: {e}");
            ExitCode::FAILURE
        }
    }
}

fn dump() -> Result<(usize, PathBuf), String> {
    let (mut root, mut manifest, mut catalog, mut out) = (PathBuf::from("."), None, None, None);
    let (mut infer, mut indirect) = (true, false);
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().map(PathBuf::from).ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--out" => out = Some(value()?),
            "--project" => root = value()?,
            "--manifest" => manifest = Some(value()?),
            "--catalog" => catalog = Some(value()?),
            "--no-infer" => infer = false,
            "--indirect" => indirect = true,
            _ => return Err(format!("unknown flag {flag}\n\n{USAGE}")),
        }
    }
    let out = out.ok_or(format!("--out is required\n\n{USAGE}"))?;
    // Only the inputs matter: the cache and the report are never written here.
    let mut opts = Options::new(root);
    opts.manifest = manifest;
    opts.catalog = catalog;
    opts.infer = infer;
    opts.indirect = indirect;

    // A model that disappeared would otherwise leave its old file behind, and a
    // diff of two dumps would read it as current.
    if std::fs::read_dir(&out).is_ok_and(|mut d| d.next().is_some()) {
        return Err(format!("{} is not empty", out.display()));
    }
    let Loaded { project, warehouse, .. } = lineage::load(&opts)?;
    std::fs::create_dir_all(&out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;

    // A versioned model shares its name with its other versions, so those take
    // their unique_id rather than overwrite one another.
    let mut names: HashMap<&str, usize> = HashMap::new();
    for m in project.nodes.values().filter(|n| n.is_model()) {
        *names.entry(m.name.as_str()).or_default() += 1;
    }

    let (mut written, mut failed) = (0, None);
    lineage::run_observed(&project, warehouse, &opts, |seen| {
        if failed.is_some() {
            return;
        }
        let m = seen.model;
        let file = if names[m.name.as_str()] > 1 { &m.uid } else { &m.name };
        let path = out.join(format!("{file}.json"));
        let text = serde_json::to_string_pretty(&describe(seen));
        let wrote = text.map_err(|e| e.to_string()).and_then(|t| {
            std::fs::write(&path, t).map_err(|e| e.to_string())
        });
        match wrote {
            Ok(()) => written += 1,
            Err(e) => failed = Some(format!("cannot write {}: {e}", path.display())),
        }
    });
    match failed {
        Some(e) => Err(e),
        None => Ok((written, out)),
    }
}

fn describe(seen: &Seen) -> Value {
    let (m, r) = (seen.model, seen.resolved);

    // An output no engine edge reaches and no literal explains: usually the gap
    // a reader opened this file to find. The engine's own list, with where the
    // walk back from each stopped, so this file cannot disagree with the report.
    let lost: Vec<&String> = r.lost.iter().map(|l| &l.column).collect();
    let lost_ends: Vec<Value> = r
        .lost
        .iter()
        .map(|l| {
            let ends: Vec<Value> = l
                .ends
                .iter()
                .map(|d| match d {
                    DeadEnd::Phantom { via, column, reads, star } => {
                        json!({"reason": "phantom", "via": via, "column": column, "reads": reads, "star": star})
                    }
                    DeadEnd::NamesUnreached { names } => json!({"reason": "names_unreached", "names": names}),
                    DeadEnd::Unresolved => json!({"reason": "unresolved"}),
                })
                .collect();
            json!({"column": l.column, "ends": ends})
        })
        .collect();

    let parents: Vec<Value> = seen
        .parents
        .iter()
        .map(|p| {
            json!({
                "uid": p.node.uid,
                "relation": norm_relation(&p.node.relation),
                "provenance": p.provenance.as_str(),
                "columns": p.columns,
            })
        })
        .collect();

    // Every edge the engine gave, whether or not the plan published it, as the
    // engine gave it. Its node and its role are in `published` when it went out.
    let edges: Vec<Value> = r
        .edges
        .iter()
        .map(|e| {
            json!({
                "from_relation": e.from_relation,
                "from_col": e.from_column,
                "to_col": e.to_column,
                "expression": e.expression,
            })
        })
        .collect();

    let indirect: Vec<Value> = r
        .indirect
        .iter()
        .map(|i| json!({"relation": i.relation, "column": i.column, "role": i.role.as_str()}))
        .collect();

    let issues: Vec<Value> = r
        .issues
        .iter()
        .map(|i| {
            json!({
                "code": i.code,
                "message": i.message,
                "severity": i.severity,
                "span": i.span,
                "degrading": i.degrading,
                "relation": i.relation,
            })
        })
        .collect();

    json!({
        "uid": m.uid,
        "name": m.name,
        "materialized": m.materialized,
        "sql_file": m.sql_source.file(),
        "plan": seen.plan.as_str(),
        "silent": seen.silent,
        "foreign": seen.foreign,
        "agreement": seen.agreement.as_str(),
        "parse_error": r.parse_error,
        "approximate": r.approximate,
        "outputs": r.outputs,
        "roots": r.roots,
        "lost_outputs": lost,
        "lost_ends": lost_ends,
        "unbacked": r.unbacked.iter().map(|u| json!({"output": u.output, "cte": u.cte, "column": u.column, "owners": u.owners})).collect::<Vec<_>>(),
        "unknown_columns": r.unknown_columns.iter().map(|u| json!({"relation": u.relation, "column": u.column})).collect::<Vec<_>>(),
        "merged_scopes": r.merged_scopes.iter().map(|m| json!({"kind": m.kind, "name": m.name, "count": m.count})).collect::<Vec<_>>(),
        "warehouse": seen.store.warehouse(&m.uid),
        "declared": seen.store.declared(&m.uid),
        "visible": parents,
        "reads": r.reads,
        "edges": edges,
        "indirect": indirect,
        "issues": issues,
        "published": seen.published,
        "published_indirect": seen.published_indirect,
    })
}
