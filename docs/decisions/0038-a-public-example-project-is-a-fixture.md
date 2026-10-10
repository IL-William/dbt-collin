# 0038. A public example project is a fixture

Date: 2026-10-10 · Status: accepted

**Trigger:** read before adding a fixture collin did not invent, changing one
under `crates/collin-core/fixtures`, or accepting a change to their expected
caches.

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

- **Two public projects are fixtures**, in `crates/collin-core/fixtures`:
  - **dbt Labs' `jaffle_shop_duckdb`**, five models. Its models, seeds,
    `dbt_project.yml` and `profiles.yml` are copied unchanged from the commit
    its README names, with its Apache-2.0 `LICENSE` beside them.
  - **Fivetran's `dbt_shopify`**, SQL written for production: its integration
    tests, seeded and run on DuckDB as Fivetran's own CI does. Only the
    artifacts are committed, not the package: its `regenerate.sh` fetches the
    tag it names, checks the commit, and installs the dependencies its
    `package-lock.yml` pins. Its Apache-2.0 `LICENSE` is beside them.
- **Each `target/` is what a pinned dbt wrote**: `manifest.json` and
  `catalog.json` from dbt-core 1.12.5 with dbt-duckdb 1.11.0, by the fixture's
  `regenerate.sh`, committed. DuckDB runs in process, so no warehouse is
  needed.
- **The caches collin gives are committed**, with and without `--indirect`,
  less the two fields a run of the same code changes, one edge a line. A test
  writes them again and fails on any difference, naming the edges gone and the
  edges new. A change meant to move them is accepted with `COLLIN_BLESS=1`, and
  the diff of `expected/` is then part of the pull request.
- **They record what collin gives, not what is right.** An edge collin is known
  to miss or to get wrong is committed as it comes out, and its fixture's
  README names it. A fix moves it, and the diff shows the fix.
- **The cache is compared, not the report.** The cache is what dbt-lens reads,
  and its format does not move (0005). The report gains a total with a feature,
  and comparing it would fail each such pull request over nothing a reader of
  the cache would see.
- **A fixture collin did not invent is public, under a license that allows
  copying it here, and carries nothing from a client.** 0007 stands for
  everything else.

## Rejected

- **dbt in CI**, compiling the projects on every run. A dbt release would move
  the test with no change here, and the job would need Python, PyPI and dbt's
  hub. dbt moves when `regenerate.sh` is run, in a pull request of its own.
- **dbt Labs' newer `jaffle-shop`.** It has no license, so it cannot be copied
  into a public repository.
- **The whole Shopify package in the tree.** Its 264 SQL files are in the
  manifest already, raw and compiled, and the tag and commit say where they
  came from.
- **Expected caches indented.** At seven lines an edge, Shopify's would weigh
  several times what they do, and a moved edge would show as a hunk of fields
  rather than as one line.
- **A larger invented project.** Written by hand, it is a test reading what a
  test wrote, which is the gap this fills.
- **Comparing coverage instead of edges.** Coverage is not lineage (0033): an
  edge can move and the figure stay.

## Consequences

**Jaffle Shop:** five models, all confirmed by the catalog: renames, a
transform, aggregates, a `CASE` inside an aggregate, join keys through CTEs. 31
edges, and 43 more with `--indirect`, each read against the SQL when they were
committed.

**Shopify:** 240 models, 211 of them tables or views confirmed by the catalog
and 29 ephemeral. 4875 edges, 14 559 with `--indirect`. Compared with sqlglot's
reading of the same SQL, 4244 of the 4245 parsed edges agree. The expected
caches hold the 630 inferred edges of 16 marts that 0018 sets aside, and the one
edge sqlglot disputes, which its README names. Its staging models are written
by a macro from what each source turns out to have: the same introspection as
a real project's, which is why its `profiles.yml` puts the seeds in the schema
the sources read.

Neither is Snowflake, and neither has the shapes the corpus has where collin is
known to be wrong. They catch a change that breaks the ordinary, and Shopify a
change to how ephemeral models and Fivetran's macros are read. The before and
after on a real project stays the check for the rest.

Shopify adds about 10 MB to the tree, 0.7 MB once compressed. The fixtures'
files stay under Apache-2.0. They are not in the release archives, which carry
what 0036 lists.
