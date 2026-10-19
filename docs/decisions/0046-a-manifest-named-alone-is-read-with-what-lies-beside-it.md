# 0046. A manifest named alone is read with what lies beside it

Date: 2026-10-18 · Status: accepted · Amends 0033

**Trigger:** read before changing where a path `generate` reads or writes
defaults to, or where a model's compiled file is looked for.

## Context

collin needs a manifest, the compiled SQL of the models the manifest has no
`compiled_code` for, and a catalog when there is one. dbt writes all three in
one directory, `target/`, and collin found the manifest and the compiled files
there, wherever `--manifest` pointed (0033). The catalog, the cache and the
report were the exception: they defaulted to `<project>/target`, and the
project to the folder the command ran in.

So a manifest copied out of its project, an artifact of a CI run, or a project
read from outside it, gave a wrong answer in silence. Run from another folder
on the Jaffle Shop fixture's manifest with its `compiled_code` taken out, as a
`parse` leaves it, and its compiled SQL in a folder of its own,
`collin generate --manifest ../artifacts/manifest.json` read the SQL of
none of the 5 models, wrote 3 edges, all inferred, and put the cache and the
report into a `target/` it created in the folder it ran in, where dbt-edith,
which looks beside the manifest, never finds them. It would have read that
folder's `target/catalog.json` too, another project's if there was one. The
compiled SQL could not be pointed at at all: it had to sit in `compiled/`
beside the manifest.

The question this answers is an agent's or a script's: the lineage of any
project, from its artifacts alone, with no project to stand in.

## Decision

- **Every path left unset is found beside the manifest**: the catalog, the
  cache and the report, as `compiled/` already was. The manifest itself still
  defaults to `<project>/target/manifest.json`, so a run that names no manifest
  reads and writes where it did.
- **`--compiled DIR` names the folder dbt compiled into**, the one holding a
  folder per package, `target/compiled` as dbt writes it. It defaults to
  `compiled/` beside the manifest. Every rule of 0033 holds inside it: the
  manifest wins wherever it has `compiled_code`, a file reading a relation the
  manifest does not give its model is set aside, a path is kept to plain names.
- **A folder named must be the one dbt compiled into.** Mistyped, the target
  directory holding `compiled/`, one level too deep or the project itself, it
  would read no file, match every model it was named for by name and exit 0,
  which is the silence this record exists to end. So a path that is not a
  folder is refused with exit 1 and nothing written, and so is a folder
  holding the package folder of none of the manifest's SQL models, while one
  of them is left to read; the error names the `compiled/` inside, when that
  one holds some. Every model counts, those with `compiled_code` too: a build
  that left a package out gives the root's models theirs, and its folder holds
  the root's files alone. A folder holding some of them is read: a compile
  that selected part of the project leaves the others out. The default folder
  may be missing, as it was before: a manifest with all its `compiled_code`
  needs none.
- **The report names the folder looked in**, as `compiled_files`, and a file
  as `compiled/<package>/...` in the default folder, which dbt-edith joins to
  the target directory, or else whole, under the folder named: the report is
  read from beside the manifest, where a path relative to the folder collin
  ran in leads nowhere.

Measured on the same manifest and folder, with `--compiled ../sql/compiled`: 5
of 5 models read from their files, 31 edges, all parsed, 27 of 27 columns
covered, the cache and the report beside the manifest and nothing written
where the command ran.

## Rejected

- **`--compiled` naming the target directory**, the folder holding `compiled/`:
  what someone hands over is the compiled SQL, and dbt's own `compiled_path`
  starts below the target directory too.
- **Defaulting the project to the manifest's grandparent**: a manifest copied
  alone has no project there, and the project names nothing `generate` reads
  once the manifest is named.
- **Keeping `<project>/target` for the catalog** when the manifest is named
  elsewhere: a catalog not written with the manifest describes another target
  or another project, which 0037 can only set aside entry by entry. Naming it
  with `--catalog` still reads it.
- **Looking in both folders** for a compiled file: two copies of one model, of
  two dates, with nothing to say which one the manifest was compiled with.

## Consequences

A run naming a manifest outside `<project>/target` without `--catalog`, `--out`
or `--report` now reads and writes beside that manifest. dbt-edith names all
four paths whenever it runs collin, so it sees no change; it passes
`--compiled` only when its own user names the folder, and that needs 0.9.0.
The finding on compiled files set aside says collin read "the files a compile
left" rather than `target/compiled`, which a folder named would make wrong.
