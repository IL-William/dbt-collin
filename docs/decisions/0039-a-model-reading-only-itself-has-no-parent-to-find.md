# 0039. A model reading only itself has no parent to find

Date: 2026-10-10 · Status: accepted · Amends 0012

**Trigger:** read before changing what counts as a lost column, what the
independent coverage leaves out, or how a model reading its own relation is
reported.

## Context

0012 keeps a model's own relation away from the engine and sets aside the
issue that leaves: a self read that only filters is no fault. It keeps a model
listed when an output is fed by its own relation and nothing else, because
that column is carried over from the model's earlier rows, and the issue was
then the only sign of it.

Some models read nothing but themselves. On a 3507 model Snowflake project,
compiled on its own environment, three do. Two audit tables are written by
jobs outside dbt: dbt creates each once, and compiles every later run as a
read of the table itself, one of them `select * from` the table `where 1 = 0`.
The third is a date spine adding hours after the last one it holds. Their 21
columns had no edge, which is right, but the report called them lost,
unresolved, or a degraded compile with columns the compile lacks, and the
independent coverage counted them as misses. Each label sends a reader looking
for a parent that does not exist in dbt.

## Decision

- **A model whose SQL reads no relation but its own** is named for it:
  `reads_only_itself` on its entry, which keeps it listed even when nothing
  else would.
- **Its columns with no edge are counted apart**, in `columns_self`, beside
  `columns_root`, and left out of the independent denominator as the roots
  are. None is listed as lost.
- **Its own relation stays away from the engine**, as 0012 has it, and no edge
  is drawn: the cache does not move.

The relation must be the model's own as dbt names it. One spelled otherwise,
in two parts where dbt wrote three, is another relation, and the model is read
as before, which is the safe direction.

## Rejected

- **Roots.** A root is built from literals in the SQL (0016). A carried over
  column is built from data, only not from dbt's: counting it apart keeps both
  figures true.
- **Leaving such a model out of the report.** Nothing else would name its
  columns, and a column with no lineage is named (0017).
- **Telling an audit table from an incremental model.** Both have no parent in
  dbt; which job writes the table is outside what the SQL shows.

## Consequences

On that project the cache is identical, default and `--indirect`. 21 columns
move to `columns_self`; columns lost in trusted models go from 11 to 8, and the
independent denominator loses those 21. The project could declare the two
audit tables as sources, which is what they are to dbt.
