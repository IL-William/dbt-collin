# 0025. A join key is read where the join is written

Date: 2026-10-10 · Status: accepted · Amends 0011

**Trigger:** read before changing how a join condition is read, or which
relation a join key is credited to.

## Context

The engine gives a join's condition in one place only: on the edge it raises
for a relation joined to filter rows, one that feeds no output column, and then
for that relation's side alone. A join between two CTEs, or with a relation
that also feeds an output, which is every join of a dbt model that selects from
both sides, gave nothing. On a 3341 model project the report held 39 join keys,
all right, against some 2000 in the SQL.

## Decision

`engine.rs` reads every `JOIN ... ON` and `JOIN ... USING` off the syntax tree,
in the `SELECT` that holds it, the same way 0023 reads a `QUALIFY`: a qualified
name goes to the source its qualifier binds, an unqualified one to the one
source having it, a CTE or derived source is followed back to its relations,
renames included, and a pair is kept only when the relation's list has the
column. A `USING` column is on both sides by definition, so every source having
it is credited. A name several sources have, or a qualifier binding none, is
read by nobody and counted. This replaces the reading of the engine's
join edge. A model's reads of its own relation stay out of the report, as 0012
keeps them out of the fault list.

## Rejected

- **Keeping the engine's join edge and adding the rest.** It says less than the
  syntax tree, on fewer joins, and would read the same condition twice.
- **Crediting every relation that has a column by the name.** The failure 0023
  took out of `QUALIFY`.

## Consequences

On the same project the default cache is identical. Join keys in the report go
from 39 to 2026, in 305 models, none of the 39 lost; the rows deciding reads go
from 2914 to 4901 and those never projected from 415 to 1765. No join condition
name was ambiguous. Under `--indirect` the cache grows from 194 615 edges to
281 571, which is what 0011 made the flag cost.
