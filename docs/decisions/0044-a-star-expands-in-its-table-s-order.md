# 0044. A star expands in its table's order

Date: 2026-10-11 · Status: accepted · Amends 0004

**Trigger:** read before sorting a column list the engine is handed, before
comparing two column lists as lists, or before reading the catalog, the YAML
or a compile's outputs into the store.

## Context

Every column list was sorted: the catalog's, which dropped each column's
`index`; the YAML's; and a compile's outputs. Sorting suited every comparison,
which wants sets. But the engine was handed the same lists, so a `select *`
expanded in alphabetical order. Most SQL cannot tell the difference. A `UNION`,
`INTERSECT` or `EXCEPT` can, because it pairs its branches by position (0028's
ninth patch). Where two branches' tables order their columns differently, or
where a branch written out faces a star, pairing by alphabetical position pairs
by name. Snowflake does otherwise. Such a mispairing is a project's bug, and
it is the one thing a lineage should show.

On a 3507 model Snowflake project, 106 models have a positional set operation
whose every branch is a star: 185 of them. sqlglot expanded each in the
tables' order and in alphabetical order, and all 106 pair the same way both
times, because each union's branches share an order. That held by chance: 5833
of the catalog's 5916 tables do not list their columns alphabetically.
Nothing would have caught a union that did not. The sqlglot comparison and the
blind audit are handed the lists the engine was handed, so they shared the
blind spot. lineage-library's `all_loans` shows the shape: 7 edges missing and
7 wrong.

## Decision

- **Each list is kept twice in `Store`.** Sorted, it is a set: the agreement,
  the YAML's staleness, inference, and every list the report and the cache
  derive from it read it this way, as before. In its source's order, it is
  what the engine is handed.
- **The order is the source's own.**
  - The catalog's comes from each column's `index`; a column dbt gave no
    `index` goes last.
  - A compile's comes from its projection, kept by the engine as `projection`
    beside the sorted `outputs`.
  - The YAML's comes from the order it is written in, read from the manifest
    without a map.
- **The YAML's order is the project's word on where a column sits**, and worth
  no more than its word on which columns there are (0004). A star over a parent
  known only by its YAML takes that order, and the dump's `declared`
  provenance says so. On the Snowflake project no star does: all 5061 lists the
  engine is handed come from the warehouse.

## Rejected

- **Pairing a union by name.** That is what sorting did, and it reads the SQL
  the way the author probably meant it, not the way the warehouse runs it.
- **One list, in order, everywhere.** Inference and the report iterate these
  lists, so inferred edges and reported columns would change their order with
  no edge moving, and the hash of the cache would no longer prove that nothing
  moved.
- **Setting aside a positional pairing through a star whose order only a YAML
  gives.** It would need the engine to say which star feeds which branch, and
  no star on the Snowflake project is in that case.

## Consequences

On the Snowflake project the cache does not move, with or without
`--indirect`, and neither do the report's totals. The dump of 3458 models
differs only in the order of its lists, and the report only in one engine
message that lists a table's columns. Jaffle Shop and Shopify do not move.
lineage-library's `all_loans` is read exactly. sqlglot is now handed each table
in its order, through the dump, so the comparison no longer shares the blind
spot.
