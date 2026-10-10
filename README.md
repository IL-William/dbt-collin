# collin

Column-level lineage for a dbt project, derived from the compiled SQL dbt
already writes. No warehouse connection, no credentials, no Jinja evaluation.

It writes the `column_lineage.collin.json` cache that
[dbt-edith](https://github.com/IL-William/dbt-edith) reads, and a coverage
report that names every gap.

```
collin generate --project /path/to/dbt-project
```

## Install

Each release carries a binary for macOS on Apple silicon and a static one for
Linux on x86_64, each with a build provenance GitHub attests:

```
curl -LO https://github.com/IL-William/dbt-collin/releases/download/v0.1.0/collin-v0.1.0-aarch64-apple-darwin.tar.gz
gh attestation verify collin-v0.1.0-aarch64-apple-darwin.tar.gz --repo IL-William/dbt-collin
tar -xzf collin-v0.1.0-aarch64-apple-darwin.tar.gz
```

The binary is not notarized. Downloaded with `curl` it runs as is; downloaded
with a browser, macOS refuses it until `xattr -d com.apple.quarantine collin`.

From source, with Rust 1.82 or later:

```
cargo install --locked --git https://github.com/IL-William/dbt-collin --tag v0.1.0 collin-cli
```

`collin --version` says which collin it is, and the cache it writes names it
too, as `producer`
([0034](docs/decisions/0034-collin-says-its-version.md)).

## Why it exists

Snowflake's `GET_LINEAGE` costs one call per column, needs warehouse access, and
says only that `a` feeds `b`. It never says whether `b` is `a` unchanged, `a`
renamed, `a` cast, or `a` buried in a window function. dbt Fusion computes column
lineage locally but only under `--static-analysis strict`, which requires both a
`dbt login` and a warehouse connection.

This needs neither, and it classifies every edge.

## What an edge claims

The `kind` field carries the role, and the provenance is part of it:

| kind | meaning |
| --- | --- |
| `passthrough` | the same column, same name |
| `rename` | the same column under another name |
| `cast` | a cast and nothing else |
| `aggregate` | inside SUM, COUNT, LISTAGG and friends |
| `window` | inside an OVER clause |
| `hash` | one call to MD5, SHA1, SHA2 or HASH, under any casts: a surrogate key or a hashdiff |
| `transform` | some other expression, nothing more claimed |
| `inferred` | **not read from the SQL**: matched by column name, because the compiled SQL was shown not to describe the object that exists |

`inferred` is the honest half of the design. It never merges with the rest.

A column meeting several expressions on its way through a CTE chain takes the
loudest of them, not the last: an aggregate wrapped in later arithmetic is still
an aggregate ([0009](docs/decisions/0009-the-loudest-role-on-a-path-wins.md)).

Three further kinds, `join_key`, `dedup_key` and `filter`, name a column the SQL
reads to decide which rows exist rather than what a value is. The report always
lists them, in a section of their own, `row_deciding_reads`, rather than in the
model list, which stays a list of faults, and says of each whether the model
also carries it through to an output. `--indirect` also puts them in the cache,
where they cost one edge per output column, which on the corpus below more than
doubles the file ([0011](docs/decisions/0011-an-indirect-edge-and-what-it-fans-out-to.md)).
A column read for two reasons still gets one edge per pair, carrying `dedup_key`
over `join_key` over `filter`, and the report keeps both reasons
([0013](docs/decisions/0013-one-edge-when-a-column-plays-two-indirect-roles.md)).

## The three sources of truth

Column lists come from three places, kept apart on purpose:

- **declared**, the YAML: what the project says it produces.
- **warehouse**, `catalog.json`: what the object actually has.
- **computed**: what this compile's SQL would produce.

Computed disagreeing with the warehouse means the compile is degraded: a macro
that introspected a relation missing from the target it was compiled against,
or a table built from older code than the SQL. A compile that gives its own
reason to doubt it, or shares no column with its table, drops to `inferred`.
Otherwise it is settled column by column
([0029](docs/decisions/0029-a-compile-its-table-is-behind-keeps-the-columns-both-have.md)):
the columns both have keep their parsed edges, the table's others are inferred,
and those only the compile has are withheld. A catalog entry describing another
table than the manifest names, one generated with another target, witnesses
nothing: it is set aside and named in the report
([0037](docs/decisions/0037-a-catalog-entry-for-another-table-is-no-witness.md)).
The warehouse disagreeing with the
YAML means the documentation is stale, which the report says out loud because
nothing else in a dbt project looks.

## Measured

On a 3341 model Snowflake project, about 5 seconds end to end:

| | |
| --- | --- |
| Models parsed | 3320 / 3341, and every failure is invalid SQL rather than a parser gap |
| Edges | 103 575: 97 812 parsed, 5763 inferred |
| Columns covered | 94.1%, and 95.8% against a list this tool did not produce |
| Columns emitted that the warehouse does not have | 0 |
| Confirmed against the warehouse | 486 of the 604 models `catalog.json` covers |
| Contradicted by it and settled column by column | 62 |
| Output columns of trusted models with no edge, named | 103 |
| Columns read to choose rows | 6305, 2337 never projected, in the report unless `--indirect` |

Coverage is quoted twice on purpose. 2737 of the 3341 models have no
`catalog.json` entry, so the usual figure measures the compile against its own
output. The second measures it against the warehouse where there is one and the
YAML otherwise, minus the columns built from literals, which have no parent to
find.

## Design

[`docs/decisions/`](docs/decisions/README.md) records why it is built this way
and what was rejected, which is the part the code cannot show you.

`engine.rs` is the only module that knows which analysis library is used. Today
it wraps [`flowscope-core`](https://crates.io/crates/flowscope-core), chosen on
measurement rather than on reading. Everything else speaks `Resolved`, so
replacing it touches one file. It is built from a fork pinned to one commit,
carrying fixes upstream has not released yet, each one a patch upstream can
take as it is ([0028](docs/decisions/0028-the-engine-runs-from-a-fork-while-its-fixes-wait-upstream.md)).

Models are walked parent first, which is what makes `select *` tractable: column
lists propagate from the leaves instead of needing a warehouse.

## Known limits

- **A model the manifest has no `compiled_code` for**, which a `parse` writes,
  and a `run` for what it did not select, is read from the file a compile left
  in `compiled/` beside the manifest, where the dbt that wrote the manifest puts
  it ([0033](docs/decisions/0033-a-model-without-compiled-code-reads-its-compiled-file.md)).
  That file may be older than the manifest, or compiled for another target. One
  that reads a relation the manifest does not give its model is set aside and
  its edges inferred; one whose code changed under the same `ref`s is read as it
  is. The report counts them, `sql_from_files` and `sql_from_files_set_aside`,
  and names the file on every model it lists. `target/run/` is never read.
- **Ephemeral models** are inlined by dbt as `__dbt__cte__<name>`, so edges pass
  through them rather than stopping at them. The engine is handed the tables an
  ephemeral parent reads, and inference names those tables, never the ephemeral
  ([0015](docs/decisions/0015-an-ephemeral-parent-is-read-through.md)). The
  ephemeral still gets edges of its own, which nothing downstream points at.
- **An inferred column several parents have** is drawn from the parents the set
  aside compile reads it from, from every candidate when the compile did not
  parse, and from the first parent in the manifest otherwise; the report's
  `ambiguous_inferred` says which, column by column
  ([0027](docs/decisions/0027-a-name-several-parents-have-is-settled-by-the-reading.md)).
  Taking every candidate is right for a union and wrong for a lookup joined
  beside it, and nothing without a reading tells the two apart.
- **A table older than its code** still decides what a child's `select *` sees,
  since the warehouse wins (0004). A column the new code adds, read through a
  star over that one relation, keeps its edge when another list of the parent
  has it, and is filed on the parent
  ([0030](docs/decisions/0030-a-column-read-through-a-star-is-evidence-about-its-parent.md));
  otherwise it is a phantom in the child, and the report names the parent's list
  as the cause. A name read inside an expression through such a star,
  `upper(s.x)` rather than `s.x`, gets no column from the engine and is counted
  among the roots. Rebuilding the dev table is the remedy.
- **A column that keeps one input and loses another** is not named. The report's
  `lost_columns` lists the output columns of a trusted compile that came out with
  no edge at all, and why the walk back from each stopped
  ([0017](docs/decisions/0017-a-column-with-no-lineage-is-named.md)); an edge
  from one parent hides the loss of the other.
- **A join key, a filter and a `QUALIFY` dedup key** have no output column to
  hang off, so they are named in the report and reach the cache only under
  `--indirect`. `QUALIFY` is read off the syntax tree here, each clause in the
  `SELECT` that holds it, because the engine's analyzer never visits it; a name
  more than one source of that `SELECT` has is counted, not read
  ([0023](docs/decisions/0023-a-qualify-key-belongs-to-its-own-select.md)).
- **A model reading itself** is never handed its own relation, since it is not
  its own parent, so the engine says it cannot find it. The report marks that
  issue `self_reference`, counts it in `self_reads`, and does not list a model
  for it alone unless an output column comes from that relation and nothing else
  ([0012](docs/decisions/0012-a-model-reading-itself-is-not-a-fault.md)). Spelled
  otherwise than dbt wrote it, in two parts for three, it is not recognised and
  stays a fault.
- **A relation dbt was not told of**, a raw table named where a source belongs or
  a node read without `ref`, is named in the report's `undeclared_relations` and
  keeps its model listed. The fix belongs in the project, and collin infers
  nothing from it: a trusted compile's edges out of it point at `rel:<name>`,
  and a distrusted one's name matches still come from the declared parents,
  which that compile may not read. The tables an ephemeral parent inlines count
  as declared. SQL that did not parse, and a table named in one part, go unseen.
  A dependency or the model itself named in two parts where dbt wrote three is
  compared as written and listed here: the database that would complete it is
  the session's, and collin does not guess it.
- **Two CTEs of one name in nested `WITH` blocks** are one scope to the engine,
  their columns crossed, so a compile with them is set aside and its edges
  inferred
  ([0018](docs/decisions/0018-two-scopes-of-one-name-are-not-trusted.md)). Two
  derived tables of one name the fork keeps apart
  ([0028](docs/decisions/0028-the-engine-runs-from-a-fork-while-its-fixes-wait-upstream.md)).
- **Two expressions between the same two columns**, one per branch of a union
  straight off one CTE, reach the walk as one: the engine keeps one edge per
  pair of columns, and the role is that edge's
  ([0032](docs/decisions/0032-the-loudest-role-over-every-path-stands.md)).
- **`* REPLACE (...)` and `* ILIKE '...'`** are not read: the star expands to
  every column, as it would without them. `EXCLUDE`, `EXCEPT` and `RENAME` are.
- **The corpus never enters this repository.** Fixtures are invented, apart
  from two public projects, dbt's Jaffle Shop and Fivetran's Shopify package
  ([0038](docs/decisions/0038-a-public-example-project-is-a-fixture.md)); point
  `--project` at a checkout outside the tree.

## Verify

```
cargo test
cargo run --release -p collin-cli -- generate --project /path/to/dbt-project
```

`cargo test` runs collin on dbt's Jaffle Shop and on Fivetran's Shopify
package, from the manifest and catalog dbt wrote for each, and fails if an edge
differs from the caches committed beside them.

The same project twice must give the same file apart from its `generated_at`.
Two dbt nodes claiming one warehouse object used to be settled by hash order,
which made it not.

## License

Functional Source License 1.1 with an MIT future license (FSL-1.1-MIT), see
[LICENSE](LICENSE). Copyright 2026 Datadorelix.

Use it free of charge for anything except a competing use: making collin
available to others in a commercial product or service that substitutes for it
or offers substantially the same features. Using it inside a company, and in
professional services for a client, is allowed by name in the license. Two
years after a version is published, that version is also available under MIT
([0036](docs/decisions/0036-free-to-use-not-to-resell.md)).

The engine, flowscope-core, is Apache-2.0, and the crates the binary links are
MIT, Apache-2.0 or similarly permissive; each keeps its own license.
