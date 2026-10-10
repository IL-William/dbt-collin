# 0027. A name several parents have is settled by the reading

Date: 2026-10-10 · Status: accepted · Amends 0003

**Trigger:** read before changing which parent an inferred edge is drawn from,
or before letting a compile set aside by 0008 inform anything else.

## Context

Inference matches an output column to a parent column of the same name. When
several parents have the name, the first in the manifest took it, and the
manifest lists parents by name. Metadata columns sit on most parents of a data
vault, so in a join the pick was alphabetical rather than read, and in a union
every branch but the first was dropped. On a 3341 model project 766 inferred
columns have more than one candidate.

## Decision

`infer_edges` settles such a column in one of three ways, and the report says
which for each column, under `ambiguous_inferred`.

- **`compile`.** The set aside compile has, for this very column, edges from a
  column of the same name, and the parents they come from are taken. 0008 says
  the compile does not describe the object; it is still the only witness to
  which of two same named columns the model was written against. It informs the
  parent and nothing else: the edge stays `inferred` and claims no role.
- **`every`.** There is no reading at all, a compile that did not parse. Every
  candidate is taken, less any known only by its YAML when another is known
  better. The compiles that fail here are unions whose branches an
  introspecting macro left empty, and each branch gives each column.
- **`order`.** Otherwise the first parent in the manifest, as before.

A reading that gives the column under another name names no parent for it: the
rung matches names, and a rename is not evidence of the same named column.

## Rejected

- **Every candidate in every case.** Judged against an independent reading of
  the SQL during the audit, taking them all in join shaped models is right for
  92.1% of edges against 97.0% for the first parent, and 43.8% in the join
  heaviest SQL: the join key and the metadata come from each side.
- **Skipping an ambiguous column and naming it.** 99.0% precise, but it drops
  right picks, and in a union it drops every branch.
- **Parsing the failed compile around its empty branches.** It would be a
  reading the engine did not make.

## Consequences

On the same project the cache goes from 100 790 edges to 101 304, inferred from
7730 to 8244: 629 added and 115 removed. 516 columns are settled by the compile,
235 by `every` and 15 by order. Simulated during the audit on 449 judged picks,
the compile turns 74 wrong parents right and 1 right parent wrong.

`every` draws 701 edges in 13 models. 8 are wrong, all in one model where lookup
joins sit beside a union that did not parse: the lookups carry the metadata
columns and a join key, and nothing without a reading can tell them from a
branch. These are the rows the report asks to be read by hand.
