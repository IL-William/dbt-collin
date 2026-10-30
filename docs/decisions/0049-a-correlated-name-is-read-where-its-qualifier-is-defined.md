# 0049. A correlated name is read where its qualifier is defined

Date: 2026-10-30 · Status: accepted · Amends 0026

**Trigger:** read before changing which sources a `WHERE` or a `HAVING` binds
its names to, or what a nested query is allowed to see of the one around it.

## Context

0026 reads a filter in its own `SELECT`, as 0031 has a name read in the scope
that wrote it: each name of a `WHERE` or a `HAVING` is bound against the
sources of that `SELECT` alone, and a qualifier binding none of them is not
counted, being, 0026 said, how a correlated subquery reads its outer query. It
is also how the read of that outer name is lost. A subquery in an expression
can name its enclosing `SELECT`'s sources, though: in `select b.book_id,
(select max(r.stars) from reviews as r where r.book_id = b.book_id) from books
as b`, the `b` of the subquery's `WHERE` is defined one `SELECT` out. Bound in
its own `SELECT` it binds to nothing and the read is dropped, so the filter on
`books.book_id`, the column the subquery is correlated on, never reaches the
cache. The same holds for `exists (...)` and `in (select ...)`.

## Decision

- **A query that is a subquery inside an expression of a `SELECT`** (its select
  list, its `WHERE` or its `HAVING`, as a scalar subquery, `EXISTS` or `IN`,
  at any depth of the expression but not inside another query) carries that
  `SELECT`'s sources as its nearest enclosing scope, then the enclosing scopes
  of that `SELECT`, outward.
- **A qualified name whose qualifier binds in none of the query's own sources**
  is bound in the nearest enclosing scope that defines the qualifier, and read
  there with the role it would have had in its own: a filter. It is not counted
  as a name the pass could not place.
- **The search stops at the first scope whose `FROM` defines the qualifier.**
  The sources the pass binds names to leave out what it cannot read: a
  `LATERAL FLATTEN`, a `TABLE(...)`, an `UNNEST`, a pivot, a parenthesised
  join, an unaliased derived table. Each still names a qualifier of its
  `SELECT`, so at the query's own `FROM` and at every scope on the way out, a
  qualifier a factor defines is that `SELECT`'s: if its sources list it, the
  name binds there, and if not, it is read by nobody, and never taken for the
  outer query's alias of the same name. A factor with no name to tell it by
  hides every qualifier.
- **A CTE body and a derived table in `FROM` get no enclosing scope.** A CTE
  body cannot name the `SELECT` that uses it. A `LATERAL` derived table can
  correlate (`lateral (select ... where r.book_id = b.book_id) as x`); that is
  a limit left as it is, and its outer names stay unread.
- **The outer query is visited before the ones it holds**, and records each
  subquery's chain by the subquery's address while it does; the syntax tree is
  borrowed for the whole walk, so an address names one query.
- Only `WHERE` and `HAVING` predicates are read this way. `JOIN` keys and
  `QUALIFY` are unchanged.

## Rejected

- **Binding an unqualified name outward.** A bare name that the query's own
  sources do not have may be an alias, a column of a source with no column list
  or a typo; which enclosing scope owns it is a guess, in a pass that labels
  what it reads. A qualifier names its scope, so only a qualified name looks
  outward.
- **Letting every nested query, CTEs and derived tables included, see the
  outer one.** A CTE body cannot name a source of the `SELECT` around it, and
  a derived table does only when `LATERAL`; for the rest the only thing it
  would add is a wrong binding: the alias a dbt model reuses across its CTEs
  (0031) bound to the scope of the outer `SELECT`.

## Consequences

The scalar subquery of the lineage-library's `book_ratings` now reads
`books.book_id` as a filter, and its mark in `lineage-library.known.txt` is
gone. The edges the engine gives a scalar subquery in a select list are another
matter, a limit of the engine this record does not touch.

Measured on Jaffle Shop and Shopify, before and after: the caches are identical,
with and without `--indirect`, as are the totals of the reports. Neither holds a
correlated subquery, so nothing there moves.
