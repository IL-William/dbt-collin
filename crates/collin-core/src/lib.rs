//! Column-level lineage for a dbt project, derived from the compiled SQL that
//! dbt already writes, with no warehouse connection and no Jinja evaluation.
//!
//! The output is the `column_lineage.json` cache dbt-lens already reads, kept at
//! version 1 on purpose so a released dbt-lens consumes it without a rebuild.
//!
//! The invariant that shapes every module: an edge always says where it came
//! from. Something read out of the SQL and something inferred from column names
//! are never mixed, because lineage whose provenance you cannot tell is worse
//! than no lineage.

pub mod cache;
pub mod catalog;
pub mod engine;
pub mod lineage;
pub mod manifest;
pub mod report;
pub mod role;
pub mod schema;

pub use lineage::{generate, Options, Outcome};
