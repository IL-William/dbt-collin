# Next steps

Where the lineage stands, and what would raise it. Measured on 2026-10-10 on a
pinned copy of a 3507 model Snowflake project, its manifest and its catalog
generated on one target with that target's environment, the engine at the
fork's `collin-engine-2`. Not a decision: a list to come back to.

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

The report names every one, with where the walk back stopped. It does not say
what to do: that is the next thing worth adding.

## A target compiled without its environment

The same project was first measured on a compile for this target made without
its environment file, so every model and source pointed at another
environment's databases, and so did the catalog. There 94.8% of columns had an
edge, and the gaps read like the project's own: 59 compiles an introspecting
macro wrote as `select ,` or `select from`, because the parent it read did not
exist in that environment; 1763 columns of tables built by older code; raw
tables no source declares. None of it was there in the right environment.
collin cannot tell which environment a target was meant for, but a project
whose models of one target sit in databases named for another is a finding it
could raise.

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
collin compares them. 3506 of the 3507 models were compared; the other, an
audit table reading only itself, has no parsed edge.

| | Edges |
| --- | --- |
| Read by both | 133 155 |
| By collin alone | 30 |
| By sqlglot alone | 1364 |

The disagreements sit in 10 models, and each was read in the SQL.

- **collin alone and right, 27.** 17 sit under `UNION ALL BY NAME`, which
  sqlglot pairs by position: its own 134 edges in that model are those
  pairings, every one wrong. 7 sit under a top level select in parentheses the
  comparison did not read, and 3 pass through a recursive CTE, which sqlglot
  does not follow.
- **sqlglot alone and wrong, 1358.** The 134 pairings, and 1224 in two models
  that build an array of objects out of a hundred columns and `LATERAL FLATTEN`
  it: sqlglot credits each element's index to every one of those columns,
  where the index is a position. Both read the element's key from them.
- **collin wrong, 3.** A window's partition key written `cte.k`, credited to
  the other side of the join, which has a `k` too. 0031 reads a window's keys
  against the scope the edge comes from, and does not read the qualifier: it
  rejected binding one by reading the `FROM` clauses.
- **collin missing, 3.** Two window `ORDER BY` keys qualified with the other
  relation of a join, dropped for the same reason. One `ARRAY_AGG ... WITHIN
  GROUP (ORDER BY c)`, where collin does not count `c` as an input of the array
  its order shapes.

Six edges in 133 185, then, as far as the SQL shows. An independent reader
backs the roots too: following each through its CTEs, 3424 of the 3441 then
counted end at no table. Of the other 17, 15 reach an incremental model's own
table (0012), one is a literal the check misread, and one, a `CASE` on a
`FLATTEN`'s key, has had an edge since the engine's eighth patch (0028).

## On public projects

The project above is Snowflake and has its own shapes. Two public ones show
others.

**Cal-ITP**, the dbt artifacts Caltrans publishes for its transit warehouse:
624 BigQuery models, manifest and catalog from one run, pinned outside the tree.
dbt quotes a BigQuery relation in backticks, which `norm_relation` kept, so no
relation matched and 19 594 of 20 386 edges left the graph through `rel:`.
With that fixed, 92.5% of columns had an edge and 93.4% against the warehouse;
with the engine's seventh patch too, 92.8% and 93.6%. The 1162 without one that
were not roots before that patch, by first cause:

| Columns | Cause | Where it would be fixed |
| --- | --- | --- |
| 470 | A field of a STRUCT, `metadata.extract_ts`, read as a column of a table named `metadata` | The engine |
| 395 | The fields of a STRUCT, which the catalog lists as columns, `device.fo_device_type`: no select can write one. They also mark 22 of the 38 degraded compiles | collin: a field is not a column |
| 121 | A column of an `UNNEST` alias or of a `PIVOT`'s output, in 57 and 8 models | The engine |
| 56 | 29 models set aside for two CTEs of one name (0018): a macro wraps each source in its own `WITH ranked`. Read since the engine's seventh patch (0028), 500 inferred edges becoming 579 parsed ones | Done |
| 120 | Not read yet: 51 settled column by column, 42 inferred where no parent has the name, 23 in one audit log model, 4 in a compile that does not parse | |

**Fivetran's Shopify package**, a fixture (0038): 240 models, 4875 edges, 89.6%
of columns covered. sqlglot reads 4244 of its 4245 parsed edges the same way.
16 marts were set aside, their 630 edges inferred: ephemeral models are inlined
as CTEs, each with its own `WITH`, and two of them name a CTE alike (0018).
Since the engine's seventh patch (0028) they are read: 1303 parsed edges,
5548 in all, and sqlglot reads 5547 of them the same way.

## Residuals already named

- **A column read beside an alias of the same name** is credited to the alias:
  in `country_code as country, country as country_name`, the SQL reads the
  table's `country`. One edge in Shopify.
- **An inferred column several parents could give** is settled by the compile,
  by every candidate, or by manifest order (0027). None here: one edge is
  inferred.
- **`* REPLACE (...)` and `* ILIKE '...'`** are not read: the star expands to
  every column.
- **A qualified window key** is read against the scope its edge comes from, so
  `partition by b.k` in a join of `a` and `b` that both have `k` credits `a.k`
  when the windowed column comes from `a`, and an `order by b.x` is dropped: 3
  edges wrong and 2 missing here.
- **An `ARRAY_AGG ... WITHIN GROUP (ORDER BY c)`** does not count `c` as an
  input of the array its order shapes: 1 edge here.
