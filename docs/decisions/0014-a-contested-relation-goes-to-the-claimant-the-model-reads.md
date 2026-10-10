# 0014. A contested relation goes to the claimant the model reads

Date: 2026-10-10 · Status: accepted

**Trigger:** read before changing how an edge's source node is found from the
relation the engine names, or before touching `by_relation` and the
`relation_collisions` it reports.

## Context

A project can declare one warehouse object twice. Here four source tables are
each declared by two source nodes whose names differ only in case, so two dbt
nodes claim one relation. The relation index has to hold one of them, and since
a0b58e4 (merged in 570c9ba) it holds the lowest unique_id, which made the
cache reproducible where a hash ordered choice had not. No record was written
for that call; this one supersedes the part of it that decided every edge.

The index answers for the project, and an edge belongs to a model. Each of these
models depends on one claimant, the one its `source()` call named, and the upper
case node sorts first, so every edge the four models drew from those tables
named the other one: a node the model does not depend on, and that dbt-lens
then shows feeding a model with no dependency between them. The name reached
the engine as the SQL spells it, but `norm_relation` folds the case away before
lineage sees it, so the SQL cannot tell the two apart either. The dependency
can.

On a 3341 model project, 158 published edges came from a node outside their
model's `parent_map`. 122 of them were these, all `passthrough`, in 4 models
(52, 33, 21 and 16 edges). The other 36 come through ephemeral models, whose
tables dbt inlines into the child: their node is a parent's parent, and right.

## Decision

A model's reads are settled on what it depends on first. The pass collects the
nodes dbt says the model reads, its parents with an ephemeral parent replaced by
that parent's own, and maps each relation to its node. An edge whose relation
is in that map names that node. A relation two of those nodes claim maps to
nothing, and so does one the model does not depend on: both fall back to the
project's index, the lowest unique_id, which is still the only choice that
depends on nothing but the project.

The same map serves the direct edges and the `--indirect` fan-out. Inference
was never affected: it names the parent it matched, by construction.

A self read is now dropped on the relation as well as on the node. When a
model's own relation is also claimed by a node with a lower unique_id, the
index names that node, and the self read would have gone out as an edge from
it.

## Rejected

- **The lowest unique_id everywhere, as before.** Deterministic, which was
  a0b58e4's point, and wrong for every model built from the other claimant.
- **Keeping the case the SQL wrote.** `norm_relation` exists so that a quoted and
  an unquoted spelling of one object meet, and the two claimants differ in
  exactly the way it erases. Undoing it for this would split relations that are
  one everywhere else.
- **Emitting the edge from both claimants.** Two edges for one read, and one of
  them from a node the model never read.
- **Reporting the collision and leaving the edges.** The report has named the
  four relations since a0b58e4, and the edges stayed on the wrong node.

## Consequences

On the same project the default cache keeps 100 719 edges: 122 change their
`from` to the claimant their model depends on, in 4 models, and nothing else
moves. The `--indirect` cache moves by the same 122 and nothing else. Edges from
a node outside `parent_map` go from 158 to 36, the ephemeral ones.

A model that depends on both claimants, or on neither, still gets the first by
name. That is a choice without a witness, and the collision in the report is
still the place that says so.
