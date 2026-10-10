//! The analysis engine, behind an interface that hides it.
//!
//! Today this wraps `flowscope-core`, which parses with `sqlparser` and resolves
//! columns against a supplied schema. It was chosen on measurement rather than
//! on reading: on a 3341 model Snowflake project it recovered 92.3% of the
//! columns the warehouse actually has.
//!
//! The interface is the point. Everything downstream speaks `Resolved`, so
//! swapping in a hand written resolver later touches this file and nothing else.
//!
//! Invariant: this module reports what the engine found and what it admitted it
//! could not find. It never fills a gap with a guess; that decision belongs to
//! `lineage`, which has to label the guess.

use flowscope_core::analyzer::helpers::line_col_to_offset;
use flowscope_core::types::{
    issue_codes, AnalyzeRequest, ColumnSchema, Dialect, EdgeType, FilterClauseType, Node as FsNode,
    NodeType, SchemaMetadata, SchemaTable, Severity,
};
use sqlparser::ast::{Expr, Query, SelectItem, SetExpr, TableFactor, Visit, Visitor};
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use crate::manifest::norm_relation;

/// One relation the SQL may read, with the columns it exposes.
pub struct Visible {
    /// The relation exactly as dbt wrote it in `relation_name`, quotes and all.
    ///
    /// Not uppercased. dbt quotes an identifier precisely when the warehouse
    /// needs it quoted, and the compiled SQL then carries the same spelling, so
    /// the raw string is the only one that can be matched against the SQL. See
    /// `as_table`.
    pub relation: String,
    pub columns: Vec<String>,
}

/// A column the SQL reads to decide which rows exist: a join key, a filter, a
/// dedup key.
///
/// Read that way it bears on which rows exist rather than on what any one value
/// is, so it has no output column of its own to point at. That is the whole
/// difficulty: the cache names a source column and a target column, and here
/// there is no target. The same column may also be carried through to an
/// output, which is another edge and another claim.
pub struct IndirectRead {
    /// Uppercased `DB.SCHEMA.NAME`.
    pub relation: String,
    pub column: String,
    pub role: crate::role::Role,
}

/// A column to column link as the engine saw it, before dbt identities are
/// attached.
pub struct RawEdge {
    /// Uppercased `DB.SCHEMA.NAME` of the relation the column belongs to.
    pub from_relation: String,
    pub from_column: String,
    pub to_column: String,
    /// The SQL text behind the output column, empty when it was copied through.
    pub expression: String,
}

/// An issue the engine raised, kept whole rather than reduced to its code.
///
/// The code on its own names a category, not a cause: on a 3341 model project
/// every parse failure read `PARSE_ERROR` and 950 models read
/// `UNRESOLVED_REFERENCE`, which gives a reader nothing to go and fix. The
/// message names the table or the column, and the span says where in the
/// compiled SQL to look.
#[derive(Clone)]
pub struct Issue {
    pub code: String,
    pub message: String,
    /// `Error`, `Warning` or `Info`, as the engine graded it. Carried rather
    /// than acted on: see `DEGRADING`.
    pub severity: &'static str,
    /// Byte range in the compiled SQL, when the engine placed it. For a parse
    /// error, the token the parser stopped at: see `stopped_at`.
    ///
    /// For a column, the first place the statement spells its name, which need
    /// not be the reference that failed: the engine finds it by searching. It is
    /// carried as given, since placing it better here would be the same search.
    pub span: Option<(usize, usize)>,
    /// The code is one of `DEGRADING`. Given per issue so that `lineage` can
    /// weigh an admission by what it is about, which `approximate` cannot.
    pub degrading: bool,
    /// Uppercased `DB.SCHEMA.NAME` of the table an `UNRESOLVED_REFERENCE`
    /// could not find. See `unresolved_relation`.
    pub relation: Option<String>,
}

/// The codes that mean the engine did not see all of the SQL.
///
/// Named from the library's own constants rather than matched as substrings, so
/// a rename upstream fails the build instead of quietly weakening the verdict.
///
/// Severity is deliberately not part of the test. `APPROXIMATE_LINEAGE` is
/// graded `Info` by the engine and is the plainest statement of approximation it
/// makes, so gating on severity would discard the clearest signal there is.
const DEGRADING: &[&str] = &[
    issue_codes::PARSE_ERROR,
    issue_codes::INVALID_REQUEST,
    issue_codes::UNSUPPORTED_SYNTAX,
    issue_codes::UNSUPPORTED_RECURSIVE_CTE,
    issue_codes::APPROXIMATE_LINEAGE,
    // Not UNKNOWN_COLUMN. With implied columns allowed the engine still reads
    // the name and draws the edge; what it says is that the list it was handed
    // for the relation lacks the column, which is a finding about that list and
    // its model, not about this SQL. See `Resolved::unknown_columns`.
    issue_codes::UNKNOWN_TABLE,
    issue_codes::UNRESOLVED_REFERENCE,
    issue_codes::SCHEMA_CONFLICT,
    issue_codes::CANCELLED,
    issue_codes::MEMORY_LIMIT_EXCEEDED,
    issue_codes::DIALECT_FALLBACK,
];

#[derive(Default)]
pub struct Resolved {
    /// Output column names, lower case.
    pub outputs: Vec<String>,
    pub edges: Vec<RawEdge>,
    /// Every relation the SQL reads, uppercased and qualified as the SQL wrote
    /// it, `DB.SCHEMA.NAME` or `SCHEMA.NAME`, sorted, whether or not it was
    /// among those handed in.
    ///
    /// Taken from the tables the engine placed in the statement rather than from
    /// its `UNRESOLVED_REFERENCE`, which it raises only once it knows at least one
    /// table: a model none of whose parents came with columns reads a table
    /// nobody declared without a word said. A table function such as `lateral
    /// flatten` is no table and is not here. Neither is a name in one part, which
    /// has nothing to be matched against.
    pub reads: Vec<String>,
    /// Issues the engine raised, verbatim.
    pub issues: Vec<Issue>,
    /// The engine said it could not resolve something. Its own admission, not
    /// our inference.
    pub approximate: bool,
    /// The SQL did not parse.
    pub parse_error: Option<String>,
    /// Columns the SQL reads to decide which rows exist: join keys, filter
    /// predicates, and the partition and order keys of a `QUALIFY`. Whether a
    /// column is projected as well is another matter; see `IndirectRead`.
    pub indirect: Vec<IndirectRead>,
    /// Output columns that no relation feeds, because the SQL builds them from
    /// literals: `cast(null as text) as x`, `current_timestamp() as loaded_at`,
    /// whether in the final select or in a CTE the column is copied out of.
    ///
    /// Kept apart from the columns that simply came out with no lineage. A root
    /// has no parent to find, so counting it as a gap makes coverage read lower
    /// than the truth and sends a reader looking for something that is not there.
    pub roots: Vec<String>,
    /// Output columns no edge reaches that are no root either: the lineage the
    /// SQL has and the engine lost, each with where the walk back stopped.
    /// Sorted by column.
    pub lost: Vec<Lost>,
    /// Names the statement gives to more than one scope. See `MergedScope`.
    pub merged_scopes: Vec<MergedScope>,
    /// Names read from a CTE that does not project them. See `UnbackedRead`.
    pub unbacked: Vec<UnbackedRead>,
    /// Every column the engine said the list it was handed for a relation
    /// lacks, from its `UNKNOWN_COLUMN`, in the order raised.
    pub unknown_columns: Vec<UnknownColumn>,
    /// Names a `QUALIFY` reads that more than one source of its `SELECT` has,
    /// or that a qualifier binds to no source: read by nobody rather than by
    /// every candidate.
    pub unplaced_qualify: usize,
    /// The same, for the names a `JOIN ... ON` reads.
    pub unplaced_join: usize,
    /// The same, for the names a `WHERE` or `HAVING` reads. A qualifier that
    /// binds no source of the `SELECT` is not counted here: a correlated
    /// subquery reads its outer query that way.
    pub unplaced_filter: usize,
}

/// A column the SQL reads from a relation whose list, as handed to the engine,
/// does not have it. The engine says so and draws the edge anyway.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownColumn {
    /// Normalised. Empty when the message did not read as expected, so that the
    /// entry can match no edge and a change of wording upstream can only make
    /// the report louder, never a model cleaner.
    pub relation: String,
    /// Lower case; the whole message when it did not read as expected.
    pub column: String,
}

/// "Column 'x' not found in table 'DB.SCH.T'. Available columns: ...", which
/// is all the engine gives: no span, no structured field.
fn unknown_column(message: &str) -> UnknownColumn {
    let parts: Vec<&str> = message.split('\'').collect();
    match parts.as_slice() {
        [_, column, _, relation, ..] if !column.is_empty() && !relation.is_empty() => {
            UnknownColumn { relation: norm_relation(relation), column: column.to_lowercase() }
        }
        _ => UnknownColumn { relation: String::new(), column: message.to_string() },
    }
}

/// A name a scope reads from a CTE whose select list is written out and does
/// not project it.
///
/// Such SQL cannot run: the warehouse rejects the name. A compile shows it when
/// a macro that reads the warehouse at compile time found nothing, so a CTE that
/// should have listed a relation's columns lists none, and the next CTE reads
/// them anyway. The engine checks a name against a table it was handed but
/// never against a CTE, so it raises nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnbackedRead {
    /// The output column whose walk met the read.
    pub output: String,
    /// The CTE read, as the engine names it.
    pub cte: String,
    pub column: String,
    /// Every relation that CTE reads, through the CTEs it reads, whose column
    /// list has the name, normalised and sorted. A fact, not a choice: one, many
    /// or none, it is for the pass to decide what to do with them.
    pub owners: Vec<String>,
}

/// A name one statement gives to two derived tables, or to two CTEs.
///
/// Released flowscope-core keys a derived table and a CTE by its name within a
/// statement, and keeps the first node with a key: the second scope's columns
/// become the first's, and an output of either can come out fed by both, with
/// nothing said. The fork keys both by occurrence (0028), so this is a fact
/// about the SQL, named in the report, and no longer a failure to act on
/// (0018). The tests reading two of each apart are what would catch a pin that
/// lost it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergedScope {
    /// `derived` or `cte`.
    pub kind: &'static str,
    /// As the SQL spells it: the engine does not fold the case of the key.
    pub name: String,
    pub count: usize,
}

/// An output column that came out with no edge and is not built from literals.
pub struct Lost {
    pub column: String,
    /// Every place the walk back from it stopped, sorted, never empty.
    pub ends: Vec<DeadEnd>,
}

/// Where a walk back from an output column stopped short of a relation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeadEnd {
    /// Copied from a CTE or a derived table that does not have the column, the
    /// shape of a `select *` over a parent whose column list lacks it. `via` is
    /// its name, `column` the name read from it, and `reads` every relation it
    /// reads, through other CTEs. All of them and never one picked out: naming
    /// one relation of a join as the owner would be a guess. `star` says the
    /// scope's select list is a `*` over a relation, so that its columns are
    /// whatever list that relation came with, rather than a list of its own
    /// that the SQL itself left the name out of.
    Phantom { via: String, column: String, reads: Vec<String>, star: bool },
    /// An expression on the way reads these names and the walk never reached
    /// them: a column inside a form the engine does not read, or a name it
    /// could not place.
    NamesUnreached { names: Vec<String> },
    /// Anything else the engine left unplaced.
    Unresolved,
}

/// The derived table aliases and CTE names of one statement, as written, and
/// for each of those scopes the relations its select list takes `*` from.
#[derive(Default)]
struct ScopeNames {
    derived: Vec<String>,
    ctes: Vec<String>,
    /// Lower case scope name, and the last part of each name its `*` reads.
    stars: Vec<(String, Vec<String>)>,
    /// Lower case names of the derived tables whose rows are a `VALUES` list,
    /// `(derived)` for one with no alias, as the engine labels it.
    values: Vec<String>,
    /// Lower case names of the `LATERAL FLATTEN` calls, `flatten` for one with
    /// no alias, as the engine labels it.
    flattens: Vec<String>,
}

impl Visitor for ScopeNames {
    type Break = ();

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
        if let Some(with) = &query.with {
            for cte in &with.cte_tables {
                let name = cte.alias.name.to_string();
                self.stars.push((name.trim_matches('"').to_lowercase(), star_sources(&cte.query)));
                self.ctes.push(name);
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, factor: &TableFactor) -> ControlFlow<()> {
        if let TableFactor::Function { name, alias, .. } = factor {
            if name.to_string().eq_ignore_ascii_case("flatten") {
                let label = alias.as_ref().map_or_else(|| name.to_string(), |a| a.name.to_string());
                self.flattens.push(label.trim_matches('"').to_lowercase());
            }
        }
        if let TableFactor::Derived { alias, subquery, .. } = factor {
            if matches!(subquery.body.as_ref(), SetExpr::Values(_)) {
                let label = alias.as_ref().map(|a| a.name.to_string());
                self.values.push(label.map_or("(derived)".into(), |l| l.trim_matches('"').to_lowercase()));
            }
            if let Some(alias) = alias {
                let name = alias.name.to_string();
                self.stars.push((name.trim_matches('"').to_lowercase(), star_sources(subquery)));
                self.derived.push(name);
            }
        }
        ControlFlow::Continue(())
    }
}

/// The names a query's select list takes `*` from, lower case and last part
/// only; empty when it writes its columns out.
fn star_sources(query: &Query) -> Vec<String> {
    fn body(expr: &SetExpr, out: &mut Vec<String>) {
        match expr {
            SetExpr::Select(select) => {
                let star = select.projection.iter().any(|item| {
                    matches!(item, SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(..))
                });
                if !star {
                    return;
                }
                for from in &select.from {
                    let factors = std::iter::once(&from.relation).chain(from.joins.iter().map(|j| &j.relation));
                    for factor in factors {
                        if let TableFactor::Table { name, .. } = factor {
                            let full = name.to_string();
                            let last = full.rsplit('.').next().unwrap_or_default();
                            out.push(last.trim_matches('"').to_lowercase());
                        }
                    }
                }
            }
            SetExpr::Query(query) => body(&query.body, out),
            SetExpr::SetOperation { left, right, .. } => {
                body(left, out);
                body(right, out);
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    body(&query.body, &mut out);
    out
}

/// The names a `WHERE` or a `HAVING` reads, with what its `SELECT` reads; or
/// the names the select list of a subquery inside such a predicate reads, with
/// what that subquery reads.
struct Predicate {
    sources: Vec<(String, Option<String>)>,
    names: Vec<(Option<String>, String)>,
}

/// The subqueries a predicate holds at its own level, `x in (select a ...)`,
/// `x = (select max(a) ...)` or `exists (...)`: what each one's select list
/// reads is compared with the outer row, so it decides which rows exist.
#[derive(Default)]
struct PredicateSubqueries<'a> {
    found: Vec<&'a Query>,
}

impl<'a> PredicateSubqueries<'a> {
    fn collect(expr: &'a Expr) -> Vec<&'a Query> {
        let mut v = PredicateSubqueries::default();
        v.walk(expr);
        v.found
    }

    fn walk(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Subquery(q) | Expr::Exists { subquery: q, .. } => self.found.push(q),
            Expr::InSubquery { expr, subquery, .. } => {
                self.walk(expr);
                self.found.push(subquery);
            }
            Expr::BinaryOp { left, right, .. } => {
                self.walk(left);
                self.walk(right);
            }
            Expr::UnaryOp { expr, .. } | Expr::Nested(expr) | Expr::IsNull(expr) | Expr::IsNotNull(expr) => {
                self.walk(expr)
            }
            Expr::Between { expr, low, high, .. } => {
                self.walk(expr);
                self.walk(low);
                self.walk(high);
            }
            Expr::InList { expr, list, .. } => {
                self.walk(expr);
                list.iter().for_each(|e| self.walk(e));
            }
            _ => {}
        }
    }
}

/// The keys of one `JOIN`, with what its `SELECT` reads.
struct JoinKeys {
    sources: Vec<(String, Option<String>)>,
    /// Names its `ON` reads, qualifier first when written.
    on: Vec<(Option<String>, String)>,
    /// Names its `USING` lists, which both sides have by definition.
    using: Vec<String>,
}

/// Each `FROM` and `JOIN` source of a `SELECT`: its name as written, lower
/// case, and the alias it goes by. A derived table has only its alias.
fn select_sources(select: &sqlparser::ast::Select) -> Vec<(String, Option<String>)> {
    let mut sources = Vec::new();
    for from in &select.from {
        let factors = std::iter::once(&from.relation).chain(from.joins.iter().map(|j| &j.relation));
        for factor in factors {
            match factor {
                TableFactor::Table { name, alias, .. } => sources.push((
                    name.to_string().to_lowercase(),
                    alias.as_ref().map(|a| a.name.value.to_lowercase()),
                )),
                TableFactor::Derived { alias: Some(alias), .. } => {
                    let a = alias.name.value.to_lowercase();
                    sources.push((a.clone(), Some(a)));
                }
                _ => {}
            }
        }
    }
    sources
}

/// A `QUALIFY`, with what its own `SELECT` reads and the names it reads.
struct Qualify {
    /// Each `FROM` and `JOIN` source: its name as written, lower case, and
    /// the alias it goes by, if any. A derived table has only its alias.
    sources: Vec<(String, Option<String>)>,
    /// Qualifier, if any, and name, lower case. A name the `SELECT` defines as
    /// an alias stands for the names its expression reads.
    names: Vec<(Option<String>, String)>,
}

/// The names an expression reads, not descending into a subquery.
#[derive(Default)]
struct ExprNames {
    depth: usize,
    names: Vec<(Option<String>, String)>,
}

impl Visitor for ExprNames {
    type Break = ();

    fn pre_visit_query(&mut self, _: &Query) -> ControlFlow<()> {
        self.depth += 1;
        ControlFlow::Continue(())
    }

    fn post_visit_query(&mut self, _: &Query) -> ControlFlow<()> {
        self.depth -= 1;
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        if self.depth == 0 {
            match expr {
                Expr::Identifier(i) => self.names.push((None, i.value.to_lowercase())),
                Expr::CompoundIdentifier(parts) if parts.len() >= 2 => self.names.push((
                    Some(parts[parts.len() - 2].value.to_lowercase()),
                    parts[parts.len() - 1].value.to_lowercase(),
                )),
                _ => {}
            }
        }
        ControlFlow::Continue(())
    }
}

fn names_in(expr: &Expr) -> Vec<(Option<String>, String)> {
    let mut v = ExprNames::default();
    let _ = expr.visit(&mut v);
    v.names
}

/// Every `QUALIFY` and every `JOIN` of a statement, each taken in the `SELECT`
/// that holds it.
#[derive(Default)]
struct Qualifies {
    qualifies: Vec<Qualify>,
    joins: Vec<JoinKeys>,
    predicates: Vec<Predicate>,
}

impl Qualifies {
    fn body(&mut self, expr: &SetExpr) {
        match expr {
            SetExpr::Select(select) => {
                let sources = select_sources(select);
                for predicate in select.selection.iter().chain(select.having.iter()) {
                    self.predicates.push(Predicate { sources: sources.clone(), names: names_in(predicate) });
                    for sub in PredicateSubqueries::collect(predicate) {
                        if let SetExpr::Select(inner) = sub.body.as_ref() {
                            let names = inner
                                .projection
                                .iter()
                                .flat_map(|item| match item {
                                    SelectItem::UnnamedExpr(e) | SelectItem::ExprWithAlias { expr: e, .. } => names_in(e),
                                    _ => Vec::new(),
                                })
                                .collect();
                            self.predicates.push(Predicate { sources: select_sources(inner), names });
                        }
                    }
                }
                for from in &select.from {
                    for join in &from.joins {
                        use sqlparser::ast::{JoinConstraint, JoinOperator as J};
                        let constraint = match &join.join_operator {
                            J::Join(c) | J::Inner(c) | J::Left(c) | J::LeftOuter(c) | J::Right(c)
                            | J::RightOuter(c) | J::FullOuter(c) | J::CrossJoin(c) | J::Semi(c)
                            | J::LeftSemi(c) | J::RightSemi(c) | J::Anti(c) | J::LeftAnti(c)
                            | J::RightAnti(c) | J::StraightJoin(c) => Some(c),
                            J::AsOf { constraint, .. } => Some(constraint),
                            _ => None,
                        };
                        let (on, using) = match constraint {
                            Some(JoinConstraint::On(expr)) => (names_in(expr), Vec::new()),
                            Some(JoinConstraint::Using(names)) => (
                                Vec::new(),
                                names
                                    .iter()
                                    .filter_map(|n| n.to_string().rsplit('.').next().map(|p| p.trim_matches('"').to_lowercase()))
                                    .collect(),
                            ),
                            _ => continue,
                        };
                        self.joins.push(JoinKeys { sources: sources.clone(), on, using });
                    }
                }
                let Some(qualify) = &select.qualify else { return };
                // `qualify rn = 1` reads the window the select list named `rn`.
                let aliases: HashMap<String, &Expr> = select
                    .projection
                    .iter()
                    .filter_map(|item| match item {
                        SelectItem::ExprWithAlias { expr, alias } => Some((alias.value.to_lowercase(), expr)),
                        _ => None,
                    })
                    .collect();
                let mut names = Vec::new();
                for (qualifier, name) in names_in(qualify) {
                    match (&qualifier, aliases.get(&name)) {
                        (None, Some(expr)) => names.extend(names_in(expr)),
                        _ => names.push((qualifier, name)),
                    }
                }
                self.qualifies.push(Qualify { sources, names });
            }
            SetExpr::SetOperation { left, right, .. } => {
                self.body(left);
                self.body(right);
            }
            _ => {}
        }
    }
}

impl Visitor for Qualifies {
    type Break = ();

    // A nested query is visited on its own, so only this body is read here.
    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
        self.body(&query.body);
        ControlFlow::Continue(())
    }
}

/// What the syntax tree says of a statement's scopes that the engine's graph
/// cannot.
#[derive(Default)]
struct Scopes {
    merged: Vec<MergedScope>,
    /// Lower case names of the CTEs and derived tables whose select list is a
    /// `*` over a relation rather than over another CTE: their columns are
    /// whatever list that relation came with.
    star_over_relation: HashSet<String>,
    /// Lower case names of the derived tables written as a `VALUES` list.
    values: HashSet<String>,
    /// Lower case names of the `LATERAL FLATTEN` calls.
    flattens: HashSet<String>,
    qualifies: Vec<Qualify>,
    joins: Vec<JoinKeys>,
    predicates: Vec<Predicate>,
}

/// Every name the SQL gives to two scopes of one statement, see `MergedScope`,
/// and the scopes that take `*` from a relation.
///
/// Read off the syntax tree the engine's own parser gives, a second time: the
/// analysis result keeps one node per key and so cannot say that two were
/// written. Keyed exactly as the engine keys them, by statement and by the name
/// as spelled, so `t` and `T` are two names and a derived table and a CTE of one
/// name are two scopes.
fn scopes(sql: &str, dialect: Dialect) -> Scopes {
    let Ok(statements) = flowscope_core::parse_sql_with_dialect(sql, dialect) else {
        return Scopes::default();
    };
    let mut out = Scopes::default();
    for statement in &statements {
        let mut names = ScopeNames::default();
        let _ = statement.visit(&mut names);
        let mut read = Qualifies::default();
        let _ = statement.visit(&mut read);
        out.qualifies.extend(read.qualifies);
        out.joins.extend(read.joins);
        out.predicates.extend(read.predicates);
        let ctes: HashSet<String> =
            names.ctes.iter().map(|c| c.trim_matches('"').to_lowercase()).collect();
        out.values.extend(names.values.iter().cloned());
        out.flattens.extend(names.flattens.iter().cloned());
        for (scope, sources) in &names.stars {
            if sources.iter().any(|s| !ctes.contains(s)) {
                out.star_over_relation.insert(scope.clone());
            }
        }
        for (kind, list) in [("derived", names.derived), ("cte", names.ctes)] {
            let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
            for name in list {
                *counts.entry(name).or_default() += 1;
            }
            out.merged.extend(
                counts
                    .into_iter()
                    .filter(|(_, count)| *count > 1)
                    .map(|(name, count)| MergedScope { kind, name, count }),
            );
        }
    }
    out
}

fn split_dialect(adapter: &str) -> Dialect {
    match adapter {
        "snowflake" => Dialect::Snowflake,
        "bigquery" => Dialect::Bigquery,
        "databricks" => Dialect::Databricks,
        "redshift" => Dialect::Redshift,
        "postgres" => Dialect::Postgres,
        "duckdb" => Dialect::Duckdb,
        _ => Dialect::Generic,
    }
}

/// Split a qualified name on the dots that separate its parts, leaving the dots
/// inside a quoted identifier alone.
fn qualified_parts(relation: &str) -> Vec<&str> {
    let bytes = relation.as_bytes();
    let mut out = Vec::new();
    let (mut start, mut quoted) = (0, false);
    for i in 0..bytes.len() {
        match bytes[i] {
            b'"' => quoted = !quoted,
            b'.' if !quoted => {
                out.push(relation[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(relation[start..].trim());
    out
}

/// `DB.SCHEMA.NAME` split the way the schema API wants it.
///
/// Each part goes across **verbatim, quotes included**, because that is what the
/// engine needs to match the SQL. It normalises a quoted identifier by unquoting
/// it and keeping its case, and an unquoted one by folding it to the dialect's
/// case. dbt writes `relation_name` with exactly the quoting the warehouse
/// requires, and the compiled SQL repeats it, so passing it through unchanged
/// makes the two agree under either rule.
///
/// Uppercasing here instead registered `"DB"."dbo"."shop_address"` as
/// `DB.DBO.SHOP_ADDRESS`, which matched nothing. Measured on a 3341 model project:
/// 935 source relations went unresolved, and with them 781 models were reported
/// for a failure that had not happened. It cost no edges there, because
/// attribution runs through `owning_relation` rather than through the engine's
/// schema, and those models list their columns explicitly. It costs every column
/// as soon as the schema is actually needed, which is `select *` and
/// `columns_named_in`: see the test below.
fn as_table(relation: &str, columns: &[String]) -> SchemaTable {
    let parts = qualified_parts(relation);
    let (catalog, schema, name) = match parts.len() {
        0 => (None, None, ""),
        1 => (None, None, parts[0]),
        2 => (None, Some(parts[0]), parts[1]),
        _ => (
            Some(parts[parts.len() - 3]),
            Some(parts[parts.len() - 2]),
            parts[parts.len() - 1],
        ),
    };
    SchemaTable {
        catalog: catalog.map(str::to_string),
        schema: schema.map(str::to_string),
        name: name.to_string(),
        columns: columns
            .iter()
            .map(|c| ColumnSchema {
                name: c.clone(),
                data_type: None,
                is_primary_key: None,
                foreign_key: None,
            })
            .collect(),
    }
}

/// The relation a column node belongs to, or None when it belongs to a CTE, a
/// subquery or the output: those have no identity outside this statement.
fn owning_relation(node: &FsNode) -> Option<String> {
    let c = node.canonical_name.as_ref()?;
    match (&c.catalog, &c.schema) {
        (Some(cat), Some(sch)) => Some(norm_relation(&format!("{cat}.{sch}.{}", c.name))),
        (None, Some(sch)) => Some(norm_relation(&format!("{sch}.{}", c.name))),
        _ => None,
    }
}

/// True when the engine named an output column after the function that built
/// it, instead of after an alias.
///
/// It does that for a scalar subquery in a WHERE or HAVING clause, which it
/// hangs off the statement's single Output node alongside the real projections.
/// There is no structural way to tell the two apart: one Output node holds
/// both, and column nodes carry no span in this engine. So the name is the
/// evidence. A column genuinely called `coalesce` would have to be a quoted
/// identifier, and its expression would not be a call to `coalesce`.
///
/// The cost is a top-level unaliased call, whose edges are dropped rather than
/// attributed to a column name that does not exist in the warehouse. Dropping
/// is the right side to err on: an invented column is visible and wrong,
/// a missing one is only missing.
fn synthesised(label: &str, expr: &str) -> bool {
    match crate::role::leading_call(expr) {
        Some(call) => call.eq_ignore_ascii_case(label.trim()),
        None => false,
    }
}

/// The identifiers in `expr` that name a column the relation really has.
///
/// The engine attributes some derived columns to the relation rather than to a
/// column of it. A window function is the common case: `row_number() over
/// (partition by a)` arrives as a single derivation edge from the table, which
/// carries the expression but names no source column. Following only column to
/// column edges therefore cost every window column its lineage entirely.
///
/// The expression names its own inputs and the relation's column list is known,
/// so the intersection is a reading rather than a guess: a name is claimed only
/// when the relation actually has a column by that name. A SQL keyword that also
/// happens to be a column name would slip through, which adds one source to an
/// edge that already exists, and never invents a column.
fn columns_named_in(expr: &str, known: &HashSet<String>) -> Vec<String> {
    const KEYWORDS: &[&str] = &[
        "and", "as", "asc", "between", "by", "case", "cast", "current", "desc", "distinct",
        "else", "end", "following", "from", "group", "having", "in", "interval", "is", "like",
        "not", "null", "on", "or", "order", "over", "partition", "preceding", "range", "row",
        "rows", "select", "then", "unbounded", "using", "when", "where",
    ];
    let bytes = expr.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            // A string literal names nothing.
            b'\'' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'\'' {
                    i += 1;
                }
                i += 1;
            }
            c if c.is_ascii_alphabetic() || c == b'_' || c == b'"' => {
                let quoted = c == b'"';
                let start = i + usize::from(quoted);
                let mut j = start;
                while j < bytes.len()
                    && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'$')
                {
                    j += 1;
                }
                let word = expr[start..j].to_lowercase();
                let mut k = j + usize::from(quoted);
                while k < bytes.len() && (bytes[k] as char).is_whitespace() {
                    k += 1;
                }
                // A name before `(` is a function, and one before `.` qualifies
                // the column rather than being it.
                let structural = matches!(bytes.get(k), Some(b'(') | Some(b'.'));
                if !structural
                    && !KEYWORDS.contains(&word.as_str())
                    && known.contains(&word)
                    && !out.contains(&word)
                {
                    out.push(word);
                }
                i = j + usize::from(quoted);
            }
            _ => i += 1,
        }
    }
    out.sort();
    out
}

/// True when `bytes` at `at` begins the word `word`, ignoring ASCII case and
/// requiring an identifier boundary on both sides.
///
/// Compared byte for byte against the original rather than against a lowercased
/// copy. Lowercasing can change a string's length, so an index taken from one
/// and used in the other can land inside a character: this corpus has an arrow
/// in a comment, and that was enough to panic.
fn word_at(bytes: &[u8], at: usize, word: &[u8]) -> bool {
    if at + word.len() > bytes.len() || !bytes[at..at + word.len()].eq_ignore_ascii_case(word) {
        return false;
    }
    let before_ok = at == 0 || !bytes[at - 1].is_ascii_alphanumeric() && bytes[at - 1] != b'_';
    let after = bytes.get(at + word.len());
    before_ok && after.is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_')
}

/// The text inside each `OVER ( ... )` of an expression.
///
/// The engine does not read these. `collect_column_refs` in flowscope 0.9
/// descends into a function's arguments and stops there, so the `PARTITION BY`
/// and `ORDER BY` of a window are never resolved to columns. When the rest of
/// the expression happens to resolve to nothing, the derivation is hung off the
/// relation and `columns_named_in` picks the keys up; one resolvable column
/// anywhere else in the expression takes that path away and loses every key the
/// window reads. On a 3341 model project, 303 of the 391 models with a window
/// had at least one partition or order key with no edge.
///
/// Returned as slices of the input so the caller can hand them straight to
/// `columns_named_in`, which is the only thing allowed to decide that a name is
/// a column.
fn over_clauses(expr: &str) -> Vec<&str> {
    let bytes = expr.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        // A string literal can spell anything, including `over (`.
        if bytes[i] == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                i += 1;
            }
            i += 1;
            continue;
        }
        if !word_at(bytes, i, b"over") {
            i += 1;
            continue;
        }
        let mut j = i + 4;
        while j < bytes.len() && (bytes[j] as char).is_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'(' {
            i += 4;
            continue;
        }
        let body = j + 1;
        let mut depth = 1;
        let mut k = body;
        while k < bytes.len() && depth > 0 {
            match bytes[k] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            k += 1;
        }
        if depth == 0 {
            out.push(&expr[body..k - 1]);
        }
        i = k.max(i + 4);
    }
    out
}

/// The table an `UNRESOLVED_REFERENCE` could not find, normalised the way
/// `owning_relation` normalises, or None when the issue is about anything else.
///
/// The code covers a table the schema does not list and a column the engine
/// could not place, and only the first names a relation. The engine names it in
/// its own words, `Table '<name>' could not be resolved`. Should those words
/// change, the span still names it: the engine finds the span by searching the
/// SQL for that same name. A column's span is the bare column, which is why a
/// name in one part is never taken for a table.
///
/// Whether a table the engine could not find matters is for `lineage` to say.
fn unresolved_relation(code: &str, message: &str, spelled: Option<&str>) -> Option<String> {
    if code != issue_codes::UNRESOLVED_REFERENCE {
        return None;
    }
    let named = message
        .strip_prefix("Table '")
        .and_then(|rest| rest.split_once("' could not be resolved"))
        .map(|(name, _)| name);
    let name = named.or_else(|| spelled.filter(|s| qualified_parts(s).len() > 1))?;
    Some(norm_relation(name))
}

/// The token a parse error stopped at, found from the position the parser gave,
/// or None when there is no position to read or it lands on nothing.
///
/// The engine spans a parse error over the whole statement, though the message
/// ends with sqlparser's own `Line: L, Column: C`. That position counts from the
/// statement the engine parsed on its own, not from the file, and a statement
/// starts past any leading blank lines: 13 of the 21 failures on a 3341 model
/// project start past byte 0, and read against the file every one of them lands
/// on the wrong text. Columns count characters, not bytes.
///
/// A select list with nothing in it takes `from` for a column, and the parser
/// then stops on the token after it, so the span is the relation the list was
/// to come from rather than the keyword.
fn stopped_at(sql: &str, statement: (usize, usize), message: &str) -> Option<(usize, usize)> {
    let (_, at) = message.rsplit_once("Line: ")?;
    let (line, column) = at.split_once(", Column: ")?;
    let (line, column) = (line.parse().ok()?, column.parse().ok()?);
    let slice = sql.get(statement.0..statement.1)?;
    let from = line_col_to_offset(slice, line, column)?;
    let token = slice[from..].split(char::is_whitespace).next()?;
    let start = statement.0 + from;
    (!token.is_empty()).then_some((start, start + token.len()))
}

/// Where the walk back from an output column ends, for a column no edge reached.
///
/// A root is decided there rather than at the output. The engine gives a column
/// copied by name or through `select *` no expression, and hangs a projection
/// that reads no column off the base relation of its scope with the text it was
/// computed from. So `'LEDGER' as origin` set in one CTE and copied out through
/// two more shows nothing at the output, and `'LEDGER'` where the walk stops.
#[derive(Clone, Default)]
struct Ends {
    /// Ends at a literal: a relation or a CTE reached with an expression naming
    /// none of its columns, or a column computed in a CTE that reads nothing.
    literal: usize,
    /// Ends anywhere else: a copy that carried no expression, a column the CTE
    /// never had, a read the engine did not resolve.
    other: usize,
    /// Every column label reached, lower case.
    labels: HashSet<String>,
    /// Some name an expression on the way reads was not reached below it.
    ///
    /// The guard, and permanent. A projection reading no column the engine can
    /// place, an unqualified name two relations have or a form it does not
    /// read, is hung off the scope's base relation as if it read nothing, and
    /// that can be a CTE of constants. The text still names the column, so a
    /// name read and never reached is a read the engine lost, and the column a
    /// loss.
    unreached: bool,
    /// Why, for each end that is not a literal, and for each guard that failed.
    dead: Vec<DeadEnd>,
}

impl Ends {
    fn absorb(&mut self, other: Ends) {
        self.literal += other.literal;
        self.other += other.other;
        self.labels.extend(other.labels);
        self.unreached |= other.unreached;
        self.dead.extend(other.dead);
    }

    fn only_literals(&self) -> bool {
        self.literal > 0 && self.other == 0 && !self.unreached
    }

    /// An end that is not a literal, and why.
    fn dead_end(why: DeadEnd) -> Ends {
        Ends { other: 1, dead: vec![why], ..Default::default() }
    }

    /// The reasons, each once: one `NamesUnreached` holding every name, then
    /// the rest in order, and `Unresolved` when nothing better was recorded.
    fn reasons(&self) -> Vec<DeadEnd> {
        let mut names: Vec<String> = Vec::new();
        let mut out: Vec<DeadEnd> = Vec::new();
        for d in &self.dead {
            match d {
                DeadEnd::NamesUnreached { names: n } => names.extend(n.iter().cloned()),
                other => out.push(other.clone()),
            }
        }
        names.sort();
        names.dedup();
        if !names.is_empty() {
            out.push(DeadEnd::NamesUnreached { names });
        }
        out.sort();
        out.dedup();
        if out.is_empty() {
            out.push(DeadEnd::Unresolved);
        }
        out
    }
}

/// The engine's graph, as `resolve` indexed it.
struct Graph<'r, 'm> {
    by_id: &'m HashMap<&'r str, &'r FsNode>,
    owner: &'m HashMap<&'r str, &'r str>,
    owned: &'m HashMap<&'r str, Vec<&'r str>>,
    feeds: &'m Feeds<'r>,
    known: &'m HashMap<String, HashSet<String>>,
    vocabulary: &'m HashSet<String>,
    /// The last part of every relation the SQL reads, lower case: a qualified
    /// name ending in one names the table of a subquery, not a column.
    tables: &'m HashSet<String>,
    star_over_relation: &'m HashSet<String>,
    values: &'m HashSet<String>,
    flattens: &'m HashSet<String>,
}

impl<'r> Graph<'r, '_> {
    /// Walks back from `id` the way `resolve` does, carrying `text`, the
    /// expression the node was reached with.
    fn ends(
        &self,
        id: &'r str,
        text: Option<&'r str>,
        seen: &mut HashMap<(&'r str, Option<&'r str>), Option<Ends>>,
    ) -> Ends {
        match seen.get(&(id, text)) {
            Some(Some(done)) => return done.clone(),
            // A loop: the first visit speaks for the node.
            Some(None) => return Ends::default(),
            None => {}
        }
        seen.insert((id, text), None);
        let out = self.ends_of(id, text, seen);
        seen.insert((id, text), Some(out.clone()));
        out
    }

    fn ends_of(
        &self,
        id: &'r str,
        text: Option<&'r str>,
        seen: &mut HashMap<(&'r str, Option<&'r str>), Option<Ends>>,
    ) -> Ends {
        let unresolved = || Ends::dead_end(DeadEnd::Unresolved);
        let literal = Ends { literal: 1, ..Default::default() };
        let Some(node) = self.by_id.get(id) else { return unresolved() };
        let said = text.filter(|t| !t.trim().is_empty());
        if node.node_type.is_table_or_view() {
            // Reached by a derivation hung off the relation. A text naming one
            // of its columns would have made an edge; a relation whose columns
            // nobody knows cannot say it names none; no text was a copy.
            let Some(columns) = owning_relation(node).and_then(|r| self.known.get(&r)) else {
                return unresolved();
            };
            return match said {
                Some(t) if columns_named_in(t, columns).is_empty() => literal,
                _ => unresolved(),
            };
        }
        if node.node_type == NodeType::Cte {
            let mut out = Ends::default();
            let mut went = false;
            if let (Some(cols), Some(t)) = (self.owned.get(id), said) {
                let labels: HashMap<String, &str> = cols
                    .iter()
                    .filter_map(|c| self.by_id.get(c).map(|n| (n.label.to_lowercase(), *c)))
                    .collect();
                let names: HashSet<String> = labels.keys().cloned().collect();
                for want in columns_named_in(t, &names) {
                    if let Some(next) = labels.get(&want) {
                        out.absorb(self.ends(next, text, seen));
                        went = true;
                    }
                }
            }
            for (from, _, edge_text) in self.feeds.get(id).into_iter().flatten() {
                out.absorb(self.ends(from, edge_text.or(text), seen));
                went = true;
            }
            if !went {
                // A CTE reading nothing, and an expression none of its columns
                // answers: a CTE of constants.
                out.absorb(if said.is_some() { literal } else { unresolved() });
            }
            return out;
        }
        if node.node_type != NodeType::Column {
            return unresolved();
        }
        let owner = self.owner.get(id);
        if owner.and_then(|o| self.by_id.get(o)).is_some_and(|o| o.node_type.is_table_or_view()) {
            // A column of a real relation makes an edge, so an output reaching
            // one is fed and never asks.
            return unresolved();
        }
        let ups = self.feeds.get(id).map(Vec::as_slice).unwrap_or_default();
        let mut below = Ends::default();
        for (from, _, edge_text) in ups {
            below.absorb(self.ends(from, edge_text.or(text), seen));
        }
        let own = node.expression.as_deref().filter(|e| !e.trim().is_empty());
        for t in own.into_iter().chain(ups.iter().filter_map(|u| u.2)) {
            let names: Vec<String> = read_names(t, self.vocabulary, self.tables)
                .into_iter()
                .filter(|n| !below.labels.contains(n))
                .collect();
            if !names.is_empty() {
                below.unreached = true;
                below.dead.push(DeadEnd::NamesUnreached { names });
            }
        }
        if ups.is_empty() {
            // Nothing feeds it. With an expression that reads no name, it is
            // built from literals: in a CTE that reads nothing, a spine or
            // `select '' as x`, and in a CTE that joins several relations too,
            // where the engine hangs a projection reading no column off none of
            // them. With no expression it is a name the CTE never had, and with
            // one reading a name the engine never placed, the guard above has
            // it: both losses.
            let reads_nothing = owner.is_none_or(|o| self.feeds.get(o).is_none_or(Vec::is_empty));
            let via = owner
                .and_then(|o| self.by_id.get(o).map(|n| (*o, n)))
                .filter(|(_, n)| n.node_type != NodeType::Output);
            if own.is_some() && (reads_nothing || !below.unreached) {
                below.absorb(literal);
            } else if let (None, Some((o, n))) = (own, via) {
                let reads = self.relations_under(o);
                let label = n.label.trim_matches('"').to_lowercase();
                let column = node.label.to_lowercase();
                if reads.is_empty() && self.values.contains(&label) {
                    // A column of a `VALUES` list, `column1` and the rest: the
                    // engine gives the list no columns, but its rows are written
                    // in the SQL, so the walk ends at literals.
                    below.absorb(literal);
                } else if self.flattens.contains(&label) && matches!(column.as_str(), "seq" | "index") {
                    // A FLATTEN's SEQ and INDEX number the rows and the elements:
                    // a position, which no column of the input holds.
                    below.absorb(literal);
                } else {
                    let star = self.star_over_relation.contains(&label);
                    below.absorb(Ends::dead_end(DeadEnd::Phantom { via: n.label.to_lowercase(), column, reads, star }));
                }
            } else if below.unreached {
                // The guard above has already named what it reads.
                below.other += 1;
            } else {
                below.absorb(unresolved());
            }
        }
        below.labels.insert(node.label.to_lowercase());
        below
    }
}

impl<'r> Graph<'r, '_> {
    fn relations_under(&self, id: &'r str) -> Vec<String> {
        relations_under(self.by_id, self.feeds, id)
    }
}

/// For each node of the engine's graph, the nodes that feed it: data flows
/// backwards from an output column. Each with whether the edge is a
/// derivation, and the expression it carries if any.
type Feeds<'r> = HashMap<&'r str, Vec<(&'r str, bool, Option<&'r str>)>>;

/// The engine's graph, enough of it to find an `UnbackedRead`.
struct Unbacked<'r, 'm> {
    by_id: &'m HashMap<&'r str, &'r FsNode>,
    owner: &'m HashMap<&'r str, &'r str>,
    owned: &'m HashMap<&'r str, Vec<&'r str>>,
    feeds: &'m Feeds<'r>,
    known: &'m HashMap<String, HashSet<String>>,
    star_over_relation: &'m HashSet<String>,
    values: &'m HashSet<String>,
    flattens: &'m HashSet<String>,
}

/// The columns Snowflake documents for a `LATERAL FLATTEN`, lower case.
const FLATTEN_COLUMNS: [&str; 6] = ["seq", "key", "path", "index", "value", "this"];

impl<'r> Unbacked<'r, '_> {
    /// A column the engine made for a CTE because a scope read it, with
    /// nothing feeding it and no expression.
    fn phantom(&self, id: &str) -> bool {
        self.by_id.get(id).is_some_and(|n| {
            n.node_type == NodeType::Column
                && n.expression.is_none()
                && self.feeds.get(id).is_none_or(Vec::is_empty)
        })
    }

    /// The names the feed from `from` reads out of a CTE that does not
    /// project them, each with the CTE and the relations under it whose list
    /// has the name.
    ///
    /// Two shapes. A derivation the engine hung off the CTE, whose text names
    /// what the CTE does not project: a projection reading nothing the engine
    /// could place lands there. And a phantom of the CTE: the same read, where
    /// the engine did see the name.
    ///
    /// Not when the CTE is a `*` over a relation: its columns are whatever
    /// list that relation came with, so a missing name says that list is
    /// short, not that the SQL cannot run.
    fn reads(&self, from: &'r str, derived: bool, text: Option<&str>) -> Vec<(String, String, Vec<String>)> {
        let Some(node) = self.by_id.get(from) else { return Vec::new() };
        let (cte, names): (&'r str, Vec<String>) = match (node.node_type, text) {
            (NodeType::Cte, Some(text)) if derived => {
                let projected: HashSet<String> = self
                    .owned
                    .get(from)
                    .into_iter()
                    .flatten()
                    .filter(|c| !self.phantom(c))
                    .filter_map(|c| self.by_id.get(c).map(|n| n.label.to_lowercase()))
                    .collect();
                let under: HashSet<String> = relations_under(self.by_id, self.feeds, from)
                    .iter()
                    .filter_map(|r| self.known.get(r))
                    .flatten()
                    .cloned()
                    .collect();
                let names = columns_named_in(text, &under).into_iter().filter(|n| !projected.contains(n));
                (from, names.collect())
            }
            (NodeType::Column, _) if self.phantom(from) => {
                let Some(cte) = self.owner.get(from).copied() else { return Vec::new() };
                if self.by_id.get(cte).is_none_or(|n| n.node_type != NodeType::Cte) {
                    return Vec::new();
                }
                (cte, vec![node.label.to_lowercase()])
            }
            _ => return Vec::new(),
        };
        let Some(cte_node) = self.by_id.get(cte) else { return Vec::new() };
        let label = cte_node.label.to_lowercase();
        let bare = label.trim_matches('"');
        // A `VALUES` list's columns are its rows', and a FLATTEN has the six it
        // documents: the engine gives the one none and the other's position
        // columns nothing to feed them, which is no read the SQL cannot back.
        let mut names = names;
        if self.flattens.contains(bare) {
            names.retain(|n| !FLATTEN_COLUMNS.contains(&n.as_str()));
        }
        if names.is_empty() || self.star_over_relation.contains(bare) || self.values.contains(bare) {
            return Vec::new();
        }
        let under = relations_under(self.by_id, self.feeds, cte);
        names
            .into_iter()
            .map(|column| {
                let owners = under
                    .iter()
                    .filter(|r| self.known.get(*r).is_some_and(|k| k.contains(&column)))
                    .cloned()
                    .collect();
                (label.clone(), column, owners)
            })
            .collect()
    }
}

/// The base columns a column of a CTE or derived table comes from, following
/// the engine's column to column edges back to a relation, renames included.
fn scope_column_sources<'r>(
    by_id: &HashMap<&'r str, &'r FsNode>,
    owner: &HashMap<&'r str, &'r str>,
    owned: &HashMap<&'r str, Vec<&'r str>>,
    feeds: &Feeds<'r>,
    scope: &'r str,
    column: &str,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let start = owned
        .get(scope)
        .into_iter()
        .flatten()
        .filter(|c| by_id.get(*c).is_some_and(|n| n.label.eq_ignore_ascii_case(column)));
    let mut stack: Vec<&'r str> = start.copied().collect();
    let mut seen: HashSet<&'r str> = HashSet::new();
    while let Some(at) = stack.pop() {
        if !seen.insert(at) {
            continue;
        }
        let Some(node) = by_id.get(at) else { continue };
        let table = owner.get(at).and_then(|o| by_id.get(o)).filter(|o| o.node_type.is_table_or_view());
        if let Some(rel) = table.and_then(|t| owning_relation(t)) {
            out.push((rel, node.label.to_lowercase()));
            continue;
        }
        for (from, _, _) in feeds.get(at).into_iter().flatten() {
            if by_id.get(from).is_some_and(|n| n.node_type == NodeType::Column) {
                stack.push(from);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Every relation a CTE reads, through the CTEs it reads in turn, normalised
/// and sorted.
fn relations_under<'r>(
    by_id: &HashMap<&'r str, &'r FsNode>,
    feeds: &Feeds<'r>,
    id: &'r str,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut stack: Vec<&'r str> = vec![id];
    let mut seen: HashSet<&'r str> = HashSet::new();
    while let Some(at) = stack.pop() {
        if !seen.insert(at) {
            continue;
        }
        for (from, _, _) in feeds.get(at).into_iter().flatten() {
            match by_id.get(from) {
                Some(n) if n.node_type.is_table_or_view() => out.extend(owning_relation(n)),
                Some(n) if n.node_type == NodeType::Cte => stack.push(from),
                _ => {}
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The names `expr` reads: the ones in `vocabulary`, as `columns_named_in`
/// finds them, and the column part of every qualified name, unless it is one of
/// `tables`, which is a subquery's `from db.sch.t` rather than a column.
fn read_names(expr: &str, vocabulary: &HashSet<String>, tables: &HashSet<String>) -> Vec<String> {
    let mut out = columns_named_in(expr, vocabulary);
    for tail in qualified_tails(expr) {
        if !out.contains(&tail) && !tables.contains(&tail) {
            out.push(tail);
        }
    }
    out
}

/// The column part of each qualified name in `expr`, lower case: `o.amount`
/// gives `amount`, `db.sch.t.x` gives `x`. A name before `(` or `.` is not a
/// column, and a string literal names nothing.
fn qualified_tails(expr: &str) -> Vec<String> {
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let bytes = expr.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'\'' {
                    i += 1;
                }
                i += 1;
            }
            b'.' if i > 0 && (ident(bytes[i - 1]) || bytes[i - 1] == b'"') => {
                let quoted = bytes.get(i + 1) == Some(&b'"');
                let start = i + 1 + usize::from(quoted);
                let mut j = start;
                while j < bytes.len() && ident(bytes[j]) {
                    j += 1;
                }
                if j > start && !bytes[start].is_ascii_digit() {
                    let mut k = j + usize::from(quoted && bytes.get(j) == Some(&b'"'));
                    while k < bytes.len() && (bytes[k] as char).is_whitespace() {
                        k += 1;
                    }
                    let word = expr[start..j].to_lowercase();
                    if !matches!(bytes.get(k), Some(b'(') | Some(b'.')) && !out.contains(&word) {
                        out.push(word);
                    }
                }
                i = j.max(i + 1);
            }
            _ => i += 1,
        }
    }
    out
}

/// Adds `edge` unless its column pair has one already. Then the louder of the
/// two expressions stands (0009), and of two as loud the greater text, so that
/// neither the role nor the text depends on which path the walk took first:
/// the engine lists its edges in no order the SQL decides, and swapping the
/// branches of a union swapped the role.
fn put_edge(edges: &mut Vec<RawEdge>, at: &mut HashMap<(String, String, String), usize>, edge: RawEdge) {
    let key = (edge.from_relation.clone(), edge.from_column.clone(), edge.to_column.clone());
    match at.get(&key) {
        None => {
            at.insert(key, edges.len());
            edges.push(edge);
        }
        Some(&i) => {
            let held = &edges[i].expression;
            let (was, now) = (
                crate::role::of_expression(held).rank(),
                crate::role::of_expression(&edge.expression).rank(),
            );
            if now > was || (now == was && edge.expression > *held) {
                edges[i].expression = edge.expression;
            }
        }
    }
}

pub fn resolve(sql: &str, adapter: &str, visible: &[Visible]) -> Resolved {
    let req = AnalyzeRequest {
        sql: sql.to_string(),
        files: None,
        dialect: split_dialect(adapter),
        source_name: None,
        options: None,
        schema: Some(SchemaMetadata {
            default_catalog: None,
            default_schema: None,
            search_path: None,
            case_sensitivity: None,
            allow_implied: true,
            tables: visible.iter().map(|v| as_table(&v.relation, &v.columns)).collect(),
        }),
        template_config: None,
    };

    let res = flowscope_core::analyze(&req);
    let mut out = Resolved {
        issues: res
            .issues
            .iter()
            .map(|i| {
                let mut span = i.span.map(|s| (s.start, s.end));
                if i.code == issue_codes::PARSE_ERROR {
                    span = span.map(|s| stopped_at(sql, s, &i.message).unwrap_or(s));
                }
                let spelled = span.and_then(|(a, b)| sql.get(a..b));
                Issue {
                    code: i.code.clone(),
                    message: i.message.clone(),
                    severity: match i.severity {
                        Severity::Error => "error",
                        Severity::Warning => "warning",
                        Severity::Info => "info",
                    },
                    span,
                    degrading: DEGRADING.contains(&i.code.as_str()),
                    relation: unresolved_relation(&i.code, &i.message, spelled),
                }
            })
            .collect(),
        ..Default::default()
    };
    // The engine gives some warnings in an order that changes from one run to
    // the next, so the report did too. By where they sit in the SQL, which is
    // also the order a parse meets them in.
    out.issues.sort_by(|a, b| {
        let at = |i: &Issue| i.span.unwrap_or((usize::MAX, usize::MAX));
        (at(a), &a.code, &a.message).cmp(&(at(b), &b.code, &b.message))
    });
    out.approximate = out.issues.iter().any(|i| i.degrading);
    out.unknown_columns = out
        .issues
        .iter()
        .filter(|i| i.code == issue_codes::UNKNOWN_COLUMN)
        .map(|i| unknown_column(&i.message))
        .collect();
    if res.statements.is_empty() {
        // The first error is the one that stopped the parse; a later warning
        // describes the wreckage rather than the cause.
        let first = out
            .issues
            .iter()
            .find(|i| i.severity == "error")
            .or_else(|| out.issues.first());
        out.parse_error = Some(match first {
            Some(i) if i.message.is_empty() => i.code.clone(),
            Some(i) => format!("{}: {}", i.code, i.message),
            None => "no statement parsed".into(),
        });
        return out;
    }
    let Scopes { merged, star_over_relation, values, flattens, qualifies, joins, predicates } =
        scopes(sql, split_dialect(adapter));
    out.merged_scopes = merged;

    let by_id: HashMap<&str, &FsNode> = res.nodes.iter().map(|n| (&*n.id, n)).collect();

    let mut reads: Vec<String> = res
        .nodes
        .iter()
        .filter(|n| n.node_type.is_table_or_view())
        .filter_map(owning_relation)
        .collect();
    reads.sort();
    reads.dedup();
    out.reads = reads;

    // What each relation really has, to check a recovered column name against.
    let known: HashMap<String, HashSet<String>> = visible
        .iter()
        .map(|v| {
            (
                norm_relation(&v.relation),
                v.columns.iter().map(|c| c.to_lowercase()).collect(),
            )
        })
        .collect();

    // Ownership tells which table, CTE or output a column hangs off, and the
    // inverse says which columns a CTE has, which is how a derivation hung off
    // the CTE itself is followed rather than abandoned.
    let mut owner: HashMap<&str, &str> = HashMap::new();
    let mut owned: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &res.edges {
        if e.edge_type == EdgeType::Ownership {
            owner.insert(&e.to, &e.from);
            owned.entry(&e.from).or_default().push(&e.to);
        }
    }

    // Data flows backwards: for an output column, which columns fed it.
    let mut feeds: Feeds = HashMap::new();
    for e in &res.edges {
        match e.edge_type {
            EdgeType::DataFlow | EdgeType::Derivation => {
                let derived = e.edge_type == EdgeType::Derivation;
                feeds.entry(&e.to).or_default().push((
                    &e.from,
                    derived,
                    e.expression.as_deref(),
                ));
            }
            _ => {}
        }
    }

    let output_ids: HashSet<&str> = res
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::Output)
        .map(|n| &*n.id)
        .collect();

    let mut seen_output: HashSet<String> = HashSet::new();
    // One edge per column pair, the loudest expression any path gave it: see
    // `put_edge`.
    let mut seen_edge: HashMap<(String, String, String), usize> = HashMap::new();
    let mut seen_unbacked: HashSet<(String, String, String)> = HashSet::new();
    // The text behind each output column, kept to tell a root from a loss once
    // the walk is done, and the nodes behind it, to walk again for that.
    let mut expr_of: HashMap<String, String> = HashMap::new();
    let mut nodes_of: HashMap<String, Vec<&str>> = HashMap::new();

    for col in res.nodes.iter().filter(|n| n.node_type == NodeType::Column) {
        let Some(own) = owner.get(&*col.id) else { continue };
        if !output_ids.contains(own) {
            continue;
        }
        if synthesised(&col.label, col.expression.as_deref().unwrap_or("")) {
            continue;
        }
        let to_column = col.label.to_lowercase();
        nodes_of.entry(to_column.clone()).or_default().push(&col.id);
        if seen_output.insert(to_column.clone()) {
            out.outputs.push(to_column.clone());
            expr_of.insert(
                to_column.clone(),
                col.expression.as_deref().unwrap_or_default().to_string(),
            );
        }

        // Walk back through CTE and subquery columns until a real relation is
        // reached. A self join or a recursive CTE can loop, hence `visited`.
        //
        // Each step carries two texts. `expr` is the loudest expression met on
        // the path, which decides the role (0009). `reading` is the one names
        // are read out of: the expression of the edge last followed, or across
        // a plain copy the one before it. A name belongs to the scope that
        // wrote it, and the loudest text can come from a scope further out,
        // whose names are another scope's columns.
        let mut stack: Vec<(&str, bool, Option<&str>, Option<&str>)> =
            vec![(&col.id, false, col.expression.as_deref(), col.expression.as_deref())];
        // Keyed by the whole state, so that a node two paths reach is read for
        // each text and weighed for each role: the first path to arrive must
        // not decide either (0009, 0031). Every text is the engine's, so the
        // states are finite and a recursive CTE still ends.
        let mut visited: HashSet<(&str, bool, Option<&str>, Option<&str>)> = HashSet::new();
        while let Some((id, derived, expr, reading)) = stack.pop() {
            if !visited.insert((id, derived, expr, reading)) {
                continue;
            }
            // A derivation the engine hung off the relation itself rather than
            // off one of its columns. See `columns_named_in`.
            if let Some(node) = by_id.get(id) {
                if node.node_type == NodeType::Table || node.node_type == NodeType::View {
                    if let Some(rel) = owning_relation(node) {
                        let text = expr.unwrap_or_default();
                        let empty = HashSet::new();
                        let names = columns_named_in(reading.unwrap_or_default(), known.get(&rel).unwrap_or(&empty));
                        for from_column in names {
                            put_edge(
                                &mut out.edges,
                                &mut seen_edge,
                                RawEdge {
                                    from_relation: rel.clone(),
                                    from_column,
                                    to_column: to_column.clone(),
                                    expression: text.to_string(),
                                },
                            );
                        }
                    }
                    continue;
                }
                // The same derivation, hung off a CTE instead. A dbt model is a
                // chain of CTEs, so this is where it lands far more often than
                // on a table, and following only the CTE's own upstream relation
                // cost every window and aggregate computed mid chain its lineage.
                //
                // The CTE's own columns are the vocabulary: a name is followed
                // only when the CTE really has a column by it, and then the walk
                // carries on from that column towards a real relation.
                //
                // Nothing else is followed from here. The CTE also carries a
                // relation level edge to whatever it reads, and mining the
                // text against that relation made an edge of every name the
                // CTE does not project. In valid SQL a scope can only read what
                // the CTE projects, so such a name is either the SQL reading a
                // column that is not there, an `UnbackedRead` taken where the
                // derivation is met below, or a column of another relation of
                // the scope, which that relation does not say it is.
                if node.node_type == NodeType::Cte {
                    if let (Some(cols), Some(text)) = (owned.get(id), reading) {
                        let labels: HashMap<String, &str> = cols
                            .iter()
                            .filter_map(|c| by_id.get(c).map(|n| (n.label.to_lowercase(), *c)))
                            .collect();
                        let names: HashSet<String> = labels.keys().cloned().collect();
                        for want in columns_named_in(text, &names) {
                            if let Some(next) = labels.get(&want) {
                                stack.push((next, true, expr, Some(text)));
                            }
                        }
                    }
                    continue;
                }
            }
            if let (Some(node), Some(own)) = (by_id.get(id), owner.get(id)) {
                if let Some(owner_node) = by_id.get(own) {
                    if owner_node.node_type == NodeType::Table || owner_node.node_type == NodeType::View {
                        if let Some(rel) = owning_relation(owner_node) {
                            let from_column = node.label.to_lowercase();
                            let text = if derived { expr.unwrap_or_default() } else { "" };
                            put_edge(
                                &mut out.edges,
                                &mut seen_edge,
                                RawEdge {
                                    from_relation: rel.clone(),
                                    from_column,
                                    to_column: to_column.clone(),
                                    expression: text.to_string(),
                                },
                            );
                            continue;
                        }
                    }
                }
            }
            if let Some(ups) = feeds.get(id) {
                // A read of a name the CTE it comes from does not project. Taken
                // here, where the scope doing the reading is known: a name that
                // scope projects itself is a lateral alias, not such a read.
                // Built on the first such read only: a scope can hold hundreds
                // of columns, every path from an output crosses it, and such
                // reads are rare.
                let mut lateral: Option<HashSet<String>> = None;
                for (from, is_derived, edge_expr) in ups {
                    let found = Unbacked {
                        by_id: &by_id,
                        owner: &owner,
                        owned: &owned,
                        feeds: &feeds,
                        known: &known,
                        star_over_relation: &star_over_relation,
                        values: &values,
                        flattens: &flattens,
                    }
                    .reads(from, *is_derived, *edge_expr);
                    for (cte, column, owners) in found {
                        let lateral = lateral.get_or_insert_with(|| {
                            owner
                                .get(id)
                                .and_then(|o| owned.get(o))
                                .into_iter()
                                .flatten()
                                .filter(|c| **c != id)
                                .filter_map(|c| by_id.get(c).map(|n| n.label.to_lowercase()))
                                .collect()
                        });
                        if lateral.contains(&column) {
                            continue;
                        }
                        if seen_unbacked.insert((to_column.clone(), cte.clone(), column.clone())) {
                            let output = to_column.clone();
                            out.unbacked.push(UnbackedRead { output, cte, column, owners });
                        }
                    }
                    // Once an expression is involved anywhere on the path, the
                    // column is not a passthrough, however many hops follow.
                    let carried = derived || *is_derived;
                    // Of two expressions met on one path, keep the one that
                    // claims the most. See `role::rank`.
                    let held = if derived { expr } else { None };
                    let text = match (held, *edge_expr) {
                        (Some(a), Some(b)) => {
                            let (ra, rb) = (
                                crate::role::of_expression(a).rank(),
                                crate::role::of_expression(b).rank(),
                            );
                            Some(if rb > ra { b } else { a })
                        }
                        (a, b) => a.or(b),
                    };
                    // A column the engine resolved took the column to column
                    // path, so `columns_named_in` never ran on its expression,
                    // and whatever a window there reads is unread. Read it at
                    // this hop, against the scope the column comes from: a table
                    // gives the edge at once, a CTE the columns to go on from.
                    // See `over_clauses`.
                    if let (Some(own), Some(scope)) =
                        (*edge_expr, owner.get(from).and_then(|o| by_id.get(o).map(|n| (*o, *n))))
                    {
                        for body in over_clauses(own) {
                            if scope.1.node_type.is_table_or_view() {
                                let Some(rel) = owning_relation(scope.1) else { continue };
                                let empty = HashSet::new();
                                for from_column in columns_named_in(body, known.get(&rel).unwrap_or(&empty)) {
                                    put_edge(
                                        &mut out.edges,
                                        &mut seen_edge,
                                        RawEdge {
                                            from_relation: rel.clone(),
                                            from_column,
                                            to_column: to_column.clone(),
                                            expression: text.unwrap_or_default().to_string(),
                                        },
                                    );
                                }
                            } else if scope.1.node_type == NodeType::Cte {
                                let labels: HashMap<String, &str> = owned
                                    .get(scope.0)
                                    .into_iter()
                                    .flatten()
                                    .filter_map(|c| by_id.get(c).map(|n| (n.label.to_lowercase(), *c)))
                                    .collect();
                                let names: HashSet<String> = labels.keys().cloned().collect();
                                for want in columns_named_in(body, &names) {
                                    if let Some(next) = labels.get(&want) {
                                        stack.push((next, true, text, Some(own)));
                                    }
                                }
                            }
                        }
                    }
                    stack.push((from, carried, text, edge_expr.or(reading)));
                }
            }
        }
    }

    // What the SQL reads to decide which rows exist. Three separate blind spots, each
    // resolved the same way: take the clause text the engine does give us, and
    // let `columns_named_in` decide which words in it are columns the relation
    // really has.
    let mut seen_indirect: HashSet<(String, String, crate::role::Role)> = HashSet::new();
    let note = |rel: &str, text: &str, role: crate::role::Role,
                known: &HashMap<String, HashSet<String>>,
                out: &mut Vec<IndirectRead>,
                seen: &mut HashSet<(String, String, crate::role::Role)>| {
        let empty = HashSet::new();
        for column in columns_named_in(text, known.get(rel).unwrap_or(&empty)) {
            if seen.insert((rel.to_string(), column.clone(), role)) {
                out.push(IndirectRead { relation: rel.to_string(), column, role });
            }
        }
    };

    // Predicates the engine localised to the relation whose columns they read.
    for node in res.nodes.iter().filter(|n| n.node_type.is_table_or_view()) {
        let Some(rel) = owning_relation(node) else { continue };
        for f in &node.filters {
            let role = match f.clause_type {
                FilterClauseType::JoinOn => crate::role::Role::JoinKey,
                FilterClauseType::Where | FilterClauseType::Having => crate::role::Role::Filter,
            };
            note(&rel, &f.expression, role, &known, &mut out.indirect, &mut seen_indirect);
        }
    }

    // `QUALIFY`, which the engine's analyzer never visits: it hides the dedup
    // key of every Data Vault satellite and every `row_number() = 1`. Each
    // clause is read in the `SELECT` that holds it, from the syntax tree, and a
    // name goes to the one source of that `SELECT` having it, through any CTE
    // back to the relation, renames followed. A name several sources have, or
    // a qualifier that binds to none, is counted and read by nobody: Snowflake
    // refuses an ambiguous name, so ambiguity means the scope was misread.
    let ctes: HashMap<String, Vec<&str>> = res
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::Cte)
        .fold(HashMap::new(), |mut m, n| {
            m.entry(n.label.to_lowercase()).or_insert_with(Vec::new).push(&*n.id);
            m
        });
    // Each name in the scope of its own `SELECT`: to the source its qualifier
    // binds, or to the one source having it, followed through CTEs back to a
    // relation whose list has the column. `None` when several sources have it
    // or a qualifier binds none, which the caller counts.
    let place = |sources: &[(String, Option<String>)],
                 qualifier: &Option<String>,
                 name: &String|
     -> Option<Vec<(String, String)>> {
        let bound: Vec<&(String, Option<String>)> = sources
            .iter()
            .filter(|(source, alias)| match qualifier {
                Some(qn) => {
                    alias.as_ref() == Some(qn)
                        || (alias.is_none() && source.rsplit('.').next() == Some(qn.as_str()))
                }
                None => true,
            })
            .collect();
        let mut owners: Vec<Vec<(String, String)>> = Vec::new();
        for (source, _) in &bound {
            let found: Vec<(String, String)> = match ctes.get(source.as_str()) {
                Some(ids) if !source.contains('.') => ids
                    .iter()
                    .flat_map(|id| scope_column_sources(&by_id, &owner, &owned, &feeds, id, name))
                    .collect(),
                _ => {
                    let rel = norm_relation(source);
                    vec![(rel, name.clone())]
                }
            };
            let found: Vec<(String, String)> =
                found.into_iter().filter(|(r, c)| known.get(r).is_some_and(|k| k.contains(c))).collect();
            if !found.is_empty() {
                owners.push(found);
            }
        }
        match owners.len() {
            0 if qualifier.is_some() && bound.is_empty() => None,
            0 => Some(Vec::new()),
            1 => Some(owners.remove(0)),
            _ => None,
        }
    };
    let mut record = |pairs: Vec<(String, String)>, role: crate::role::Role, out: &mut Resolved| {
        for (rel, column) in pairs {
            if seen_indirect.insert((rel.clone(), column.clone(), role)) {
                out.indirect.push(IndirectRead { relation: rel, column, role });
            }
        }
    };
    for q in &qualifies {
        for (qualifier, name) in &q.names {
            match place(&q.sources, qualifier, name) {
                Some(pairs) => record(pairs, crate::role::Role::DedupKey, &mut out),
                None => out.unplaced_qualify += 1,
            }
        }
    }
    // A join key, read where the JOIN is written, in the scope of its SELECT:
    // the engine hands over a join's condition only for a relation feeding no
    // output, and only the side that relation is on. A `USING` column is on
    // both sides by definition, so every source having it is credited.
    for j in &joins {
        for (qualifier, name) in &j.on {
            match place(&j.sources, qualifier, name) {
                Some(pairs) => record(pairs, crate::role::Role::JoinKey, &mut out),
                None => out.unplaced_join += 1,
            }
        }
        for name in &j.using {
            for (source, alias) in &j.sources {
                let one = [(source.clone(), alias.clone())];
                if let Some(pairs) = place(&one, &None, name) {
                    record(pairs, crate::role::Role::JoinKey, &mut out);
                }
            }
        }
    }
    // A filter read in the scope of its SELECT. The engine localises a
    // predicate to a table it reads directly, and those stand; one over a
    // CTE's columns it leaves on the CTE, and a subquery inside the predicate
    // it reads as if it were a column of the output.
    for p in &predicates {
        for (qualifier, name) in &p.names {
            let binds = qualifier.as_ref().is_none_or(|qn| {
                p.sources.iter().any(|(source, alias)| {
                    alias.as_ref() == Some(qn) || (alias.is_none() && source.rsplit('.').next() == Some(qn.as_str()))
                })
            });
            match place(&p.sources, qualifier, name) {
                Some(pairs) => record(pairs, crate::role::Role::Filter, &mut out),
                None if binds => out.unplaced_filter += 1,
                None => {}
            }
        }
    }
    out.indirect.sort_by(|a, b| {
        (&a.relation, &a.column, a.role.as_str()).cmp(&(&b.relation, &b.column, b.role.as_str()))
    });

    // A column with no edge is a root when the SQL builds it from literals, and a
    // loss otherwise. Telling them apart matters both ways. `cast(null as text)
    // as x` has nowhere to come from, so calling it a gap sends a reader looking
    // for a parent that cannot exist. But a column copied from somewhere that lost
    // its edge on the way is a loss, and labelling that a root would hide the very
    // gap the report exists to show.
    //
    // Two readings, either one enough. The output's own expression names no
    // column any relation in scope has. Or the walk, followed to where it ends,
    // ends only at literals: a copy carries no expression, so a literal set in a
    // CTE and copied out by name or through `select *` shows nothing at the
    // output and everything at the far end. See `Ends`.
    let fed: HashSet<&str> = out.edges.iter().map(|e| e.to_column.as_str()).collect();
    let anywhere: HashSet<String> = known.values().flatten().cloned().collect();
    // Every name that could be a column here, so that the guard in `Ends` sees a
    // read whatever the engine made of it. The text under an unresolved reference
    // and the column an unknown one names are in, so that a name the engine
    // could not place is never taken for no name at all.
    let mut vocabulary: HashSet<String> = anywhere.clone();
    vocabulary.extend(
        res.nodes.iter().filter(|n| n.node_type == NodeType::Column).map(|n| n.label.to_lowercase()),
    );
    for i in out.issues.iter().filter(|i| i.code == issue_codes::UNKNOWN_COLUMN) {
        // "Column 'x' not found in table ...", and the message is all it gives.
        if let Some(name) = i.message.split('\'').nth(1) {
            vocabulary.insert(name.to_lowercase());
        }
    }
    for i in out.issues.iter().filter(|i| i.code == issue_codes::UNRESOLVED_REFERENCE) {
        if let Some(text) = i.span.and_then(|(a, b)| sql.get(a..b)) {
            vocabulary.extend(
                text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
                    .filter(|w| !w.is_empty())
                    .map(str::to_lowercase),
            );
        }
    }
    let tables: HashSet<String> = out
        .reads
        .iter()
        .chain(known.keys())
        .filter_map(|r| r.rsplit('.').next())
        .map(str::to_lowercase)
        .collect();
    let graph = Graph {
        by_id: &by_id,
        owner: &owner,
        owned: &owned,
        feeds: &feeds,
        known: &known,
        vocabulary: &vocabulary,
        tables: &tables,
        star_over_relation: &star_over_relation,
        values: &values,
        flattens: &flattens,
    };
    // Lost is measured against the engine's own edges. A column fed only by the
    // model's own relation counts as fed here although the pass drops that edge:
    // what it does with a self read is its call, not a fact about the SQL.
    for c in out.outputs.iter().filter(|c| !fed.contains(c.as_str())) {
        let named = match expr_of.get(c.as_str()) {
            Some(text) if !text.trim().is_empty() => columns_named_in(text, &anywhere).is_empty(),
            _ => false,
        };
        if named {
            out.roots.push(c.clone());
            continue;
        }
        let mut ends = Ends::default();
        let mut literal = true;
        for id in nodes_of.get(c.as_str()).into_iter().flatten() {
            let text = by_id.get(id).and_then(|n| n.expression.as_deref());
            let found = graph.ends(id, text, &mut HashMap::new());
            literal &= found.only_literals();
            ends.absorb(found);
        }
        if literal && ends.literal > 0 {
            out.roots.push(c.clone());
        } else {
            out.lost.push(Lost { column: c.clone(), ends: ends.reasons() });
        }
    }

    out.outputs.sort();
    out.roots.sort();
    out.lost.sort_by(|a, b| a.column.cmp(&b.column));
    out.unbacked.sort_by(|a, b| (&a.output, &a.cte, &a.column).cmp(&(&b.output, &b.cte, &b.column)));
    out.edges.sort_by(|a, b| {
        (&a.to_column, &a.from_relation, &a.from_column)
            .cmp(&(&b.to_column, &b.from_relation, &b.from_column))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vis(rel: &str, cols: &[&str]) -> Visible {
        Visible {
            relation: rel.to_string(),
            columns: cols.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn a_star_expands_from_the_supplied_schema() {
        let r = resolve(
            "select * from db.sch.orders",
            "snowflake",
            &[vis("DB.SCH.ORDERS", &["id", "amount"])],
        );
        assert_eq!(r.outputs, vec!["amount", "id"]);
        assert_eq!(r.edges.len(), 2);
        assert!(r.edges.iter().all(|e| e.from_relation == "DB.SCH.ORDERS"));
        assert!(r.edges.iter().all(|e| e.expression.is_empty()), "a star is a copy");
    }

    #[test]
    fn a_star_travels_through_a_cte_chain() {
        let r = resolve(
            "with a as (select * from db.sch.orders), b as (select * from a) select * from b",
            "snowflake",
            &[vis("DB.SCH.ORDERS", &["id", "amount"])],
        );
        assert_eq!(r.outputs, vec!["amount", "id"]);
        assert_eq!(r.edges.len(), 2, "edges must reach the table, not stop at the CTE");
        assert!(r.edges.iter().all(|e| e.from_relation == "DB.SCH.ORDERS"));
    }

    #[test]
    fn an_expression_is_carried_out_to_the_edge() {
        let r = resolve(
            "select amount * 1.2 as gross from db.sch.orders",
            "snowflake",
            &[vis("DB.SCH.ORDERS", &["id", "amount"])],
        );
        let e = r.edges.iter().find(|e| e.to_column == "gross").expect("gross");
        assert_eq!(e.from_column, "amount");
        assert!(!e.expression.is_empty(), "a computed column must carry its expression");
    }

    #[test]
    fn two_sources_joined_keep_their_own_relations() {
        let r = resolve(
            "select o.id, c.name from db.sch.orders o join db.sch.customers c on o.cid = c.id",
            "snowflake",
            &[vis("DB.SCH.ORDERS", &["id", "cid"]), vis("DB.SCH.CUSTOMERS", &["id", "name"])],
        );
        let rels: HashSet<&str> = r.edges.iter().map(|e| e.from_relation.as_str()).collect();
        assert!(rels.contains("DB.SCH.ORDERS") && rels.contains("DB.SCH.CUSTOMERS"));
    }

    #[test]
    fn an_unknown_relation_is_admitted_not_invented() {
        let r = resolve("select * from db.sch.nowhere", "snowflake", &[]);
        assert!(r.approximate, "the engine must say it could not resolve this");
        assert!(r.edges.is_empty());
    }

    #[test]
    fn a_scalar_subquery_in_a_where_clause_is_not_an_output_column() {
        // The engine hangs the subquery's column off the same Output node as the
        // real projections, and names it after its function. Left alone, the
        // lineage grows a column called "coalesce" that exists nowhere.
        let r = resolve(
            "select id, amount from db.sch.orders \
             where amount = (select coalesce(max(o2.amount), 0) from db.sch.orders o2)",
            "snowflake",
            &[vis("DB.SCH.ORDERS", &["id", "amount"])],
        );
        assert_eq!(r.outputs, vec!["amount", "id"], "coalesce is not a column");
        assert!(r.edges.iter().all(|e| e.to_column != "coalesce"));
    }

    #[test]
    fn an_alias_that_merely_uses_a_function_is_kept() {
        let r = resolve(
            "select coalesce(a, b) as amount from db.sch.orders",
            "snowflake",
            &[vis("DB.SCH.ORDERS", &["a", "b"])],
        );
        assert_eq!(r.outputs, vec!["amount"]);
        assert!(r.edges.iter().any(|e| e.to_column == "amount"));
    }

    #[test]
    fn invalid_sql_is_reported_as_a_parse_error() {
        let r = resolve("select from db.sch.orders", "snowflake", &[vis("DB.SCH.ORDERS", &["id"])]);
        assert!(r.parse_error.is_some());
    }

    #[test]
    fn a_quoted_mixed_case_relation_finds_its_schema() {
        // dbt writes `relation_name` quoted when the warehouse needs it quoted,
        // and the compiled SQL repeats that spelling. Uppercasing it before
        // handing it to the engine made the two unmatchable, so the star below
        // expanded to nothing at all.
        let r = resolve(
            "select * from \"DB\".\"dbo\".\"shop_address\"",
            "snowflake",
            &[vis("\"DB\".\"dbo\".\"shop_address\"", &["id", "amount"])],
        );
        assert_eq!(r.outputs, vec!["amount", "id"]);
        assert_eq!(r.edges.len(), 2);
        assert!(r.edges.iter().all(|e| e.from_relation == "DB.DBO.SHOP_ADDRESS"),
            "attribution still normalises, so the dbt node is still found");
    }

    #[test]
    fn an_unquoted_relation_still_matches_after_the_engine_folds_its_case() {
        let r = resolve(
            "select * from db.sch.orders",
            "snowflake",
            &[vis("db.sch.orders", &["id", "amount"])],
        );
        assert_eq!(r.outputs, vec!["amount", "id"]);
    }

    #[test]
    fn a_dot_inside_a_quoted_identifier_is_not_a_separator() {
        assert_eq!(qualified_parts("\"a.b\".\"c\""), vec!["\"a.b\"", "\"c\""]);
        assert_eq!(qualified_parts("db.sch.tbl"), vec!["db", "sch", "tbl"]);
    }

    #[test]
    fn a_parse_error_says_what_went_wrong_not_only_that_it_did() {
        // The code alone is the same string for every failure in a project, so
        // it tells a reader nothing to go and fix.
        let r = resolve("select from db.sch.orders", "snowflake", &[vis("DB.SCH.ORDERS", &["id"])]);
        let e = r.parse_error.expect("a parse error");
        assert!(e.starts_with("PARSE_ERROR: "), "{e}");
        assert!(e.len() > "PARSE_ERROR: ".len(), "the message must survive: {e}");
    }

    fn parse_error_span(sql: &str) -> (usize, usize) {
        let r = resolve(sql, "snowflake", &[vis("DB.SCH.ORDERS", &["id", "amount"])]);
        r.issues.iter().find(|i| i.code == "PARSE_ERROR").and_then(|i| i.span).expect("a span")
    }

    #[test]
    fn a_parse_error_position_counts_from_the_statement_not_the_file() {
        // The engine's statement starts after the blank lines and the comment,
        // so the line its parser calls 5 is the file's line 8.
        let sql = "\r\n\r\n-- note\r\nwith u as (\r\n  (\r\n    select\r\n\r\n    \
                   from db.sch.unbuilt_parent\r\n  )\r\n)\r\nselect * from u";
        let (a, b) = parse_error_span(sql);
        assert_eq!(&sql[a..b], "db.sch.unbuilt_parent", "{a}..{b}");
    }

    #[test]
    fn a_list_with_an_empty_element_points_at_the_second_comma() {
        let sql = "select\r\n  id\r\n  ,\r\n  , amount\r\nfrom db.sch.orders";
        assert_eq!(parse_error_span(sql), (21, 22));
    }

    #[test]
    fn a_parse_error_column_counts_characters_not_bytes() {
        let sql = "select 'é' as note, , id from db.sch.orders";
        let (a, b) = parse_error_span(sql);
        assert_eq!((a, &sql[a..b]), (21, ","), "counted in bytes it would be the space before");
    }

    #[test]
    fn a_parse_error_the_position_cannot_be_read_from_keeps_the_engine_span() {
        let sql = "select from db.sch.orders";
        assert_eq!(stopped_at(sql, (0, 25), "Parse error: no position here"), None);
        assert_eq!(stopped_at(sql, (0, 25), "found: x at Line: 1, Column: 7 and then"), None);
        assert_eq!(stopped_at(sql, (0, 25), "at Line: 9, Column: 1"), None, "past the end");
        assert_eq!(stopped_at(sql, (0, 25), "at Line: 1, Column: 26"), None, "on nothing");
        assert_eq!(stopped_at(sql, (0, 25), "at Line: 1, Column: 13"), Some((12, 25)));
    }

    #[test]
    fn only_a_parse_error_is_narrowed() {
        let sql = "select id, x.ts from db.sch.src x, db.sch.nowhere \
                   where ts > (select max(ts) from db.sch.m) and y = 1";
        let columns = vec!["id".to_string(), "ts".to_string()];
        let engine = flowscope_core::analyze(&AnalyzeRequest {
            sql: sql.to_string(),
            files: None,
            dialect: Dialect::Snowflake,
            source_name: None,
            options: None,
            schema: Some(SchemaMetadata {
                default_catalog: None,
                default_schema: None,
                search_path: None,
                case_sensitivity: None,
                allow_implied: true,
                tables: vec![as_table("db.sch.src", &columns)],
            }),
            template_config: None,
        });
        let r = resolve(sql, "snowflake", &[Visible { relation: "db.sch.src".into(), columns }]);
        let theirs: Vec<_> =
            engine.issues.iter().map(|i| (&*i.code, i.span.map(|s| (s.start, s.end)))).collect();
        let ours: Vec<_> = r.issues.iter().map(|i| (&*i.code, i.span)).collect();
        assert!(ours.iter().filter(|(_, s)| s.is_some()).count() > 1, "{ours:?}");
        assert_eq!(ours, theirs);
        // Narrowed, a parse error still says the same thing.
        let r = resolve("select from db.sch.orders", "snowflake", &[vis("DB.SCH.ORDERS", &["id"])]);
        let e = r.issues.iter().find(|i| i.code == "PARSE_ERROR").expect("a parse error");
        assert_eq!(r.parse_error, Some(format!("PARSE_ERROR: {}", e.message)));
        assert_eq!(e.span, Some((12, 25)), "the relation after the empty list");
    }

    #[test]
    fn an_issue_keeps_the_engine_s_own_words() {
        let r = resolve("select * from db.sch.nowhere", "snowflake", &[]);
        let i = r.issues.first().expect("an issue");
        assert!(!i.code.is_empty() && !i.message.is_empty(), "{} / {}", i.code, i.message);
        assert!(matches!(i.severity, "error" | "warning" | "info"));
    }

    #[test]
    fn a_table_the_engine_cannot_find_is_named_on_its_issue() {
        // How dbt compiles an incremental model's filter: the model reads
        // itself, and it is never among its own parents.
        let sql = "select id, ts from db.sch.src where ts > (select max(ts) from db.sch.m)";
        let r = resolve(sql, "snowflake", &[vis("db.sch.src", &["id", "ts"])]);
        let named: Vec<(&str, &str, bool)> = r
            .issues
            .iter()
            .filter_map(|i| i.relation.as_deref().map(|rel| (i.code.as_str(), rel, i.degrading)))
            .collect();
        assert_eq!(named, vec![("UNRESOLVED_REFERENCE", "DB.SCH.M", true)]);
        assert!(r.approximate, "what the engine admits is unchanged");
        // The span fallback rests on this: it spells the table as the SQL does.
        let (a, b) = r.issues.iter().find_map(|i| i.span.filter(|_| i.relation.is_some())).unwrap();
        assert_eq!(&sql[a..b], "db.sch.m");
    }

    #[test]
    fn a_column_the_engine_cannot_place_names_no_table() {
        let r = resolve(
            "select id from db.sch.a x join db.sch.b y on x.k = y.k",
            "snowflake",
            &[vis("db.sch.a", &["id", "k"]), vis("db.sch.b", &["id", "k"])],
        );
        let i = r.issues.iter().find(|i| i.code == "UNRESOLVED_REFERENCE").expect("ambiguous id");
        assert!(i.message.starts_with("Column '"), "{}", i.message);
        assert_eq!((i.relation.as_deref(), i.degrading), (None, true));
    }

    #[test]
    fn every_table_the_sql_reads_is_listed_whether_or_not_it_was_handed_in() {
        let sql = "with cur as (select a, b from db.sch.parent), \
                   base as (select * from \"RAW_DB\".\"dbo\".\"orders_base\") \
                   select a, b from cur union all select \"A\" as a, \"B\" as b from base";
        let both = vec!["DB.SCH.PARENT", "RAW_DB.DBO.ORDERS_BASE"];
        let told = resolve(sql, "snowflake", &[vis("DB.SCH.PARENT", &["a", "b"])]);
        assert_eq!(told.reads, both);
        // Told of no table, the engine stays quiet about the one nobody declared,
        // which is why the list is not taken from what it complains of.
        let untold = resolve(sql, "snowflake", &[]);
        assert_eq!(untold.reads, both);
        assert!(untold.issues.iter().all(|i| i.code != "UNRESOLVED_REFERENCE"));
    }

    #[test]
    fn a_table_function_is_not_a_table_read() {
        let r = resolve(
            "select p.a, f.value as v from db.sch.parent p, lateral flatten(input => p.b) f",
            "snowflake",
            &[vis("DB.SCH.PARENT", &["a", "b"])],
        );
        assert_eq!(r.reads, vec!["DB.SCH.PARENT"]);
        let r = resolve(
            "select seq4() as n from table(generator(rowcount => 10))",
            "snowflake",
            &[],
        );
        assert!(r.reads.is_empty(), "{:?}", r.reads);
    }

    #[test]
    fn the_unresolved_table_is_read_from_the_span_when_the_words_change() {
        let code = issue_codes::UNRESOLVED_REFERENCE;
        let said = "Table 'DB.SCH.M' could not be resolved using provided schema metadata";
        assert_eq!(unresolved_relation(code, said, None).as_deref(), Some("DB.SCH.M"));
        let reworded = "no such relation";
        let quoted = Some("\"DB\".\"sch\".\"m\"");
        assert_eq!(unresolved_relation(code, reworded, quoted).as_deref(), Some("DB.SCH.M"));
        // A column's span is the bare column, and a column is not a table.
        assert_eq!(unresolved_relation(code, reworded, Some("first_seen")), None);
        assert_eq!(unresolved_relation(code, reworded, None), None);
        assert_eq!(unresolved_relation(issue_codes::UNKNOWN_COLUMN, said, None), None);
    }

    #[test]
    fn a_literal_column_is_a_root_rather_than_a_gap() {
        // dbt packages ship models shaped exactly like this to declare a schema.
        // They have no parent to find, so counting them as uncovered understates
        // coverage and sends a reader looking for something that cannot exist.
        let r = resolve(
            "select id, cast(null as varchar) as note from db.sch.orders",
            "snowflake",
            &[vis("DB.SCH.ORDERS", &["id", "amount"])],
        );
        assert_eq!(r.roots, vec!["note"]);
        assert!(r.edges.iter().all(|e| e.to_column != "note"));
    }

    #[test]
    fn a_copied_column_that_lost_its_edge_is_a_gap_not_a_root() {
        // No expression means the column came from somewhere. Whatever went
        // wrong, calling it a root would hide the gap.
        let r = resolve("select * from db.sch.nowhere", "snowflake", &[]);
        assert!(r.roots.is_empty(), "{:?}", r.roots);
    }

    #[test]
    fn a_column_that_names_a_real_one_is_never_called_a_root() {
        // `amount` is a column of the relation, so whatever happened to this
        // column, it is a loss and not a root.
        let r = resolve(
            "select max(amount) over (partition by id) as top from db.sch.orders",
            "snowflake",
            &[vis("DB.SCH.ORDERS", &["id", "amount"])],
        );
        assert!(r.roots.is_empty(), "{:?}", r.roots);
    }

    #[test]
    fn a_literal_copied_out_of_a_cte_is_a_root() {
        // A copy carries no expression, so the output shows nothing: the
        // literal is where the walk ends, two CTEs up.
        let r = resolve(
            "with base as (select id, 'LEDGER' as origin, current_timestamp() as loaded_at \
             from db.sch.src), final as (select id, origin, loaded_at from base) \
             select * from final",
            "snowflake",
            &[vis("DB.SCH.SRC", &["id", "code", "amount"])],
        );
        assert_eq!(r.roots, vec!["loaded_at", "origin"]);
        assert!(r.lost.is_empty(), "a root is never also lost");
        let edges: Vec<(&str, &str)> =
            r.edges.iter().map(|e| (e.from_column.as_str(), e.to_column.as_str())).collect();
        assert_eq!(edges, vec![("id", "id")]);
    }

    #[test]
    fn a_spine_with_no_parent_is_all_roots() {
        // `year(day_start)` reads a column, and the column it reads is itself
        // built from a literal: reached below it, so the guard lets it be.
        let r = resolve(
            "with spine as (select to_date('2000-01-01') as day_start), \
             cal as (select day_start, year(day_start) as yr from spine) \
             select day_start, yr from cal",
            "snowflake",
            &[],
        );
        assert_eq!(r.roots, vec!["day_start", "yr"]);
    }

    #[test]
    fn a_literal_in_a_cte_that_joins_is_a_root() {
        // The engine hangs a projection reading no column off the scope's
        // driving relation, and decides which relation drives from whether it
        // was joined anywhere in the statement. `p` drives `j` but is joined in
        // `q`, so the literals of `j` hang off nothing and have nothing feeding
        // them, like a read the engine could not resolve. What tells them apart
        // is that these read no name.
        let r = resolve(
            "with p as (select id from db.sch.a), \
             q as (select b.id from db.sch.b b join p on p.id = b.id), \
             j as (select p.id, cast((0) as decimal(19, 4)) as amount, 'n/a' as note \
             from p join q on p.id = q.id) select * from j",
            "snowflake",
            &[vis("DB.SCH.A", &["id"]), vis("DB.SCH.B", &["id"])],
        );
        assert_eq!(r.roots, vec!["amount", "note"]);
        assert!(r.lost.is_empty(), "{:?}", r.lost.iter().map(|l| &l.column).collect::<Vec<_>>());
    }

    #[test]
    fn a_read_the_engine_cannot_place_is_not_a_root() {
        // The walk from `code` ends where it does for a literal, and the text
        // still reads `code`, which the walk never reached.
        assert_eq!(unplaced_code().roots, vec!["ccy"]);
    }

    #[test]
    fn an_unresolved_name_is_not_a_root() {
        let r = resolve(
            "with j as (select upper(ghost) as g from db.sch.src a join db.sch.oth b on a.id = b.id) \
             select g from j",
            "snowflake",
            &[vis("DB.SCH.SRC", &["id", "code", "amount"]), vis("DB.SCH.OTH", &["id", "note"])],
        );
        assert!(r.roots.is_empty(), "{:?}", r.roots);
    }

    #[test]
    fn a_literal_only_cte_with_missing_names_stays_a_gap() {
        // `a` and `b` are names the CTE never had, whatever it was built from.
        let r = resolve(
            "with lit as (select 1 as dummy), picked as (select a, b from lit) select * from picked",
            "snowflake",
            &[],
        );
        assert!(r.roots.is_empty(), "{:?}", r.roots);
    }

    #[test]
    fn a_column_a_starred_cte_does_not_have_is_lost_and_says_where() {
        // The star expands from the list the parent came with, and the column
        // read from the CTE afterwards is a name it never had. The engine says
        // nothing about it, so this is the only place it is named.
        let r = resolve(
            "with o as (select * from db.sch.shop_orders) \
             select o.order_id, o.coupon_code as coupon from o",
            "snowflake",
            &[vis("DB.SCH.SHOP_ORDERS", &["order_id", "amount"])],
        );
        let edges: Vec<(&str, &str)> =
            r.edges.iter().map(|e| (e.from_column.as_str(), e.to_column.as_str())).collect();
        assert_eq!(edges, vec![("order_id", "order_id")]);
        assert!(r.roots.is_empty(), "{:?}", r.roots);
        assert_eq!(r.lost.len(), 1);
        assert_eq!(r.lost[0].column, "coupon");
        assert_eq!(
            r.lost[0].ends,
            vec![DeadEnd::Phantom {
                via: "o".into(),
                column: "coupon_code".into(),
                reads: vec!["DB.SCH.SHOP_ORDERS".into()],
                star: true,
            }]
        );
    }

    /// Two relations with a `code`, read unqualified beside a CTE of
    /// constants: the engine places the name on neither.
    fn unplaced_code() -> Resolved {
        resolve(
            "with consts as (select 'EUR' as ccy), \
             j as (select upper(code) as code, consts.ccy \
             from consts cross join db.sch.src s cross join db.sch.oth o) \
             select code, ccy from j",
            "snowflake",
            &[vis("DB.SCH.SRC", &["id", "code", "amount"]), vis("DB.SCH.OTH", &["id", "code"])],
        )
    }

    #[test]
    fn a_name_the_engine_cannot_place_is_lost_with_it() {
        let r = unplaced_code();
        let lost: Vec<(&str, &Vec<DeadEnd>)> = r.lost.iter().map(|l| (l.column.as_str(), &l.ends)).collect();
        assert_eq!(lost, vec![("code", &vec![DeadEnd::NamesUnreached { names: vec!["code".into()] }])]);
    }

    fn read_from(r: &Resolved) -> Vec<(&str, &str)> {
        let mut out: Vec<(&str, &str)> =
            r.edges.iter().map(|e| (e.from_relation.as_str(), e.from_column.as_str())).collect();
        out.sort();
        out.dedup();
        out
    }

    #[test]
    fn a_window_key_is_read_in_the_scope_that_writes_it() {
        // `j.k` is the hub's: the link has a `k` too, and reaching the link
        // through `j.v` must not make it the key's source.
        let r = resolve(
            "with j as (select h.k, l.v from db.s.hub h join db.s.lnk l on h.k = l.k) \
             select max(j.v) over (partition by j.k) as top_v from j",
            "snowflake",
            &[vis("DB.S.HUB", &["k", "ts"]), vis("DB.S.LNK", &["k", "v"])],
        );
        assert_eq!(read_from(&r), vec![("DB.S.HUB", "k"), ("DB.S.LNK", "v")]);
    }

    #[test]
    fn a_relation_two_derivations_reach_is_read_for_each() {
        let r = resolve(
            "with d as (select row_number() over (partition by a) as r1, \
             row_number() over (partition by b) as r2 from db.s.t) \
             select coalesce(r1, r2) as r from d",
            "snowflake",
            &[vis("DB.S.T", &["a", "b", "c"])],
        );
        assert_eq!(read_from(&r), vec![("DB.S.T", "a"), ("DB.S.T", "b")]);
    }

    #[test]
    fn the_loudest_role_over_every_path_stands_whatever_the_branch_order() {
        let totals = "with totals as (select cid, sum(amount) as amount from db.sch.orders group by cid) ";
        for body in [
            "select cid, amount from totals union all select cid, amount from db.sch.orders",
            "select cid, amount from db.sch.orders union all select cid, amount from totals",
        ] {
            let r = resolve(&format!("{totals}{body}"), "snowflake", &[vis("DB.SCH.ORDERS", &["amount", "cid"])]);
            let amount: Vec<crate::role::Role> = r
                .edges
                .iter()
                .filter(|e| e.to_column == "amount")
                .map(|e| crate::role::classify(&e.expression, &e.from_column, &e.to_column))
                .collect();
            assert_eq!(amount, vec![crate::role::Role::Aggregate], "{body}");
        }
    }

    #[test]
    fn a_node_two_paths_read_the_same_way_is_still_weighed_for_each_role() {
        // Both branches reach `a` through `a + 1`, one of them under a window
        // and the other through a second CTE: the same text to read at `a`, two
        // roles to weigh. (Both straight off `y`, the engine keeps one of the
        // two edges between the same columns, and no walk can help.)
        let y = "with y as (select a + 1 as e from db.s.t), y2 as (select e from y) ";
        for body in [
            "select max(e) over () as e from y union all select e from y2",
            "select e from y2 union all select max(e) over () as e from y",
        ] {
            let r = resolve(&format!("{y}{body}"), "snowflake", &[vis("DB.S.T", &["a", "b"])]);
            let roles: Vec<crate::role::Role> = r
                .edges
                .iter()
                .map(|e| crate::role::classify(&e.expression, &e.from_column, &e.to_column))
                .collect();
            assert_eq!(roles, vec![crate::role::Role::Window], "{body}");
        }
    }

    #[test]
    fn a_column_inside_trim_is_read() {
        // The same shape through TRIM, which the engine did not read before its
        // fork (0028).
        let r = resolve(
            "with consts as (select 'EUR' as ccy), \
             j as (select trim(s.code) as code, consts.ccy from consts cross join db.sch.src s) \
             select code, ccy from j",
            "snowflake",
            &[vis("DB.SCH.SRC", &["id", "code", "amount"])],
        );
        let edges: Vec<(&str, &str)> =
            r.edges.iter().map(|e| (e.from_column.as_str(), e.to_column.as_str())).collect();
        assert_eq!(edges, vec![("code", "code")]);
        assert!(r.lost.is_empty());
    }

    #[test]
    fn a_column_with_its_edge_is_neither_lost_nor_a_root() {
        let r = resolve(
            "select order_id, amount * 2 as doubled from db.sch.shop_orders",
            "snowflake",
            &[vis("DB.SCH.SHOP_ORDERS", &["order_id", "amount"])],
        );
        assert!(r.lost.is_empty() && r.roots.is_empty());
    }

    #[test]
    fn a_subquery_s_table_is_not_a_name_read() {
        let v = HashSet::from(["v".to_string()]);
        let tables = HashSet::from(["orders".to_string()]);
        assert_eq!(
            read_names("coalesce((select max(v) from db.sch.orders), o.amount)", &v, &tables),
            vec!["v", "amount"]
        );
    }

    fn spans(first: &str, second: &str) -> String {
        format!(
            "with paid as (select acct_id, seen_at from \
             (select distinct acct_id, seen_at from db.s.payments) as {first}), \
             open as (select acct_id, seen_at from \
             (select distinct acct_id, seen_at from db.s.invoices) as {second}) \
             select k.acct_id, p.seen_at as paid_seen_at, o.seen_at as open_seen_at \
             from db.s.accounts as k \
             left join paid as p on k.acct_id = p.acct_id \
             left join open as o on k.acct_id = o.acct_id"
        )
    }

    fn span_tables() -> Vec<Visible> {
        vec![
            vis("DB.S.PAYMENTS", &["acct_id", "seen_at"]),
            vis("DB.S.INVOICES", &["acct_id", "seen_at"]),
            vis("DB.S.ACCOUNTS", &["acct_id"]),
        ]
    }

    fn seen_at_sources(r: &Resolved) -> Vec<(&str, &str)> {
        let mut out: Vec<(&str, &str)> = r
            .edges
            .iter()
            .filter(|e| e.from_column == "seen_at")
            .map(|e| (e.from_relation.as_str(), e.to_column.as_str()))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn two_derived_tables_of_one_name_are_named() {
        let r = resolve(&spans("t", "t"), "snowflake", &span_tables());
        assert_eq!(r.merged_scopes, vec![MergedScope { kind: "derived", name: "t".into(), count: 2 }]);
        assert!(r.issues.is_empty(), "the engine says nothing of it");
    }

    #[test]
    fn two_derived_tables_of_one_name_are_read_apart() {
        // Since the fork's fourth patch the engine keys a derived table by its
        // occurrence (0028), so the plan need not set such a compile aside
        // (0018). Should a pin undo that, this fails first.
        let r = resolve(&spans("t", "t"), "snowflake", &span_tables());
        assert_eq!(
            seen_at_sources(&r),
            vec![("DB.S.INVOICES", "open_seen_at"), ("DB.S.PAYMENTS", "paid_seen_at")]
        );
    }

    #[test]
    fn scopes_of_distinct_names_are_not_named_and_read_right() {
        for (first, second) in [("t1", "t2"), ("t", "T")] {
            let r = resolve(&spans(first, second), "snowflake", &span_tables());
            assert!(r.merged_scopes.is_empty(), "{first} and {second}");
            assert_eq!(
                seen_at_sources(&r),
                vec![("DB.S.INVOICES", "open_seen_at"), ("DB.S.PAYMENTS", "paid_seen_at")],
                "{first} and {second}"
            );
        }
        // One derived table over a union is one scope, however many relations
        // feed it.
        let r = resolve(
            "select min(u.seen_at) as low_ts from \
             (select seen_at from db.s.payments union all select seen_at from db.s.invoices) as u",
            "snowflake",
            &span_tables(),
        );
        assert!(r.merged_scopes.is_empty());
    }

    #[test]
    fn a_cte_name_given_twice_in_nested_withs_is_named() {
        let r = resolve(
            "select p.seen_at as paid_seen_at, q.seen_at as open_seen_at \
             from (with z as (select seen_at from db.s.payments) select seen_at from z) as p \
             cross join (with z as (select seen_at from db.s.invoices) select seen_at from z) as q",
            "snowflake",
            &span_tables(),
        );
        assert_eq!(r.merged_scopes, vec![MergedScope { kind: "cte", name: "z".into(), count: 2 }]);
    }

    #[test]
    fn two_ctes_of_one_name_in_nested_withs_are_read_apart() {
        // Since the fork's seventh patch the engine keys a CTE by its occurrence
        // and scopes it to its `WITH` (0028), so the plan need not set such a
        // compile aside (0018). Should a pin undo that, this fails first.
        let r = resolve(
            "select p.seen_at as paid_seen_at, q.seen_at as open_seen_at \
             from (with z as (select seen_at from db.s.payments) select seen_at from z) as p \
             cross join (with z as (select seen_at from db.s.invoices) select seen_at from z) as q",
            "snowflake",
            &span_tables(),
        );
        assert_eq!(
            seen_at_sources(&r),
            vec![("DB.S.INVOICES", "open_seen_at"), ("DB.S.PAYMENTS", "paid_seen_at")]
        );
    }

    #[test]
    fn a_column_of_a_values_list_is_a_root() {
        // The engine gives a `VALUES` list no columns, so `column1` reads as a
        // name its derived table never had, where its rows are in the SQL.
        for (sql, roots) in [
            ("select column1 as a, column2 as b from values (1, 'x'), (2, 'y')", vec!["a", "b"]),
            ("select column1 as a, column2 as b from (values (1, 'x'), (2, 'y'))", vec!["a", "b"]),
            ("select v.column1 as a from (values (1), (2)) as v", vec!["a"]),
            ("select a from (values (1), (2)) as v(a)", vec!["a"]),
        ] {
            let r = resolve(sql, "snowflake", &[]);
            assert_eq!(r.roots, roots, "{sql}");
            assert!(r.lost.is_empty(), "{sql}");
        }
        // A derived table that is not one, missing the name, is still a loss.
        let r = resolve("select d.z as z from (select a from db.s.p) as d", "snowflake", &p_table());
        assert!(r.roots.is_empty());
        assert_eq!(r.lost.len(), 1);
    }

    #[test]
    fn issues_come_in_the_order_they_sit_in_the_sql() {
        // The engine gives the later of these two warnings first. On a project
        // it gave others in an order that changed from run to run, and the
        // report with it.
        let sql = "with c1 as (select a, b from db.s.p), c2 as (select a, b from db.s.q) \
                   select coalesce(round(b, 2), 0) as b2, coalesce(round(a, 2), 0) as a2 \
                   from c1 join c2 on c1.a = c2.a";
        let tables = [vis("DB.S.P", &["a", "b", "c"]), vis("DB.S.Q", &["a", "b", "c"])];
        let r = resolve(sql, "snowflake", &tables);
        let at: Vec<usize> = r.issues.iter().filter_map(|i| i.span.map(|s| s.0)).collect();
        assert!(at.len() > 1, "{at:?}");
        assert!(at.windows(2).all(|w| w[0] <= w[1]), "{at:?}");
    }

    #[test]
    fn a_flatten_key_is_read_from_its_input_and_its_index_is_a_root() {
        // Since the fork's eighth patch a key and a path come from the input
        // (0028); an index is a position, which no column of the input holds.
        let r = resolve(
            "select f.key as element_name, f.index + 1 as position, f.value::string as element_value \
             from db.s.hx as h, lateral flatten(input => try_parse_json(h.data)) as f",
            "snowflake",
            &[vis("DB.S.HX", &["id", "data"])],
        );
        let mut edges: Vec<(&str, &str)> =
            r.edges.iter().map(|e| (e.from_column.as_str(), e.to_column.as_str())).collect();
        edges.sort();
        assert_eq!(edges, vec![("data", "element_name"), ("data", "element_value")]);
        assert_eq!(r.roots, vec!["position"]);
        assert!(r.lost.is_empty());
    }

    #[test]
    fn a_values_column_and_a_flatten_index_are_not_unbacked() {
        // The engine gives a `VALUES` list no columns and a FLATTEN's index
        // nothing to feed it: neither is a name the SQL reads and cannot back.
        let tables = [vis("DB.S.P", &["id", "arr"])];
        for sql in [
            "select column1 as a, column2 as b from values (1, 'x'), (2, 'y')",
            "select p.id, fl.index + 1 as option_id from db.s.p as p, lateral flatten(input => p.arr) as fl",
        ] {
            let r = resolve(sql, "snowflake", &tables);
            assert!(r.unbacked.is_empty(), "{sql}: {:?}", unbacked_of(&r));
        }
    }

    fn p_table() -> Vec<Visible> {
        vec![vis("DB.S.P", &["a", "b", "c", "d"])]
    }

    fn unbacked_of(r: &Resolved) -> Vec<(&str, &str, &str, Vec<&str>)> {
        r.unbacked
            .iter()
            .map(|u| {
                let owners = u.owners.iter().map(String::as_str).collect();
                (u.output.as_str(), u.cte.as_str(), u.column.as_str(), owners)
            })
            .collect()
    }

    #[test]
    fn a_name_its_cte_never_projected_is_read_and_not_made_an_edge() {
        // A stage whose macro found its source missing from the warehouse: the
        // CTE lists its derived column and nothing else, and the hash reads a
        // column from it anyway. Inside TRIM the engine sees no name at all;
        // without it, it makes one up for the CTE. Either way the SQL cannot
        // run, and the column is not the relation's by any SQL that does.
        for hash in [
            "sha1_binary(concat_ws('||', upper(trim(cast(b as varchar)))))",
            "sha1_binary(concat_ws('||', upper(cast(b as varchar))))",
        ] {
            let r = resolve(
                &format!(
                    "with d as (select a, 'x' as rs from db.s.p), \
                     h as (select rs, {hash} as hk from d) select * from h"
                ),
                "snowflake",
                &p_table(),
            );
            assert!(r.edges.iter().all(|e| e.from_column != "b"), "{hash}");
            assert_eq!(unbacked_of(&r), vec![("hk", "d", "b", vec!["DB.S.P"])], "{hash}");
        }
    }

    #[test]
    fn an_empty_union_read_by_name_is_nothing_but_unbacked_reads() {
        let r = resolve(
            "with f as (select cast('x' as text) as src from db.s.p), \
             n as (select src, coalesce(c, 'n/a') as c, coalesce(nullif(trim(d), ''), 'n/a') as d from f) \
             select * from n",
            "snowflake",
            &p_table(),
        );
        assert!(r.edges.is_empty(), "{:?}", r.edges.iter().map(|e| &e.from_column).collect::<Vec<_>>());
        let read: Vec<(&str, &str)> = r.unbacked.iter().map(|u| (u.cte.as_str(), u.column.as_str())).collect();
        assert_eq!(read, vec![("f", "c"), ("f", "d")]);
    }

    #[test]
    fn a_name_its_cte_projects_keeps_its_edge_and_its_source() {
        // Valid SQL reads only what the CTE projects, so there is nothing to
        // admit. And the name is followed through the CTE's own column: a CTE
        // that renamed `a` to `b` gives `a`, where mining the table gave both.
        let hash = "sha1_binary(upper(trim(cast(b as varchar))))";
        for (cte, want) in [("select a, b from db.s.p", "b"), ("select a as b from db.s.p", "a")] {
            let r = resolve(
                &format!("with d as ({cte}), h as (select {hash} as hk from d) select * from h"),
                "snowflake",
                &p_table(),
            );
            assert!(r.unbacked.is_empty(), "{cte}");
            let from: Vec<&str> = r.edges.iter().map(|e| e.from_column.as_str()).collect();
            assert_eq!(from, vec![want], "{cte}");
        }
    }

    #[test]
    fn a_lateral_alias_and_a_star_over_a_relation_are_not_unbacked() {
        // `x` is a column the reading scope makes itself.
        let lateral = resolve(
            "with d as (select a from db.s.p), h as (select a as x, sha1(trim(x)) as y from d) \
             select * from h",
            "snowflake",
            &p_table(),
        );
        assert!(lateral.unbacked.is_empty(), "{:?}", unbacked_of(&lateral));
        // Over a relation, the star is whatever list the relation came with:
        // a missing name says that list is short, not that the SQL cannot run.
        let short = resolve(
            "with d as (select * from db.s.p) select d.a, d.z from d",
            "snowflake",
            &p_table(),
        );
        assert!(short.unbacked.is_empty(), "{:?}", unbacked_of(&short));
        // Over another CTE written out, it is not.
        let spelled = resolve(
            "with u as (select 'x' as src from db.s.p), f as (select * from u) select f.k from f",
            "snowflake",
            &p_table(),
        );
        assert_eq!(unbacked_of(&spelled), vec![("k", "f", "k", vec![])]);
    }

    #[test]
    fn a_column_the_list_lacks_is_read_as_the_list_s_finding() {
        let r = resolve(
            "select s.id, s.amount from db.sch.stage as s",
            "snowflake",
            &[vis("DB.SCH.STAGE", &["id"])],
        );
        assert_eq!(
            r.unknown_columns,
            vec![UnknownColumn { relation: "DB.SCH.STAGE".into(), column: "amount".into() }]
        );
        assert!(!r.approximate, "the engine read the name and drew the edge");
        assert!(r.edges.iter().any(|e| e.from_column == "amount"));
        // A message that reads otherwise can match no edge.
        assert_eq!(unknown_column("no quotes here").relation, "");
    }

    #[test]
    fn a_qualified_name_is_read_whatever_the_vocabulary() {
        assert_eq!(qualified_tails("trim(s.code) || o.\"Amount\""), vec!["code", "amount"]);
        assert_eq!(qualified_tails("db.sch.fn(x) + t.a.b"), vec!["b"]);
        assert!(qualified_tails("'s.code' || 1.5").is_empty());
    }

    #[test]
    fn a_window_column_keeps_the_columns_it_reads() {
        // The engine hangs this derivation off the table, not off a column, so
        // a column to column walk alone would report no lineage at all.
        let r = resolve(
            "select row_number() over (partition by id order by amount) as rn from db.sch.orders",
            "snowflake",
            &[vis("DB.SCH.ORDERS", &["id", "amount"])],
        );
        assert_eq!(r.outputs, vec!["rn"]);
        let mut from: Vec<&str> = r.edges.iter().map(|e| e.from_column.as_str()).collect();
        from.sort();
        assert_eq!(from, vec!["amount", "id"]);
        assert!(r.edges.iter().all(|e| crate::role::classify(
            &e.expression,
            &e.from_column,
            &e.to_column
        ) == crate::role::Role::Window));
    }

    #[test]
    fn a_window_keeps_its_keys_even_when_the_expression_resolves() {
        // `lineid` resolves, so the engine takes the column to column
        // path and the relation level fallback never runs. `orderid` and
        // `storeid` live only inside the OVER clause, which the engine
        // does not read, so without mining them this column reports one source
        // out of three.
        let r = resolve(
            "select concat(cast(lineid as varchar), '|', \
             cast(row_number() over (order by lineid, orderid, storeid) as varchar)) as pk \
             from db.sch.lines",
            "snowflake",
            &[vis("DB.SCH.LINES", &["lineid", "orderid", "storeid", "other"])],
        );
        let mut from: Vec<&str> = r.edges.iter().map(|e| e.from_column.as_str()).collect();
        from.sort();
        from.dedup();
        assert_eq!(from, vec!["lineid", "orderid", "storeid"]);
        assert!(!from.contains(&"other"), "a column the window never names must not appear");
    }

    #[test]
    fn a_window_computed_inside_a_cte_keeps_its_lineage() {
        // A dbt model is a chain of CTEs, so the engine hangs this derivation off
        // the CTE rather than off a table. Abandoning the path there cost every
        // window and aggregate computed mid chain its lineage.
        let r = resolve(
            "with a as (select id, row_number() over (partition by id order by amount) as rn \
             from db.sch.orders), b as (select * from a) select * from b",
            "snowflake",
            &[vis("DB.SCH.ORDERS", &["id", "amount"])],
        );
        assert!(r.outputs.contains(&"rn".to_string()), "{:?}", r.outputs);
        let mut from: Vec<&str> =
            r.edges.iter().filter(|e| e.to_column == "rn").map(|e| e.from_column.as_str()).collect();
        from.sort();
        from.dedup();
        assert_eq!(from, vec!["amount", "id"], "the window reads both, through two CTEs");
        assert!(r
            .edges
            .iter()
            .filter(|e| e.to_column == "rn")
            .all(|e| e.from_relation == "DB.SCH.ORDERS"));
    }

    #[test]
    fn an_aggregate_computed_inside_a_cte_reaches_the_table() {
        let r = resolve(
            "with a as (select cid, sum(amount) as total from db.sch.orders group by cid) \
             select * from a",
            "snowflake",
            &[vis("DB.SCH.ORDERS", &["cid", "amount"])],
        );
        let e = r.edges.iter().find(|e| e.to_column == "total").expect("total");
        assert_eq!(e.from_column, "amount");
        assert_eq!(e.from_relation, "DB.SCH.ORDERS");
    }

    #[test]
    fn an_over_clause_is_found_whatever_surrounds_it() {
        assert_eq!(over_clauses("row_number() over (partition by a)"), vec!["partition by a"]);
        assert_eq!(
            over_clauses("sum(x) over (partition by nvl(a, 0)) + max(y) over (order by b)"),
            vec!["partition by nvl(a, 0)", "order by b"]
        );
        // A literal spells anything, and `handover(` is not a window.
        assert!(over_clauses("'over (partition by a)'").is_empty());
        assert!(over_clauses("handover(x)").is_empty());
        // Windows are written across lines, and this corpus is CRLF throughout.
        assert_eq!(over_clauses("row_number() over\r\n  (\r\n partition by a\r\n)"),
            vec!["\r\n partition by a\r\n"]);
        // A comment can hold anything at all, and an index taken from a
        // lowercased copy used to land inside one of these and panic.
        assert_eq!(
            over_clauses("-- a \u{2192} b\nrow_number() over (order by \u{e9}t\u{e9})"),
            vec!["order by \u{e9}t\u{e9}"]
        );
    }

    #[test]
    fn a_qualify_key_is_read_although_the_engine_never_looks_at_it() {
        // The dedup idiom of this corpus: 582 of 3929 models. The engine's
        // analyzer never visits `select.qualify`, so without reading it the
        // columns the model really picks its rows by are invisible.
        let r = resolve(
            "select id, amount from db.sch.orders \
             qualify row_number() over (partition by cid order by ts desc) = 1",
            "snowflake",
            &[vis("DB.SCH.ORDERS", &["id", "amount", "cid", "ts"])],
        );
        let keys: Vec<&str> = r
            .indirect
            .iter()
            .filter(|i| i.role == crate::role::Role::DedupKey)
            .map(|i| i.column.as_str())
            .collect();
        assert_eq!(keys, vec!["cid", "ts"]);
        assert!(r.indirect.iter().all(|i| i.relation == "DB.SCH.ORDERS"));
        // And it stays out of the projection, which is the whole distinction.
        assert_eq!(r.outputs, vec!["amount", "id"]);
        assert!(r.edges.iter().all(|e| e.from_column != "cid"));
    }

    fn dedup_tables() -> Vec<Visible> {
        vec![
            vis("DB.SCH.ORDERS", &["id", "amount", "cid", "ts"]),
            vis("DB.SCH.CUSTOMERS", &["cid", "ts", "region"]),
            vis("DB.SCH.RETURNS", &["customer_id", "ts", "reason"]),
            vis("DB.SCH.ORDERS_ARCHIVE", &["cid", "ts", "amount"]),
        ]
    }

    fn dedup_keys(r: &Resolved) -> Vec<(&str, &str)> {
        let mut out: Vec<(&str, &str)> = r
            .indirect
            .iter()
            .filter(|i| i.role == crate::role::Role::DedupKey)
            .map(|i| (i.relation.as_str(), i.column.as_str()))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn a_qualify_key_belongs_to_the_select_that_holds_it() {
        // Customers has a `cid` and a `ts` too; its SELECT deduplicates nothing.
        let r = resolve(
            "with a as (select * from db.sch.orders qualify row_number() over (partition by cid order by ts desc) = 1), \
             b as (select cid, ts, region from db.sch.customers) \
             select a.id, a.amount, b.region from a join b on a.cid = b.cid",
            "snowflake",
            &dedup_tables(),
        );
        assert_eq!(dedup_keys(&r), vec![("DB.SCH.ORDERS", "cid"), ("DB.SCH.ORDERS", "ts")]);
    }

    #[test]
    fn a_qualify_key_is_followed_through_a_rename() {
        let r = resolve(
            "with o as (select cid as customer_id, ts, amount from db.sch.orders), \
             d as (select * from o qualify row_number() over (partition by customer_id order by ts desc) = 1) \
             select d.customer_id, d.amount, r.reason from d join db.sch.returns r on d.customer_id = r.customer_id",
            "snowflake",
            &dedup_tables(),
        );
        assert_eq!(dedup_keys(&r), vec![("DB.SCH.ORDERS", "cid"), ("DB.SCH.ORDERS", "ts")]);
    }

    #[test]
    fn each_union_branch_keeps_its_own_qualify() {
        let r = resolve(
            "with u as (select cid, ts, amount from db.sch.orders \
             qualify row_number() over (partition by cid order by ts desc) = 1 \
             union all select cid, ts, amount from db.sch.orders_archive \
             qualify row_number() over (partition by cid order by ts desc) = 1) \
             select cid, amount from u",
            "snowflake",
            &dedup_tables(),
        );
        assert_eq!(
            dedup_keys(&r),
            vec![
                ("DB.SCH.ORDERS", "cid"),
                ("DB.SCH.ORDERS", "ts"),
                ("DB.SCH.ORDERS_ARCHIVE", "cid"),
                ("DB.SCH.ORDERS_ARCHIVE", "ts"),
            ]
        );
    }

    #[test]
    fn a_qualify_in_a_derived_table_is_read_in_it() {
        let r = resolve(
            "select x.id, c.region from (select id, cid, ts from db.sch.orders \
             qualify row_number() over (partition by cid order by ts desc) = 1) as x \
             join db.sch.customers c on x.cid = c.cid",
            "snowflake",
            &dedup_tables(),
        );
        assert_eq!(dedup_keys(&r), vec![("DB.SCH.ORDERS", "cid"), ("DB.SCH.ORDERS", "ts")]);
    }

    #[test]
    fn a_qualify_key_never_names_a_column_the_relation_lacks() {
        let r = resolve(
            "with o as (select cid, bogus_ts from db.sch.orders), \
             d as (select * from o qualify row_number() over (partition by cid order by bogus_ts desc) = 1) \
             select cid from d",
            "snowflake",
            &dedup_tables(),
        );
        assert_eq!(dedup_keys(&r), vec![("DB.SCH.ORDERS", "cid")]);
    }

    #[test]
    fn a_commented_out_qualify_is_not_read() {
        let r = resolve(
            "with a as (select id, cid from db.sch.orders\n\
             -- qualify row_number() over (partition by cid order by id) = 1\n\
             ), b as (select cid, ts from db.sch.customers \
             qualify row_number() over (partition by cid order by ts) = 1) \
             select a.id, b.ts from a join b on a.cid = b.cid",
            "snowflake",
            &dedup_tables(),
        );
        assert_eq!(dedup_keys(&r), vec![("DB.SCH.CUSTOMERS", "cid"), ("DB.SCH.CUSTOMERS", "ts")]);
    }

    #[test]
    fn a_qualify_on_an_alias_reads_the_window_the_alias_names() {
        // The select list names the window and the QUALIFY names only that.
        let r = resolve(
            "select id, amount, row_number() over (partition by cid order by ts desc) as rn \
             from db.sch.orders qualify rn = 1",
            "snowflake",
            &dedup_tables(),
        );
        assert_eq!(dedup_keys(&r), vec![("DB.SCH.ORDERS", "cid"), ("DB.SCH.ORDERS", "ts")]);
    }

    #[test]
    fn a_qualify_key_two_sources_have_is_counted_and_read_by_nobody() {
        let r = resolve(
            "select o.id from db.sch.orders o join db.sch.customers c on o.cid = c.cid \
             qualify row_number() over (partition by cid order by o.ts) = 1",
            "snowflake",
            &dedup_tables(),
        );
        assert_eq!(dedup_keys(&r), vec![("DB.SCH.ORDERS", "ts")]);
        assert_eq!(r.unplaced_qualify, 1);
    }

    fn join_keys(r: &Resolved) -> Vec<(&str, &str)> {
        let mut out: Vec<(&str, &str)> = r
            .indirect
            .iter()
            .filter(|i| i.role == crate::role::Role::JoinKey)
            .map(|i| (i.relation.as_str(), i.column.as_str()))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn a_join_key_is_read_where_the_join_is_written_and_traced_to_its_table() {
        // Both sides are CTEs, the right one renaming its key: the engine hands
        // over no condition for a join like this one, whose sides both feed.
        let r = resolve(
            "with a as (select id, cid from db.sch.orders), \
             b as (select cid as bid, region from db.sch.customers) \
             select a.id, b.region from a join b on a.cid = b.bid",
            "snowflake",
            &dedup_tables(),
        );
        assert_eq!(join_keys(&r), vec![("DB.SCH.CUSTOMERS", "cid"), ("DB.SCH.ORDERS", "cid")]);
    }

    #[test]
    fn a_qualifier_decides_and_an_ambiguous_name_is_counted() {
        let r = resolve(
            "select o.id, c.region from db.sch.orders o join db.sch.customers c on o.cid = c.cid and ts > 0",
            "snowflake",
            &dedup_tables(),
        );
        assert_eq!(join_keys(&r), vec![("DB.SCH.CUSTOMERS", "cid"), ("DB.SCH.ORDERS", "cid")]);
        assert_eq!(r.unplaced_join, 1, "`ts` is a column of both");
    }

    #[test]
    fn an_alias_reused_in_another_cte_never_lends_its_relation() {
        let r = resolve(
            "with x as (select o.cid from db.sch.orders o), \
             y as (select o.region from db.sch.customers o join x on o.cid = x.cid) \
             select * from y",
            "snowflake",
            &dedup_tables(),
        );
        assert_eq!(join_keys(&r), vec![("DB.SCH.CUSTOMERS", "cid"), ("DB.SCH.ORDERS", "cid")]);
    }

    #[test]
    fn a_using_column_is_a_key_on_both_sides() {
        let r = resolve(
            "select o.id, c.region from db.sch.orders o join db.sch.customers c using (cid)",
            "snowflake",
            &dedup_tables(),
        );
        assert_eq!(join_keys(&r), vec![("DB.SCH.CUSTOMERS", "cid"), ("DB.SCH.ORDERS", "cid")]);
    }

    fn filter_tables() -> Vec<Visible> {
        vec![
            vis("DB.SCH.ORDERS", &["id", "amount", "cid", "ts", "status"]),
            vis("DB.SCH.MARK", &["ts"]),
        ]
    }

    fn reads_of(r: &Resolved, role: crate::role::Role) -> Vec<(&str, &str)> {
        let mut out: Vec<(&str, &str)> = r
            .indirect
            .iter()
            .filter(|i| i.role == role)
            .map(|i| (i.relation.as_str(), i.column.as_str()))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn a_filter_on_a_cte_column_is_read_back_to_its_table() {
        // The engine localises the predicate to the CTE it reads and stops.
        let r = resolve(
            "with a as (select * from db.sch.orders), b as (select id from a where status = 'x') select * from b",
            "snowflake",
            &filter_tables(),
        );
        assert_eq!(reads_of(&r, crate::role::Role::Filter), vec![("DB.SCH.ORDERS", "status")]);
    }

    #[test]
    fn a_subquery_a_filter_compares_with_is_read_as_a_filter() {
        let r = resolve(
            "select id from db.sch.orders where ts = (select max(ts) from db.sch.mark)",
            "snowflake",
            &filter_tables(),
        );
        assert_eq!(
            reads_of(&r, crate::role::Role::Filter),
            vec![("DB.SCH.MARK", "ts"), ("DB.SCH.ORDERS", "ts")]
        );
        assert_eq!(r.outputs, vec!["id"], "the subquery is no output column");
    }

    #[test]
    fn a_group_key_and_a_distinct_column_are_no_row_deciding_reads() {
        // Measured on a 3341 model project before deciding against a role for
        // them: nearly every group key and distinct column is projected too,
        // and has its direct edge. 0026 says why no role.
        let grouped = resolve(
            "with g as (select status, cid, max(ts) as last_ts from db.sch.orders group by status, cid) \
             select cid, last_ts from g",
            "snowflake",
            &filter_tables(),
        );
        assert!(grouped.indirect.iter().all(|i| i.column != "status"), "{:?}", reads_of(&grouped, crate::role::Role::Filter));
        let distinct = resolve("select distinct cid from db.sch.orders", "snowflake", &filter_tables());
        assert!(distinct.indirect.is_empty());
    }

    #[test]
    fn a_join_key_that_feeds_no_column_is_still_read() {
        let r = resolve(
            "select o.id from db.sch.orders o join db.sch.customers c on o.cid = c.id where c.region = 'eu'",
            "snowflake",
            &[vis("DB.SCH.ORDERS", &["id", "cid"]), vis("DB.SCH.CUSTOMERS", &["id", "region"])],
        );
        assert_eq!(r.outputs, vec!["id"], "customers feeds no output column");
        let mut got: Vec<(&str, &str, &str)> = r
            .indirect
            .iter()
            .map(|i| (i.relation.as_str(), i.column.as_str(), i.role.as_str()))
            .collect();
        got.sort();
        assert!(got.contains(&("DB.SCH.CUSTOMERS", "region", "filter")), "{got:?}");
        assert!(
            got.iter().any(|(r, c, k)| *r == "DB.SCH.CUSTOMERS" && *c == "id" && *k == "join_key"),
            "{got:?}"
        );
    }

    #[test]
    fn an_aggregate_survives_being_wrapped_further_down_the_chain() {
        // The last expression on the path is `total * 1.2`, which says only
        // that something happened. The aggregate is the fact worth keeping.
        let r = resolve(
            "with a as (select cid, sum(amount) as total from db.sch.orders group by cid) \
             select cid, total * 1.2 as gross from a",
            "snowflake",
            &[vis("DB.SCH.ORDERS", &["cid", "amount"])],
        );
        let e = r.edges.iter().find(|e| e.to_column == "gross").expect("gross");
        assert_eq!(e.from_column, "amount");
        assert_eq!(
            crate::role::classify(&e.expression, &e.from_column, &e.to_column),
            crate::role::Role::Aggregate
        );
    }

    #[test]
    fn a_recovered_name_must_be_a_column_the_relation_has() {
        let known = HashSet::from(["id".to_string(), "amount".to_string()]);
        // `total` is not a column of the relation, and `max` is a call.
        assert_eq!(
            columns_named_in("max(amount) over (partition by total)", &known),
            vec!["amount"]
        );
        // A qualifier is not the column it qualifies.
        assert_eq!(columns_named_in("o.amount", &known), vec!["amount"]);
        // A string literal names nothing, even when it spells a column.
        assert!(columns_named_in("'amount'", &known).is_empty());
        // A keyword is never taken for a column.
        assert!(columns_named_in("case when x then y end", &HashSet::from(["end".to_string()]))
            .is_empty());
    }
}
