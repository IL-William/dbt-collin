# 0022. A hash is a role

Date: 2026-10-10 · Status: accepted · Amends 0003 and 0009

**Trigger:** read before adding a role, or before changing what `hash` covers
or where it ranks.

## Context

A Data Vault project hashes: every hub, link and satellite has a key that is a
hash of business columns, and a hashdiff that is a hash of every payload
column. On a 3341 model project, 6708 edges, two thirds of all those not copied
straight through, fed an expression that is one call to a hash function under a
cast. They were `transform`, which claims nothing, and which dbt-lens paints in
its loudest colour: the graph shouted about the least interesting fact in it.

A hash is not a transform in the sense that word is kept for. The value depends
on every input exactly, and on nothing else, and says nothing readable about
any of them.

## Decision

`hash` is a role. An expression is a hash when, once the casts round it are
peeled, `cast(...)`, `try_cast(...)` or a trailing `::type`, and the
parentheses that wrap it all, what is left is one call to `md5`, `sha1`, `sha2`
or `hash` in any of their binary or hex forms, closing at the end of the text.
A hash with something done to it, `coalesce(md5(a), '-1')` or `md5(a) =
md5(b)`, is a transform, and `hash_agg` stays an aggregate.

One role for a key and a hashdiff alike: telling them apart would be reading the
column's name, and a role is read from the expression.

It ranks above `transform`, which it is a named case of, and below `aggregate`
and `window`, which a hash over one of them still is.

## Rejected

- **`key` and `hashdiff` as two roles.** The expression is the same; only the
  name says which, and a name is a heuristic.
- **Counting a hash as a `cast`.** 0003 exists because that would claim the
  value came through unchanged.
- **A hash of anything wrapped in anything.** `upper(md5(a))` and a comparison
  of two hashes are not a hash of their inputs.

## Consequences

On the same project 6703 edges go from `transform` to `hash`, in 167 models,
into 284 output columns, and `transform` drops to 2216. Five edges appear where
the walk, now carrying the hash as the expression that claims most, reads the
columns a key is hashed from.

dbt-lens draws a kind it does not know in its neutral edge colour, on purpose,
so `hash` is readable there as it is; a colour of its own is a change there.
The cache stays at version 1: `kind` was always a free string.
