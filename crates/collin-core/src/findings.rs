//! What the report's models say, grouped by what someone can do about it.
//!
//! `models` lists every model with something to say, field by field, which is
//! what a tool needs and not what a person reads. A finding gathers the models
//! that share one cause, says in words what collin saw and what it usually
//! means, and what to do (0040). It adds no fact: every finding is read off the
//! report, so the two can never disagree, and a finding is never a guess the
//! report does not back. Where the cause is a likelihood rather than a fact,
//! the words say so.
//!
//! Who can act decides the order: the files collin was handed first, since
//! they make every other figure wrong, then the project, then collin itself.

use serde::Serialize;

use crate::report::{ModelReport, Report};

/// The models that share one cause.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Finding {
    /// Stable, for a reader that groups, hides or counts them.
    pub category: &'static str,
    /// Who can act: `inputs`, the manifest and catalog collin was handed;
    /// `project`, the dbt project or the tables its target holds; `collin`, a
    /// limit of collin or its engine.
    pub owner: &'static str,
    /// `error` when lineage is missing because of it, `warning` when it is
    /// partial or unchecked, `info` when nothing is lost.
    pub severity: &'static str,
    pub title: String,
    /// What collin saw, and what it usually means.
    pub why: &'static str,
    pub action: &'static str,
    /// Columns without lineage, or kept from the cache, because of it, where
    /// the report counts them.
    #[serde(skip_serializing_if = "is_zero")]
    pub columns: usize,
    pub models: Vec<FindingModel>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FindingModel {
    pub unique_id: String,
    pub name: String,
    /// What this model's entry says, in a line.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

const OWNERS: [&str; 3] = ["inputs", "project", "collin"];
const SEVERITIES: [&str; 3] = ["error", "warning", "info"];

/// Every finding the report backs, most urgent first.
pub fn of(report: &Report) -> Vec<Finding> {
    let mut out = Vec::new();

    if !report.catalog_elsewhere.is_empty() {
        let name_of = |uid: &str| uid.rsplit('.').next().unwrap_or(uid).to_string();
        out.push(Finding {
            category: "catalog_of_another_target",
            owner: "inputs",
            severity: "error",
            title: count(report.catalog_elsewhere.len(), "catalog entry describes", "catalog entries describe")
                + " another table than the manifest's",
            why: "catalog.json was generated on another target than the manifest, or in another \
                  environment: these entries name other tables than the ones the manifest gives \
                  their models, so they are not used, and those models are checked against nothing.",
            action: "Generate catalog.json with the manifest's target and its environment, in the \
                     same run as the manifest: dbt docs generate --target <target>.",
            columns: 0,
            models: report
                .catalog_elsewhere
                .iter()
                .map(|e| FindingModel {
                    unique_id: e.unique_id.clone(),
                    name: name_of(&e.unique_id),
                    detail: format!("manifest: {}, catalog: {}", e.relation, e.catalog_relation),
                })
                .collect(),
        });
    }

    group(&mut out, report, Group {
        category: "compiled_file_from_another_graph",
        owner: "inputs",
        severity: "warning",
        one: "compiled file was compiled for another target",
        many: "compiled files were compiled for another target",
        why: "The manifest has no SQL for these models, so collin read target/compiled, and those \
              files read relations the manifest does not give the models: they come from another \
              target or older code. Their edges are inferred by name.",
        action: "Compile the project for this target so the manifest carries each model's SQL: \
                 dbt compile --target <target>.",
        pick: |m| m.sql_file_set_aside.then(|| m.sql_file.clone().unwrap_or_default()),
        columns: |_| 0,
    });

    group(&mut out, report, Group {
        category: "compile_does_not_parse",
        owner: "project",
        severity: "error",
        one: "compile does not parse",
        many: "compiles do not parse",
        why: "The SQL dbt compiled for these models is not valid SQL, so none of their columns has \
              parsed lineage. A compile that reads `select ,` or `select from` is usually a macro \
              that listed a relation's columns at compile time and found none, because that \
              relation does not exist in this target. Many at once usually mean the target was \
              compiled against another environment's databases, without its environment \
              variables for instance.",
        action: "Build the relations those macros read in this target before compiling, or make \
                 the macros fail on an empty column list rather than write invalid SQL.",
        pick: |m| m.parse_error.clone(),
        columns: |_| 0,
    });

    group(&mut out, report, Group {
        category: "reads_a_name_its_cte_lacks",
        owner: "project",
        severity: "warning",
        one: "compile reads a name its own CTE does not have",
        many: "compiles read a name their own CTE does not have",
        why: "These compiles read a name from a CTE that does not project it, so they cannot run \
              as compiled. Usually a macro listed a relation's columns at compile time and found \
              the relation missing, or found an older version of it.",
        action: "Build the relation the macro reads in this target, then compile again.",
        pick: |m| some_list(m.unbacked.iter().map(|u| format!("{}.{}", u.cte, u.column))),
        columns: |m| m.unbacked.len(),
    });

    group(&mut out, report, Group {
        category: "relation_not_declared",
        owner: "project",
        severity: "warning",
        one: "model reads a relation dbt was not told of",
        many: "models read a relation dbt was not told of",
        why: "These models read a table dbt does not know: a raw table no source declares, or a \
              model named without ref(). Its edges point at a table no node owns, so the lineage \
              stops there, and dbt cannot order the build around it.",
        action: "Declare the table as a source and read it with source(), or read the model \
                 with ref().",
        pick: |m| some_list(m.undeclared_relations.iter().cloned()),
        columns: |_| 0,
    });

    group(&mut out, report, Group {
        category: "parent_without_columns",
        owner: "project",
        severity: "warning",
        one: "model reads a parent with no known columns",
        many: "models read a parent with no known columns",
        why: "A relation these models read has no catalog entry and no columns in its YAML, so \
              the engine was never told what it holds and cannot place what is read from it.",
        action: "Build the parent in this target and generate catalog.json again, or document \
                 its columns in YAML.",
        pick: |m| some_list(m.unknown_relations.iter().cloned()),
        columns: |_| 0,
    });

    group(&mut out, report, Group {
        category: "table_older_than_code",
        owner: "project",
        severity: "warning",
        one: "table has columns the code no longer writes",
        many: "tables have columns the code no longer writes",
        why: "The table has columns the current SQL does not produce: it was built by an earlier \
              version of the model and not rebuilt since. Those columns get no edge, because no \
              SQL writes them any more. Many at once usually mean the manifest and the catalog \
              were made against an environment whose tables are not kept up to date.",
        action: "Rebuild these models in this target: dbt run --full-refresh -s <model>.",
        // Not a model that does not parse or reads only itself: its missing
        // columns are what that finding already says.
        pick: |m| match m.parse_error.is_some() || m.reads_only_itself {
            true => None,
            false => some_list(m.missing_columns.iter().cloned()),
        },
        columns: |m| m.missing_columns.len(),
    });

    group(&mut out, report, Group {
        category: "code_ahead_of_table",
        owner: "project",
        severity: "warning",
        one: "table lacks columns the code writes",
        many: "tables lack columns the code writes",
        why: "The current SQL produces columns the table does not have: the code changed and the \
              table was not rebuilt. Their edges are withheld, since no table holds them.",
        action: "Rebuild these models in this target: dbt run -s <model>.",
        pick: |m| some_list(m.unexpected_columns.iter().cloned()),
        columns: |m| m.unexpected_columns.len(),
    });

    group(&mut out, report, Group {
        category: "thin_compile",
        owner: "project",
        severity: "info",
        one: "compile produces far fewer columns than its YAML documents",
        many: "compiles produce far fewer columns than their YAML documents",
        why: "With no catalog to say which is right, either the YAML documents columns the model \
              no longer has, or a macro found its source missing at compile time and the compile \
              came out short.",
        action: "Compare the YAML with the model's SQL, and build its sources in this target.",
        pick: |m| match m.parse_error {
            Some(_) => None,
            None => m.thin.as_ref().map(|t| format!("{} of {} documented columns", t.computed, t.declared)),
        },
        columns: |_| 0,
    });

    group(&mut out, report, Group {
        category: "reads_only_itself",
        owner: "project",
        severity: "info",
        one: "model reads nothing but its own table",
        many: "models read nothing but their own table",
        why: "These models read no relation but their own: an incremental model that only adds to \
              itself, or a table a job outside dbt writes and dbt only creates. Their columns \
              have no parent in dbt, which is not a gap.",
        action: "Nothing to fix for lineage. A table written outside dbt can be declared as a \
                 source, which tells dbt what it is.",
        pick: |m| m.reads_only_itself.then(String::new),
        columns: |_| 0,
    });

    group(&mut out, report, Group {
        category: "yaml_out_of_date",
        owner: "project",
        severity: "info",
        one: "model's YAML disagrees with its table",
        many: "models' YAML disagrees with their table",
        why: "The columns documented in YAML are not the columns the table has. A documentation \
              finding: lineage does not depend on it where the catalog has the table.",
        action: "Update the columns documented in YAML.",
        pick: |m| m.yaml_stale.then(String::new),
        columns: |_| 0,
    });

    group(&mut out, report, Group {
        category: "lineage_collin_lost",
        owner: "collin",
        severity: "warning",
        one: "model has columns collin could not follow",
        many: "models have columns collin could not follow",
        why: "The SQL is trusted, and these columns came out with no edge and are not built from \
              literals: collin or its engine lost the way back. Each says where the walk stopped.",
        action: "Nothing to fix in the project. Worth reporting to collin when the SQL is \
                 ordinary.",
        pick: |m| {
            some_list(m.lost_columns.iter().map(|l| {
                let reasons: Vec<&str> = l.ends.iter().map(|e| e.reason).collect();
                format!("{} ({})", l.column, reasons.join(", "))
            }))
        },
        columns: |m| m.lost_columns.len(),
    });

    out.sort_by_key(|f| {
        let at = |list: &[&str], v: &str| list.iter().position(|x| *x == v).unwrap_or(list.len());
        (at(&OWNERS, f.owner), at(&SEVERITIES, f.severity))
    });
    out
}

/// One category read off the models: those for which `pick` says something.
struct Group {
    category: &'static str,
    owner: &'static str,
    severity: &'static str,
    /// The title is the count, then `one` or `many`.
    one: &'static str,
    many: &'static str,
    why: &'static str,
    action: &'static str,
    /// None when the model is not in this group, else its detail.
    pick: fn(&ModelReport) -> Option<String>,
    columns: fn(&ModelReport) -> usize,
}

fn group(out: &mut Vec<Finding>, report: &Report, g: Group) {
    let mut columns = 0;
    let mut models = Vec::new();
    for m in &report.models {
        if let Some(detail) = (g.pick)(m) {
            columns += (g.columns)(m);
            models.push(FindingModel { unique_id: m.unique_id.clone(), name: m.name.clone(), detail });
        }
    }
    if models.is_empty() {
        return;
    }
    models.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.unique_id.cmp(&b.unique_id)));
    out.push(Finding {
        category: g.category,
        owner: g.owner,
        severity: g.severity,
        title: count(models.len(), g.one, g.many),
        why: g.why,
        action: g.action,
        columns,
        models,
    });
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The items, comma separated, ten at most and how many more: None for none.
fn some_list(items: impl Iterator<Item = String>) -> Option<String> {
    let all: Vec<String> = items.collect();
    if all.is_empty() {
        return None;
    }
    let shown = all.iter().take(10).cloned().collect::<Vec<_>>().join(", ");
    Some(match all.len() {
        n if n > 10 => format!("{shown} and {} more", n - 10),
        _ => shown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{CatalogElsewhere, LostColumn, LostEnd};

    fn model(name: &str) -> ModelReport {
        ModelReport {
            name: name.into(),
            unique_id: format!("model.p.{name}"),
            provenance: "parsed",
            agreement: "confirmed",
            ..Default::default()
        }
    }

    #[test]
    fn each_cause_is_a_finding_with_its_models() {
        let mut broken = model("b_broken");
        broken.parse_error = Some("PARSE_ERROR: Expected an expression, found: ,".into());
        let mut also = model("a_also");
        also.parse_error = Some("PARSE_ERROR: Expected an expression, found: FROM".into());
        let mut behind = model("behind");
        behind.missing_columns = vec!["old_a".into(), "old_b".into()];
        let mut raw = model("raw");
        raw.undeclared_relations = vec!["DB.RAW.T".into()];
        let mut log = model("log");
        log.reads_only_itself = true;
        let report = Report { models: vec![broken, also, behind, raw, log], ..Default::default() };

        let found = of(&report);
        let shape: Vec<(&str, &str, &str, usize)> =
            found.iter().map(|f| (f.category, f.severity, f.title.as_str(), f.models.len())).collect();
        assert_eq!(
            shape,
            vec![
                ("compile_does_not_parse", "error", "2 compiles do not parse", 2),
                ("relation_not_declared", "warning", "1 model reads a relation dbt was not told of", 1),
                ("table_older_than_code", "warning", "1 table has columns the code no longer writes", 1),
                ("reads_only_itself", "info", "1 model reads nothing but its own table", 1),
            ]
        );
        // Sorted by name, each with its own entry's words.
        let parse = &found[0].models;
        assert_eq!(parse[0].name, "a_also");
        assert_eq!(parse[1].detail, "PARSE_ERROR: Expected an expression, found: ,");
        assert_eq!(found[2].columns, 2);
        assert_eq!(found[2].models[0].detail, "old_a, old_b");
    }

    #[test]
    fn the_inputs_come_before_the_project_and_collin_last() {
        let mut lost = model("lost");
        lost.lost_columns = vec![LostColumn {
            column: "x".into(),
            ends: vec![LostEnd { reason: "phantom", via: Some("c".into()), relations: vec![], names: vec![] }],
        }];
        let mut thin = model("thin");
        thin.yaml_stale = true;
        let report = Report {
            models: vec![lost, thin],
            catalog_elsewhere: vec![CatalogElsewhere {
                unique_id: "model.p.orders".into(),
                relation: "db.sales.orders".into(),
                catalog_relation: "DB.DBT_ME.ORDERS".into(),
            }],
            ..Default::default()
        };
        let order: Vec<&str> = of(&report).iter().map(|f| f.category).collect();
        assert_eq!(order, vec!["catalog_of_another_target", "yaml_out_of_date", "lineage_collin_lost"]);
        assert_eq!(of(&report)[2].models[0].detail, "x (phantom)");
    }

    #[test]
    fn a_clean_report_has_no_finding_and_a_long_list_is_cut() {
        assert!(of(&Report::default()).is_empty());
        let mut wide = model("wide");
        wide.missing_columns = (0..13).map(|i| format!("c{i:02}")).collect();
        let found = of(&Report { models: vec![wide], ..Default::default() });
        assert_eq!(found[0].models[0].detail, "c00, c01, c02, c03, c04, c05, c06, c07, c08, c09 and 3 more");
        assert_eq!(found[0].columns, 13);
    }
}
