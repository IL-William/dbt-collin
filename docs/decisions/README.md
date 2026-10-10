# Decisions

Why collin is shaped the way it is, and what was rejected along the way. The
README says what collin does for the person running it; these files say why it
is built this way, for whoever changes it next.

| | Decision | Read it before |
| --- | --- | --- |
| [0001](0001-lineage-from-the-compiled-sql.md) | Column lineage comes from the compiled SQL, never from a warehouse | adding a driver, a credential, or a dependency on a dbt Platform account |
| [0002](0002-flowscope-is-the-engine.md) | flowscope-core is the engine, behind an interface that hides it | touching `engine.rs`, or proposing we write our own resolver |
| [0003](0003-every-edge-says-where-it-came-from.md) | Every edge says where it came from | emitting an edge, or filling a gap with anything the SQL did not say |
| [0004](0004-three-column-sources-kept-apart.md) | Three column sources, kept apart | merging the column lists in `schema.rs`, or adding a fourth source |
| [0005](0005-the-cache-stays-at-version-one.md) | The cache stays at version 1 | adding a field to the cache, or bumping its version |
| [0006](0006-the-gaps-are-a-deliverable.md) | The gaps are a deliverable, not a log line | skipping a model quietly, or writing a warning and moving on |
| [0007](0007-the-corpus-never-enters-this-repository.md) | The corpus never enters this repository | writing a fixture, a test, an example or a commit message |
| [0008](0008-what-makes-a-compile-untrustworthy.md) | What makes a compile untrustworthy, and what follows | changing `Agreement`, calling `set_computed`, or deciding a model is fine because it parsed |
| [0009](0009-the-loudest-role-on-a-path-wins.md) | The loudest role on a path wins, not the last | changing what an edge's expression carries, or adding a role |
| [0010](0010-a-thin-compile-is-a-documentation-finding.md) | A thin compile is a documentation finding, not a degraded one | adding an `Agreement` rung, or deciding a model producing four of its two hundred columns must be broken |
| [0011](0011-an-indirect-edge-and-what-it-fans-out-to.md) | An indirect edge, and why it is off by default | emitting an edge for a column that feeds no output column, or changing the default of `--indirect` |
| [0012](0012-a-model-reading-itself-is-not-a-fault.md) | A model reading itself is not a fault | changing what keeps a model out of `models`, or handing the engine a model's own relation |
| [0013](0013-one-edge-when-a-column-plays-two-indirect-roles.md) | One edge when a column plays two indirect roles | changing how `--indirect` fans a read out, or adding an indirect role |
| [0014](0014-a-contested-relation-goes-to-the-claimant-the-model-reads.md) | A contested relation goes to the claimant the model reads | changing how an edge's source node is found from a relation, or touching `by_relation` |
| [0015](0015-an-ephemeral-parent-is-read-through.md) | An ephemeral parent is read through | changing which relations the engine is handed, or which parents inference matches against |
| [0016](0016-a-root-is-read-where-the-walk-ends.md) | A root is read where the walk ends | changing what `Resolved::roots` holds, or counting a column with no edge as a gap |
| [0017](0017-a-column-with-no-lineage-is-named.md) | A column with no lineage is named | changing what keeps a trusted model out of `models`, or filling a column left without an edge |
| [0018](0018-two-scopes-of-one-name-are-not-trusted.md) | Two scopes of one name are not trusted | changing what sets a compile aside, or relying on the engine to keep two derived tables or CTEs apart |
| [0019](0019-a-read-the-sql-cannot-back-is-not-parsed.md) | A read the SQL cannot back is not parsed | following a derivation past a CTE, retracting a compile's list, or publishing an inferred edge from a parsed model |
| [0020](0020-a-column-read-downstream-is-the-parent-s-finding.md) | A column read downstream is the parent's finding | changing which issues keep a model out of the clean ones, or acting on a column a child reads that its parent's list lacks |
| [0021](0021-an-aggregate-anywhere-in-an-expression-names-it.md) | An aggregate anywhere in an expression names it | changing how `role::classify` recognises an aggregate |
| [0022](0022-a-hash-is-a-role.md) | A hash is a role | adding a role, or changing what `hash` covers or where it ranks |
| [0023](0023-a-qualify-key-belongs-to-its-own-select.md) | A QUALIFY key belongs to its own SELECT | changing how a `QUALIFY` is read, or which relation a dedup key is credited to |
| [0024](0024-a-row-deciding-read-says-whether-it-is-projected.md) | A row deciding read says whether it is projected | renaming a report section, or counting the columns read to decide which rows exist |
| [0025](0025-a-join-key-is-read-where-the-join-is-written.md) | A join key is read where the join is written | changing how a join condition is read, or which relation a join key is credited to |
| [0026](0026-a-filter-is-read-in-its-own-select.md) | A filter is read in its own SELECT | changing how a `WHERE` or `HAVING` is read, or adding a role for a column that decides which rows exist |
| [0027](0027-a-name-several-parents-have-is-settled-by-the-reading.md) | A name several parents have is settled by the reading | changing which parent an inferred edge is drawn from, or letting a set aside compile inform anything else |
| [0028](0028-the-engine-runs-from-a-fork-while-its-fixes-wait-upstream.md) | The engine runs from a fork while its fixes wait upstream | adding a patch to the engine, moving the pinned revision, or going back to a released flowscope-core |
| [0029](0029-a-compile-its-table-is-behind-keeps-the-columns-both-have.md) | A compile its table is behind keeps the columns both have | changing what a warehouse mismatch does to a compile, or emitting a column the warehouse does not have |
| [0030](0030-a-column-read-through-a-star-is-evidence-about-its-parent.md) | A column read through a star is evidence about its parent | changing what the engine is handed for a relation, resolving a model more than once, or publishing an edge out of a column no handed list has |
| [0031](0031-a-name-is-read-in-the-scope-that-wrote-it.md) | A name is read in the scope that wrote it | changing which text the walk reads names out of, or where it reads a window's keys |
| [0032](0032-the-loudest-role-over-every-path-stands.md) | The loudest role over every path stands | changing how the walk remembers where it has been, or how it settles two expressions reaching one column pair |
| [0033](0033-a-model-without-compiled-code-reads-its-compiled-file.md) | A model the manifest gives no SQL reads its compiled file | changing where a model's SQL comes from, reading anything under `target/run/`, or joining a path from the manifest onto a directory |
| [0034](0034-collin-says-its-version.md) | collin says its version, and the version moves when dbt-lens needs it to | changing what `collin --version` prints, the cache's `producer`, or the version |
| [0035](0035-merging-a-version-bump-releases-it.md) | Merging a version bump releases it | changing how a release is started, what makes the tag, or the install commands in the README |
| [0036](0036-free-to-use-not-to-resell.md) | Free to use, not to resell | changing the license or the copyright line, or linking code under another license |
| [0037](0037-a-catalog-entry-for-another-table-is-no-witness.md) | A catalog entry for another table is no witness | changing how `catalog.json` is matched to the manifest, or trusting an entry because its unique_id is right |
| [0038](0038-a-public-example-project-is-a-fixture.md) | A public example project is a fixture | adding a fixture collin did not invent, or accepting a change to the Jaffle Shop expected caches |

## The other side of the boundary

collin writes a file dbt-lens reads, so two of its decisions bind this one:
dbt-lens 0008 fixes the cache format and says a second producer may fill it, and
dbt-lens 0021 gives each producer its own file, `column_lineage.<source>.json`.
0005 here is what keeps that promise honest.

## Keeping these honest

- **One decision per file**, numbered in order, dated, with what was rejected.
  The rejected options are the part worth writing: the decision itself is usually
  visible in the code, the discarded alternatives never are.
- **Write the decision the day it is taken**, and date it that day: a decision
  recorded once it has already shipped has lost the alternatives that made it a
  decision.
- **Never edit a record to change its meaning.** Reversing one means adding a new
  file that says which number it supersedes, and marking the old one
  `Status: superseded by NNNN`. The reasoning then reads in order, including the
  mistakes.
- **A measurement belongs in the record that relies on it**, with the figure, so
  that a later reader can tell a decision made on evidence from one made on
  taste.
