# 0034. collin says its version, and the version moves when dbt-lens needs it to

Date: 2026-10-10 · Status: accepted

**Trigger:** read before changing what `collin --version` prints, the
`producer` field of the cache, or the version in `Cargo.toml`.

## Context

dbt-lens runs collin when Collin is picked, and installs it on request with
`cargo install --git`. It needs to tell an old collin from a new one:
rerunning the install updates collin when GitHub has a newer commit, but
nothing would tell the user to.

## Decision

- **`collin --version`**, or `-V`, prints one line, `collin 0.1.0 (9e8d7c6)`:
  the version, and the short hash of the commit it was built from, with
  `-dirty` when tracked files differ from it. The checkout `cargo install --git`
  builds in still has its `.git`. A tree that is not the top of its own
  repository prints the version alone, rather than the commit of whatever
  repository encloses it.
- **The version moves when dbt-lens relies on a change**, not on every commit.
  dbt-lens holds the lowest version it accepts, and offers the update below
  it, or when `--version` gives no answer it can read.
- **The cache names its producer**, `"producer": "collin 0.1.0"`, beside the
  format `version`, which stays at 1. A reader that does not know the field
  skips it, as dbt-lens does.

## Rejected

- **Checking GitHub for a newer collin**, from collin or from dbt-lens. Both
  stay offline, and the machine dbt-lens runs on may not reach GitHub. cargo
  already does that check when the install command is rerun.
- **The commit in the cache.** It is the binary's to print. The library is
  built into other things, and the cache only needs to say how old it is.
- **`git describe`.** Between the bump and its tag it names the previous tag,
  and `collin 0.2.0 (v0.1.0-1-g9e8d7c6)` says two versions on one line.
- **Bumping the version on every commit.** A floor that moves on every commit
  would ask for an update that changes nothing dbt-lens shows.

## Consequences

A release has to bump the version in `Cargo.toml` before its tag, or the tag
and `--version` disagree, and the install commands in the README with it.

The cache hash in the AGENTS.md comparison leaves `producer` out, as it leaves
out `generated_at`: a bump changes it without moving an edge.
