# 0051. A column with an edge and a lost name is partial

Date: 2026-11-04 · Status: accepted · Amends 0017

**Trigger:** read before changing what the walk records where it gives up on a
name, adding a reason to `Partial`, or deciding how a column that kept an edge
and lost another is called.

## Context

0017 named the column that comes out with no edge at all, and said in its
Context what it left open: a column that keeps one input and loses another is
not named. It has an edge, and nothing compares the names its expression reads
with the edges it got. To a reader of the cache, and to dbt-edith, such a column
looks complete, and a column that looks complete is read as one nothing was lost
from.

The walk already knows the output column it is walking for at every place it
gives up on a name, and drops the name without a word. An engine hangs a window
off a table and the key the table has no column for goes with it: in
`row_number() over (partition by a.id order by b.w)` over a join, `a.id` is an
edge and `b.w` is nowhere. The `Ends` guard that 0017 built for a lost column
cannot be reused here. A column of a base relation returns early, before it is
labelled, so the guard would flag every column.

## Decision

**Record the loss where it happens, with the column and the reason, and publish
it as one more additive list.** `partial`, beside `edges`, `roots`, `rules` and
`expressions`, each entry `{node, column, reason, name}`, left out when empty.
The cache stays at version 1, as 0045 and 0050 left it.

The walk pushes a record at each place it gives up, and weighs them after it,
once the edges of every column are known. Four of the five reasons are a name
dropped; `steps_capped` is not, and is the one reason where the edges are all
there. The reasons are:

| Reason | Where the walk gave up | `name` |
| --- | --- | --- |
| `name_unresolved` | an expression the engine hung off a relation that has a column list reads a name the list lacks | the name |
| `relation_unknown` | the same, off a relation handed over with no list, from the text hung off it or from a window read by its text | the column part of the qualified name |
| `cte_lacks_name` | a scope reads a name from a CTE that does not project it: the `UnbackedRead`, written beside it | the name |
| `window_key_unbound` | a qualified partition or ordering key whose qualifier binds a source that lacks the column, from the keys read in the `SELECT` that writes them (0041); or a qualified name in a window's text, read against a relation or CTE that lacks it | the key |
| `steps_capped` | the count of expressions reached the ceiling of 32 (0050) and an expression was met after it: the edges are all there, and the chain of expressions that computes the column is longer than the cache's `steps` says | the column the walk was entering |

- **For the text and key reasons, only a qualified name is claimed, and a bare
  name never is.** (`cte_lacks_name` is the SQL reading what its CTE does not
  have, and names what it read, bare or not.) What a text
  is claimed for is the column part of a two-part name, `o.amount`, and a window
  key only when it is qualified and its qualifier binds a source that lacks the
  column. A bare word cannot be told from a keyword, a date part (`dateadd(day,
  1, a.x)`), a type (`a.x::date`), a lateral alias or a function's argument,
  however a statement happens to name its columns, so none is claimed. Nor is
  part of a longer path, which may be a struct's field or a variant's key
  (`t.rec.field`, `a.payload:customer.ts`), nor a name whose qualifier is itself
  a column of some relation. A nested query is skipped whole, as are the
  parameters of a lambda: a correlated predicate reads the columns of tables
  the text knows nothing of, and `e.v` in `e -> e.v > 0` is no relation's. The
  limits are plain. A bare name is never claimed, so a window key written
  without its qualifier, or a bare name in a text, that a relation lacks is not
  named. A quoted name read in a text (`b."NOPE"`) is never claimed, though as
  a key it is. A name equal to the last part of a relation's name is never
  claimed. A subquery's own names are never claimed. A qualifier named like any
  lambda parameter of the same text is passed over, wherever it is written:
  `filter(a.arr, a -> a.v > 0), a.nope` loses `a.nope` too. The record errs towards
  silence and never towards a claim it cannot back.
- **A name another relation gave an edge for is not lost.** The text the engine
  hangs off one relation can name the columns of the next. For each output the
  walk keeps every column node it popped, and the edges the column got say the
  rest: a name among either is dropped. This is not applied to `cte_lacks_name`,
  which is the SQL reading what its CTE does not have, nor to `steps_capped`,
  which is about a count.
- **Only a column with an edge.** One with none is a lost column and stays in
  `lost_columns`. The check is against the direct edges the cache gets, so a
  column whose only edge the pass dropped has nothing to be partial of.
- **Only where the SQL is trusted**, as `lost` is: `Plan::Parsed`, and
  `Plan::PerColumn` for the columns it shares with the table. A compile set
  aside gives none. A model reading only its own relation gives none either, with
  no guard of its own: every edge it has is a read of itself, which the pass
  drops (0039), so it has no column with an edge to be partial of.
- **One entry for a column, a reason and a name**, in the order of column,
  reason, name, so that the cache is written in one order.

### Where it shows

- **The cache**: `partial`, as above.
- **The report**: `partial_columns` on a model's entry, each column once with its
  `gaps` (a reason and a name), `columns_partial` in the totals, and a finding,
  `lineage_collin_partial`, owner collin, worded as `lineage_collin_lost` is
  (0040) and true of both kinds: a name dropped, or an expression chain cut at 32.
  It leaves out `cte_lacks_name`, which the project finding
  `reads_a_name_its_cte_lacks` already says and which the project is to fix; the
  entry stays in the cache and in `partial_columns`. A model with a partial column is in `models`, and so is not read as
  clean.
- **The CLI** prints `N columns of trusted models partly traced, named in the
  report` beside the line for the lost ones, when there are any.
- **`examples/dump.rs`** writes the engine's list as `partial` and the cache's
  as `published_partial`.

### What is out of scope

Sites that do not know the output column are not covered, and the record is not
a proof of completeness:

- the clause-level placements, which count `unplaced_qualify`, `unplaced_join`
  and `unplaced_filter` and have no output column to name;
- the statement-level `merged_scopes`;
- the derived tables and union branches the engine drops before a node exists;
- the derivation the engine hangs off a CTE when no column of the scope carries
  its text: it is read for the CTE's own columns and a name the CTE lacks is
  named only when it is an unbacked read;
- a name written bare, and a name in a path or under a qualifier that is a
  column, as above.

## Consequences

Measured on the two public fixtures with the committed caches, with and without
`--indirect`, and on lineage-library, the pass gives no partial column: the
expected caches do not move. Jaffle Shop has 0 entries and Shopify has 0, for
each of the five reasons, and lineage-library's 221 facts raise none. The shapes
that give one are in the tests, each reason by one.

dbt-edith reading the list is its own change. Until then it skips `partial`, as
it skips `roots`, `rules` and `expressions`. What it would do with it: show
`(partial: name)` in the compact `from` of such a column, add a count to the
`column lineage:` line on its stderr, mark the column on the canvas, and give the
skill a cue beside the ones for `(cut)` and `32+`: a partial column means collin
gave up inside that model, so read its SQL with `--describe name --sql` or
`--compiled-sql` and trace the rest by hand.

The lineage-library observer (`lineage_library_reading`) is left as it is, so
that the hand-written known file is not regenerated in this change. With no
entry on its facts today, `partial` lines would move nothing; the regeneration
stays a residual in `docs/next-steps.md`.

A reason pair can name one name twice. A key `w` of a window over a join, read
by its qualifier in a `SELECT` whose source lacks it and by its text against a
relation with no list, is `window_key_unbound` and `relation_unknown` both. Each is true of its relation,
and the entries are not merged.

## Rejected

- **Comparing, after the walk, the names each expression reads with the edges
  the column got** (the idea in 0017). It needs the base-relation names among
  the labels, which the early return keeps out, and a second pass over every
  output.
- **Folding the partial columns into `lost_columns`.** A lost column has
  nothing, a partial one has edges a reader may still trust. Two lists keep the
  two facts apart.
- **A version 2 of the cache** (0005). The list is additive.
- **A `loop` reason for a state reached twice.** The walk keys a state by its
  node, texts and count, and a state popped a second time was walked the first
  time: nothing was dropped, so there is nothing to name.

## Cost

No extra pass. A record is pushed only when a site has a qualified name to
claim (the text is scanned once for them, against sets built before the walk),
or a CTE read or a cap to name, and the walk keeps its popped column nodes by id
only for a column that pushed one. The weighing is once per record after it. The
figures on the fixtures are above, all nil, so the cache does not grow on them. A run
of `generate` over Shopify's 240 models takes 238 ms, as before, the median of
five runs of each binary taken in turn on one machine.
