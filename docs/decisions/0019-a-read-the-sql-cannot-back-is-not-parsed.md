# 0019. A read the SQL cannot back is not parsed

Date: 2026-10-10 · Status: accepted · Amends 0003, 0008 and 0010

**Trigger:** read before following a derivation past a CTE to the relations it
reads, before retracting a compile's column list, or before publishing an
inferred edge from a model whose other edges are parsed.

## Context

Three macros of this project read the warehouse at compile time: a staging macro
asked to include its source's columns, a satellite generator, and a union of
relations. When the relation is missing from the warehouse the compile targets,
they find nothing, and the SQL they write lists no columns where it should list
the relation's. A stage's CTE then projects its three derived columns and
nothing else, and the next CTE hashes eighty names from it. The warehouse would
reject that: the compile cannot run.

The engine checks a name against a table it was handed and never against a CTE,
so it raised nothing. Inside `TRIM` it sees no name at all and hangs the hash
off the CTE, and the walk then followed the CTE's relation level edge down to
the table and mined the text against it: every name the table has became a
parsed edge. The walk's own comment called that how a hashdiff is written. It
is how a hashdiff is written when the compile could not see its source.

The same mining ran in valid SQL too, wherever an expression hung off a CTE
named a column the CTE does not project: it credited the name to every relation
under the CTE having a column by it, one key to seven relations in one case.

## Decision

The walk no longer follows a derivation from a CTE to the relations it reads.
What the CTE projects is followed through its own columns, as before; what it
does not is not an edge.

`engine.rs` reports such a name as an unbacked read, with the CTE, the output
whose walk met it, and every relation under the CTE whose list has the name:
where the engine saw no name, from the text of the derivation hung off the CTE;
where it did, from the column it made for the CTE with nothing feeding it. Not
for a name the reading scope projects itself, a lateral alias, and not for a CTE
that is a `*` over a relation, where the list is short rather than the SQL
wrong.

`lineage.rs` decides:

- The edges the SQL supports stay parsed, with their roles. One stray read does
  not condemn what the rest of the compile says.
- An unbacked read with exactly one relation having the name becomes an edge
  from that relation's column to the output, labelled `inferred`: the column
  the SQL was written to read, but no SQL that runs reads it. With none, or
  several, nothing is chosen. A model can now carry both rungs; each edge says
  its own, and the report counts the inferred ones on the entry.
- The compile's column list is retracted, as 0008 retracts one it does not
  trust, when the warehouse or the YAML can stand in for it downstream. With
  neither, it stays: the reads say the SQL cannot run, not that the names it
  projects are wrong, and retracting would leave nothing at all.
- A model with an unbacked read is not clean, and the report lists the reads.

0010 read its two examples by hand and concluded the YAML was stale. It was
not. The preparation model that documents 88 columns and selects five is a
stage whose compile cannot run; the satellite that documents 277 and selects
four runs, and is short for the same reason one step earlier. 0010's rule
stands, a short compile against the YAML being a finding and not a verdict, and
its examples were this.

## Rejected

- **Keeping the mining.** It publishes SQL that cannot run as parsed, and in
  valid SQL it names relations the expression never read.
- **Setting the whole model aside.** Its supported edges are right, and in 32 of
  the 34 cases with a list to fall back on, that list is the YAML the stage was
  meant to match.
- **Retracting the list whatever stands in for it.** One model here has no
  catalog entry and no YAML; retracting left its child 12 fewer edges and
  gained nothing.
- **Choosing among several owners.** A guess, which 0003 forbids.
- **Inferring the columns the compile dropped from the YAML or from the
  children.** A different claim, with its own evidence, for its own record.

## Consequences

On a 3341 model project: edges 100 744 to 100 785, parsed 93 958 to 93 055,
inferred 6 786 to 7 730. 865 edges from compiles that cannot run keep their
triple and go from parsed `transform` to `inferred`. 39 parsed edges the mining
made in valid SQL are gone; those read by hand credited a name to a relation the
expression did not read, and one window key, reached only that way, is lost.
27 edges become `window` and one `aggregate`, the walk now carrying the
expression the CTE's own column has. One model whose compile read nothing its
CTE had now has no edge at all, is silent, and gets 73 inferred edges by name.

1695 unbacked reads are named in 71 models, 35 of them trusted. `thin` goes
from 42 models to 22, the other 20 being these stages. 15 models leave the fault
list, their `UNKNOWN_COLUMN` having come from reading a stage's short list, now
replaced by its YAML; 3 enter it for their unbacked reads. The lost columns of
trusted models go from 428 to 354. The cache stays at version 1.

## Amended the same day

A `VALUES` list's columns and a `LATERAL FLATTEN`'s six are not such reads,
though the engine gives the first no columns and the position columns of the
second nothing to feed them: the SQL backs both. On a 3507 model Snowflake
project, compiled on its own environment, unbacked reads go from 6 to none and
the cache does not move.
