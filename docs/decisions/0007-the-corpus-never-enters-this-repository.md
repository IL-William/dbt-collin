# 0007. The corpus never enters this repository

Date: 2026-10-10 · Status: accepted

**Trigger:** read before writing a fixture, a test, an example or a commit
message.

## Context

This crate is developed against a real dbt project: 3341 models, 2422 sources,
a 109 MB manifest, client data in every table and column name. That corpus is
what makes the measurements real, and it is exactly what must not be committed.
The repository being private changes nothing: private is an access setting, not
a licence to hold someone else's data.

dbt-lens reached the same conclusion for a different reason, being public, and
landed on the same rule (its 0014). Two reasons, one rule, so the rule is worth
stating rather than inheriting by habit.

## Decision

No real project SQL, column names, model names or manifest fragments in this
tree, in any form: fixtures, tests, examples, commit messages, documentation.
Every fixture is invented. `/corpus` is in `.gitignore`.

Measurements are quoted as numbers, and the numbers are reproducible because the
corpus is pinned outside the tree with a fingerprint: file sizes, sha256 of the
manifest and catalog, the engine version, the branch and the compile time.

## Rejected

- **A trimmed real manifest as a test fixture.** Convenient, and it would carry
  the very shapes that break, but trimming is not anonymising and it is a rule
  that erodes with every exception.
- **Measuring against the live project directory.** It moves under us: a
  `dbt compile` rewrites `target/` and every number becomes unreproducible. The
  snapshot exists for that reason.

## Consequences

The hard shapes, nested stars, UNION, windows, ephemeral models, self joins,
have to be written by hand as invented SQL. That is slower, and it has an upside:
an invented fixture states the behaviour it is testing, which a captured one
never does.
