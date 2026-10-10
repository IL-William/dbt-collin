# 0038. A public example project is a fixture

Date: 2026-10-10 · Status: accepted

**Trigger:** read before adding a fixture collin did not invent, changing
`crates/collin-core/fixtures/jaffle_shop`, or accepting a change to its
expected caches.

## Context

Every test hands the pass a manifest, a catalog or a project that a test wrote.
None reads what dbt wrote, so a dbt release that reshapes the manifest, or a
change that reads a field of it differently, can pass them all. What shows that
nothing moved is the before and after on a real project in AGENTS.md, and it is
run by hand, when someone thinks to.

0007 keeps the corpus out with "every fixture is invented". Its reason is the
corpus's: client data, which trimming does not anonymise. A public example
project carries none, and the rule as written kept it out too.

## Decision

- **dbt Labs' `jaffle_shop_duckdb` is a fixture**, in
  `crates/collin-core/fixtures/jaffle_shop`: its models, seeds,
  `dbt_project.yml` and `profiles.yml`, copied unchanged from the commit its
  README names, with its Apache-2.0 `LICENSE` beside them.
- **Its `target/` is what a pinned dbt wrote**: `manifest.json` and
  `catalog.json` from dbt-core 1.12.5 with dbt-duckdb 1.11.0, by
  `regenerate.sh`, committed. DuckDB runs in process, so no warehouse is
  needed.
- **The caches collin gives for it are committed**, with and without
  `--indirect`, less the two fields a run of the same code changes. A test
  writes them again and fails on any difference, naming the edges gone and the
  edges new. A change meant to move them is accepted with `COLLIN_BLESS=1`, and
  the diff of `expected/` is then part of the pull request.
- **The cache is compared, not the report.** The cache is what dbt-lens reads,
  and its format does not move (0005). The report gains a total with a feature,
  and comparing it would fail each such pull request over nothing a reader of
  the cache would see.
- **A fixture collin did not invent is public, under a license that allows
  copying it here, and carries nothing from a client.** 0007 stands for
  everything else.

## Rejected

- **dbt in CI**, compiling the project on every run. A dbt release would move
  the test with no change here, and the job would need Python and PyPI. dbt
  moves when `regenerate.sh` is run, in a pull request of its own.
- **dbt Labs' newer `jaffle-shop`.** It has no license, so it cannot be copied
  into a public repository.
- **A larger invented project.** Written by hand, it is a test reading what a
  test wrote, which is the gap this fills.
- **Comparing coverage instead of edges.** Coverage is not lineage (0033): an
  edge can move and the figure stay.

## Consequences

Five models, all confirmed by the catalog: renames, a transform, aggregates, a
`CASE` inside an aggregate, join keys through CTEs. 31 edges, and 43 more with
`--indirect`, each read against the SQL when they were committed.

It is DuckDB, not Snowflake, and it has none of the shapes where collin is known
to be wrong. It catches a change that breaks the ordinary, not one that moves a
hard case: the before and after on a real project stays the check for those.

The fixture's files stay under Apache-2.0. They are not in the release
archives, which carry what 0036 lists.
