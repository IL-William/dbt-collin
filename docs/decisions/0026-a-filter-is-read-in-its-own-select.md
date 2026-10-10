# 0026. A filter is read in its own SELECT

Date: 2026-10-10 · Status: accepted · Amends 0011

**Trigger:** read before changing how a `WHERE` or `HAVING` is read, or before
adding a role for a column that decides which rows exist.

## Context

The engine localises a `WHERE` predicate to the relation whose columns it reads,
and collin reported those on tables. A predicate over a CTE's columns, which is
most predicates in a dbt model, the engine localises to the CTE, and collin
read no further. A subquery inside a predicate, `x = (select max(ts) ...)`, it
read as if it were a column of the output.

## Decision

`engine.rs` reads every `WHERE` and `HAVING` off the syntax tree in the `SELECT`
that holds it, as 0023 and 0025 read a `QUALIFY` and a join: each name to the
one source of that `SELECT` having it, through CTEs back to the relation. A
subquery the predicate compares with, `in (...)`, `= (...)` or `exists (...)`,
has its select list read as a filter in its own scope. These add to the
predicates the engine localises to tables, which stand. A name several sources
have is counted and read by nobody; a qualifier binding no source of the
`SELECT` is not counted, being how a correlated subquery reads its outer query.

## Rejected

- **Replacing the engine's own filters.** They were all found in an independent
  reading of the SQL; this adds what they miss.
- **A `group_key` role, and one for `DISTINCT`.** Measured before deciding: on a
  3341 model project, 94.9% of the columns a `GROUP BY` names and 99.4% of those
  a `DISTINCT` covers are projected too and have their direct edge, and once
  joins and filters are read, 12 group keys in 3 models and 6 distinct columns
  remain that nothing else records. A role for that few would be one more kind
  for every reader to learn. Two tests pin that neither is read as a row
  deciding read.

## Consequences

On the same project the default cache is identical. Filters in the report go
from 1631 to 1984, 353 new in 149 models and none lost; no name was ambiguous.
Under `--indirect` the cache grows from 281 571 edges to 298 340.
