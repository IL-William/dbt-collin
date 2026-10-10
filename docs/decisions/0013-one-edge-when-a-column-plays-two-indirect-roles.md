# 0013. One edge when a column plays two indirect roles

Date: 2026-10-10 · Status: accepted · Amends 0009 and 0011

**Trigger:** read before changing how `--indirect` fans a read out to the output
columns, before adding an indirect role, or before deduplicating edges anywhere.

## Context

0009 put the three indirect roles at zero because they never compete with a role
that describes a value, and on a path that holds. It said nothing of two
indirect roles meeting on one column pair, and they do. A point in time model
writes `where valid_from <= current_date` and then `qualify valid_from =
max(valid_from) over ()`, so one column is both a filter and a dedup key. The
engine reports each reason as a read of its own, which is right: the report
lists reads. The fan-out of 0011 then drew one edge per read per output column,
so every such pair was written twice, once per role. That is the option 0009
rejected, carrying every expression and emitting several edges.

Nobody saw it in dbt-lens, which keys an edge by its four names and keeps the
first one written. The role it showed was whichever came first in the engine's
sort of reads by role name, and `dedup_key` sorts before `filter` and
`join_key`. The choice was made, only by the alphabet.

On a 3341 model project, 243 columns in 196 models were read for two reasons,
242 as a dedup key and a filter and 1 as a dedup key and a join key. That wrote
10 424 column pairs twice, 20 848 of the 225 151 edges in the `--indirect`
cache: 10 362 pairs as `dedup_key` and `filter`, 62 as `dedup_key` and
`join_key`.

Not every one of those dedup keys is in the SQL. `QUALIFY` is read from the
text, and a clause's keys go to every parent with a column of that name,
whatever `SELECT` the clause belongs to. Checked against the SQL, 238 of the 243
columns do play both roles. The other 5 take their dedup key from a `QUALIFY`
over another relation: the one column read as a dedup key and a join key, which
its model uses only in the `ON` of an aggregating CTE, and 4 of those read as a
dedup key and a filter. They make 271 of the pairs, 62 and 209.

## Decision

One edge per column pair, as 0009 has it. When a column plays more than one
indirect role, the edge carries the one `role::indirect_precedence` puts first:
`dedup_key`, then `join_key`, then `filter`.

- `filter` is the floor. It says only that the column was tested, and a join key
  or a dedup key implies as much.
- `dedup_key` tops `join_key`. `QUALIFY` runs last, on rows already joined and
  filtered, and it fixes the grain of the model, one row per key, which is the
  most specific thing to say about which rows exist.

The choice is made in `lineage.rs`, in the fan-out, where the cache gets its
edges. `engine.rs` still reports one read per role and chooses nothing, and the
report still lists every read: `read_not_projected` is about reads, not edges,
so the role an edge does not carry stays on record there.

The order is kept apart from `rank`, which stays at zero for the three, so that
an indirect role can never be compared with a value role. A direct edge standing
over an indirect one stays structural, the guard in the fan-out, not a matter of
rank.

## Rejected

- **Keeping both edges.** 0009's rejection holds, and dbt-lens throws the second
  away on load, so the extra edges bought nothing a reader could see.
- **`join_key` over `dedup_key`.** Defensible, since a join key decides which
  rows meet at all, and today it would even label the 62 pairs right. But the
  one column read both ways is a join key whose dedup key is the broadcast
  above, which goes once each `QUALIFY` is scoped to its own `SELECT`, so the
  project cannot choose between the two orders. When both roles are real, the
  grain is the stronger statement.
- **A compound kind, `dedup_key+filter`.** A list for the reader to reconcile,
  and kinds that multiply for dbt-lens to colour, for a fact the report already
  holds whole.
- **Giving the three non-zero ranks.** They would then compare with `rename`,
  `cast` and `transform`, which 0009 says they must never do.
- **Collapsing in `engine.rs`.** It drops a true role from the report, and it is
  a choice the engine must not make.
- **Deduplicating in `cache.rs`.** It would also hide a direct edge written
  twice, which would be a bug, and it decides far from where the reason is
  known.

## Consequences

On the same project the `--indirect` cache goes from 225 151 edges to 214 727,
`totals.edges_indirect` from 124 432 to 114 008, and the file from 41.6 MB to
39.6 MB. No pair is added or removed: 10 424 pairs lose their second edge, and
what is left is exactly the old cache with each pair's first edge kept, so
dbt-lens shows the same role on every pair as before. The default cache is
identical, and so is the report but for `edges_indirect`: `read_not_projected`
keeps its 3327 reads.

Keeping every role as it was keeps the wrong ones too. The 271 pairs whose
dedup key is broadcast keep `dedup_key`, 62 where the SQL says `join_key` and
209 where it says `filter`, until each `QUALIFY` is scoped to its own `SELECT`.
What is wrong there is the read, not the order, and the report carries the same
read.

A column that is both a join key and a filter, which none is on this project
today, now reads `join_key` where the alphabet gave `filter`. More indirect
reads will make such pairs, and the order is what decides them.
