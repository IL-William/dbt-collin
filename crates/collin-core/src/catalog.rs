//! `catalog.json`, written by `dbt docs generate` or `dbt compile --write-catalog`.
//!
//! This is the only input that says what the warehouse actually holds, which
//! makes it the tie breaker whenever the compiled SQL and the YAML disagree.
//! Optional: without it the tool still runs, and the report says accuracy is
//! capped.
//!
//! Invariant: an entry is a witness only for the relation the manifest gives
//! its node. dbt keys the catalog by unique_id, and one unique_id names a
//! different table under each target, so an entry describing another table says
//! nothing about the one this compile builds (0037).

use serde::Deserialize;
use std::collections::HashMap;

use crate::manifest::norm_relation;

#[derive(Deserialize, Default)]
pub struct RawCatalog {
    #[serde(default)]
    pub nodes: HashMap<String, CatalogNode>,
    #[serde(default)]
    pub sources: HashMap<String, CatalogNode>,
}

#[derive(Deserialize, Default)]
pub struct CatalogNode {
    #[serde(default)]
    pub metadata: CatalogMetadata,
    #[serde(default)]
    pub columns: HashMap<String, CatalogColumn>,
}

#[derive(Deserialize, Default)]
pub struct CatalogMetadata {
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Deserialize, Default)]
pub struct CatalogColumn {
    #[serde(default)]
    pub name: String,
}

/// A catalog entry describing another table than the one the manifest gives
/// its node, so not used.
#[derive(Debug, PartialEq, Eq)]
pub struct Elsewhere {
    pub unique_id: String,
    /// The manifest's, as dbt wrote it.
    pub relation: String,
    /// The table the entry describes, `database.schema.name`.
    pub catalog_relation: String,
}

/// What the catalog witnesses, and what it was set aside for.
#[derive(Default)]
pub struct Witness {
    /// unique_id -> column names, lower case and sorted.
    pub columns: HashMap<String, Vec<String>>,
    /// Sorted by unique_id.
    pub elsewhere: Vec<Elsewhere>,
}

impl RawCatalog {
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("cannot parse {}: {e}", path.display()))
    }

    /// The column lists of the entries that describe the table the manifest
    /// names, `relation_of` giving it by unique_id.
    ///
    /// An entry is kept when there is nothing to compare: a node the manifest
    /// gives no relation, or an entry without a schema and a name. Neither says
    /// the entry is about another table, and dropping it would cost a witness
    /// on evidence of nothing.
    pub fn columns<'a>(self, relation_of: impl Fn(&str) -> Option<&'a str>) -> Witness {
        let mut out = Witness::default();
        for (uid, entry) in self.nodes.into_iter().chain(self.sources) {
            if let Some(relation) = relation_of(&uid) {
                if !same_relation(relation, &entry.metadata) {
                    let m = &entry.metadata;
                    let catalog_relation = [m.database.as_deref().unwrap_or(""), &m.schema, &m.name]
                        .iter()
                        .filter(|p| !p.is_empty())
                        .copied()
                        .collect::<Vec<_>>()
                        .join(".");
                    out.elsewhere.push(Elsewhere {
                        unique_id: uid,
                        relation: relation.to_string(),
                        catalog_relation,
                    });
                    continue;
                }
            }
            let mut cols: Vec<String> = entry
                .columns
                .into_values()
                .map(|c| c.name.to_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
            cols.sort();
            cols.dedup();
            if !cols.is_empty() {
                out.columns.insert(uid, cols);
            }
        }
        out.elsewhere.sort_by(|a, b| a.unique_id.cmp(&b.unique_id));
        out
    }
}

/// Whether an entry describes `relation`, compared on the parts both spell: an
/// adapter without databases writes none in the catalog, and a relation in two
/// parts has none in the manifest.
fn same_relation(relation: &str, m: &CatalogMetadata) -> bool {
    if relation.is_empty() || m.schema.is_empty() || m.name.is_empty() {
        return true;
    }
    let manifest: Vec<String> = norm_relation(relation).split('.').map(str::to_string).collect();
    let mut catalog: Vec<&str> = Vec::new();
    if let Some(db) = m.database.as_deref().filter(|d| !d.is_empty()) {
        catalog.push(db);
    }
    catalog.push(&m.schema);
    catalog.push(&m.name);
    let catalog: Vec<String> = norm_relation(&catalog.join(".")).split('.').map(str::to_string).collect();
    let n = manifest.len().min(catalog.len());
    manifest[manifest.len() - n..] == catalog[catalog.len() - n..]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(database: Option<&str>, schema: &str, name: &str, cols: &[&str]) -> CatalogNode {
        CatalogNode {
            metadata: CatalogMetadata {
                database: database.map(str::to_string),
                schema: schema.into(),
                name: name.into(),
            },
            columns: cols.iter().map(|c| (c.to_uppercase(), CatalogColumn { name: c.to_uppercase() })).collect(),
        }
    }

    fn witness(nodes: Vec<(&str, CatalogNode)>, relations: &[(&str, &str)]) -> Witness {
        let raw = RawCatalog {
            nodes: nodes.into_iter().map(|(u, e)| (u.to_string(), e)).collect(),
            sources: HashMap::new(),
        };
        let relations: HashMap<String, String> =
            relations.iter().map(|(u, r)| (u.to_string(), r.to_string())).collect();
        raw.columns(|uid| relations.get(uid).map(String::as_str))
    }

    #[test]
    fn an_entry_for_the_manifest_s_table_is_a_witness() {
        // dbt writes the manifest's relation lower case, the warehouse answers
        // upper case, and either may be quoted.
        let w = witness(
            vec![("model.shop.orders", entry(Some("DB_DEV"), "SALES", "ORDERS", &["id", "amount"]))],
            &[("model.shop.orders", "db_dev.sales.\"orders\"")],
        );
        assert_eq!(w.columns["model.shop.orders"], vec!["amount", "id"]);
        assert!(w.elsewhere.is_empty());
    }

    #[test]
    fn an_entry_for_another_target_s_table_is_set_aside_and_named() {
        // The same unique_id, built under a personal schema: what that table
        // has says nothing about the one this compile builds.
        let w = witness(
            vec![("model.shop.orders", entry(Some("DB_DEV"), "DBT_ME", "ORDERS", &["id", "legacy"]))],
            &[("model.shop.orders", "db_dev.sales.orders")],
        );
        assert!(w.columns.is_empty());
        assert_eq!(
            w.elsewhere,
            vec![Elsewhere {
                unique_id: "model.shop.orders".into(),
                relation: "db_dev.sales.orders".into(),
                catalog_relation: "DB_DEV.DBT_ME.ORDERS".into(),
            }]
        );
    }

    #[test]
    fn a_renamed_table_is_another_table() {
        // An alias changed since the catalog: the entry is the old table's.
        let w = witness(
            vec![("model.shop.orders", entry(Some("DB"), "SALES", "ORDERS_V1", &["id"]))],
            &[("model.shop.orders", "db.sales.orders")],
        );
        assert!(w.columns.is_empty());
        assert_eq!(w.elsewhere.len(), 1);
    }

    #[test]
    fn the_parts_one_side_does_not_spell_are_not_compared() {
        // An adapter without databases, and a relation in two parts.
        let w = witness(
            vec![
                ("model.shop.a", entry(None, "sales", "a", &["x"])),
                ("model.shop.b", entry(Some("DB"), "SALES", "B", &["y"])),
            ],
            &[("model.shop.a", "db.sales.a"), ("model.shop.b", "sales.b")],
        );
        assert_eq!(w.columns.len(), 2);
        assert!(w.elsewhere.is_empty());
    }

    #[test]
    fn nothing_to_compare_keeps_the_entry() {
        // No relation in the manifest, an entry without metadata, a node the
        // manifest does not have: none of them shows the entry is elsewhere.
        let w = witness(
            vec![
                ("model.shop.a", entry(Some("DB"), "SALES", "A", &["x"])),
                ("model.shop.b", entry(None, "", "", &["y"])),
                ("model.shop.gone", entry(Some("DB"), "OLD", "GONE", &["z"])),
            ],
            &[("model.shop.a", ""), ("model.shop.b", "db.sales.b")],
        );
        assert_eq!(w.columns.len(), 3);
        assert!(w.elsewhere.is_empty());
    }
}
