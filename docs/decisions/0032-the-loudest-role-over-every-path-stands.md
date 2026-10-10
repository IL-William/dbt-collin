# 0032. The loudest role over every path stands

Date: 2026-10-10 · Status: accepted · Amends 0009

**Trigger:** read before changing how the walk remembers where it has been, or
how it settles two expressions reaching one column pair.

## Context

0009 gives a column pair the loudest expression met on its path. It said
nothing of two paths reaching one pair, and the walk let the first one decide:
it remembered each node it had passed, so a second path through that node was
cut before its expression was weighed, and the first edge written for a pair
kept its text. Which path came first was the order the engine listed its edges
in, which the SQL does not decide: swapping the two branches of a union swapped
the role.

## Decision

The walk remembers where it has been by the whole state it arrived in, node,
role text and the text it reads names out of, so a node two paths reach is
weighed for each. When a pair already has its edge, a louder expression takes
its place, and of two as loud the greater text, so that the edge no longer
depends on the order of exploration. Every text is the engine's, so the states
are finite and a recursive CTE still ends.

## Rejected

- **Visiting a node again only for a strictly louder text.** A second text of
  the same rank is then still cut before what it reads is read.

## Consequences

On a 3341 model project no edge is added or removed, and 48 take a louder role,
47 of them `aggregate` where one branch of a union sums the column. A run takes
4.6 seconds against 4.2.

Two expressions between the same two columns, one per union branch straight off
one CTE, stay beyond reach: the engine keeps one edge per pair of columns and
drops the other before the walk starts.
