# 0017. A column with no lineage is named

Date: 2026-10-10 · Status: accepted · Amends 0006

**Trigger:** read before changing what keeps a trusted model out of `models`, or
before filling a column the engine left without an edge.

## Context

0006 made the gaps a deliverable: a model is in `models` when there is
something to fix, and absent means clean. A trusted compile could still bring a
column out with no edge and no issue at all. The engine says nothing when a
`select *` over a parent expands from a list that lacks a column and a later
CTE reads that column anyway, and nothing when it hangs a read inside `TRIM`
off the wrong relation. The totals counted such columns anonymously, as
uncovered, and no entry named them.

On a 3341 model project, after 0016 took the literals out, 458 output columns of
trusted compiles had no edge, in 64 models, and 52 of those models were absent
from `models`: listed as clean, and not.

## Decision

The engine reports, for every output column with no edge that is not a root,
where each walk back from it stopped. The report names these columns on the
model's entry, as `lost_columns`, and such a model is not clean.

Three reasons, each a statement of where the walk stopped and none a guess about
why:

- `phantom`: the column is copied from a CTE or a derived table that does not
  have it. The entry names the CTE and every relation it reads, through other
  CTEs, never one picked out: saying which relation of a join the column should
  have come from would be a guess.
- `names_unreached`: an expression reads names the engine never placed, inside a
  form it does not read or unresolved. The names are listed; a qualified name
  that is a table a subquery reads is not one.
- `unresolved`: anything else.

For a phantom, the pass adds where the column list it handed the engine for
each relation came from, warehouse, computed or declared, because a list short
of the column is the usual cause and that is the pass's knowledge, not the
engine's.

Only a compile the plan trusted. A compile set aside is in `models` for that
already, and its edges come from inference, so what its SQL lost is not what the
cache lacks. Nothing is filled: a lost column stays without an edge.

## Rejected

- **Degrading a model that loses more than some share of its columns.** 0008
  refuses a threshold on a verdict, and the edges the compile does give are
  right.
- **Filling a lost column by name match.** It would mix inferred edges into a
  parsed model, and 204 of the relations behind the phantoms here came with the
  project's YAML as their only column list, the list least worth matching
  against.
- **A section of its own, as 0011 gave the reads.** A read that decides which
  rows exist is lineage to be had; a column with no lineage is a thing to fix,
  which is what `models` is for.
- **Naming the one relation a phantom should have come from.** In a join it
  cannot be told, and a wrong name would send a reader to the wrong parent.

## Consequences

On the same project the cache is identical, default and `--indirect`, and so are
the roots. `columns_lost` is 458 in 64 models: 246 phantoms, 168 with names
unreached, 37 unresolved and 7 with two reasons. `models` goes from 443 entries
to 495, and every one of the 52 new entries is there for this alone, one of them
also carrying an issue the engine does not count against a model. Behind the
phantoms, the column lists came from the YAML for 204 relations, from the
compile for 100 and from the warehouse for 87.

A column that keeps one input and loses another is not named: it has an edge,
and nothing here compares the inputs an expression reads with the edges it got.
The README says so.
