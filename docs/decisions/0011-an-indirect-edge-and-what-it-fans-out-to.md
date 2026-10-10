# 0011. An indirect edge, and why it is off by default

Date: 2026-10-10 · Status: accepted · Amends 0003 and 0005

**Trigger:** read before emitting an edge for a column that feeds no output
column, or before changing the default of `--indirect`.

## Context

A column can decide which rows a model contains without ever reaching one of its
values. `QUALIFY row_number() over (partition by order_hk order by load_dts)
= 1` is how 582 of 3929 compiled models here deduplicate, and the engine's
analyzer never visits `select.qualify` at all. Join keys and `WHERE` predicates
are the same shape: the engine hands us the ON clause text and the localised
filter predicates, and nothing read them.

Some of these columns appear nowhere else in their model. `lr.hashdiff`, joined
in from a `latest_records` CTE purely to be compared against, is real lineage
that no other producer records either.

The cache names a source column and a target column. Here there is no target.

## Decision

Three roles, `join_key`, `dedup_key` and `filter`, ranked at zero by 0009 so they
never compete with a role that describes a value.

**The report always carries them**, one entry per column read, in a section of
their own beside `relation_collisions`. Not in `models`, which is the list of
faults: a column read but never projected is lineage to be had, not a thing to
fix. 0006 gives `models` the property that makes it worth opening, that
everything in it wants doing something about, and that property is easy to spend
and hard to get back.

**The cache carries them only when asked**, behind `--indirect`. A column that
bears on which rows exist bears on every output column and on none in
particular, so the only shape the cache format has for it is one edge per output
column. That is also how OpenLineage models an `INDIRECT` transformation, so it
is not wrong. It is expensive: on the frozen corpus, 3301 facts became 122 204
edges and the cache went from 17.9 MB to 39.2 MB, with 5361 edges on
the largest model alone, which has 37 such columns and 145 outputs.

The flag buys seeing them in dbt-lens. It does not buy knowing them.

## Rejected

- **On by default.** 40 edges written per fact is a poor way to hold a fact, and
  a column graph with 5361 extra lines on one model is harder to read, not
  easier. Measured first, defaulted second.
- **Targeting the model's own column of the same name.** A dedup key that is
  projected already has a direct edge, and one that is not projected is the
  interesting case and has no column to point at. It would emit almost nothing,
  and what it emitted would duplicate a stronger claim.
- **A sentinel `to_col`, empty or `*`.** dbt-lens would draw a column that does
  not exist. 0008 exists because an invented column is worse than a missing one.
- **Listing them on the model in `models`**, which is what this record said
  first, before the file was read. 1247 models read such a column and 492 have a
  fault, so the fault list came out three quarters noise and being absent from it
  no longer meant clean. The information was right and the place was wrong.
- **Bumping the cache version to hold a relation level edge.** 0005 is what lets
  a released dbt-lens read what this writes. A flag costs nothing and breaks
  nothing.

## Consequences

The default cache is unchanged in size and shape. `column_lineage.report.json`
grew from 0.6 MB to 2.0 MB, and `models` stayed a fault list, going from 1198
entries to 492 as the false `UNRESOLVED_REFERENCE` of the same pass cleared.

An indirect edge duplicates no direct one: where a pair already has a direct
edge, the direct edge stands, because it says more.
