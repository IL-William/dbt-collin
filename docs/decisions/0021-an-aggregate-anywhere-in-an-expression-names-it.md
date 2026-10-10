# 0021. An aggregate anywhere in an expression names it

Date: 2026-10-10 · Status: accepted · Amends 0009

**Trigger:** read before changing how `role::classify` recognises an aggregate.

## Context

The classifier read the call an expression starts with. `sum(amount)` was an
aggregate; `coalesce(sum(amount), 0)`, `(sum(debit) - sum(credit))` and
`iff(count(distinct c) = 1, max(c), null)` were transforms, although each is
an aggregate with something done to its result. 0009 already ranks an aggregate
above the arithmetic round it when the two sit in different CTEs; within one
expression the aggregate was simply not seen.

## Decision

An expression that calls an aggregate anywhere, a whole word from the list
followed by a parenthesis and outside quotes, is an aggregate. A window is
still tested first and still wins, whatever it wraps or is wrapped in; a cast of
an aggregate is an aggregate. `leading_call` is left as it is, since the engine
uses it to spot a column named after its own function.

A column read in the condition of a `CASE` inside a `SUM` is labelled aggregate
too: it is inside the aggregate, which is what the README's `aggregate` says,
and it would be the next finer role, not a wrong one.

## Rejected

- **Reading only the outermost call, as before.** It hid the fact a reader
  opens the graph for behind a `coalesce` with a default.
- **Keeping arguments and conditions of an aggregate apart.** A finer claim the
  expression text supports only with a parse of it; no role for it exists.

## Consequences

On a 3341 model project, 48 edges in 7 models go from `transform` to `aggregate`,
each read against its expression. Nothing else moves.
