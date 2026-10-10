# 0033. A model the manifest gives no SQL reads its compiled file

Date: 2026-10-10 · Status: accepted · Amends 0001, 0007 and 0020

**Trigger:** read before changing where a model's SQL comes from, before
reading anything under `target/run/`, or before joining a path from the
manifest onto a directory.

## Context

collin read a model's SQL from the manifest's `compiled_code` and nowhere else
(0001). dbt writes it when it compiles a node: a `parse`, from dbt-core or dbt
Fusion, writes none, and a `run` or a `build` writes it only for the nodes it
selected. The files a compile wrote stay in `target/compiled/`. On a 3248 model
project with a parse-only manifest, collin read the SQL of none of them, and
every edge in the cache was a match by column name.

Those files are not the manifest's, and on that project it showed three ways:

- **They can be compiled for another target.** The manifest came from a `parse`
  on one machine, the files from a compile on another, against another database
  or schema for every one of the 3240 models that build a relation. Read as
  they were, 59 090 of 100 302 parsed edges named a table no node of the
  manifest owns, and not one parsed edge started at a model.
- **dbt writes a path with the separator of the machine that parsed.** The
  project's current manifest, from Fusion on Windows, spells all 3472 model
  paths with `\`, and joined as written on macOS they name no file.
- **The two dbts lay an installed package out differently.** dbt-core writes
  `compiled/<package>/<original_file_path>`. Fusion writes the root project the
  same way, and a package under its path from the project root,
  `compiled/<package>/dbt_packages/<package>/...`. Their compile manifests say
  so in `compiled_path`, for 3549 models of one and 3472 of the other. A `parse`
  records no `compiled_path`.

## Decision

- **A SQL model the manifest has no `compiled_code` for reads the file dbt
  compiled for it**, in the `compiled/` directory beside the manifest, at the
  path the dbt that wrote the manifest uses. Fusion is dbt 2.0 and later.
- **The manifest wins wherever it has `compiled_code`, even empty.** It was
  written with the graph the pass walks. An empty compile is its answer, and a
  file left over from before is not.
- **The path is the manifest's, split on both separators and kept to plain
  names.** A `..`, a root or a drive refuses it, and only a regular file is
  read: the manifest is not trusted, and a FIFO would hang the run.
- **A file that reads a relation the manifest does not give its model is set
  aside**, and the model's edges are inferred. The file was compiled against
  another graph: another target, or code from before a `ref` moved. The same
  read in the manifest's own SQL is the project's to fix: it is listed under
  `undeclared_relations`, and its edge still goes out.
- **A file's read of a column a parent's compile lacks indicts only the file**
  when the parent's SQL is the manifest's. The file can be the older of the
  two, so the read is not the parent's finding (amends 0020) and the child is
  listed.
- **A Python model is never handed to the engine**, whether its code comes from
  the manifest or not.
- **The report says all of it.** `totals.without_compiled_code`,
  `sql_from_files` and `sql_from_files_set_aside`, the directory looked in as
  `compiled_files`, and on a listed model `sql_file` and `sql_file_set_aside`.
  A file that could not be used says why in `parse_error`. The command line
  prints the counts, a lookup that found nothing included, and the directory.

## Rejected

- **`target/run/`.** What dbt writes there wraps the select in the statement
  that builds the table, a `create ... as` or a `merge`, which is not the
  model's SQL.
- **The same path without the package**, which the first version of this tried
  second. No dbt writes there, so it could only find another package's file.
- **Both layouts, whichever is found.** On the measured project the dbt-core
  layout held a copy of each of the 34 package models written 36 days before
  the compile, left by another tool.
- **Refusing a file older than the manifest.** A `parse` rewrites the manifest
  every time, so every such file is older, and refusing them leaves exactly the
  empty cache this fixes.
- **Refusing a file older than its model's source**, which is how dbt-lens
  marks one stale. 276 of the 3472 files of the measured compile are older than
  their source and hold exactly the SQL its manifest does, and a pinned copy
  resets every date.
- **Reading `compiled_path`.** A manifest that records it carries
  `compiled_code` too.
- **Saying nothing in the report.** The edges look the same either way.

## Consequences

Measured on pinned copies, with the code before this record, its first version,
and this one. Parsed edges are counted by where they start: a model, a source,
or a table no node owns.

| manifest | code | parsed | parsed edges: model / source / `rel:` | inferred | model pairs | covered |
| --- | --- | --- | --- | --- | --- | --- |
| parse, files of another target | before | 0 / 3248 | 0 / 0 / 0 | 68 192 | 2052 | 78.3% |
| | first version | 3238 | 0 / 41 212 / 59 090 | 3415 | 49 | 96.1% |
| | this | 3238 | 0 / 41 212 / 0 | 33 956 | 1789 | 82.9% |
| parse, files of its own compile, `\` | before | 0 / 3472 | 0 / 0 / 0 | 108 666 | | 89.5% |
| | first version | 0 / 3472 | 0 / 0 / 0 | 108 666 | | 89.5% |
| | this | 3470 | 82 165 / 42 153 / 0 | 1247 | | 96.6% |
| compile | all three | 3470 / 3472 | 82 165 / 42 153 / 0 | 1247 | | 96.6% |

With files of its own compile, a parse-only manifest now gives the cache the
compile manifest gives, byte for byte. With files of another target, 1815 of
the 3248 models are set aside. With `\` in every path, the first row gives the
same cache as with `/`. A manifest that carries its SQL gives the same cache as
before.

A compiled file whose code changed under the same `ref`s is read as it is. The
agreement with `catalog.json` compares column names (0008): it sets aside a
file whose columns the table lacks, not one whose expressions moved, and only
where the catalog describes the table, 463 of the 3248 models in the first
row. Such a file's columns also stand for the model downstream, as any
compile's do, ahead of its YAML. The counts in the report are the warning.

A pinned copy now holds `target/compiled/` beside its manifest, and its
fingerprint has to cover it (amends 0007): one manifest and one catalog with
different compiled files give different caches. The inputs are the manifest,
the catalog when present, and that directory (amends 0001).
