# 0045. A root goes in the cache with its value

Date: 2026-10-15 · Status: accepted · Amends 0016

**Trigger:** read before changing what `Resolved::roots` carries, before adding
a field to the cache, or before drawing an edge into a column built from no
column.

## Context

0016 tells a root, a column the SQL builds from no column, from a column whose
lineage was lost, and the report counts the roots apart. The cache heard of
neither: it holds edges, and a root has none. A reader tracing a column back in
dbt-edith reached a column with no edge into it and saw it badged `raw`,
whatever it was: a source's own column, `current_timestamp() as loaded_at` set
in a staging model, or a column whose lineage collin lost. On a large real
Snowflake project the commonest root is `current_timestamp()`, set in one
staging model after another.

Lineage tools do not settle it. OpenLineage's column facet maps an output to
the input fields it is computed from, and a literal has none, so it says
nothing of one. sqlglot ends its walk on a leaf holding the expression itself,
`CURRENT_TIMESTAMP() AS LOADED_AT`, where a copied column ends on its table.

## Decision

- **The cache lists the roots beside the edges**, as `roots`, each with its
  model, its column and `expressions`: what the SQL sets it to. The cache stays
  at version 1 (0005): a reader that does not know the field skips it, as it
  skips `producer` (0034), and dbt-edith does.
- **The value is the literal, read where it is set.** Along each path back from
  the output the walk follows copies, renames and casts down to the literal, so
  one copied out of a CTE three deep shows `current_timestamp()`, and a
  package's typed null recast on the way out shows `CAST(NULL AS TEXT)`. One
  text for each branch of a union that sets it differently, as the engine
  prints it.
- **Said whole or not at all.** `expressions` is empty when:
  - something on the way does more than cast: a date spine's date is an
    expression over numbers, and no literal is its value;
  - the engine read a union in part. It keeps the first branch of a union of
    selects with no `FROM` and drops the others without a word, and it drops a
    branch's literal when it has nothing to hang it off. A union reaches its
    column once for each branch, and the branches are counted on the syntax
    tree, so fewer arrivals than branches means one is missing. A literal
    arrives once for each distinct text, since the engine hangs one branch's
    literal off every relation that branch reads, a comma join or a `FLATTEN`.
    A name several scopes share counts the largest union among them, and an
    `EXCEPT` or an `INTERSECT` is never read whole: the engine walks both
    sides, and what the right side sets never comes out of an `EXCEPT`;
  - the SQL writes none, as for a column of a `VALUES` list.

  Part of a list would say the column holds only what the other branches set.
  The engine says whether its list is whole, as `complete`, and the pass
  decides what goes out.
- **Only where the edges go out parsed.** A compile set aside is no account of
  its columns, and a degraded one only of those its table shares (0029). A root
  never goes out on a column a direct edge reached. Under `--indirect`, the
  edges of the reads that decide which rows exist reach it, as they reach every
  output column (0011): a reader may not take the roots and the targets of the
  edges to be apart.

## Rejected

- **An edge from a pseudo origin**, `lit:current_timestamp()`. Every count of
  edges would take it for lineage, coverage first, and a released dbt-edith
  drops an edge whose origin is no node without a word, so it would show
  nothing anyway.
- **The report rather than the cache.** A root is not a gap (0006), and the
  cache is what a reader of the lineage reads.
- **The first expression met walking back.** It shows a spine's date well, but
  for the typed null recast on the way out it shows `CAST(coupon AS TEXT)`,
  which hides the null, the one fact a reader came for.
- **The literal the walk ends at, always.** For a date spine that is `0`, the
  first branch of the generator's `select 0 union all select 1`, and only that
  branch, which is all the engine kept.

## Consequences

On a large real Snowflake project, pinned, the edges and the report's totals
do not move, with or without `--indirect`, and the dump differs only by
`published_roots`. Most of its roots go out with their value, and nearly all of
those that do not are in models with a union. Every value told is in its
model's SQL, and for the roots told in a model with a union sqlglot finds as
many values, written its own way. Shopify's cache gains 146 roots, 122 with
their value. Jaffle Shop has none. Cal-ITP's edges and totals do not move
either, and its cache gains 163 roots, 92 with their value. Of the others, 58
are in models with a union, most in three that union dozens of selects with no
`FROM`, where the engine kept the first, and 13 are computed.

Two losses come from counting arrivals rather than branches, and each loses a
value without claiming a wrong one. A union whose branches copy one column from
one CTE reaches it by one edge: on Shopify that is most of the 24. A union
whose branches all set the same literal arrives as one text: on the Snowflake
project, a few values that the first count of edges told rightly, and that a
reading of each branch's own relations on the syntax tree would tell again.
A column that one branch of a union sets to a literal and another copies from a
table has an edge and is no root, so its literal is in no list (0017).

dbt-edith reading `roots`, to badge such a column `generated` with its value
rather than `raw`, is its own change.
