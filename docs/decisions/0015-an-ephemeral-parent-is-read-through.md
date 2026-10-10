# 0015. An ephemeral parent is read through

Date: 2026-10-10 · Status: accepted

**Trigger:** read before changing which relations the engine is handed for a
model, or which parents inference matches against.

## Context

dbt never builds an ephemeral model. It inlines the model's SQL into each child
as a CTE named `__dbt__cte__<name>`, so the child's compiled SQL reads the
tables the ephemeral reads, under their own names. The ephemeral has no relation
of its own.

The pass fed the engine a model's parents with a relation, which skipped an
ephemeral parent and put nothing in its place. A child whose parents were all
ephemeral ran with no schema at all: a `select *` through the inlined CTE had
nothing to expand into, and a `QUALIFY` or a filter over the inlined tables had
no column to name. Inference walked the parents as dbt lists them, so a child
whose compile was set aside could only match names against the ephemeral, a node
its SQL never reads, and where the ephemeral itself had failed there was nothing
to match at all.

On a 3341 model project, 8 models are ephemeral. Five children of one of them
failed to parse, because the ephemeral's own macro compiled to invalid SQL, and
came out with no edge at all.

## Decision

A model reads what dbt says it reads, with each ephemeral parent replaced by
what that ephemeral reads, in the ephemeral's place in the list, through any
depth of ephemerals, each node once. The engine is handed those relations, and
inference matches against those nodes. The order stays the manifest's, because
inference keeps the first parent carrying a name and must not change its answer
because of how the list was walked.

This is the same list 0014 settles a contested relation on, and the one that
decides which relations count as declared.

## Rejected

- **Handing the engine the ephemeral under its CTE name.** The ephemeral has no
  relation to register, and the child's SQL defines the CTE itself: the engine
  already resolves the CTE, it only lacked the tables under it.
- **Inferring from the ephemeral.** The parsed edges of every sibling name the
  tables, since that is what the SQL reads, so an inferred edge from the
  ephemeral would draw a path the rest of the graph skips, and dbt-lens would
  show the ephemeral as the source of a column it never holds.

## Consequences

On the same project the 5 children that failed to parse get 25 edges, inferred,
from the tables their ephemeral reads, and go from unresolved to inferred. No
edge is removed or relabelled, and the fault list keeps its 443 models. Two
more self reads are recognised, labelled by 0012. The report gains 24 dedup
keys in 3 children, read from a `QUALIFY` over the inlined union now that the
engine knows its tables, 151 more `--indirect` edges with the 25 above.

An ephemeral model still gets the edges its own SQL gives it, 50 here, and no
edge leaves it.
