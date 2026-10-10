# 0004. Three column sources, kept apart

Date: 2026-10-10 · Status: accepted

**Trigger:** read before merging the column lists in `schema.rs`, or before
adding a fourth source.

## Context

Three things claim to know a model's columns, and they disagree.

- **declared**, the YAML: what the project says it produces. Intent.
- **warehouse**, `catalog.json`: what the object actually has. The only list
  that is not an opinion.
- **computed**: what this compile's SQL would produce.

The tempting move is to merge them into one best-guess list. It would work, and
it would destroy the two findings worth having.

## Decision

Keep all three, separately, in `Store`. Each disagreement is a different verdict:

- computed disagrees with warehouse, so **this compile is degraded** and its
  edges must not go out as `parsed`;
- warehouse disagrees with declared, so **the YAML is stale**, which is a
  documentation problem, not a lineage one. Measured: **262 of the 585** models
  the catalog covers, 44.8%.

For expansion of `select *` downstream, the warehouse wins, then computed, then
declared. Downstream SQL was compiled against the objects that actually exist, so
a star there sees the warehouse's columns, not whatever this compile would have
produced.

## Rejected

- **One merged list.** Hides both verdicts, and there is then no way to tell a
  broken compile from a documentation gap.
- **Trusting the YAML as the reference.** Stale on nearly half the models we can
  check. It is a witness, not a judge. 0008 sets the one verdict it can support
  alone.

## Consequences

Models with no catalog entry can only be checked weakly, and there are 2744 of
them here. That is a reason to build a catalog, and the report says so rather
than pretending the check happened.
