# 0024. A row deciding read says whether it is projected

Date: 2026-10-10 · Status: accepted · Amends 0011

**Trigger:** read before renaming a report section, or before counting the
columns a model reads to decide which rows exist.

## Context

0011 put the columns a model reads to decide which rows exist, join keys,
filters and dedup keys, in a report section of their own, `read_not_projected`,
and promised in its name, its docs and the README that none of them reached an
output. The engine hands those reads over whether or not the model also carries
the column through. On a 3341 model project, three quarters of the entries were
columns the model projects as well, so the section's name described a quarter
of it, and its total counted what the name excluded.

## Decision

The section is `row_deciding_reads`, and each entry says `projected`: whether
the model also has a parsed edge from that column. The total
`row_deciding_reads` counts every entry, and `read_not_projected` now counts
only the ones never carried through, which is the lineage no direct edge holds.
Every doc and the CLI line that promised otherwise say what is true.

The `--indirect` fan-out is unchanged: it already stands aside for a direct edge
on the same pair.

## Rejected

- **Dropping the projected ones from the section.** A join key the model also
  selects still decides which rows exist; that is a second fact about the
  column, not a duplicate of the first.
- **Keeping the name and fixing only the docs.** A report is read by its keys.

## Consequences

On the same project the caches are identical. 2914 reads are listed; 415 are
never projected: 214 dedup keys, 168 filters, 33 join keys. dbt-lens does not
read the report, so the rename breaks nothing there.
