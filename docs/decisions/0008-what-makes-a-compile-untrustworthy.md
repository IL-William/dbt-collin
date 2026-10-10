# 0008. What makes a compile untrustworthy, and what follows

Date: 2026-10-10 · Status: accepted · Amends 0003 and 0004

**Trigger:** read before changing `Agreement`, before calling `set_computed`, or
before deciding a model is fine because it parsed.

## Context

Parsing is not understanding. Auditing the emitted cache turned up models that
parsed cleanly, agreed with the warehouse on their output column names, and
carried no lineage whatsoever: one model sat in the graph with 133 columns
and zero edges, reported as confirmed. Its compiled SQL opens with a CTE that
selects one literal and nothing else, because an introspecting macro returned an
empty column list, and the later CTEs then select names that do not exist in it.

Worse, an untrustworthy compile spread. Its computed column list still defined
what every downstream `select *` expanded to, so one broken model truncated a
whole branch. And with no catalog entry, nothing could contradict a compile at
all: 218 columns named with an underscore between every letter, produced by a
Jinja bug in the project's own macro, went into the graph unchallenged.

## Decision

Three rules, all of them about what counts as evidence.

- **Silence is a verdict.** A model that parsed, had parents whose columns were
  known, and still produced no edge has been read but not understood. It is
  degraded, and the ladder falls to inference.
- **A compile we do not trust does not define what downstream sees.** Its
  computed columns are retracted from the store, so a star below it expands from
  the YAML instead. Stale beats a list already shown to be wrong.
- **The YAML may contradict a compile only totally.** With no warehouse, the
  declared columns are the only reference, and they are stale on 44.8% of the
  models we can check. So they support one verdict and no other: a compile that
  shares **not a single column name** with what the project says this model
  builds is not describing this model. Anything short of that stays unchecked.

## Rejected

- **A threshold, such as "fewer than half the declared columns matched".** It
  would catch more, and every number in it would be arbitrary. Zero overlap is
  not a threshold but a categorical statement, and drift does not rename every
  column at once. A broken macro does.
- **Trusting the parse and reporting the oddity.** That is what produced a
  confirmed model with no lineage. A report nobody reads is not a safeguard.
- **Falling back to inference for every model with no catalog entry.** 2744
  models here, most of them fine. It would trade observed roles for name matches
  across most of the project.

## Consequences

Measured on the frozen corpus: edges 92 377 to 100 202, warehouse column
coverage 93.5% to 94.5%, models with no lineage at all 39 to 25, mangled column
names 218 to 0, and still not one column emitted that the warehouse does not
have. Some models legitimately lose edges, one of them going from 111
to 16, because the 111 were named after columns that exist nowhere.

A model can now be reported as `confirmed` on its column names and `inferred` on
its lineage at the same time. That reads oddly and it is accurate: the two axes
answer different questions.
