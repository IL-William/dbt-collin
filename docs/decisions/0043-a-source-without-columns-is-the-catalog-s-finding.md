# 0043. A source without columns is the catalog's finding

Date: 2026-10-11 · Status: accepted · Amends 0040

**Trigger:** read before changing what is said of a parent the engine was not
told of, or before proposing that collin read a warehouse's information schema
for the columns a catalog lacks.

## Context

A parent with no catalog entry and no column in its YAML is never handed to
the engine, so nothing read from it can be placed. 0040 gathered the models
reading one under `parent_without_columns`, with one remedy: build the parent
in this target and generate the catalog again, or document its columns.

dbt builds no source. Building it again is no remedy, and sources are nearly
every such parent: without a catalog, 182 of the 184 models in that finding on
Cal-ITP, and 1465 of the 1474 on a 3507 model Snowflake project, name sources
only. A project run without a catalog, as dbt-edith may run one, was told to
build tables dbt does not build.

The columns of a source come from the catalog, which is dbt querying the
warehouse's information schema for every source and model. collin reads its
`sources` as it reads its `nodes`, held to the same relation (0037). On
Cal-ITP, the same catalog with its 207 source entries removed loses 467 of the
5616 edges drawn from a source, leaves 925 columns lost instead of 625, and
degrades 21 compiles instead of 12. On Shopify it moves nothing: Fivetran's
staging models name every column they read.

## Decision

- **A model's entry gains `unknown_sources`**: those of `unknown_relations`
  that are sources, spelled the same. A subset rather than a split:
  `unknown_relations` still lists every parent the engine was not told of, as
  dbt-edith reads it, and 0040 keeps `models` as it is.
- **A finding of its own, `source_without_columns`**, owned by the inputs,
  since the remedy is to run dbt again: generate `catalog.json` with the
  manifest's target, with credentials that can read these tables, or document
  their columns in the sources' YAML. A warning, as before: lineage is partial.
- **`parent_without_columns` keeps the other parents**, a model, a seed or a
  snapshot, with its words unchanged, which are right for those.

## Rejected

- **collin querying the information schema itself** for every source the
  manifest lists. The catalog is that query, made by dbt's adapter with the
  project's own credentials, quoting and case. collin would need a driver per
  warehouse and a credential, which 0001 refuses, to repeat what one dbt
  command already writes.
- **A catalog from any target where the tables exist.** A catalog of another
  target sets aside every model entry it has (0037): the manifest's target is
  the one to generate it with.
- **Owned by the project, as its sibling is.** The first remedy is a dbt
  command, and dbt-edith files the inputs' findings under running dbt again.
  They also come first in the report (0040), and a source's columns move other
  figures, as the Cal-ITP figures above show.
- **The sources by unique_id.** Spelled as `unknown_relations` spells them, the
  subset reads as one, and a reader finds a source from its relation as it
  finds any parent there.

## Consequences

Measured with the code before this record and with this one, default and
`--indirect`, on Jaffle Shop, Shopify, Cal-ITP with its catalog, with its
catalog less its sources and with none, and the Snowflake project with its
catalog and with none. Every cache is the same, byte for byte, as are the
totals and every entry but for the new field.

| project | catalog | `parent_without_columns` before | after: `source_without_columns` / `parent_without_columns` |
| --- | --- | --- | --- |
| Cal-ITP | complete | none | none |
| | less its sources | 182 models | 182 / none |
| | none | 184 models | 182 / 2 |
| Snowflake, 3507 models | complete | none | none |
| | none | 1474 models | 1465 / 9 |

Where one model reads both, it is in both findings, each naming its own
parents.
