# 0056. The Python and the shell are linted by pinned tools, fetched by hash

Date: 2026-11-12 · Status: accepted

**Trigger:** read before adding or changing a linter rule, before bumping Ruff or
ShellCheck, or before adding a file type or a folder the linters should cover.

## Context

Nothing checked the Python of `tools/` or the shell of the fixtures'
`regenerate.sh` beyond the eye and a syntax parse. dbt-edith made the same
check for its own scripts in its 0173; this is that decision on collin's smaller
surface, minus the frontend collin does not have. With the rules below, `main`
gave Ruff 11 findings, all in `tools/` (64 before the three switches below), and
ShellCheck none.

The linters have to run where `cargo test` does, a machine with Rust and
nothing else, and in CI at the same versions: a Ruff newer on one side is a
check that disagrees with itself.

## Decision

- **`scripts/lint.py` fetches Ruff 0.17.3 and ShellCheck v0.11.0** from their
  GitHub releases for macOS (arm64, x64) and Linux (x64), each asset pinned by
  the SHA-256 GitHub records for it, the same versions and hashes as dbt-edith's
  0173. The download is hashed before it is opened or made executable; a
  mismatch deletes it and exits 1. Only the expected member of an archive is
  read, and written under `target/lint-tools/<tool>-<version>-<hash>/`, which
  `target/` already keeps out of git. The script is standard library Python.
- **It lives in `scripts/`, not `tools/`.** `tools/` is what the figures in
  `docs/next-steps.md` were measured with and is read as that; a script that
  checks the repository is not a measurement.
- **A version is at least seven days old and bumped by hand**, with its URLs,
  hashes and binary path. Dependabot cannot see these pins.
- **Exit 0 clean, 1 on a finding or a refused download, 2 when a linter cannot
  run** (offline with nothing cached, no pinned build for the platform). CI
  fails on 2 and skips the Monday run, which is for advisories.
- **What is read** is what git tracks or would add (`git ls-files --cached
  --others --exclude-standard`), existing files only, less the lineage-library
  submodule, which has its own checks, and the fixtures' `target/`, which is
  dbt's output.
- **The rules aim at bugs, not style.** Ruff: `E, F, W, B, UP, SIM, RUF, PLE,
  PLW` on `py310`, minus `E501` (the scripts are not wrapped to a width). Three
  per-file switches, each with its reason in `ruff.toml`: `SIM115` in
  `tools/*.py` (read and write handles that CPython closes at the end of their
  statement, in one-shot scripts), `E701` in
  `tools/bq_causes.py` (its cases are one line each, as a table), and `B023` in
  `tools/classify.py` (a closure called in the iteration that defines it). The
  rest were fixed without changing what a script does: split imports and
  statements, an unused import dropped, `l` renamed, an unused loop name
  prefixed with `_`. ShellCheck runs at its default severity and found nothing.
- **Nothing is formatted.** Ruff's formatter is not run, and the shell is not
  reformatted: a formatter would rewrite scripts nobody asked to touch.

## Rejected

- **Whatever is installed.** Versions drift between machines and CI, and most
  machines have neither Ruff nor ShellCheck, so the check would skip nearly
  everywhere.
- **`pip install ruff` or `uv tool run`.** The version can be pinned, but the
  file that comes down is not checked against a hash written in the repository,
  and the shell would still need another route to ShellCheck.
- **A pre-commit framework.** One more tool and a hooks cache to keep in step,
  for two linters a script runs.
- **Copying dbt-edith's `ignore` list.** `RUF005`, `PLW1510`, `SIM105` and
  `E731` have no hit here; a switch with nothing behind it is noise. A new
  one is added with the hit that needs it.
- **Fixing `SIM115` and `B023` in the scripts.** Both would change the shape of
  measurement scripts whose figures are quoted in `docs/next-steps.md`, with
  nothing broken to show for it.

## Consequences

The measurement scripts are not run by any test, and the project they read
stays outside this tree (0007), so the fixes above were checked by Ruff and a
byte-compile only. They are renames and line splits; a reader running one on
a pinned copy next time is its first real test.

A Ruff or ShellCheck release that adds a rule can turn a clean tree red only
when someone bumps the pin, and that bump is a pull request of its own.
