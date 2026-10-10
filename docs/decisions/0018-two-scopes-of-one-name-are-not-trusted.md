# 0018. Two scopes of one name are not trusted

Date: 2026-10-10 · Status: accepted · Amends 0008

**Trigger:** read before changing what makes the plan set a compile aside, or
before relying on the engine to keep two derived tables or two CTEs apart.

## Context

The engine keys a derived table, `(select ...) as t`, and a CTE by their name
within a statement, and keeps the first node it builds for a key. When one
statement gives the same name to two derived tables, or to two CTEs in nested
`WITH` blocks, the second scope's columns become the first's. An output of
either can then come out fed by both, and the engine raises nothing: the
compile looks sound, and its edges cross.

A dbt macro writes exactly this. A point in time model over several satellites
opens one `(select distinct ...) as v` per satellite. On a 3341 model project, 8
models do it, with 2 to 12 derived tables named `v` each, and an audit of this
project found 724 of their engine edges crossed between satellites. Today all 8
are set aside anyway, because an unrelated leak makes their output list wrong,
so none of those edges is published. The day that leak is fixed, they would be.

## Decision

`engine.rs` reads the SQL's syntax tree with the engine's own parser and names
every name one statement gives to two derived tables or to two CTEs, keyed as
the engine keys them: by statement, and by the name as written, so `t` and `T`
are two names and a derived table and a CTE of one name are two scopes. It
reports this as `merged_scopes`, a fact about the SQL, and changes nothing.

`lineage.rs` sets such a compile aside, as it does a degraded one: its edges go
out inferred, or not at all. The report lists the names on the model's entry,
and counts the models.

A canary test asserts that the engine still crosses two derived tables of one
name. When the engine keys them by occurrence, it fails, and the plan no longer
has to set these compiles aside; the check stays, as a guard.

## Rejected

- **Setting `approximate`.** It feeds the report and not the plan, and making
  the plan read it would set aside every model with an `UNKNOWN_COLUMN`.
- **An issue minted by collin in `issues`.** That list carries the engine's own
  words, and this is not one of them.
- **A derived table fed by several relations.** One derived table over a union
  is one scope, read right.
- **Folding the case of the names.** The engine does not, and `t` and `T` read
  right.
- **Patching the walk.** The crossing is in the engine's graph before the walk
  starts; the walk follows it faithfully.
- **Sparing two scopes with identical bodies**, which cross harmlessly. None
  here, and not worth what it costs to tell.

## Consequences

On the same project 9 models are named, the cache is identical, default and
`--indirect`, and the fault list keeps its 495 models. The 8 point in time
models were already set aside. The ninth defines one CTE twice in nested
`WITH` blocks; it published no edge, so it only moves from parsed to
unresolved, and its one lost column leaves the count, 429 to 428.
