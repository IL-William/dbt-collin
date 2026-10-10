//! `collin`: write a column lineage cache for a dbt project.
//!
//! Arguments are parsed by hand. The flag set is small and fixed, and a parser
//! crate would be more build time than the whole binary is worth.

use std::path::PathBuf;
use std::process::ExitCode;

use collin_core::lineage::{generate, Options};

const USAGE: &str = "\
collin - column level lineage for dbt, from compiled SQL

USAGE
    collin generate [--project DIR] [options]

OPTIONS
    --project DIR     dbt project root (default: .)
    --manifest PATH   default: <project>/target/manifest.json. A model it has
                      no compiled_code for is read from compiled/ beside it.
    --catalog PATH    default: <project>/target/catalog.json
    --out PATH        default: <project>/target/column_lineage.json
    --report PATH     default: <project>/target/column_lineage.report.json
    --target NAME     recorded in the cache, shown by dbt-lens
    --no-infer        leave degraded models empty rather than name matching
    --indirect        also emit join keys, dedup keys and filter predicates,
                      one edge per output column: on this corpus that more
                      than doubles the cache. The report lists them either way.
    -h, --help
    -V, --version     the version, and the commit it was built from

EXIT
    0  a cache was written
    1  it could not be written
";

/// `collin 0.1.0 (9e8d7c6)`: one line, the shape dbt-lens reads to decide
/// whether this collin is recent enough for it (0034).
fn version_line() -> String {
    let commit = env!("COLLIN_COMMIT");
    if commit.is_empty() {
        format!("collin {}", env!("CARGO_PKG_VERSION"))
    } else {
        format!("collin {} ({commit})", env!("CARGO_PKG_VERSION"))
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args[0] == "-V" || args[0] == "--version" {
        println!("{}", version_line());
        return ExitCode::SUCCESS;
    }
    if args[0] != "generate" {
        eprintln!("unknown command {:?}\n", args[0]);
        print!("{USAGE}");
        return ExitCode::from(2);
    }

    let mut project = PathBuf::from(".");
    let mut manifest = None;
    let mut catalog = None;
    let mut out = None;
    let mut report = None;
    let mut target = String::new();
    let mut infer = true;
    let mut indirect = false;

    let mut i = 1;
    while i < args.len() {
        let need = |i: usize, what: &str| -> Result<String, String> {
            args.get(i + 1).cloned().ok_or_else(|| format!("{what} needs a value"))
        };
        let step = match args[i].as_str() {
            "--project" => match need(i, "--project") {
                Ok(v) => {
                    project = PathBuf::from(v);
                    2
                }
                Err(e) => return fail(&e),
            },
            "--manifest" => match need(i, "--manifest") {
                Ok(v) => {
                    manifest = Some(PathBuf::from(v));
                    2
                }
                Err(e) => return fail(&e),
            },
            "--catalog" => match need(i, "--catalog") {
                Ok(v) => {
                    catalog = Some(PathBuf::from(v));
                    2
                }
                Err(e) => return fail(&e),
            },
            "--out" => match need(i, "--out") {
                Ok(v) => {
                    out = Some(PathBuf::from(v));
                    2
                }
                Err(e) => return fail(&e),
            },
            "--report" => match need(i, "--report") {
                Ok(v) => {
                    report = Some(PathBuf::from(v));
                    2
                }
                Err(e) => return fail(&e),
            },
            "--target" => match need(i, "--target") {
                Ok(v) => {
                    target = v;
                    2
                }
                Err(e) => return fail(&e),
            },
            "--no-infer" => {
                infer = false;
                1
            }
            "--indirect" => {
                indirect = true;
                1
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => return fail(&format!("unknown flag {other}")),
        };
        i += step;
    }

    let mut opts = Options::new(project);
    opts.manifest = manifest;
    opts.catalog = catalog;
    opts.target = target;
    opts.infer = infer;
    opts.indirect = indirect;
    if let Some(p) = out {
        opts.out = p;
    }
    if let Some(p) = report {
        opts.report = p;
    }

    let started = std::time::Instant::now();
    match generate(&opts) {
        Ok(o) => {
            let t = o.totals;
            let pct = |a: usize, b: usize| if b == 0 { 0.0 } else { 100.0 * a as f64 / b as f64 };
            println!();
            println!("  {} models in {} ms", t.models, started.elapsed().as_millis());
            println!("    parsed                  {} ({} failed)", t.parsed, t.parse_failed);
            // Printed whenever the manifest lacked compiled_code somewhere, so a
            // lookup that found nothing shows as 0 rather than not at all.
            if t.without_compiled_code > 0 {
                let set_aside = match t.sql_from_files_set_aside {
                    0 => String::new(),
                    n => format!(" ({n} set aside, compiled against another graph)"),
                };
                println!(
                    "    SQL from compiled files {} of {} without compiled_code{set_aside}",
                    t.sql_from_files, t.without_compiled_code
                );
            }
            println!(
                "    confirmed by catalog    {}  {:.1}%",
                t.confirmed,
                pct(t.confirmed, t.models)
            );
            println!("    degraded compile        {} ({} kept per column)", t.degraded, t.per_column);
            println!("    unchecked, no catalog   {}", t.unchecked);
            println!();
            println!("  {} edges", t.edges_parsed + t.edges_inferred + t.edges_indirect);
            println!("    parsed                  {}", t.edges_parsed);
            println!("    inferred                {}", t.edges_inferred);
            if t.edges_indirect > 0 {
                println!("    indirect, row choosing  {}", t.edges_indirect);
            }
            println!(
                "    columns covered         {} / {}  {:.1}%",
                t.columns_covered,
                t.columns_total,
                pct(t.columns_covered, t.columns_total)
            );
            // The line above measures a model with no catalog entry against the
            // list this run computed for it. This one uses a reference collin
            // did not produce, which is the figure to trust.
            println!(
                "    against warehouse/yaml  {} / {}  {:.1}%",
                t.columns_covered_independent,
                t.columns_total_independent,
                pct(t.columns_covered_independent, t.columns_total_independent)
            );
            println!("    roots, no parent to find {}", t.columns_root);
            if t.yaml_stale > 0 {
                println!("\n  {} models whose YAML disagrees with the warehouse", t.yaml_stale);
            }
            if t.row_deciding_reads > 0 {
                println!(
                    "  {} columns read to decide which rows exist, {} never projected{}",
                    t.row_deciding_reads,
                    t.read_not_projected,
                    if t.edges_indirect > 0 { "" } else { ", in the report only" }
                );
            }
            if t.thin > 0 {
                println!("  {} models whose compile is far thinner than their YAML", t.thin);
            }
            if t.undeclared > 0 {
                println!("  {} models reading a relation dbt was not told of", t.undeclared);
            }
            if t.columns_withheld > 0 {
                println!(
                    "  {} columns compiled but absent from the warehouse, withheld, named in the report",
                    t.columns_withheld
                );
            }
            if t.columns_lost > 0 {
                println!(
                    "  {} columns of trusted models came out with no edge, named in the report",
                    t.columns_lost
                );
            }
            println!("\n  cache   {}", o.out.display());
            println!("  report  {}", o.report.display());
            if t.without_compiled_code > 0 {
                println!("  sql     {}", opts.target_dir().join("compiled").display());
            }
            println!();
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("collin: {msg}");
    ExitCode::FAILURE
}
