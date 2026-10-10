# 0023. A QUALIFY key belongs to its own SELECT

Date: 2026-10-10 · Status: accepted · Amends 0011 and 0013

**Trigger:** read before changing how a `QUALIFY` is read, or which relation a
dedup key is credited to.

## Context

The engine's analyzer never visits `QUALIFY`, so collin read it from the SQL
text: each clause to the end of its parenthesis, and every name in it credited
to every relation the model reads that has a column by that name. A model that
deduplicates one CTE and joins another relation sharing the key's name credited
both. On a 3341 model project, 461 of 1657 dedup keys were not in the SQL, and
under `--indirect` they fanned out into some 21 000 edges. 0013 recorded that
271 pairs kept a `dedup_key` the SQL did not give them.

The text reader also read a commented out `QUALIFY`, let the first branch of a
union sweep in the next branch's select list, and missed the commonest form of
all, `row_number() over (...) as rn ... qualify rn = 1`, where the clause names
only an alias.

## Decision

`engine.rs` reads each `QUALIFY` off the syntax tree, the engine's own parser
reading the SQL a second time, in the `SELECT` that holds it. Its scope is that
`SELECT`'s own `FROM` and `JOIN` sources. A qualified name goes to the source its
qualifier binds; an unqualified one to the one source having it. A name the
`SELECT` defines as an alias stands for the names its expression reads. A CTE or
derived table source is followed back to its relations through the engine's
column edges, renames included, and a pair is kept only when the relation's list
has the column.

A name several sources have, or a qualifier that binds to none, is read by
nobody and counted: Snowflake refuses an ambiguous name, so ambiguity means the
scope was misread, and crediting every candidate is the failure this replaces.

## Rejected

- **Placing each clause in the text**, by the CTE whose span holds it or the
  alias after the closing parenthesis. It works for most shapes, and comments,
  unions and derived tables each need a rule the syntax tree gives for nothing.
- **Crediting every source that has the name**, as before.
- **Crediting the first source in `FROM`.** A guess dressed as a rule.

## Consequences

On the same project the default cache is identical. Dedup key facts go from
1681 to 1244: 469 credited to a relation the clause's `SELECT` does not read are
gone, and 32 appear, keys read through an alias or through a CTE column built
from base columns. One name is read by nobody. Under `--indirect` 21 152 wrong
edges go, 835 come, and 272 pairs 0013 left with a broadcast `dedup_key` take
the role their other read gives them, 210 `filter` and 62 `join_key`.
