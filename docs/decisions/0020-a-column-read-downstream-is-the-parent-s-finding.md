# 0020. A column read downstream is the parent's finding

Date: 2026-10-10 · Status: accepted · Amends 0006 and 0010

**Trigger:** read before changing which issues keep a model out of the clean
ones, or before acting on a column a child reads that its parent's list lacks.

## Context

`UNKNOWN_COLUMN` is the engine saying that a column the SQL reads is not in the
list collin handed it for a relation. With implied columns allowed it still
reads the name and draws the edge. The issue went on the child, which marked
the child approximate and put it in the fault list, while what it describes is
the parent: its compile came out short, or its table in the warehouse is older
than its code, or its YAML is behind.

On a 3341 model project there were 1125 such issues. 0019 took 886 of them away
by handing children a short stage's YAML instead of its compile; 240 remain,
223 of them in trusted children.

## Decision

`UNKNOWN_COLUMN` no longer counts as the engine missing SQL. `engine.rs` reads
each one into the relation and the column it names, from the message, which is
all the engine gives; a message that does not read as expected becomes an
entry that can match nothing, so a change of wording upstream can only make the
report louder.

`lineage.rs` files a read on the parent when the child's compile is trusted, the
read drew an edge, and the parent is a model. The entry says which list was
short, `warehouse`, `computed` or `declared`, whether the parent's own compile
has the column, and which children read it. The read leaves the child when the
parent's side explains it: the list was the parent's own short compile, or the
parent's compile has the column and the list was an older table or the YAML.
Otherwise nothing but the child says the column exists, and the child keeps
the issue as well. A read that drew no edge stays on the child: that is the
engine's false alarm, a CTE named like the alias inside it, and indicts no
parent. A read from a source stays on the child too, a source having no entry of
its own to carry it.

The issue itself stays on the child's entry, verbatim: it is the engine's words.

## Rejected

- **Keeping `UNKNOWN_COLUMN` among the codes that mark a model approximate.** It
  never meant the engine missed SQL, and it put 17 children in the fault list
  for their parents' lists.
- **Bridging the parent's column to the grandparent's with an inferred edge.**
  The child's read is evidence the column exists, not of where it comes from;
  that claim needs its own record.
- **Filing every read on the parent, edge or not.** The engine's false alarm
  would indict a parent for a column no child reads.
- **A threshold, a parent listed only once enough children read the column.**
  One trusted read with an edge is already evidence, and 0008 refuses
  thresholds on findings it can state outright.

## Consequences

On the same project the cache is identical. 189 columns are filed on 22 parents:
188 the parent's compile has and its warehouse table does not, one only the
child says exists. 17 children leave the fault list, which goes from 483 models
to 466; no parent enters it, all 22 being already listed as degraded. 24 reads
drew no edge and stay on 13 children.

The report says, for the first time, which dev tables are older than the code
that builds them, column by column.
