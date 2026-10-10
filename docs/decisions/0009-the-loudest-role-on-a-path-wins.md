# 0009. The loudest role on a path wins, not the last

Date: 2026-10-10 · Status: accepted · Amends 0003

**Trigger:** read before changing what `RawEdge::expression` carries, or before
adding a role to `role.rs`.

## Context

An edge is named by one expression, but a column rarely passes through only one.
A dbt model is a chain of CTEs: one computes `sum(amount) as total`, a later one
selects `total * 1.2 as gross`, and the walk from `gross` back to
`orders.amount` meets both.

The walk kept whichever expression it was holding, which is the outermost. So
`gross` was reported `transform`. That is true and useless: `transform` is what
every expression the classifier cannot place already says, and the aggregate is
the fact a reader opened the graph for.

Measured on the frozen corpus before this: 77 `aggregate` edges and 27 `window`
edges out of 100 202, on a project where 121 models group and 638 use a window.

## Decision

When two expressions meet on one path, carry the one that claims more.
`role::rank` orders them: window, aggregate, transform, cast, rename,
passthrough. The indirect roles of 0011 sit at zero and never compete, because
they describe a different relationship rather than a stronger one.

The ordering is not the ramp dbt-lens paints with, which runs quiet to loud for
the eye and puts `transform` at the top. This one is about how much is known. An
aggregate is a specific claim; a transform is the absence of one.

## Rejected

- **Carrying every expression and emitting several edges.** One edge per column
  pair is what the cache format holds, and a reader wants one answer for
  "what happened to this column", not a list to reconcile.
- **Keeping the outermost and mentioning the rest in the report.** The role is
  the product here. Moving the only interesting half of it into a second file
  means the graph still says `transform` everywhere it matters.
- **Ranking by how far down the path the expression sat.** Depth is an artefact
  of how the model was written, not of what happened to the value.

## Consequences

On the frozen corpus, together with the two recoveries of the same pass:
`aggregate` 77 to 160, `window` 27 to 80, `cast` 1595 to 1588, `transform` 9825
to 9901.

A column can now be labelled `aggregate` when the expression physically nearest
its output is arithmetic. That reads oddly next to the SQL and it is the more
useful answer: the value was collapsed across rows somewhere, and that is what
the label is for.
