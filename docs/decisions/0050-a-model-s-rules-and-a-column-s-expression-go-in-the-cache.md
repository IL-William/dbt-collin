# 0050. A model's rules and a column's expression go in the cache

Date: 2026-11-02 · Status: accepted · Amends 0011, 0045 and 0005

**Trigger:** read before changing what `Rule` or `Expression` carry, before
adding a list to the cache, or before reading a predicate or an expression text
out of the engine.

## Context

The cache kept a predicate as names only (0011): the columns a `WHERE`, a
`HAVING`, a `QUALIFY` or a join read, each fanned out to every output column of
the model as an edge, with the clause's text nowhere. Shopify's
`int_shopify__customer_email_rollup` keeps the customers whose `email` is not
null, and the cache said only that `stg_shopify__customer.email` is a filter on
each of its columns. It kept the role of an edge and not the expression the
engine ranked it by: Jaffle Shop's `stg_payments.amount` was a `transform`, and
the cache did not say it is `amount / 100`. That text is on every `RawEdge` and
was dropped once `classify` had read it.

So two questions could not be answered from the cache: which predicates does a
model apply, and what computes a column. A reader of it can now answer both:
`email IS NOT NULL` on that Shopify model, `amount / 100` on that Jaffle Shop
column, and, for a column built over several CTEs, how many expressions it came
through.

0005 rejected a `uses` field for these in a version 2 of the cache, and was
right to: dbt-lens refuses it. 0045 showed what fits in version 1, a list
beside the edges that a reader not knowing it skips.

## Decision

- **Two lists beside the edges, `rules` and `expressions`**, each left out when
  empty, in the shape of `roots`. The cache stays at version 1.
- **A rule is a predicate**: its model, its clause (`WHERE`, `HAVING`,
  `QUALIFY` or `JOIN`; a `USING (a, b)` is a `JOIN` rule with that text), the
  text as the engine's parser prints it, and the columns its names resolve to,
  as `from` and `from_col` of an edge do. They are placed by the same function
  that places the names of the indirect reads, at the same site, so a rule and
  an indirect edge cannot disagree about where a name leads. A name the engine
  cannot place is left out. The text keeps the names as written, a CTE's alias
  included; the columns are the real relations'.
- **Rules only under `--indirect`**, as the indirect edges are, and wherever the
  compile is not set aside: a degraded one writes every rule of its SQL, as it
  writes the indirect edges of the columns its table shares. Every `SELECT`'s
  `WHERE` and `HAVING` is a rule, a subquery's included, one in a select list
  too. A rule lists the names placed in its own `SELECT`, and for a predicate
  holding a subquery also the names of the subquery's select list, each in the
  scope that wrote it; the subquery's own `WHERE` is a rule of its own, with its
  own columns. Two rules the same in clause, text and columns are written once;
  the same text over two relations is two rules. The order is the walk's: the
  outermost `SELECT` first, then the ones it holds, and inside a `SELECT` its
  joins, `WHERE`, `HAVING` and `QUALIFY`.
- **An expression is a column's**: one entry for each output column whose
  direct edges include a cast, a transform, an aggregate or a window. `kind` is
  the loudest of those edges (0009); `text` is the expression that ranked it,
  cut at 500 characters with `cut: true`. Of two as loud on different paths the
  greater text stands (0032); along one path the walk holds the outer one. And
  `steps` is the most expressions the walk met on any path back to a relation,
  a copy or a rename counting for none. A column that is a copy, a rename or a
  hash has none, its edges saying all there is, nor has a model whose compile
  is set aside. Written with or without `--indirect`.
- **The count has a ceiling of 32.** A recursive CTE meets its own expression
  once more on every lap, and the walk keys its states by the count to keep
  every path's, so without a ceiling it would not end. A column that reaches 32
  comes through a recursive CTE and its `steps` is not a path's length.

## Rejected

- **The expression on every edge.** An edge is replicated for each input of the
  expression, so `concat` of five columns would say it five times, and the
  cache grows by the text of every edge when what a reader asks is the text of
  a column.
- **Every step in order**, the chain of expressions from the output to the
  relation. It answers more, and this record does not know yet how often it is
  needed: `steps` is the measure that will decide it, a column with one step
  being said whole by its text.
- **The fork's `node.filters`** for the rules. It splits a predicate on `AND`
  and localises it to a relation, which gives the columns a predicate reads and
  not the predicate, and it never sees a `QUALIFY` (0026).
- **A version 2 of the cache** (0005).
- **A `uses` field on the edges**, which is the same fan-out as 0011 with the
  text added on every one.

## Consequences

Measured on the two public fixtures, before and after, the edges, the roots and
the totals of the report do not move, with or without `--indirect`: the cache
less its two lists hashes the same.

| | Jaffle Shop | Shopify |
| --- | --- | --- |
| cache, bytes, before and after | 4 468 and 5 864 (+31 %) | 1 042 610 and 1 224 507 (+17 %) |
| the same with `--indirect` | 10 579 and 12 972 (+23 %) | 6 848 082 and 7 205 134 (+5 %) |
| rules, with `--indirect` | 4 | 372 |
| expressions | 10 | 1 003 |
| of them with `steps` above 1 | 0 | 373 |

Shopify's expressions are 640 aggregates, 299 transforms, 45 windows and 19
casts; the median text is 45 characters and the longest 229, so none is cut.
`steps` reaches 5 there: 630 have one, 283 two, 85 three, 4 four and 1 five. A
run over Shopify with `--indirect` takes 0.25 s before and 0.26 s after.

The count sits in the walk's key, so a node two paths reach with different
counts is walked for each. On the fixtures that costs nothing measurable, but a
synthetic shape of many CTEs each reading the one before twice multiplies the
states; it is a residual in `docs/next-steps.md`, with the fix to try.

The text is the parser's print, not the compiled file's bytes: keywords come out
in capitals and spacing is normalised, the names as written. A reader wanting
the file's own text is a span away, and not done.

dbt-edith reading both lists, to show the rows a model keeps on its card and
the expression and its steps beside a column, is its own change. Until then it
skips them, as it skips `roots`.
