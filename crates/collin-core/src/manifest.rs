//! Raw reading of a dbt `manifest.json`, and the indexes built from it.
//!
//! Only the fields this tool needs are declared, so serde discards the rest
//! while parsing: a 109 MB manifest must not become a 1 GB object graph. That is
//! the same rule `src/manifest.rs` follows in dbt-lens.
//!
//! Invariant: relation keys are uppercased everywhere. dbt writes
//! `db.schema.name` unquoted and lower case, the SQL engine reports it upper
//! case, and matching the two is the whole point of the index.

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Default)]
pub struct RawManifest {
    #[serde(default)]
    pub metadata: RawMetadata,
    #[serde(default)]
    pub nodes: HashMap<String, RawNode>,
    #[serde(default)]
    pub sources: HashMap<String, RawNode>,
    #[serde(default)]
    pub parent_map: HashMap<String, Vec<String>>,
}

#[derive(Deserialize, Default)]
pub struct RawMetadata {
    #[serde(default)]
    pub dbt_version: String,
    #[serde(default)]
    pub project_name: String,
    #[serde(default)]
    pub adapter_type: String,
}

#[derive(Deserialize, Clone)]
pub struct RawNode {
    pub name: String,
    pub resource_type: String,
    #[serde(default)]
    pub package_name: String,
    #[serde(default)]
    pub original_file_path: String,
    #[serde(default)]
    pub relation_name: Option<String>,
    /// Absent until dbt compiles the node, and present, if empty, once it has:
    /// a `parse` writes none, nor does a `run` for a node it did not select.
    #[serde(default)]
    pub compiled_code: Option<String>,
    /// `sql` or `python`. Absent before dbt 1.3, which had only SQL models.
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub columns: HashMap<String, RawColumn>,
    #[serde(default)]
    pub depends_on: RawDependsOn,
    #[serde(default)]
    pub config: RawConfig,
}

#[derive(Deserialize, Clone, Default)]
pub struct RawColumn {
    #[serde(default)]
    pub name: String,
}

#[derive(Deserialize, Clone, Default)]
pub struct RawDependsOn {
    #[serde(default)]
    pub nodes: Vec<String>,
}

#[derive(Deserialize, Clone, Default)]
pub struct RawConfig {
    #[serde(default)]
    pub materialized: Option<String>,
}

impl RawManifest {
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("cannot parse {}: {e}", path.display()))
    }
}

/// A dbt node reduced to what lineage needs.
pub struct Node {
    pub uid: String,
    pub name: String,
    pub kind: String,
    pub package: String,
    /// `original_file_path`, as dbt wrote it, separators and all.
    pub file: String,
    pub relation: String,
    pub materialized: String,
    /// Empty when the manifest predates the field, which only SQL did.
    pub language: String,
    pub sql: String,
    pub sql_source: SqlSource,
    /// Column names as declared in YAML, lower case.
    pub declared: Vec<String>,
    pub parents: Vec<String>,
}

/// Where a model's SQL came from (0033).
///
/// A compiled file can be older than the manifest and compiled against another
/// target, so the pass has to know which SQL it holds, and when it holds none
/// the report has to say why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SqlSource {
    /// The manifest's `compiled_code`, even empty: the manifest compiled the
    /// node, and an empty compile is an answer.
    Manifest,
    /// No `compiled_code`, and nothing where dbt writes the compiled file.
    Missing,
    /// No `compiled_code`, and the SQL is the compiled file dbt wrote, by its
    /// path under the target directory.
    File(String),
    /// No `compiled_code`, and the compiled file could not be used, for this
    /// reason.
    Unusable(String),
}

impl SqlSource {
    pub fn file(&self) -> Option<&str> {
        match self {
            SqlSource::File(path) => Some(path),
            _ => None,
        }
    }
}

impl Node {
    pub fn is_model(&self) -> bool {
        self.kind == "model"
    }
    pub fn is_ephemeral(&self) -> bool {
        self.materialized == "ephemeral"
    }
    pub fn is_sql(&self) -> bool {
        self.language.is_empty() || self.language == "sql"
    }

    /// Why there is no SQL to hand the engine, or None when there is some.
    ///
    /// "no compiled_code and no compiled file" is what a missing compile always
    /// said. dbt-lens groups models by it and answers it with "run dbt compile",
    /// so it stays as it was.
    pub fn no_sql(&self) -> Option<String> {
        if !self.is_sql() {
            return Some(format!("a {} model, not SQL", self.language));
        }
        if !self.sql.trim().is_empty() {
            return None;
        }
        Some(match &self.sql_source {
            SqlSource::Manifest => "compiled_code is empty".into(),
            SqlSource::Missing | SqlSource::File(_) => "no compiled_code and no compiled file".into(),
            SqlSource::Unusable(why) => format!("no compiled_code, and {why}"),
        })
    }
}

pub struct Project {
    pub nodes: HashMap<String, Node>,
    /// Uppercased `DB.SCHEMA.NAME` -> unique_id.
    pub by_relation: HashMap<String, String>,
    /// Relations more than one dbt node claims, with every claimant, sorted.
    ///
    /// `by_relation` has to hold one, so it holds the first by name. What makes
    /// that safe is saying so: the alternative is a cache whose edges point at a
    /// different node each run, which is what a hash ordered choice gave. For
    /// one model the pass does better, and names the claimant that model
    /// depends on (0014).
    pub collisions: Vec<(String, Vec<String>)>,
    pub dbt_version: String,
    pub project_name: String,
    pub adapter: String,
}

/// What dbt quotes a relation's parts with when it writes `relation_name`: the
/// adapter's `Relation.quote_character`, `"` by default and a backtick on
/// BigQuery, Databricks, Spark and ClickHouse. Not `adapter.quote`, which is
/// for macros and differs on Fabric.
///
/// Both are stripped whatever the adapter. A project has one, and its SQL
/// cannot quote a name with the other, so the only name this could misread
/// begins or ends with the other character. Carrying the adapter to every
/// place a name is compared would buy nothing more.
const QUOTES: [char; 2] = ['"', '`'];

/// Uppercase and strip quotes, so a name from dbt, one from the catalog and one
/// from the SQL engine compare equal.
pub fn norm_relation(rel: &str) -> String {
    rel.split('.')
        .map(|p| p.trim().trim_matches(&QUOTES[..]).to_uppercase())
        .collect::<Vec<_>>()
        .join(".")
}

impl Project {
    pub fn from_manifest(raw: RawManifest) -> Project {
        let mut nodes: HashMap<String, Node> = HashMap::new();
        let mut claims: HashMap<String, Vec<String>> = HashMap::new();

        let parents_of = |uid: &str, fallback: &RawDependsOn| -> Vec<String> {
            match raw.parent_map.get(uid) {
                Some(p) => p.clone(),
                None => fallback.nodes.clone(),
            }
        };

        for (uid, n) in raw.nodes.iter().chain(raw.sources.iter()) {
            if !matches!(n.resource_type.as_str(), "model" | "source" | "seed" | "snapshot") {
                continue;
            }
            let relation = n.relation_name.clone().unwrap_or_default();
            if !relation.is_empty() {
                claims.entry(norm_relation(&relation)).or_default().push(uid.clone());
            }
            let mut declared: Vec<String> =
                n.columns.values().map(|c| c.name.to_lowercase()).filter(|s| !s.is_empty()).collect();
            declared.sort();
            declared.dedup();
            nodes.insert(
                uid.clone(),
                Node {
                    uid: uid.clone(),
                    name: n.name.clone(),
                    kind: n.resource_type.clone(),
                    package: n.package_name.clone(),
                    file: n.original_file_path.clone(),
                    relation,
                    materialized: n.config.materialized.clone().unwrap_or_default(),
                    language: n.language.clone().unwrap_or_default(),
                    sql: n.compiled_code.clone().unwrap_or_default(),
                    sql_source: match n.compiled_code {
                        Some(_) => SqlSource::Manifest,
                        None => SqlSource::Missing,
                    },
                    declared,
                    parents: parents_of(uid, &n.depends_on),
                },
            );
        }

        // Lowest unique_id wins, and a contested relation is named. Iterating a
        // HashMap to pick the winner made the choice depend on the hash seed, so
        // two runs of the same binary on the same project wrote different edges.
        let mut by_relation: HashMap<String, String> = HashMap::new();
        let mut collisions: Vec<(String, Vec<String>)> = Vec::new();
        for (rel, mut uids) in claims {
            uids.sort();
            by_relation.insert(rel.clone(), uids[0].clone());
            if uids.len() > 1 {
                collisions.push((rel, uids));
            }
        }
        collisions.sort();

        Project {
            nodes,
            by_relation,
            collisions,
            dbt_version: raw.metadata.dbt_version,
            project_name: raw.metadata.project_name,
            adapter: raw.metadata.adapter_type,
        }
    }

    /// Models parent first, so a model's computed schema is available to
    /// everything downstream by the time it is needed. Cycles cannot happen in a
    /// dbt DAG, but a manifest is data from outside: anything still unvisited
    /// after the sweep is appended rather than dropped.
    pub fn topological_models(&self) -> Vec<&Node> {
        let mut state: HashMap<&str, u8> = HashMap::new();
        let mut out: Vec<&Node> = Vec::new();
        let mut stack: Vec<(&str, bool)> = Vec::new();

        let mut roots: Vec<&str> = self.nodes.values().filter(|n| n.is_model()).map(|n| n.uid.as_str()).collect();
        roots.sort_unstable();

        for root in roots {
            if state.contains_key(root) {
                continue;
            }
            stack.push((root, false));
            while let Some((uid, done)) = stack.pop() {
                if done {
                    state.insert(uid, 2);
                    if let Some(n) = self.nodes.get(uid) {
                        if n.is_model() {
                            out.push(n);
                        }
                    }
                    continue;
                }
                match state.get(uid) {
                    Some(2) => continue,
                    Some(1) => continue, // already on the stack: leave the cycle alone
                    _ => {}
                }
                state.insert(uid, 1);
                stack.push((uid, true));
                if let Some(n) = self.nodes.get(uid) {
                    for p in &n.parents {
                        if !state.contains_key(p.as_str()) {
                            if let Some(pn) = self.nodes.get(p) {
                                stack.push((pn.uid.as_str(), false));
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_names_normalise_to_one_shape() {
        assert_eq!(norm_relation("db.sch.tbl"), "DB.SCH.TBL");
        assert_eq!(norm_relation("\"DB\".\"sch\".\"Tbl\""), "DB.SCH.TBL");
        assert_eq!(norm_relation("sch.tbl"), "SCH.TBL");
    }

    #[test]
    fn a_relation_dbt_quotes_in_backticks_is_the_same_relation() {
        // As dbt-bigquery writes relation_name, and as the SQL may spell it.
        assert_eq!(norm_relation("`my-proj`.`ds`.`Tbl`"), "MY-PROJ.DS.TBL");
        assert_eq!(norm_relation("`my-proj.ds.tbl`"), "MY-PROJ.DS.TBL");
        assert_eq!(norm_relation("my-proj.ds.tbl"), "MY-PROJ.DS.TBL");
    }

    fn node(uid: &str, parents: &[&str]) -> (String, Node) {
        (
            uid.to_string(),
            Node {
                uid: uid.into(),
                name: uid.into(),
                kind: "model".into(),
                package: String::new(),
                file: String::new(),
                relation: String::new(),
                materialized: "table".into(),
                language: String::new(),
                sql: String::new(),
                sql_source: SqlSource::Manifest,
                declared: Vec::new(),
                parents: parents.iter().map(|s| s.to_string()).collect(),
            },
        )
    }

    #[test]
    fn a_contested_relation_is_decided_by_name_and_reported() {
        // Two dbt nodes claiming one warehouse object is a project error, but
        // the choice between them must not depend on the hash seed: that made
        // two runs of the same binary write different edges.
        let mut raw = RawManifest::default();
        for name in ["b_lower", "A_UPPER"] {
            raw.sources.insert(
                format!("source.p.{name}"),
                RawNode {
                    name: name.into(),
                    resource_type: "source".into(),
                    package_name: String::new(),
                    original_file_path: String::new(),
                    relation_name: Some("\"DB\".\"sch\".\"tbl\"".into()),
                    compiled_code: None,
                    language: None,
                    columns: HashMap::new(),
                    depends_on: RawDependsOn::default(),
                    config: RawConfig::default(),
                },
            );
        }
        let p = Project::from_manifest(raw);
        assert_eq!(p.by_relation["DB.SCH.TBL"], "source.p.A_UPPER");
        assert_eq!(p.collisions.len(), 1);
        assert_eq!(p.collisions[0].1, vec!["source.p.A_UPPER", "source.p.b_lower"]);
    }

    #[test]
    fn parents_come_before_children() {
        let p = Project {
            nodes: HashMap::from([node("c", &["b"]), node("b", &["a"]), node("a", &[])]),
            by_relation: HashMap::new(),
            collisions: Vec::new(),
            dbt_version: String::new(),
            project_name: String::new(),
            adapter: String::new(),
        };
        let order: Vec<&str> = p.topological_models().iter().map(|n| n.uid.as_str()).collect();
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[test]
    fn a_cycle_does_not_hang_or_lose_nodes() {
        let p = Project {
            nodes: HashMap::from([node("x", &["y"]), node("y", &["x"])]),
            by_relation: HashMap::new(),
            collisions: Vec::new(),
            dbt_version: String::new(),
            project_name: String::new(),
            adapter: String::new(),
        };
        assert_eq!(p.topological_models().len(), 2);
    }
}
