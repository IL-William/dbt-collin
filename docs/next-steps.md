# Next steps

Where the lineage stands, and what would raise it. Measured on 2026-10-10 on a
pinned copy of a 3507 model Snowflake project, its manifest and its catalog
generated on one target, the engine at the fork's `collin-engine-1`. Not a
decision: a list to come back to.

## Where the columns without an edge are

Of 110 155 columns, 104 455 have an edge, 94.8%. Against the warehouse where a
model has a table, 3316 of the 3507, and its YAML otherwise, columns built from
constants aside, 97.6%. The 5700 without an edge, by cause:

| Columns | Cause | Lever |
| --- | --- | --- |
| 3372 | Built from constants: literals, functions of literals, a date spine | None: they have no parent, and the report counts them as roots |
| 1763 | Columns the table has and the code no longer produces, in 158 models settled column by column (0029). 1365 of them are 13 metadata columns 105 tables of one family still have and 104 of their sources no longer do | Rebuild those tables |
| 265 | One compile that does not parse: an introspecting macro found no column in a parent the target lacks, and wrote `select from` | Build the parent first, or make the macro fail on an empty list |
| 107 | Columns the YAML documents for compiles that read a name their own CTE does not project, from the same empty introspection | As above |
| 83 | Read through a star over a parent the target lacks, or downstream of a compile that does not parse | As above |
| 61 | Compiles set aside, where no parent has the name: downstream of the same, and an audit table with no parent | As above, or none |
| 42 | Read from a raw table no source declares (35), or from a parent the target lacks | Declare the source |
| 5 | No parent: an audit table | None |
| 2 | A date spine set aside for two CTEs of one name in nested `WITH` blocks (0018) | None that buys an edge |

58 more compiles do not parse for the same reason, another macro writing
`select ,`, and have no column list to count. Nearly all of it is fixed in the
project and in the target's tables, not here: the report names every model.

An independent reader backs the first row. Following every root through its
CTEs, it finds 3360 of the 3377 ending at no table. Of the other 17, 15 reach
an incremental model's own table (0012), one is a literal the check misread,
and one is a `LATERAL FLATTEN`'s `INDEX` (below). The name read through a star
in an expression, the residual 0030 leaves, costs nothing here.

## What a catalog of another target did

On the same day the manifest and the catalog in the project's `target/` came
from two targets: not one of the 2912 models both describe was the same table
in each. collin matched them by unique_id and used every entry. An entry now
witnesses only the table the manifest names
([0037](decisions/0037-a-catalog-entry-for-another-table-is-no-witness.md)).

## Precision

Coverage says a column has an edge, not that the edge is right. Every parsed
edge was compared with an independent reading of the same SQL by sqlglot,
handed the column lists the engine was handed, names compared without case as
collin compares them. Of the 3507 models, 3394 were compared: 71 are set aside
or do not parse, and sqlglot refuses 42 whose parent has no column list.

| | Edges |
| --- | --- |
| Read by both | 115 038 |
| By collin alone | 245 |
| By sqlglot alone | 140 |

Each disagreement was read in the SQL.

- **collin alone and right, 242.** 215 come out of a raw table no source
  declares, of which sqlglot was given no column. 17 sit under
  `UNION ALL BY NAME`, which sqlglot pairs by position: its own 134 edges in
  that model are those pairings, every one wrong. 7 sit under a top level
  select in parentheses the comparison did not read, and 3 pass through a
  recursive CTE, which sqlglot does not follow.
- **collin wrong, 3.** A window's partition key written `cte.k`, credited to
  the other side of the join, which has a `k` too. 0031 reads a window's keys
  against the scope the edge comes from, and does not read the qualifier: it
  rejected binding one by reading the `FROM` clauses.
- **collin missing, 3.** Two columns built from a `LATERAL FLATTEN`'s `KEY`
  and `INDEX`, which the engine derives from nothing, where it derives `VALUE`
  and `THIS` from the input (0028). One `ARRAY_AGG ... WITHIN GROUP (ORDER BY
  c)`, where collin does not count `c` as an input of the array its order
  shapes.

Six edges in 115 286, then, as far as the SQL shows. The three wrong ones sit
beside a right edge into the same column. Of the two `FLATTEN` columns, one is
among the 83 lost above and one among the roots.

## Residuals already named

- **Two CTEs of one name in nested `WITH` blocks** still merge in the engine
  (0018). Here one model has them, a date spine reading no table: fixing the
  engine would turn its two lost columns into roots, and add no edge. Not worth
  a patch until a model with a parent has them.
- **An inferred column several parents could give** is settled by the compile,
  by every candidate, or by manifest order (0027): 35 by every candidate and 58
  by order here.
- **`* REPLACE (...)` and `* ILIKE '...'`** are not read: the star expands to
  every column.
- **A qualified window key** is read against the scope its edge comes from, so
  `partition by b.k` in a join of `a` and `b` that both have `k` credits `a.k`
  when the windowed column comes from `a`: 3 edges here.
- **`LATERAL FLATTEN`'s `KEY`, `PATH` and `INDEX`** derive from nothing in the
  engine, though each describes the input: 2 edges here.
