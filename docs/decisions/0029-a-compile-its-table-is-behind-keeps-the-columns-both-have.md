# 0029. A compile its table is behind keeps the columns both have

Date: 2026-10-10 · Status: accepted · Amends 0004 and 0008

**Trigger:** read before changing what a warehouse mismatch does to a compile,
or before emitting a column the warehouse does not have.

## Context

0004 and 0008 read any difference between a compile's columns and the
warehouse's as the compile failing to describe the object, and every edge of
such a model went out as a name match. But the catalog describes the dev
warehouse, and a dev table built before the latest commit disagrees with a
compile that is perfectly sound. On a 3341 model project, of the models whose
table and compile disagree, the audit found about 70 to 100 whose SQL simply
runs ahead of the table: 47 of the 49 base tables in that group were last built
before their model's last commit. Judged against an independent reading of the
SQL, their parsed edges are right 5322 times in 5342, against 99.0% for
compiles the warehouse confirms and 96.7% for those it cannot check. Inference
replaced them with name matches, and a column the code renamed or derived had
no edge at all.

## Decision

A degraded compile is settled column by column when the model has a warehouse
entry, the compile gives no reason of its own to doubt it, and the two share at
least one column. No reason of its own means it parsed, it said something, it
reads no name its CTEs lack, and no two of its scopes share a name.

- **A column both have** keeps its parsed edges and their roles. Both say the
  column exists, and the SQL is the one account of it.
- **A column only the table has** is inferred by name, as a compile set aside
  whole would have it (0003, 0027).
- **A column only the compile has** goes out on no rung: nothing says the object
  has it. The report lists it under `unexpected_columns`, as before, and counts
  it in `columns_withheld`.

The agreement stays `degraded`; the report's provenance for the model is
`per_column`. What the model reads to choose its rows is the same SQL's, so it
is reported, and under `--indirect` it fans out onto the columns both have. A
child's `select *` still sees the warehouse's list (0004).

## Rejected

- **Trusting the whole of such a compile.** It publishes columns the warehouse
  does not have, the one thing 0008 kept at zero, and where the difference is a
  subquery leaking into the projection, 69 of 73 of those edges were wrong.
- **Restoring a parent once its children read the columns it adds.** The
  narrowest fix, but a broken macro that parent and child share would vouch for
  itself, and every star below the parent would change.
- **A share of common columns above which the compile is kept.** Every figure
  in it would be arbitrary. One column in common is a categorical line, like
  0008's: a compile sharing nothing with its table describes no part of it.

## Consequences

On the same project, with the engine at 0028's third patch, 86 of the 163
degraded models are settled per column. Parsed edges go from 93 569 to 97 665
and inferred from 8282 to 5794: 1634 pairs added, 26 removed, and 2462 name
matches become edges with a role. Warehouse or YAML columns with an edge go
from 89 633 to 90 174 of 94 353, and 335 columns only the compile has are
withheld. Lost columns go from 178 to 193: the per column models' own. Under
`--indirect` the cache grows by 41 516 edges.

The guard is only as good as what the engine sees. Before the fork read inside
TRIM, one model went per column with 51 names its CTE lacks hidden in a hash;
the fork finds them and it is set aside whole. That is why this landed after
0028's third patch.

A table older than its code still decides what its children's stars see, so a
column the new code adds is a phantom in each child reading it through a star,
and the report names the parent's warehouse list as the cause.
