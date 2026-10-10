# 0001. Column lineage comes from the compiled SQL, never from a warehouse

Date: 2026-10-10 · Status: accepted

**Trigger:** read before adding a database driver, a credential, or a dependency
on a dbt Platform account.

## Context

Snowflake's own column lineage, `SNOWFLAKE.CORE.GET_LINEAGE`, costs one call per
column, needs a machine that can reach the warehouse with SSO, and never says
*how* a column travels. Knowing that `orders.customer_id` feeds
`fct_orders.customer_id` is close to useless if you cannot tell a passthrough
from a join key from the branch of a CASE.

dbt already writes everything needed to do better: the compiled SQL, with the
Jinja gone and every `ref()` resolved to `db.schema.table`, plus the declared
columns and, when a catalog has been built, the warehouse's own column lists.

## Decision

Read the project, compute the lineage from the compiled SQL, and touch nothing
else. No warehouse connection, no Jinja evaluation, no dbt invocation. The only
inputs are `manifest.json`, `catalog.json` when present, and the compiled SQL the
manifest already carries.

## Rejected

- **dbt Fusion's `--write-lineage`.** It exists, it is free, and it is locked
  twice: it demands `--static-analysis strict`, which demands `dbt login` against
  a dbt Platform account, *and* a live warehouse connection. Measured on the
  corpus: Fusion wrote `target/index/column_lineage/v1_0.parquet` at **861
  bytes**, a parquet header with no rows, while `target/index/edges`, which does
  not need strict mode, came to 337 kB. The lock is measured, not assumed.
  Fusion stays useful as an oracle the day it can be run somewhere with both.
- **`SNOWFLAKE.CORE.GET_LINEAGE`.** One call per column, so a full project is
  hundreds of thousands of calls, and it still will not name the role.
- **Parsing `raw_code` and evaluating the Jinja ourselves.** 41.7% of the models
  in the corpus use `for` or `set`. Reimplementing dbt's Jinja to read SQL dbt
  has already compiled is work with no upside.

## Consequences

The lineage is only as fresh as the last `dbt compile`, and a compile made
against a target where the upstream was never built produces degraded SQL. That
is not a flaw to hide but a fact to report, which is what 0006 and 0008 are for.
