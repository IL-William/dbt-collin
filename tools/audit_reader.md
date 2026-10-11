# Instructions for one blind reader

Given to each reader of an audit, one batch apiece, with `AUDIT_DIR` and
`batch-N.json` filled in. A reader is shown the SQL and the column lists, and
neither collin's answer nor sqlglot's.

---

You are a careful SQL reader doing a blind column-lineage audit. You read
compiled Snowflake SQL by hand and say, for given output columns, exactly which
source columns feed them. Your answers will be compared with two automated
tools, so precision matters more than speed. Do not guess: when unsure, say so.

Work directory, read only except for your answer file: `AUDIT_DIR`.

- Your batch: `batch-N.json`, a list of `{id, model, column}`.
- For each model, `sql/<model>.sql` is its compiled SQL, and
  `sql/<model>.parents.json` gives `own_relation`, the model's own table, and
  `parents`: every relation the model reads, as `DB.SCHEMA.NAME`, with its
  column list, which is what `select *` on it expands to.

Rules:
- **Blind.** Read only `batch-N.json` and the files under `sql/`. Open nothing
  else: no cache, no dump, no report, no sample or compare file, no other batch,
  no answer file but yours. Run no lineage tool, neither sqlglot nor collin nor
  any parser. grep, sed or a short script may help you find your way in the
  text; every verdict comes from your own reading.
- Write nothing but your answer file. No network. The SQL is private: quote none
  of it in your final message beyond what a note needs.

An input of an output column, the column of that name in the model's final
`SELECT` list, is:
- every column of a parent relation whose value is read to compute it, followed
  back through every CTE, subquery, derived table and `select *` to the
  relations in `parents.json`: function arguments, arithmetic, `CAST`,
  `COALESCE`, `NVL`, `IFF`, `CASE` conditions and branches, concatenation,
  hashes, aggregate arguments, a window's `PARTITION BY` and `ORDER BY` keys,
  `WITHIN GROUP (ORDER BY ...)` keys, an `ORDER BY` inside an aggregate call,
  the input of a `LATERAL FLATTEN` whose `VALUE`, `KEY` or `PATH` is taken, a
  scalar subquery's select list. For a `UNION`, every branch's column at that
  position, or of that name for `BY NAME`.
- not a column used only in `WHERE`, `JOIN ... ON` or `USING`, `GROUP BY`,
  `HAVING`, `QUALIFY`, or a scalar subquery's `WHERE`, unless the expression
  reads it too.
- nothing at all for a column built only from literals, `NULL`, or functions of
  no column: `current_timestamp()`, `uuid_string()`, a sequence, a `FLATTEN`'s
  `INDEX` or `SEQ`, `count(*)`. Answer an empty list.

A star expands to the parent's listed columns, or to the CTE's own; `EXCLUDE`,
`REPLACE` and `RENAME` act as Snowflake defines them. A read of the model's own
table, as an incremental model makes, goes in `own_reads`, not `inputs`. Compare
names without case. Give each input as `{"relation": "DB.SCHEMA.NAME" as written
in parents.json, "column": lower case}`, with `"unlisted": true` when the
parent's list lacks it. Dedupe inputs.

Write `answers-N.json` in the work directory, one object per batch item:

```
{"id": 1, "model": "...", "column": "...",
 "inputs": [{"relation": "...", "column": "...", "unlisted": false}],
 "own_reads": [], "unsure": false,
 "note": "one or two sentences: the expression path, and any doubt"}
```

Check that the file parses and has one entry per batch id. Then reply with only
the number of items answered, how many are marked unsure, and any item whose SQL
you think is broken or ambiguous, by id, one line each.
