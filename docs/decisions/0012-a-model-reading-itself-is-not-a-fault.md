# 0012. A model reading itself is not a fault

Date: 2026-10-10 · Status: accepted · Amends 0006

**Trigger:** read before changing what keeps a model out of `models`, before
handing the engine a model's own relation, or before setting aside another issue
the engine raises.

## Context

dbt compiles an incremental model's filter as a read of the model itself: the
newest timestamp already loaded, the keys already there. A model is never its
own parent, so the engine is never handed that relation, and it says so with
`UNRESOLVED_REFERENCE`, one of the codes that mean it did not see all of the SQL.
That set `approximate`, and `approximate` put the model in `models`.

Nothing there wanted doing. The self edge was already dropped from the cache,
because a model never feeds its own column. On a 3341 model project, 135 of the
141 `UNRESOLVED_REFERENCE` issues about a table named the model's own relation,
all 135 in incremental models, one per model. Read at the SQL, the self read sits
in a CTE body in 105 of them, in an anti-join in 24 and in a `WHERE` subquery in
6, and in every one it decides which rows are added and reaches no value. 62
models were listed for that issue and nothing else: an eighth of a list 0006 says
is worth opening because everything in it wants doing something about.

## Decision

`engine.rs` gives two facts per issue and decides nothing: whether its code is
one of those that degrade, and, for an `UNRESOLVED_REFERENCE` about a table, the
relation it names. The relation is read from the engine's message, or from its
span should the wording change, since the engine places the span by searching the
SQL for that same name.

`lineage.rs` sets aside that one issue about that one relation. A degrading issue
naming the model's own relation does not count against it, unless some output
column is fed by that relation and by nothing else. The read is then no filter:
it is a column carried over from the model's earlier rows, whose one edge the
cache drops, or a column the engine made out of a filter subquery, and with no
catalog to contradict the compile the issue is the only sign of either. Every
other code stays a fault, `APPROXIMATE_LINEAGE` for a star over the model itself
included.

The report keeps what was set aside. `totals.self_reads` counts the models whose
own relation the engine could not find, because for a model this takes out of
`models` the count is the only place the fact survives, as `columns_root` keeps
the roots. A model listed for another reason carries the issue marked
`self_reference`. `Resolved::approximate` keeps its meaning, the engine's own
admission, and the dump still shows it.

## Rejected

- **Handing the engine the model's own relation**, from the catalog. Measured on
  the same project, it changes no edge, and it costs. The dev table can be older
  than the compile, which added 4 `UNKNOWN_COLUMN`. Two models with no known
  parent, where the engine stays permissive until it knows one table, gained 7
  warnings about other tables. 354 self reads would have entered
  `read_not_projected`, and a model reading nothing but itself changed its trust
  verdict. Without a catalog the only list left is the one this compile is
  computing, which is circular. With one, the warehouse would shape the computed
  list that `Agreement` then checks against the warehouse, which 0004 exists to
  prevent.
- **Dropping the issue.** It hides what the engine said.
- **Setting the issue aside with no guard.** A model with no catalog entry whose
  self read made up an output column would look clean, which is exactly what
  0006 says absence from `models` must not mean.

## Consequences

On the same project `models` goes from 505 entries to 443. The 62 that left are
all incremental, all confirmed against the warehouse, and each was listed for
this issue alone. `totals.self_reads` is 135, and the 73 of those models still
listed carry the issue marked. The cache is identical, with and without
`--indirect`, and so is every other total. The guard holds on 16 models, all of
them listed anyway because their compile was set aside, so it costs none of the
62.

The model's own relation spelled otherwise than dbt wrote it, in two parts where
dbt wrote three, does not match and stays a fault, which is the safe direction.

Three of the 62 have an output no edge reaches, 4 columns in all, each a literal
set in a CTE or a union branch that the root test does not recognise. The self
read was the only thing listing those models, and it never said why, so this
hides nothing the report stated. Naming a lost output is a gap of the report's
own, apart from this one.
