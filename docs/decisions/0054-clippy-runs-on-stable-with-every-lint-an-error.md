# 0054. Clippy runs on stable with every lint an error

Date: 2026-11-12 · Status: accepted

**Trigger:** read before silencing a Clippy lint, adding an `#[allow]`, changing
the Clippy step of the `rust` job in CI, or pinning a Rust toolchain.

## Context

Nothing read the Rust code for mistakes beyond what `rustc` warns of, so lints
accumulated unseen. Measured on this tree with stable Clippy 0.1.98, all targets
of the workspace: seven findings of two lints, none a correctness bug.
`type_complexity` fired five times, on a tuple a walk keeps on its stack and in
its visited set (`engine.rs`), the key of a set of rules (`lineage.rs`) and two
test tables, and `doc_overindented_list_items` twice, on the ladder in the header
of `lineage.rs`. A lint nobody has to fix is a lint nobody reads.

## Decision

- **`cargo clippy --workspace --all-targets --locked -- -D warnings`** runs in
  the `rust` job of CI after the tests, on the stable toolchain, and is in the
  Verify list of `AGENTS.md`. Any lint fails the run.
- **Both crates take the workspace's lints** (`[lints] workspace = true`), from
  `[workspace.lints.clippy]` in the root `Cargo.toml`. That table is empty: no
  lint is allowed for the whole workspace, because none of the seven was a
  design choice.
- **Each finding was fixed, with no change of behaviour**: a local `type` alias
  where a tuple was named twice or nested, and the three-space indent
  of the ladder's continuation lines.
- **A lint that is wrong for one place is allowed there**, on that item, with a
  one-line comment saying why that place is not the lint's case. A crate-wide
  allow needs a record of its own.
- **Not on the Monday run.** That run is for advisories, which appear with
  nothing pushed; a new stable Clippy brings new lints the same way, and must
  not turn it red. The next push shows them.

## Rejected

- **Pinning a toolchain** (`rust-toolchain.toml`): collin is installed from a
  release archive or with `cargo install --git --tag`, and neither reads a
  `rust-toolchain.toml` in this repository, so the pin would bind only whoever
  works on the code and CI. Every contributor would fetch it, and someone would
  have to keep bumping it. Clippy would also check with another toolchain than
  the one the releases are built with. The cost of stable is below.
- **Allowing whole groups** (`style`, `complexity`, `perf`): the groups hold the
  lints worth having along with the ones that are not, and an allow on a group
  hides the next lint added to it.
- **Allowing `type_complexity` for the workspace**, as dbt-edith does for its
  payload types: here the five sites were local tuples, and a name made each
  easier to read.
- **Warnings only, never failing**: nobody reads a warning list that stops
  nothing.

## Consequences

A new stable Clippy can fail a push that changed nothing it lints; the push it
fails fixes it. A crate-wide allow, which none is today, needs a record of its
own.
