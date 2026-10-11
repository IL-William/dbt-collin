# Next steps

Where the lineage stands, and what would raise it. Measured on 2026-10-11 with
collin 0.6.0, the engine at the fork's `collin-engine-4`, on a pinned copy of a
3507 model Snowflake project, its manifest and its catalog generated on one
target with that target's environment. Not a decision: a list to come back to.

## Where the columns without an edge are

Every compile parses. Of 114 897 columns, 111 429 have an edge, 97.0%. Against
the warehouse where a model has a table, 3499 of the 3507, and its YAML
otherwise, columns with no parent to find aside, 111 316 of 111 317: every
column but one. The 3468 without an edge, by cause:

| Columns | Cause | Lever |
| --- | --- | --- |
| 3446 | Built from constants: literals, functions of literals, a date spine, a `VALUES` list, a `LATERAL FLATTEN`'s index (0016) | None: they have no parent, and the report counts them as roots |
| 21 | Three models that read only their own table: two audit tables written outside dbt, whose incremental compile is `select * from` itself `where 1 = 0` or close to it, and a date spine reading its last date | None in dbt: the report counts them apart and names the models (0039). The project can declare a table written outside dbt as a source |
| 1 | A view older than its code: it has a column neither the code nor its source has | Rebuild the view |

The report names every one, with where the walk back stopped, and gathers
them into findings that say what to do (0040). Here there are three: two
tables older than their code, three models reading only their own table, and
YAML disagreeing with 1282 tables.

## A target compiled without its environment

The same project was first measured on a compile for this target made without
its environment file, so every model and source pointed at another
environment's databases, and so did the catalog. There 94.8% of columns have an
edge, 104 458 of 110 155, and the findings read like the project's own:

- 59 compiles do not parse: an introspecting macro wrote them as `select ,` or
  `select from`, because the parent it read did not exist in that environment;
- 149 tables are older than their code, 2912 columns, and 32 behind it;
- 39 models read a parent with no known columns, 5 a relation dbt was not told
  of;
- 17 compiles read a name their own CTE lacks, and 15 models have 108 columns
  collin could not follow.

None of it is there in the right environment. collin cannot tell which
environment a target was meant for, but a project whose models of one target
sit in databases named for another is a finding it could raise.

## What a catalog of another target did

On 2026-10-10 the manifest and the catalog in the project's `target/` came
from two targets: not one of the 2912 models both describe was the same table
in each. collin matched them by unique_id and used every entry. An entry now
witnesses only the table the manifest names
([0037](decisions/0037-a-catalog-entry-for-another-table-is-no-witness.md)).

## Precision

Coverage says a column has an edge, not that the edge is right. Every parsed
edge was compared with an independent reading of the same SQL by sqlglot,
handed the column lists the engine was handed, names compared without case as
collin compares them. 3506 of the 3507 models were compared; the other, an
audit table reading only itself, has no parsed edge. Measured again on
2026-10-11, once a window's keys were read in their own `SELECT` (0041); the
engine's ninth patch moves nothing here.

| | Edges |
| --- | --- |
| Read by both | 133 161 |
| By collin alone | 27 |
| By sqlglot alone | 1358 |

The disagreements sit in 5 models, and each was read in the SQL.

- **collin alone and right, 27.** 17 sit under `UNION ALL BY NAME`, which
  sqlglot pairs by position: its own 134 edges in that model are those
  pairings, every one wrong. 7 sit under a top level select in parentheses the
  comparison did not read, and 3 pass through a recursive CTE, which sqlglot
  does not follow.
- **sqlglot alone and wrong, 1358.** The 134 pairings, and 1224 in two models
  that build an array of objects out of a hundred columns and `LATERAL FLATTEN`
  it: sqlglot credits each element's index to every one of those columns,
  where the index is a position. Both read the element's key from them.
- **collin wrong or missing, none.** Before 0041 there were 3 wrong and 6
  missing, all keys the engine leaves unread: a window's partition key written
  `cte.k`, credited to the other side of the join, which has a `k` too, and so
  missing where it belongs; two window `ORDER BY` keys qualified with the
  second source of a join; one `ARRAY_AGG ... WITHIN GROUP (ORDER BY c)`.

None in 133 188, then, as far as the SQL shows. An independent reader
backs the roots too: following each through its CTEs, 3424 of the 3441 then
counted end at no table. Of the other 17, 15 reach an incremental model's own
table (0012), one is a literal the check misread, and one, a `CASE` on a
`FLATTEN`'s key, has had an edge since the engine's eighth patch (0028).

Two readers agreeing can share a blind spot: both are handed the same SQL and
the same column lists. A third reading saw neither answer. 115 output columns
were drawn at random: 70 across the project, 30 of any column, 30 with an edge
other than a copy and 10 roots, and 45 harder ones, 20 in models of four CTEs
or more with a join, 10 with three inputs or more, 15 under a window, an
aggregate or a hash. Language model agents, shown only each model's SQL and
the column lists, traced every one to the columns it reads, 277 in all. collin
gave exactly that set for all 115, and so did sqlglot. None wrong in 115 puts
the columns whose inputs collin gets wrong under 3% at 95% confidence; the
sample says no more than that.

## On public projects

The project above is Snowflake and has its own shapes. Two public ones show
others.

**Cal-ITP**, the dbt artifacts Caltrans publishes for its transit warehouse:
624 BigQuery models, manifest and catalog from one run, pinned outside the
tree. 95.0% of columns have an edge, 16 165 of 17 008, and 96.0% against the
warehouse, 16 161 of 16 841. It took four changes to get there: dbt's backticks
read as quotes, so that its relations match; a STRUCT's fields, which the
catalog lists, no longer taken for columns (#15); two CTEs of one name read
apart, the engine's seventh patch; and a `UNION`'s branches paired by
position, its ninth, which moved three models from contradicted by the
warehouse to confirmed. The 680 columns still without an edge, by first cause:

| Columns | Cause | Where it would be fixed |
| --- | --- | --- |
| 447 | Lost, a name unresolved: led by a STRUCT's field, `metadata.extract_ts`, read as a column of a table named `metadata` | The engine |
| 108 | Lost at a column the engine gives no source: all but a handful in models that `UNNEST` or `PIVOT` | The engine |
| 70 | Lost, a name the walk never reached | Not read yet |
| 51 | Settled column by column: the warehouse contradicts the compile | Not read yet |
| 4 | A compile that does not parse | |

BigQuery is not a priority for now: these wait.

**Fivetran's Shopify package**, a fixture (0038): 240 models and 5548 edges,
all parsed. 97.2% of columns have an edge, 4996 of 5142, and every column
against the warehouse or the YAML, 4466. sqlglot reads 5547 of its edges the
same way.

**lineage-library** (0042), the lineage of an invented project written by hand:
the pass gives its 105 facts exactly, and collin marks none of them as a known
limit.

## Residuals already named

- **A column read beside an alias of the same name** is credited to the alias:
  in `country_code as country, country as country_name`, the SQL reads the
  table's `country`. One edge in Shopify.
- **An inferred column several parents could give** is settled by the compile,
  by every candidate, or by manifest order (0027). None here: one edge is
  inferred.
- **`* REPLACE (...)` and `* ILIKE '...'`** are not read: the star expands to
  every column. No model of the three projects above uses either.
- **A window key the syntax tree cannot bind**, in a named window, an
  unaliased derived table, or a bare name several sources have, is read as
  before 0041, against the source the engine hung the window off: 3 of the 29
  window and ordered aggregate expressions here, none of them wrong.
