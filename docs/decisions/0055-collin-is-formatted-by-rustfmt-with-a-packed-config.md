# 0055. collin is formatted by rustfmt with a packed config

Date: 2026-11-12 · Status: accepted

**Trigger:** read before changing `rustfmt.toml`, resolving a conflict in
formatted Rust, or formatting a file the fork or a fixture owns.

## Context

The Rust code was written by hand, closer to 100 columns than to 140, with
short bodies on one line. Nothing enforced a layout, and `AGENTS.md` said never to run
`cargo fmt`, because rustfmt with no configuration rewrites that style. The
engine fork (0028) is formatted by rustfmt in its own repository, which this
record does not touch.

Measured on the 14 `.rs` files of the workspace (14,292 lines, 647 KB), on scratch
copies, the lines rewritten being the figures of `git diff --no-index --shortstat`
between the copy before and after:

| | lines | bytes | lines of the code rewritten |
| --- | --- | --- | --- |
| rustfmt, no `rustfmt.toml` | +21% | +6.6% | 1,010 removed, 4,084 added |
| rustfmt, the config below | -4.9% | -1.3% | 1,283 removed, 584 added |
| the config below, at `max_width = 100` | +5.2% | +1.8% | 548 removed, 1,284 added |

The code was written closer to 100 columns than to 140, so the config joins
more lines than it breaks: it moves about 9% of the lines, and the file gets
shorter. The 140 is dbt-edith's, kept so that the two repositories format alike;
at 100 the move is smaller, but the layout is not that repository's.

## Decision

- **`cargo fmt --check`** runs in the `rust` job of CI before the tests, not on
  the Monday run (a new stable rustfmt must not turn it red when nothing was
  pushed), and is in the Verify list of `AGENTS.md`.
- **`rustfmt.toml`** holds dbt-edith's four stable options as they are
  ([its 0172](https://github.com/IL-William/dbt-edith/blob/main/docs/decisions/0172-only-the-rust-code-is-formatted-by-rustfmt-with-a-packed-config.md)):
  `max_width = 140`, `use_small_heuristics = "Max"`, `fn_params_layout =
  "Compressed"` and `short_array_element_width_threshold = 40`.
- **Run `cargo fmt` before a Rust change is done.**
- **A conflict in formatted code is never resolved by hand.** Take one side of
  the file whole and run `cargo fmt` again, so the result is what the tool
  writes.
- **The formatting commit is one change of its own**, which changes no behaviour:
  the tests pass unmodified, and the caches committed beside Jaffle Shop and
  Shopify are the same bytes (`expected/` does not move).
- **A block whose alignment is the point** may carry `#[rustfmt::skip]` with a
  one-line reason.

## Rejected

- **rustfmt's defaults**: a fifth more lines, and four times as many lines
  added as removed, for a style nobody here wrote.
- **Tuning the options to this code's own width** (`max_width = 100`): it moves
  fewer lines, but it parts from the repository whose code collin sits beside,
  and every option added is one more place for the two to drift.
- **Formatting only the lines a change touches** (`--file-lines`): it is
  unstable, and a file would hold two layouts until every line had been touched.
- **Nightly-only options** (`imports_granularity`, `wrap_comments`): CI and
  `cargo install` use stable (0054), and a nightly would be the only place the
  check ran.
- **Formatting the fork**: it is another repository, with its own file.

## Consequences

`AGENTS.md` no longer says that collin is not formatted; the fork keeps its own
formatting. The formatting commit shows in `git blame` over every line it
moved: once it has a hash, list it in a `.git-blame-ignore-revs`, as dbt-edith
does.
