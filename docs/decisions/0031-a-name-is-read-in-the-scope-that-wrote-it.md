# 0031. A name is read in the scope that wrote it

Date: 2026-10-10 · Status: accepted · Amends 0009

**Trigger:** read before changing which text the walk reads names out of, or
where it reads a window's keys.

## Context

Where the engine gives a derivation no column, `engine.rs` reads the names
out of its text against the columns of the relation or CTE it hangs off, and it
reads a window's keys, which the engine never resolves, out of the `OVER`
clause. Both read the text the walk carried, which 0009 makes the loudest
expression met on the path. The loudest can come from a scope further out,
whose names are another scope's columns: a window over a CTE joining two tables
on `k` had its key credited to both, and a hash over a union read the columns
of branches that write null there. The walk also read each node once, with
whichever text reached it first.

## Decision

The walk carries two texts. The loudest still decides the role, as 0009 says.
Names are read out of the other: the expression of the edge last followed, or,
across a plain copy that has none, the one before it. A node is read once for
each text that reaches it. A window's keys are read at the hop whose edge
carries the window, against the scope its column comes from: from a table, the
edge is made there; from a CTE, the walk goes on from the CTE's columns of those
names.

## Rejected

- **Binding a qualifier to its relation by reading the FROM clauses.** An alias
  is reused across the CTEs of a dbt model, and a statement wide map would bind
  it to the wrong scope: a guess, in the one module that must not make one.
- **Reading every text met on the path.** The outer ones are exactly the ones
  that name other scopes' columns.

## Consequences

On a 3341 model project, with the engine at 0028's sixth patch, the cache loses
38 edges and gains 4; 5 edges become inferred. The 38 are a window's keys
credited to tables its CTE only joins, and hash and transform inputs credited
to union branches that write null for the column. The 4 are keys a window reads
through a CTE that renames them. The 5 are reads of names a union does not
project, which the old reading had found in a branch's list and 0019 now
bridges as inferred. Under `--indirect` the cache loses 35 edges and gains 3.
