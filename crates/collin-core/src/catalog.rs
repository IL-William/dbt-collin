//! `catalog.json`, written by `dbt docs generate` or `dbt compile --write-catalog`.
//!
//! This is the only input that says what the warehouse actually holds, which
//! makes it the tie breaker whenever the compiled SQL and the YAML disagree.
//! Optional: without it the tool still runs, and the report says accuracy is
//! capped.

use serde::Deserialize;
use std::collections::HashMap;

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
    pub columns: HashMap<String, CatalogColumn>,
}

#[derive(Deserialize, Default)]
pub struct CatalogColumn {
    #[serde(default)]
    pub name: String,
}

impl RawCatalog {
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("cannot parse {}: {e}", path.display()))
    }

    /// unique_id -> column names, lower case and sorted.
    pub fn columns(self) -> HashMap<String, Vec<String>> {
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        for (uid, entry) in self.nodes.into_iter().chain(self.sources) {
            let mut cols: Vec<String> = entry
                .columns
                .into_values()
                .map(|c| c.name.to_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
            cols.sort();
            cols.dedup();
            if !cols.is_empty() {
                out.insert(uid, cols);
            }
        }
        out
    }
}
