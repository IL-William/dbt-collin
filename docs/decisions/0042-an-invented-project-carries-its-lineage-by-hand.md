# 0042. An invented project carries its lineage by hand

Date: 2026-10-11 · Status: accepted · Amends 0038

**Trigger:** read before changing what the lineage-library test compares,
marking a line in `fixtures/lineage-library.known.txt`, moving the submodule,
or deciding what a regression test of the whole pass compares against.

## Context

Two kinds of test guard the pass. The unit tests in each module hand it a
statement or a project a test wrote, and state what comes out, one shape at a
time. The public fixtures (0038) run the whole pass on what dbt wrote, and
compare with the caches collin gave: they show that something moved, not that
it was right, and a change meant to move them is accepted by regenerating
them.

Neither states, for a project, what the SQL says. On a 3507 model project
read against sqlglot and by hand, the last disagreements were window keys the
engine leaves unread (0041): no unit test had the shape, and the public
fixtures had recorded collin's reading as it was.

## Decision

- **Invented projects whose lineage is written by hand** live in their own
  repository, [lineage-library](https://github.com/IL-William/lineage-library),
  under Apache-2.0. The first, a lending library, is twelve models over five
  sources in Snowflake's SQL, each written for one or two shapes, its domain
  and names nobody's. Its lineage was written from the SQL before the pass ran
  on it: each edge with its kind, each column built from no column, each
  column read to decide which rows exist.
- **It is a submodule here**, `fixtures/lineage-library`, pinned to a commit.
  A change to the truth is a pull request there, apart from any change to
  collin, and moving the pin is a pull request here. The test fails, rather
  than skips, when the submodule was not fetched.
- **collin's limits stay here**, in `fixtures/lineage-library.known.txt`: a line
  collin is known to miss is marked `missing:`, one it gives wrongly `extra:`,
  with the limit named above it. A mark that stops holding fails the test, so
  a fix shows. The truth says nothing of any tool.
- **The graph is assembled by the test**, from `project.json` and the SQL
  files, rather than shipped as a manifest: the SQL stays readable, and what
  dbt writes is the public fixtures' to cover.

## Rejected

- **Blessing it like the public fixtures.** It would record collin's reading
  again, which is what the public fixtures already do.
- **Keeping it in this tree**, as it was first written. One pull request could
  then change collin and the lineage it is judged by together, and no other
  reader, the engine's fork or another tool, could use it.
- **Building it with dbt and DuckDB, as Jaffle Shop is.** DuckDB has no
  `LATERAL FLATTEN`, and Snowflake's shapes are the ones collin is used on. A
  build would only add what the public fixtures already cover.
- **Taking models from a real project and renaming them.** Renaming does not
  anonymise a design.

## Consequences

Written first, the library and the pass agreed on every line but one, and on
that one the library was right: the engine pairs the branches of a `UNION` by
name, where SQL pairs them by position, so a second branch writing another
name loses that column, and at the top level adds one. It is marked
`missing:`. On the 3507 model project nothing shows it: no union there names
a column differently in two branches. On the code before 0041 the test fails
on the three window and `WITHIN GROUP` keys 0041 reads.

A clone needs `--recurse-submodules`, or `git submodule update --init` after,
and the workflows that run the tests check the submodule out.
