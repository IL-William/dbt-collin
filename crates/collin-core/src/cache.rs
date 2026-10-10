//! The `column_lineage.json` cache dbt-lens reads.
//!
//! Deliberately pinned at version 1. dbt-lens refuses a version above 1, so
//! staying here means a released binary picks this file up with no rebuild. The
//! `kind` field already travels end to end over there, from the cache through
//! the graph to the renderer, so putting the role in it costs nothing.
//!
//! `from` is a dbt unique_id, or `rel:<db.schema.object>` for a relation dbt
//! does not own. That escape hatch is part of the format already.

use serde::Serialize;

pub const VERSION: u32 = 1;

/// What this producer calls itself, in the cache and in its file name. One
/// constant for both, so the two can never drift apart: dbt-lens groups caches
/// by the `source` field and finds them by the file name.
pub const SOURCE: &str = "collin";

/// Where this producer writes, one file per producer (dbt-lens 0021), so it
/// never contends with the Snowflake sidecar over a single file.
pub fn default_path(target_dir: &std::path::Path) -> std::path::PathBuf {
    target_dir.join(format!("column_lineage.{SOURCE}.json"))
}

#[derive(Serialize)]
pub struct Cache {
    pub version: u32,
    /// What produced it. dbt-lens shows this, so a synthetic or a second source
    /// can never be mistaken for the warehouse's own answer.
    pub source: String,
    /// The collin that wrote it, `collin 0.1.0`, so a cache older than the
    /// collin now installed can be told apart from a fresh one (0034). Beside
    /// `version`, not instead of it: the format stays at 1, and a reader that
    /// does not know this field skips it.
    pub producer: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub target: String,
    pub generated_at: String,
    pub edges: Vec<Edge>,
}

#[derive(Serialize)]
pub struct Edge {
    pub from: String,
    pub from_col: String,
    pub to: String,
    pub to_col: String,
    /// The role: passthrough, rename, cast, aggregate, window, transform, or
    /// inferred when the edge came from name matching rather than from the SQL.
    pub kind: String,
}

impl Cache {
    pub fn new(target: String, edges: Vec<Edge>) -> Cache {
        Cache { version: VERSION, source: SOURCE.into(), producer: producer(), target, generated_at: now(), edges }
    }

    pub fn write(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }
}

/// This collin, as a cache names it. The version only: the commit is the
/// binary's to print, and this crate is a library built into other things.
pub fn producer() -> String {
    format!("{SOURCE} {}", env!("CARGO_PKG_VERSION"))
}

/// RFC 3339 in UTC, computed from the epoch rather than pulled from a date
/// crate: one timestamp is not worth a dependency.
pub fn now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Civil date from a day count, Howard Hinnant's algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_stays_where_dbt_lens_can_read_it() {
        assert_eq!(VERSION, 1, "dbt-lens refuses a cache above version 1");
    }

    #[test]
    fn the_timestamp_is_a_plausible_rfc3339() {
        let t = now();
        assert_eq!(t.len(), 20, "{t}");
        assert!(t.ends_with('Z') && t.contains('T'), "{t}");
        let year: i64 = t[..4].parse().unwrap();
        assert!((2024..2100).contains(&year), "{t}");
    }

    #[test]
    fn the_file_is_named_after_the_producer_that_writes_it() {
        let p = default_path(std::path::Path::new("/t"));
        assert_eq!(p, std::path::Path::new("/t/column_lineage.collin.json"));
        // The name in the file and the name of the file come from one constant.
        assert!(p.to_string_lossy().contains(SOURCE));
    }

    #[test]
    fn a_cache_names_what_produced_it() {
        let c = Cache::new("dev".into(), Vec::new());
        let text = serde_json::to_string(&c).unwrap();
        assert!(text.contains("\"source\":\"collin\""), "{text}");
        assert!(text.contains("\"version\":1"), "{text}");
        let producer = format!("\"producer\":\"collin {}\"", env!("CARGO_PKG_VERSION"));
        assert!(text.contains(&producer), "{text}");
    }
}
