# 0002. flowscope-core is the engine, behind an interface that hides it

Date: 2026-10-10 · Status: accepted

**Trigger:** read before touching `engine.rs`, or before proposing that we write
our own resolver.

## Context

"It is easier to read column lineage off an AST" is true and incomplete. The AST
is about 40% of the job. `sqlparser` says so itself: *this crate provides only a
syntax parser, and tries to avoid applying any SQL semantics*. On
`select *, a + b as c from x join y using (k)` the AST does not know which
columns hide behind `*`, nor which table owns `a`.

The other 60% is a name resolver: a scope stack, star expansion, resolution of
unqualified columns. That is the part a naive implementation gets wrong, and it
is the part that decides accuracy. It matters here: **72.5%** of the corpus's
models use `SELECT *`.

## Decision

Use `flowscope-core` (Apache-2.0, built on the same `sqlparser`) and put it
behind `engine.rs`, which is the only module in the crate that knows it exists.
Everything downstream speaks `Resolved`, `Visible` and `RawEdge`.

Chosen on measurement, not on reading its README. On 3341 models in 3.3 seconds:
**92.3%** of the columns the warehouse actually has, **73.2%** of models exactly
right, against `catalog.json` on the 585 models it covers. It expands stars
through chains of CTEs once a relation is fully qualified, and it raises
`UNRESOLVED_REFERENCE` and `APPROXIMATE_LINEAGE` instead of inventing.

## Rejected

- **Writing the resolver ourselves.** It is the expensive 60%, and we would be
  starting from a worse place than a library already at 92.3%.
- **`polyglot-sql`.** A Rust port of SQLGlot with schema-aware star expansion, so
  a real candidate. Not measured to beat flowscope here, and adopting on a
  README is what this decision exists to avoid.

## Consequences

The interface is the insurance. flowscope 0.9.0 is young and 56% documented; if
it stalls, a hand written resolver slides in behind `engine.rs` and nothing else
changes.

It also has to be paid for. The engine hangs some derived columns off the
relation rather than off one of its columns, and a column to column walk alone
silently reported nothing for them, which cost every window function its
lineage. Its `JoinDependency` edges are table level, with no expression and no
column, so join keys cannot be read off them: the `uses` field of a future cache
version needs our own reading of the ON clause. Working around what the engine
does not give is expected, and it belongs in `engine.rs`.
