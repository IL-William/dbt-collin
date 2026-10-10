//! The three sources of truth about a node's columns, kept apart on purpose.
//!
//! - **declared**: the YAML. What the project says it produces, i.e. intent.
//! - **warehouse**: `catalog.json`. What the object actually has.
//! - **computed**: what this compile's SQL would produce, filled as models resolve.
//!
//! Keeping them separate is what lets the report distinguish a stale YAML
//! (warehouse disagrees with declared) from a degraded compile (computed
//! disagrees with warehouse). Collapsing them into one "best guess" would hide
//! exactly the two failures worth reporting.

use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provenance {
    Warehouse,
    Computed,
    Declared,
    Unknown,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Provenance::Warehouse => "warehouse",
            Provenance::Computed => "computed",
            Provenance::Declared => "declared",
            Provenance::Unknown => "unknown",
        }
    }
}

/// How a model's computed columns compare with the two references.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Agreement {
    /// Computed matches the warehouse. The compiled SQL is representative.
    Confirmed,
    /// The compile is not describing the object that exists, so its edges must
    /// not be published as `parsed`. Either the warehouse contradicts it, or,
    /// with no warehouse to ask, the project's own YAML shares not one column
    /// name with it.
    Degraded,
    /// Nothing to check against, or nothing conclusive to say.
    Unchecked,
}

impl Agreement {
    pub fn as_str(self) -> &'static str {
        match self {
            Agreement::Confirmed => "confirmed",
            Agreement::Degraded => "degraded",
            Agreement::Unchecked => "unchecked",
        }
    }
}

#[derive(Default)]
pub struct Store {
    declared: HashMap<String, Vec<String>>,
    warehouse: HashMap<String, Vec<String>>,
    computed: HashMap<String, Vec<String>>,
}

impl Store {
    pub fn new(declared: HashMap<String, Vec<String>>, warehouse: HashMap<String, Vec<String>>) -> Store {
        Store { declared, warehouse, computed: HashMap::new() }
    }

    pub fn set_computed(&mut self, uid: &str, mut cols: Vec<String>) {
        cols.sort();
        cols.dedup();
        self.computed.insert(uid.to_string(), cols);
    }

    /// Drop a computed list the pass has decided not to trust.
    ///
    /// Without this, a model whose compile lost its columns still defines what
    /// every downstream `select *` expands to, so one degraded compile truncates
    /// a whole branch of the graph. Falling back to the YAML may be stale, but
    /// stale beats a list already shown to be wrong.
    pub fn retract_computed(&mut self, uid: &str) {
        self.computed.remove(uid);
    }

    pub fn declared(&self, uid: &str) -> Option<&Vec<String>> {
        self.declared.get(uid).filter(|v| !v.is_empty())
    }
    pub fn warehouse(&self, uid: &str) -> Option<&Vec<String>> {
        self.warehouse.get(uid).filter(|v| !v.is_empty())
    }
    pub fn computed(&self, uid: &str) -> Option<&Vec<String>> {
        self.computed.get(uid).filter(|v| !v.is_empty())
    }

    /// The columns to hand the engine when a downstream model reads this node.
    ///
    /// The warehouse wins: downstream SQL was compiled against the objects that
    /// actually exist, so a `select *` there sees the warehouse columns, not
    /// whatever this compile would have produced.
    pub fn visible(&self, uid: &str) -> (&[String], Provenance) {
        if let Some(c) = self.warehouse(uid) {
            return (c, Provenance::Warehouse);
        }
        if let Some(c) = self.computed(uid) {
            return (c, Provenance::Computed);
        }
        if let Some(c) = self.declared(uid) {
            return (c, Provenance::Declared);
        }
        (&[], Provenance::Unknown)
    }

    pub fn agreement(&self, uid: &str) -> Agreement {
        if let Some(real) = self.warehouse(uid) {
            return match self.computed(uid) {
                Some(c) if c == real => Agreement::Confirmed,
                _ => Agreement::Degraded,
            };
        }
        // No warehouse. The YAML is a weaker reference, stale on 44.8% of the
        // models we can check, so it may only support the one verdict staleness
        // cannot explain: a compile sharing not a single column name with what
        // the project says this model has is not describing this model. A
        // partial disagreement stays unchecked, because there it is far more
        // often the YAML that drifted.
        match (self.declared(uid), self.computed(uid)) {
            (Some(d), Some(c)) if !c.iter().any(|name| d.contains(name)) => Agreement::Degraded,
            _ => Agreement::Unchecked,
        }
    }

    /// How far the compile and the YAML overlap: (declared, computed, shared).
    ///
    /// Read only, and deliberately not a verdict. `Agreement` answers whether the
    /// compile can be trusted; this answers how much of the documentation the
    /// compile accounts for, which is a different question with a different
    /// remedy. Returns None when either list is missing, because a comparison
    /// needs two sides.
    pub fn overlap(&self, uid: &str) -> Option<(usize, usize, usize)> {
        let (d, c) = (self.declared(uid)?, self.computed(uid)?);
        let set: std::collections::HashSet<&str> = c.iter().map(String::as_str).collect();
        let shared = d.iter().filter(|n| set.contains(n.as_str())).count();
        Some((d.len(), c.len(), shared))
    }

    /// True when the YAML disagrees with the warehouse, which is a documentation
    /// problem rather than a lineage one, and worth reporting on its own.
    pub fn yaml_is_stale(&self, uid: &str) -> bool {
        match (self.declared(uid), self.warehouse(uid)) {
            (Some(d), Some(w)) => d != w,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_warehouse_is_what_downstream_sees() {
        let mut s = Store::new(
            HashMap::from([("m".into(), cols(&["a"]))]),
            HashMap::from([("m".into(), cols(&["a", "b"]))]),
        );
        s.set_computed("m", cols(&["a", "b", "c"]));
        let (c, p) = s.visible("m");
        assert_eq!(c, cols(&["a", "b"]).as_slice());
        assert_eq!(p, Provenance::Warehouse);
    }

    #[test]
    fn computed_against_warehouse_is_the_degradation_signal() {
        let mut s = Store::new(HashMap::new(), HashMap::from([("m".into(), cols(&["a", "b"]))]));
        s.set_computed("m", cols(&["a"]));
        assert_eq!(s.agreement("m"), Agreement::Degraded);
        s.set_computed("m", cols(&["b", "a"]));
        assert_eq!(s.agreement("m"), Agreement::Confirmed, "order must not matter");
    }

    #[test]
    fn without_a_catalog_a_partial_disagreement_is_not_held_against_the_compile() {
        // Far more often the YAML drifted, so this must not cost the model its
        // parsed edges.
        let mut s = Store::new(HashMap::from([("m".into(), cols(&["a", "b"]))]), HashMap::new());
        s.set_computed("m", cols(&["a", "zzz"]));
        assert_eq!(s.agreement("m"), Agreement::Unchecked);
        assert!(!s.yaml_is_stale("m"), "staleness needs a warehouse to establish");
    }

    #[test]
    fn without_a_catalog_a_total_disagreement_condemns_the_compile() {
        // Nothing the compile produced is anything the project says this model
        // has. Drift does not rename every column at once; a broken macro does.
        let mut s = Store::new(HashMap::from([("m".into(), cols(&["a", "b"]))]), HashMap::new());
        s.set_computed("m", cols(&["a_a", "b_b"]));
        assert_eq!(s.agreement("m"), Agreement::Degraded);
    }

    #[test]
    fn overlap_counts_both_sides_and_what_they_share() {
        let mut s = Store::new(HashMap::from([("m".into(), cols(&["a", "b", "c"]))]), HashMap::new());
        s.set_computed("m", cols(&["a", "z"]));
        assert_eq!(s.overlap("m"), Some((3, 2, 1)));
    }

    #[test]
    fn overlap_needs_two_sides() {
        let mut s = Store::new(HashMap::new(), HashMap::new());
        s.set_computed("m", cols(&["a"]));
        assert_eq!(s.overlap("m"), None, "a comparison needs something to compare with");
    }

    #[test]
    fn a_stale_yaml_is_reported_separately() {
        let s = Store::new(
            HashMap::from([("m".into(), cols(&["a"]))]),
            HashMap::from([("m".into(), cols(&["a", "b"]))]),
        );
        assert!(s.yaml_is_stale("m"));
    }
}
