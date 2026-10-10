# 0005. The cache stays at version 1

Date: 2026-10-10 · Status: accepted

**Trigger:** read before adding a field to the cache, or before bumping its
version number.

## Context

dbt-lens reads `column_lineage.json` and **refuses any version above 1**. It is
already released and installed on machines that will not be rebuilt on our
schedule. A cache at version 2 is a cache nobody can read.

The format already has what this project needs. It names dbt nodes and columns,
it reserves the `rel:<db.schema.object>` prefix for objects dbt does not own, and
it carries a `kind` per edge that travels the whole chain, from `src/collin.rs`
through `merge_col_lineage` to `Lineage.edge_kinds` and into the front end. That
`kind` used to hold a Snowflake domain, `table` or `view`. We fill it with the
semantic role instead.

## Decision

Stay at version 1. A released dbt-lens consumes what this writes with no rebuild
and no coordination.

## Rejected

- **Version 2 now, with a `uses` field for join keys, filters and group keys.**
  It is the right shape: an influence has no natural target column, since a join
  key influences every column of the model rather than one. It also makes every
  released dbt-lens refuse the file. It waits, and it is the next increment, not
  the first.
- **Smuggling influences into version 1 as ordinary edges.** A join key aimed at
  an arbitrary column would be a lie in the field this project exists to make
  trustworthy.

## Consequences

Roles for projection edges ship today with zero changes required in dbt-lens,
which was proved end to end before any dbt-lens code was touched. Join keys,
filters and the window functions consumed only by `QUALIFY`, which is 341 models
here, stay invisible until version 2. That cost is named in the report rather
than hidden.
