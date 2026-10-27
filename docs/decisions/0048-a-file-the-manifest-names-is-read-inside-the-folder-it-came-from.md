# 0048. A file the manifest names is read inside the folder it came from

Date: 2026-10-27 · Status: accepted · Amends 0033 and 0047

**Trigger:** read before changing how a compiled file or a seed's CSV is
opened, or how the cache and the report are written.

## Context

collin reads files a manifest names (a model's compiled SQL, a seed's CSV) and
writes two files, the cache and the report, inside the project's `target/` by
default. A project arrives from elsewhere: a zip, a clone, a share. It can hold
a link where collin reads or writes.

0033 and 0047 confine the path the manifest gives, so `..`, a drive or an
absolute path is refused. They look at the spelling only. A link at
`target/compiled/<package>/models/a.sql` pointing at any readable file passed
the spelling checks, and its text then went into the cache (as a root's value)
and the report (as the SQL of a model). Writing had the mirror problem:
`std::fs::write` follows a link at the cache or report path and overwrites what
it names.

## Decision

- **A file is read only when it really lies under the folder it was found
  from.** The path is resolved (links followed) and must start with the
  resolved folder. For a compiled file that folder is the one holding the
  manifest (`target/`), not `compiled/`: a `compiled` committed as a link to
  `/` is below `target/` and its files resolve outside, so they are refused.
  Only when `--compiled` names the folder is it the folder itself, being the
  user's own choice. A root seed is held to the project folder; a package's
  seed to the package's own folder (`dbt_packages/<package>`); one found
  through `root_path` to `root_path`. What the path resolves to is what is then
  tested for being a regular file and read, so the check and the read name one
  path.
- **A refusal is worded like its neighbours.** A compiled file that resolves
  elsewhere is `SqlSource::Unusable` ("it is a link to a file outside
  <folder>, the folder it is held to"); a seed's is a file with no header, and
  the next candidate is not tried, as for a candidate that is not a file.
- **Only what is below the folder is held.** A target directory the user links
  elsewhere, or a project folder reached through a link, still works: the
  folder is resolved first. dbt-edith's `files::resolve` has the same rule.
- **A package's folder is its own base**, not the project's, because `dbt deps`
  makes a local package a link to a folder elsewhere. A package folder that
  resolves outside the project is accepted only when it holds a
  `dbt_project.yml`, as `root_path` is; otherwise the seed has no header. The
  package name with a `..` in it is refused as before (0047).
- **An output replaces a link, never writes through it.** The text goes to a
  new file beside the target, opened with `create_new` (which does not follow a
  link, and fails on one planted at its name), then is renamed over the target.
  A link at the target is replaced by a regular file and what it named is left
  alone. The temporary file is removed on failure. One helper, `output.rs`,
  serves the cache and the report.
- **The dump example refuses a model name with a path separator or a colon**,
  since it writes one file per name under `--out` (`C:x` leaves it on Windows,
  `a:b` names a stream).

## Rejected

- **Refusing any link**: a target directory linked to a build folder is
  ordinary, and dbt writes local packages as links.
- **Checking the spelling harder**: a link needs no `..` in the path.
- **Opening with `O_NOFOLLOW`**: it covers the last name only, a folder on the
  way can be a link, and it has no portable form on Windows.
- **Checking before writing, then writing**: a link planted between the check
  and the write wins; a rename does not follow the link at all.

## Consequences

A project whose compiled files are links into another folder (a build cache
shared between checkouts) now reads as if those models had no compiled file,
and says why in the report. The files are copied, not linked, by dbt itself.
A link out of the project that leads to a folder holding a `dbt_project.yml`
is followed, so a seed header (its first line) of a `.csv` in such a folder
can still be read. That needs a real dbt project to be linked, which is what a
local package is. A cache or report that was a link is a regular file after
the next run, and the file it named keeps its old content. The new file has
the mode a new file gets, not the old one's. The check and the read are not
one atomic step: a link swapped in between is still read, which needs a
writer in the project while collin runs.

An output is now written by creating a file beside it and renaming that over
it, so a path where collin can write the file but not create one beside it (a
folder it may not create in, a device such as `/dev/null` or `/dev/stdout`, a
single file bind-mounted alone) is refused where it used to be written in
place. Write to a regular path and copy it.
