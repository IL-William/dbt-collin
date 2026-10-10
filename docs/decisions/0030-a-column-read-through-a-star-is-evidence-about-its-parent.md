# 0030. A column read through a star is evidence about its parent

Date: 2026-10-10 · Status: accepted · Amends 0004 and 0008, notes 0010 and 0020

**Trigger:** read before changing what the engine is handed for a relation,
before resolving a model more than once, or before publishing an edge out of a
column no list the engine was given has.

## Context

The engine expands a `select *` from the column list it is handed. When that
list is short, because the parent's table is older than its code, or its compile
was thinned by a macro that found nothing to introspect, a later read of the
missing name is a phantom of the CTE: no edge, and no issue either. The same
read written straight off the relation, `t.x`, raises `UNKNOWN_COLUMN` and keeps
its edge. One meaning, two outcomes, decided by whether a star sits in between.

## Decision

A trusted compile's phantom is extended when the SQL alone says where the name
comes from and another list says it is there.

- **The anchor is the SQL's.** The phantom's CTE takes `*` from exactly one
  relation. A star over a join, a list the CTE writes out, or a name the CTE
  defines itself anchors nothing.
- **The list only vetoes.** The relation's node must have the column in another
  of its lists: its table, its own compile even when set aside, or its YAML.
  Where a name comes from is never read off a list.
- **One more call, and nothing else moves.** The statement is resolved again
  with those columns added to that relation for that call only. The edges out of
  them are kept, parsed, if the outputs are the same and every edge of the
  first reading survives; otherwise nothing is. The first reading's issues,
  outputs and row choosing reads stand.
- **The store never hears of it.** The column defines nothing downstream, and no
  list is merged: 0004's three sources stay three.
- **The parent is told.** A read that got its edge is filed on the parent under
  `read_downstream_but_absent`, as 0020 files a direct read.

## Rejected

- **Extending a list with every name a schema free pass finds.** That pass
  copies every column a CTE owns back onto the table, aliases included: 110
  wrong edges in one model of a 3341 model project.
- **Letting the list decide attribution.** A stale YAML naming an alias would
  then give that alias a parsed edge out of a table that never had it.
- **Keeping the second reading whole.** It would clear issues and could change
  the outputs, so a child's read would decide what stars below the model see.
- **Stopping the direct reads instead, for symmetry.** They are edges the SQL
  proves; dropping them to match a blind spot helps nobody.

## Consequences

On the same project the cache gains 77 edges, all parsed, 68 of them renames, in
4 models, and loses none; lost columns go from 193 to 117. 80 reads are filed
on 4 parents: three whose table is behind their compile, which has the column,
and one satellite whose compile an empty introspection thinned, which its YAML
lists. 0010 took a thin compile for a documentation finding; a child reading 58
columns through a star over it says the compile is the short side.

A name read inside an expression through such a star, `upper(s.x)` rather than
`s.x`, gets no column from the engine at all, so there is no phantom to anchor
on, and the output is counted among the roots as if built from literals.
