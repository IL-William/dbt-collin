# 0006. The gaps are a deliverable, not a log line

Date: 2026-10-10 · Status: accepted

**Trigger:** read before skipping a model quietly, or before writing a warning to
stderr and moving on.

## Context

A generator that covers 94% of columns has to say something about the other 6%.
Written to stderr, that something scrolls past once and is gone. The user's own
framing settled it: *if a column cannot be found because it is not in the
project, then show the gap so the user can fix it by editing the YAML.*

A gap here is usually actionable. The relation a failing macro interrogates is
itself a dbt node whose columns we know; 15 of the 22 unreadable models had every
parent already resolved.

## Decision

`column_lineage.report.json`, written beside the cache, every run. Per model:
the provenance rung, the agreement verdict, the engine's issue codes verbatim,
unknown relations, missing and unexpected columns, whether the YAML is stale, and
the edge count. Plus totals.

Only models with something to say are listed, so the file stays readable. A model
that is clean is absent, and that absence means clean rather than unexamined,
which is exactly why a model that resolved to nothing must not be allowed to look
clean. See 0008.

## Rejected

- **Warnings on stderr.** Unreadable at 3341 models, and gone after the run.
- **Failing the run on a gap.** The gaps are the normal state of a real project.
  A generator that refuses to produce output until the YAML is perfect produces
  nothing, forever.

## Consequences

The report is the artifact to iterate against, and the totals line is the
regression test for accuracy work. It is also the thing to hand a colleague when
the fix is theirs: a stale YAML, an unbuilt dev target, a broken macro.
