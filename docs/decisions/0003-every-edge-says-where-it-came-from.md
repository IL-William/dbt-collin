# 0003. Every edge says where it came from

Date: 2026-10-10 · Status: accepted

**Trigger:** read before emitting an edge, or before filling a gap with anything
the SQL did not say.

## Context

Some models cannot be read. In the corpus, 22 fail to parse outright, all of them
because a macro calling `adapter.get_columns_in_relation` returned nothing and
left a `select` with no column list or a leading comma. Others parse and resolve
to nothing useful.

There are two bad answers. Emitting nothing throws away the fact that we often
know the model's output columns and its parents' columns, and name matching
recovers **98.3%** of them. Emitting a guess that looks exactly like a reading is
worse: lineage whose provenance you cannot tell is worse than no lineage, because
you cannot know which parts to trust.

## Decision

A ladder, and the rung is written into the edge's `kind`.

1. **`parsed`.** Read out of the SQL, and it carries its role: `passthrough`,
   `rename`, `cast`, `aggregate`, `window` or `transform`.
2. **`inferred`.** The compile was degraded, so the edge comes from matching the
   model's known output columns against its parents' known columns. The role is
   **not** claimed, because it was not observed.
3. **unresolved.** Nothing could be said. The report names what was missing.

The rungs never mix inside one edge, and never mix silently.

## Rejected

- **One `kind` for everything, with a confidence score.** A number invites
  averaging. A name forces a reader to decide.
- **Emitting inferred edges with a guessed role.** Name matching proves a link
  exists. It says nothing about whether the value was transformed on the way, and
  claiming otherwise would poison the one thing this project is for.

## Consequences

`engine.rs` never guesses; it reports what the engine found and what the engine
admitted it could not find. Deciding what to do about a gap belongs to
`lineage.rs`, which has to label the decision. The UI can colour the rungs
differently, and does.
