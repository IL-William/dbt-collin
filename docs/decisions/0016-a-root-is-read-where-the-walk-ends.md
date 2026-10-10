# 0016. A root is read where the walk ends

Date: 2026-10-10 · Status: accepted · Amends 0006

**Trigger:** read before changing what `Resolved::roots` holds, or before
counting a column with no edge as a gap or as nothing to find.

## Context

A root is an output column the SQL builds from literals, so it has no parent to
find and must not count as a gap: `current_timestamp() as loaded_at`. Commit
94e519c, merged in 570c9ba, decided what makes one, in its message only: a column with no edge,
an expression of its own, and no known column named in that expression. It also
decided the other half, that a column with no expression is never a root on
that evidence alone, since it was copied from somewhere and losing its edge is a
loss. Both halves were right, and the first read the expression in the wrong
place.

The engine gives a column copied by name or through `select *` no expression.
A dbt model sets its literals early, `'SRC' as record_source` in a staging CTE,
and copies them out through the rest of the chain. At the output the column
shows nothing, so it was called lost. The engine hangs a projection that reads
no column off the base relation of its scope, carrying the text it was computed
from, so the walk back does reach `'SRC'`; the root test never looked there.

On a 3341 model project, 1358 output columns of trusted models had no edge and
were not roots. Read one by one against the SQL, most were literals set in a CTE.

## Decision

A column with no edge is a root when its own expression names no column, as
before, or when the walk back from it ends only at literals and a guard holds.

The walk ends at a literal when it reaches a relation or a CTE carrying an
expression that names none of its columns, or a column computed in a CTE that
reads nothing, a date spine or a CTE of constants. It ends at a loss when it
reaches a copy that carried no expression, a column its CTE never had, a column
computed in a CTE that reads a relation and fed by nothing, which is a read the
engine did not resolve, or a relation whose columns nobody knows.

The guard: every name an expression on the way reads must be a column the walk
reached below that expression. The names are those of any known column, any
column the engine placed, any text the engine could not resolve, and the column
part of every qualified name. It stays for good. The engine reads no column
inside `TRIM`, `SUBSTRING`, `col:path` and a dozen other forms and hangs such a
projection off the base relation as if it read nothing, and that relation can be
a CTE of constants: the text still names the column, so the guard sees the read
the engine lost.

This lives in `engine.rs`, beside the walk, as a second walk over the columns
the first one left with no edge: whether the SQL builds a column from literals
is a reading of the SQL, and the walk that makes edges is left exactly as it
was.

## Rejected

- **No edge and no expression means a root.** 94e519c measured it at 2435 roots,
  and it hides every copy that lost its edge.
- **The leaf alone, with no guard.** On this project it adds 221 roots, 168 in
  trusted models, and every one read by hand hides a real read: a column inside
  `trim(...)`, the value of a `FLATTEN`, a model's own column read back.
- **Waiting for the engine to read inside those forms.** The fork will, and the
  guard costs nothing once it does; an engine that drops one more form would
  otherwise turn a loss into a root without a word.
- **A lexical rule, no identifier at all in the text.** It needs closed lists of
  type names, date parts and niladic functions, and `year(day_start)` over a
  date spine is a root with an identifier in it.
- **Letting roots soften the silence of 0008**, a model whose every column is a
  root being no longer silent. A different question, left for its own record.

## Consequences

On the same project the cache is identical, default and `--indirect`. In trusted
models, 900 more columns are roots and the lost ones go from 1358 to 458; 158
more roots are in compiles the plan set aside, which count as they always have.
`columns_root` goes from 551 to 1582 and the independent denominator from 95 399
to 94 368, and independent coverage from 93.7% to 94.7%. Twelve of the new roots
had an inferred edge, being columns of a compile the plan set aside that
inference matched to a parent.

Every one of the 1058 new roots was read against the SQL: 990 are defined by
literals only, and the 68 that name something read a date spine, a CTE of
constants or a union of literals. The losses the audit had established by hand,
a hash key built through `TRIM` among them, stay losses.

A column whose hidden read names no known column and no column the engine
placed, and is not qualified, would still pass the guard. None was found here.

## Amended the same day

Naming the lost columns (0017) showed 29 of them in one model defined as
`cast((0) as decimal(19, 4))` and the like: literals, in a CTE that joins several
relations. The engine hangs a projection reading no column off the scope's
driving relation, and decides which relation drives from whether it was joined
anywhere in the statement, not in that scope. A CTE driven by a relation joined
elsewhere gets its literals hung off nothing, so the column has nothing feeding
it, which the rule above read as a read the engine did not resolve.

The two differ in what the expression names, which is what the guard already
reads. A column fed by nothing whose expression reads no name is a literal,
whatever its CTE reads; one reading a name is a loss. The vocabulary also takes
the column an `UNKNOWN_COLUMN` names, so that a name the engine said it could
not find is never read as no name.

On the same project: 37 more roots, every one defined by literals, 29 in trusted
models; the lost columns of trusted models go from 458 to 429; `columns_root`
from 1582 to 1602. The cache is identical.

## Amended again, the same day

A `VALUES` list is a derived table the engine gives no columns: `select
column1 as a from values (1), (2)` reads a name the list never had, so the walk
ended at a phantom and the column was lost. Its rows are written in the SQL. A
walk that ends at a derived table whose body is a `VALUES` list, read off the
syntax tree, and that reads no relation now ends at literals, so the column is
a root. A derived table that is not one, missing the name, is still a loss.

On a 3507 model Snowflake project 4 columns of one model, a list of claims to
exclude, go from lost to roots; the cache is identical.
