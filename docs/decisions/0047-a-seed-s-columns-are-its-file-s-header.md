# 0047. A seed's columns are its file's header

Date: 2026-10-20 · Status: accepted · Amends 0004, 0043 and 0046

**Trigger:** read before changing what is known of a seed's columns, how a CSV
header is read, or which files `generate` reads from the project.

## Context

A parent with no catalog entry and no column in its YAML is never handed to the
engine (0043), so a `select *` over it does not expand and the edges from it
are lost. A seed is often that parent: the catalog lists it only when `dbt seed`
and `dbt docs generate` ran on the manifest's target, and a seed's YAML rarely
lists columns. The model reading it then starts its own column graph, as if
nothing fed it.

Yet a seed's columns are known without a warehouse. `dbt seed` loads its CSV,
and the table has the columns of its header. Checked against dbt-core 1.12 and
dbt-common:

- `load_agate_table` reads `<project>/<original_file_path>` for the root
  package and `<project>/<packages_install_path>/<package>/<original_file_path>`
  for another, and falls back to `root_path/<original_file_path>`;
- `from_csv` reads UTF-8 and skips one leading BOM, through Python's `csv`
  default dialect with the seed's `delimiter` config (`,` by default). Names
  stay as written; agate gives an empty name a letter and a repeated one a
  `_2`;
- a Fusion manifest written on Windows has `\` in `original_file_path` and a
  Windows `root_path`.

## Decision

- **A seed's header is a fourth source, kept apart (0004)**: `Store.file`,
  `Provenance::File`, written `seed_file` in the report and the dump. A star
  over the seed expands to it, in the file's order (0044).
- **Its rank is warehouse, then file, then computed, then declared.** The
  catalog still wins: it is the table `dbt seed` built, and a file edited since
  is not. A seed has no computed list, and the header is newer than the YAML.
- **It is read in `load`, not in `run`**, as compiled files are (0033): `run`
  reads no file. The catalog is loaded first, and a seed it has is not read.
  The file is read in reads that double in size until the first newline
  outside quotes, and given up on, with no list, at 1 MiB.
- **Where, in dbt's order**: the project given, with `dbt_packages/<package>`
  for a package's seed, then the manifest's `root_path`, which only a run on
  the machine that wrote the manifest finds, and which is tried only when it is
  an absolute path to a folder holding a `dbt_project.yml`. `\` is read as a
  separator. Only a regular file is read; a path dbt could not stat (a parent
  that is not a folder, no permission) is a file that is not there, and the
  next place is tried. A manifest named alone, without `--project`, is read
  alone (0046): the folder collin ran in is not its project, so only
  `root_path` is tried. Without `metadata.project_name` no seed is known for
  the root's, and the project's own folder is not tried.
- **The path is the manifest's, so it is confined** as a compiled file's is
  (0033): one that climbs with `..`, names a drive or does not end in `.csv` is
  not read, and a header goes into a cache. `root_path` is the exception to the
  rule that an absolute path is not read: it is a folder holding a dbt project,
  and the file under it is held to the same rule.
- **The header is parsed by hand, as Python's `csv` does**: one BOM, the
  seed's `delimiter` when it is one character, `"` quoting with `""` for a
  quote, CRLF. Names are lower cased, as the catalog's and the YAML's are.
- **No list rather than a guessed one.** Each of these leaves the seed with no
  list: a delimiter of more than one character (dbt raises); an empty name,
  which includes a delimiter left trailing; two names equal once lower cased;
  a name with a space at either end; a name with a line break in it (Python's
  text mode would change it); a quote never closed; a header past the
  limit; a header that is not UTF-8. Agate renames the first two in a way the
  file does not show, and dbt-snowflake does not quote a seed's columns by
  default, so ` Label` is a table's `LABEL`: the list would put an edge on a
  column the table lacks, which is worse than a missing edge.
- **`parent_without_columns` keeps a seed** whose file is not found or not
  readable, and one whose header is refused for any reason above. Its words
  are unchanged.

## Limits

- **`root_path` is trusted as a dbt project's folder.** The file named is held
  to the rule above, the folder only to being absolute and holding a
  `dbt_project.yml`.
- **A `packages-install-path` other than `dbt_packages`** is found only through
  `root_path`: the manifest does not record it.
- **A seed path outside the project** (`seed-paths: ["../shared/seeds"]`) is
  refused, by the rule above, and the seed stays unknown.
- **A manifest parsed with a relative `--project-dir`** carries a relative
  `root_path`, which is never tried: a relative path depends on a working
  folder collin cannot know (0046). Seeds are then read only from the project
  folder collin is given.

## Rejected

- **Querying the warehouse for a seed's columns.** The catalog is that query
  (0043), and 0001 refuses a driver and a credential.
- **Taking the file over the catalog.** The catalog is what the table has; the
  file is what it will have after the next `dbt seed`. A file ahead of its
  table would give an edge to a column the table cannot yet serve.
- **A CSV crate.** Only one record is read, and the dialect is a few rules: a
  dependency to keep pinned and audited for that is not worth it.
- **Renaming an empty or repeated name as agate does.** Its rule is dbt's
  internals, not a contract, and a wrong guess gives a wrong edge.
- **Reading `root_path` first.** In a manifest from a CI run or another
  machine it names a folder that is not on this machine, or an older checkout
  of it.

## Consequences

Measured with the code before this record and with this one, on Jaffle Shop
and Shopify with their catalogs: the expected cache committed beside each is
still given (0038). On Jaffle Shop with no catalog the cache is the same, byte
for byte, since its staging models name each seed column explicitly and the
engine already traced them to the seed without its column list, and
`parent_without_columns` goes from three models to none: its seeds are no
longer parents without columns.

A project run without a catalog, as dbt-edith may run one, now has the edges
from every seed whose file is found and whose header is trusted.

## Amended by 0048

The file is read only when it resolves, links followed, under the folder it was
found from.
