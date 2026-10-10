# Working on collin

A library and a CLI that turn a dbt project's compiled SQL into a column lineage
cache. It reads a project, it never runs dbt and never reaches a warehouse.

## Verify

```
cargo test
```

It reads two public projects end to end, dbt's Jaffle Shop and Fivetran's
Shopify package, from the manifest and catalog dbt wrote, against the caches
committed beside them (0038). A change that moves an edge fails it. One meant to
is accepted with `COLLIN_BLESS=1 cargo test --release committed_beside`, and
the diff of `expected/` goes into the pull request. Neither is Snowflake, so
they do not replace what follows.

Then, against a real project outside this tree:

```
cargo run --release -p collin-cli -- generate --project /path/to/dbt-project
```

A change is shown on such a project, not argued: run the code before it and the
code after it on the same pinned copy, its `target/compiled/` included (0033),
never the live directory (0007), each into its own directory outside this tree,
and compare.

```
P=/path/to/pinned-copy
O=/tmp/collin/after                  # and /tmp/collin/before, from the code before
cargo run --release -p collin-cli -- generate --project $P --out $O/cache.json --report $O/report.json
cargo run --release -p collin-core --example dump -- --project $P --out $O/dump

jq -c 'del(.generated_at, .producer)' $O/cache.json | shasum -a 256    # the cache, less its timestamp and its producer
jq -r '.edges[].kind' $O/cache.json | sort | uniq -c        # edges by kind
jq -r '.edges[].from | split(".")[0] | split(":")[0]' $O/cache.json | sort | uniq -c   # by origin
diff <(jq .totals /tmp/collin/before/report.json) <(jq .totals $O/report.json)
diff -r /tmp/collin/before/dump $O/dump                     # what the engine saw, what went out
```

- **The hash is the claim that nothing moved.** The cache is written in one
  order, so two runs of the same code differ in their timestamp and nowhere else.
  The producer names the version, which a release bumps without moving an edge,
  so it is left out too.
- **Coverage is not lineage.** A column counts as covered with any edge, one
  from a `rel:` table no node owns included, so a change can raise coverage and
  cut the graph. The edges by origin show it (0033).
- **Run both again with `--indirect`**, each into a directory of its own, when
  the change touches a join key, a filter or a dedup key: without it those never
  reach the cache.
- **The dump is one JSON file per model**, for reading why rather than what:
  each parent with the columns the engine was handed and where they came from,
  the outputs and roots, every edge the engine gave with its expression whether
  or not it was published, the indirect reads, the issues, the agreement, the
  plan the pass chose, the compiled file the SQL came from, and the edges the
  model put in the cache as they were written. The pass hands it all of this.
  Where an edge comes from and what role it plays are shown only as the pass
  published them, never worked out again, so the dump cannot describe an older
  pass.
- **The figures go in the commit message, the names never do.** Everything these
  commands write is the project's.

## Rules

- **An edge always says where it came from.** `parsed` and `inferred` never mix.
  Lineage whose provenance cannot be told is worse than no lineage.
- **Never guess in `engine.rs`.** It reports what the engine found and what the
  engine admitted it could not find. Choosing what to do about a gap belongs to
  `lineage.rs`, which has to label the choice.
- **The cache stays at version 1.** dbt-lens refuses anything higher, so a
  released binary must keep reading what this writes.
- **No real project SQL in this repository.** Fixtures are invented, or public
  under a license that allows copying them here (0038).
- **Comments say why, not what.**
- **No em dash** in code, comments or documentation.

## Decisions

[`docs/decisions/`](docs/decisions/README.md) records why collin is shaped this
way and what was rejected. Read the index before changing anything structural,
and add a record when you make a call the code alone will not explain.

## Release

A pull request that bumps the version in `Cargo.toml`, and the install
commands in the README with it, is the whole release: once it is merged, the
release workflow tags that commit and publishes it (0035). Bump when dbt-lens
relies on a change, or for a release worth installing (0034).

## Layout

One module, one concern, each with a header comment stating the concern and any
invariant.

| Module | Concern |
| --- | --- |
| `manifest.rs` | the dbt manifest, the relation index, the topological order |
| `catalog.rs` | `catalog.json`, the only witness to the warehouse |
| `schema.rs` | the three column sources and the agreement verdicts |
| `engine.rs` | the analysis library, behind an interface that hides it |
| `role.rs` | an expression to a role |
| `lineage.rs` | the pass, and the provenance ladder |
| `report.rs` | coverage and gaps |
| `findings.rs` | the report's models grouped by cause, worded for whoever can act |
| `cache.rs` | the `column_lineage.json` writer |

Tests live beside the code in `#[cfg(test)]` modules, the way dbt-lens does it.
