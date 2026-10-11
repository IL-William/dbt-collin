# Measurement tools

What the precision and coverage figures in
[`docs/next-steps.md`](../docs/next-steps.md) were measured with. They read a
pinned project's manifest and what collin's `dump` example wrote for it, never
the warehouse.

Everything they write names the project's models. Write it outside this tree,
and never commit it: the figures go in a commit message, the names never do
(AGENTS.md, 0007).

Run each with Python isolated from the working directory, `-I`. sqlglot is the
only dependency, pinned to the version the figures were measured with:

```
D=/tmp/collin/after/dump                    # from: cargo run --release -p collin-core --example dump
M=/path/to/pinned-copy/target/manifest.json
uv run --no-project --with sqlglot==30.18.0 python -I tools/precision.py $M $D /tmp/collin/precision.json
```

| Tool | What it says |
| --- | --- |
| `precision.py` | Every parsed edge against sqlglot's reading of the same SQL, handed the same column lists. `DIALECT` sets sqlglot's dialect, `snowflake` by default. It lists only the models where the two disagree. It does not work on BigQuery: there it agrees on almost no edge, plain copies included. |
| `classify.py` | Every column without an edge, by cause. |
| `roots.py` | Each root followed through its CTEs: a root reaching a table is a candidate false root. |
| `bq_causes.py` | The columns without an edge of a BigQuery project, by first cause. |
| `audit_kit.py`, `audit_kit_hard.py` | Draw a blind audit's sample of output columns, write each model's SQL and parents beside it, and split it into batches. The second draws harder columns: deep models, many inputs, loud kinds. |
| `audit_reader.md` | The instructions each blind reader is given, one batch apiece. |
| `audit_compare.py` | Each column's blind reading against collin's edges and sqlglot's. |

A blind audit, as `docs/next-steps.md` reports one:

```
python3 -I tools/audit_kit.py $M $D /tmp/collin/audit 20261011
# one reader a batch, given audit_reader.md, shown only sql/ and its batch
python3 -I tools/audit_kit_hard.py $M $D /tmp/collin/audit 20261012
uv run --no-project --with sqlglot==30.18.0 python -I tools/precision.py $M $D /tmp/collin/precision.json
python3 -I tools/audit_compare.py $M $D /tmp/collin/precision.json /tmp/collin/audit
```
