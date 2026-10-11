# 0041. A window key is read in its own SELECT

Date: 2026-10-11 · Status: accepted · Amends 0031

**Trigger:** read before changing where the keys of a window, of a `WITHIN
GROUP` or of an `ORDER BY` inside a call are read, or which source such a key
is credited to.

## Context

The engine resolves none of these keys. 0031 reads a window's keys out of the
`OVER` text at the hop whose edge carries the window, against the scope the
column comes from. That scope is the engine's choice, not the SQL's: a window
whose function takes no column, `row_number()` or `dense_rank()`, is hung off
the first source of its `SELECT` alone. A key qualified with another source was
then read against the first one, and lost when the first had no column of that
name, or credited to it when it had one. `WITHIN GROUP (ORDER BY c)` and
BigQuery's `ARRAY_AGG(x ORDER BY c)` were never read at all.

On a 3507 model Snowflake project, read against sqlglot, this was what was left
in collin's own reading: three edges credited to the wrong side of a join, and
six missing, the three right ones among them.

## Decision

- **Each select list item holding a window or an ordered aggregate is read off
  the syntax tree**, in the `SELECT` that writes it, with that `SELECT`'s
  sources, the way 0023, 0025 and 0026 read a `QUALIFY`, a join and a filter. A
  qualified key goes to the source its qualifier names, a bare one to the one
  source having it. A key of a relation is an edge; a key of a CTE or derived
  table is the column the walk goes on from.
- **The walk finds the item by its text and its scope**: the expression as the
  engine prints it, and the CTE, derived table or output owning the column. Two
  items of one text in one scope, the two sides of a `UNION`, count only when
  they bind the same keys, since the walk cannot tell which side it is on.
- **The text read further on loses the keys bound here**, so that no step
  beyond reads them a second time against another scope.
- **Where the tree cannot bind a key, the text is read as 0031 has it**: a named
  window, a `SELECT` reading a table function, an `UNNEST` or a nested join, an
  unaliased derived table, a bare name several sources have, a relation with no
  column list, a CTE name the statement gives to several scopes.

## Rejected

- **A statement wide map of aliases**, which 0031 rejected and still is: an
  alias reused across CTEs would bind to the wrong one. The binding here is per
  `SELECT`, where an alias means one thing.
- **Teaching the engine to resolve the keys.** It would change the edges of
  every window in every model, their roles included, to reach the same result,
  where collin already reads three clauses of a `SELECT` off the tree.
- **Dropping the text reading.** It still serves the cases above: on that
  project 3 of the 29 window and ordered aggregate expressions the walk meets,
  on a 624 model BigQuery project 13 of 123.

## Consequences

On the Snowflake project the cache moves by six edges in four models, and
nothing else does: three window keys move from the source the engine hung the
window off to the one the SQL names, two `ORDER BY` keys of the second source
of a join arrive, and one `WITHIN GROUP` key. sqlglot then reads those four
models exactly as collin does. Under `--indirect` the same six, and the join key
the three wrong edges hid is a join key again. The same project compiled for
another environment moves the same way.

On the BigQuery project the cache gains 28 edges in 11 models and loses none,
each read against the SQL: keys of `ARRAY_AGG`, `STRING_AGG` and
`ARRAY_CONCAT_AGG` ordered inside the call, and two window keys qualified with a
CTE. Under `--indirect`, 5 indirect edges of the same pairs give way to the
louder role (0013). The two fixtures do not move.
