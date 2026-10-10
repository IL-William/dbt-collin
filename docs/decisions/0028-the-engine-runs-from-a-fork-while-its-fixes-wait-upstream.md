# 0028. The engine runs from a fork while its fixes wait upstream

Date: 2026-10-10 · Status: accepted · Amends 0002

**Trigger:** read before adding a patch to the engine, before moving the pinned
revision, or before going back to a released flowscope-core.

## Context

An audit of the published lineage against the compiled SQL traced part of what
was missing to flowscope-core 0.9.0 itself rather than to collin. A long chain
of `||`, which every generated surrogate key is, tripped the engine's recursion
guard and lost its first operands. A column inside TRIM, SUBSTRING, POSITION and
a dozen other forms the parser gives their own node was never a source, and no
issue said so: about 400 edges and 163 output columns on a 3341 model project.
A `LATERAL FLATTEN` was taken for a table name.

None of these can be worked around in `engine.rs` short of reading every
expression a second time, which is the resolver 0002 decided not to write.
Upstream has no release carrying the fixes.

## Decision

flowscope-core comes from a fork, `IL-William/flowscope`, branch `collin`, pinned
in the workspace manifest to one commit. The fork holds to three rules.

- **Only patches upstream could take as they are:** fixes that apply
  unconditionally, with no option or behaviour of collin's own, each pinned by
  an invented test in the engine's own suite.
- **One commit per fix, on the release tag,** so that each can be offered alone.
- **Nothing goes upstream without William:** no pull request, no push.

The patches, in the order they are pinned:

1. A chain of binary operators is walked as a chain, so its length no longer
   counts as nesting depth.
2. A column inside a form the parser gives its own node is read: TRIM,
   SUBSTRING, POSITION, CEIL and FLOOR, AT TIME ZONE, IS DISTINCT FROM, the path
   and index accessors, COLLATE, OVERLAY and the rest.
3. `LATERAL FLATTEN` is a relation with the six columns Snowflake documents,
   VALUE and THIS derived from its input.
4. A derived table reusing the alias of another in the same statement gets a
   node of its own, where it shared the first one's.
5. A subquery a predicate reads is analysed against a node of its own and adds
   nothing to the outputs; a derived table without an alias gets a node as one
   with an alias does.
6. A wildcard's EXCLUDE, EXCEPT and RENAME are read when it is expanded.

When upstream releases a patch, the fork rebases onto that release and drops
it. When it has released them all, the pin goes back to a crates.io version and
the fork is left.

## Rejected

- **Working around each gap in `engine.rs`.** It would take a second reading of
  every expression the engine skips, a resolver beside the resolver, and a fix
  upstream would then be doubled.
- **Waiting for upstream.** The losses are measured and the fixes are small;
  the lineage would stay wrong in the meantime for no gain.
- **A `[patch]` pointing at a local checkout.** It builds on one machine, and CI
  runs `cargo test`.

## Consequences

The fingerprint 0007 asks for names the engine as the fork and its commit from
now on, not as a crate version.

Pinned at the first patch, the cache of the same project gains 3 edges, all
`hash`, in the one model whose key runs past the guard; nothing else moves.

Moved to the third, it gains 554 edge pairs and loses 10: parsed edges go from
93 063 to 93 569 and inferred from 8244 to 8282. 109 of the new pairs are the
columns a flattened payload is built from, in the 7 models with a
`LATERAL FLATTEN`; most of the rest are columns read inside TRIM. Columns of
trusted models with no edge go from 353 to 178. 4 edges the old gap had pinned
on the scope's first relation go to the relation the SQL names.

Seeing inside TRIM also shows reads a compile cannot back: unbacked reads go
from 1695 to 1760, and 22 hash edges over them become inferred, as 0019 has it.
One union branch whose own compile turns out to read such names falls back to
its YAML, and under 0027 a candidate known only by its YAML stands aside: 5
inferred edges of that union go with it.

Moved to the fifth, 33 models go from degraded to confirmed, a subquery in a
filter no longer adding its column to their outputs. The cache gains 94 edge
pairs and loses 5, each of the 5 a source or an output such a subquery had
made up; 35 name matches become edges with a role, and 51 reads the engine had
taken for unbacked, beside a derived table with no alias, are gone. As 0018
foresaw, two derived tables of one name no longer cross, so the plan stops
setting their compiles aside, and the 8 point in time models it had set aside
are read, each satellite's columns from that satellite. Two CTEs of one name in
nested `WITH` blocks still merge and are still set aside.

Moved to the sixth, the cache loses 16 edges and gains none: 13 made up where
`* exclude (x)` still expanded the column it excluded next to the expression
that replaced it, and 3 into columns an exclusion had removed. 11 edges take
the role their replacing expression shows.

The six patches were then merged into the fork's `master`, on top of upstream
0.9.2, and the pin moved there, to a commit tagged `collin-engine-1` so that no
rewrite of a branch can take it away. Of upstream's changes since 0.9.0 only
the MSSQL `GO` batch splitter touches the engine: the cache, `--indirect` too,
and the report of the same project are byte for byte what they were. Branch
`collin` stays.
