# 0037. A catalog entry for another table is no witness

Date: 2026-10-10 · Status: accepted · Amends 0004 and 0008

**Trigger:** read before changing how `catalog.json` is matched to the
manifest, or before trusting an entry because its unique_id is right.

## Context

dbt keys `catalog.json` by unique_id, and collin took every entry for the
table of the node it names. But one unique_id names a different table under
each target: a model built in a personal schema by one profile is built in a
shared schema by another. `dbt docs generate` and `dbt compile` write their
artifacts into the same `target/`, so a catalog from one target and a manifest
from another sit side by side, and nothing in either file says so.

On a 3507 model Snowflake project the two met that way: a manifest compiled on
a shared target, and a catalog four days older, generated on a personal one.
Not one of the 2912 models both describe was the same table in each. Read by
unique_id, every one of those entries was the witness for its model: the
agreement compared the compile with a table it does not build (0008), and a
child's `select *` expanded to that table's columns, since the warehouse wins
(0004). Personal tables are rebuilt when their owner runs them, so their
columns are the code of whenever that was, and the verdicts followed.

## Decision

- **An entry is a witness only for the relation the manifest gives its node.**
  `catalog.rs` compares the entry's `database.schema.name` with the node's
  `relation_name`, normalised as every relation is, on the parts both spell: an
  adapter without databases writes none, and a relation can come in two parts.
- **An entry for another table is set aside.** Its model has no warehouse list
  and is checked as a model with no catalog entry is. A source is held to the
  same rule, and is usually its own table under every target.
- **Nothing to compare keeps the entry**: a node the manifest gives no relation,
  an entry without a schema or a name. Neither shows the entry is about another
  table.
- **The report names each one** under `catalog_elsewhere`, at the top, since it
  is a finding about the inputs rather than a model, with the table the
  manifest names and the one the entry describes, and counts them in
  `totals.catalog_elsewhere`. The command line prints the count and says to
  regenerate the catalog with the manifest's target.

## Rejected

- **Keeping the entry and warning.** The edges would still rest on a table the
  compile does not build, labelled as if the warehouse had confirmed them.
- **Refusing to run** when the entries are elsewhere. A run without a catalog is
  allowed (0001), and the compile still says what it says.
- **Matching across schemas**, the personal table standing for the shared one
  of the same name. That is the guess the catalog exists to spare: the two are
  built by different runs, from code of different days.
- **Comparing the targets once**, from the metadata of the two files. Neither
  records its target, and a catalog of the right target can still describe a
  table an alias has since renamed, which only the relation shows.

## Consequences

Measured on a pinned copy, the manifest compiled on a shared target, with the
code before this record and with this one.

| catalog | code | confirmed / degraded / unchecked | set aside | parsed / inferred | covered | against warehouse or YAML |
| --- | --- | --- | --- | --- | --- | --- |
| its own target, 3316 of 3507 models | both | 3154 / 168 / 185 | 0 | 117 926 / 1647 | 94.8% | 97.6% |
| the same, every model entry relabelled to another schema | before | 3154 / 168 / 185 | | 117 926 / 1647 | 94.8% | 97.6% |
| | this | 0 / 9 / 3498 | 3325 | 116 591 / 521 | 96.3% | 95.5% |

With a catalog of the manifest's own target the cache is the same, byte for
byte, and the report gains a zero. The relabelled catalog is the case least
kind to this record: its columns are the right tables', only its relations
are not, and the code before could not tell it from the first row. This one
sets every model entry aside and keeps the 2419 sources, which name the same
tables under both targets. Its models are then checked against the YAML alone,
and a parent's list is its compile's: children read 2071 columns such a list
lacks and the table had, each filed on the parent (0020). The real catalog of the other target was regenerated before it could be
measured: its columns were its own tables', and those verdicts are the ones
this record stops.
