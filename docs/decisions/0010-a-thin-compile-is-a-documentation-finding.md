# 0010. A thin compile is a documentation finding, not a degraded one

Date: 2026-10-10 · Status: accepted · Amends 0006, and narrows a rejection in 0008

**Trigger:** read before adding an `Agreement` rung, before changing
`thin_finding`, or before deciding that a model producing four columns out of two
hundred and seventy seven must be broken.

## Context

0008 established that with no catalog entry the YAML may contradict a compile
only totally, and rejected a threshold outright. That rejection was about a
**verdict**, and it still stands. It left a hole in the **report**.

37 models on the frozen corpus produced six columns or fewer while documenting
twenty or more, and every one of them was absent from the report, which by 0006
means clean. Two were read by hand. One satellite documents 277 columns and its
compiled SQL selects four. One preparation model documents 88 and selects five.
In both the compile is correct: an AutomateDV satellite really does produce a
hash key, a hashdiff and two metadata columns. The YAML is stale.

Nothing said so. The model had no catalog entry, so `Agreement` was `Unchecked`,
and being unreported read as examined and fine.

## Decision

`thin_finding` reports, and reports only. No rung, no retraction, no inference.
It fires when there is no warehouse entry, the YAML documents at least eight
columns, and the compile accounts for fewer than half of them. The report carries
the three numbers, `declared`, `computed` and `shared`, so a reader judges rather
than trusting the verdict.

A threshold is admissible here for the reason it was inadmissible in 0008: a
finding that is wrong costs a line in a file, and a verdict that is wrong throws
away edges the SQL proves and replaces them with name matches against columns it
proves absent. Getting the satellite above wrong as a verdict would invent 273
edges.

## Rejected

- **A fourth `Agreement` rung.** The three answer "can this compile be trusted",
  and the answer here is yes. Putting "the YAML disagrees" on that axis would
  make a correct compile look broken, which is the failure this record exists to
  avoid.
- **Counting it as stale YAML.** `yaml_stale` means the YAML disagrees with the
  **warehouse**, which is an established fact. This is a compile disagreeing with
  a document, and nothing here can say which is right.
- **A ratio tuned until the count looked right.** Half, and a floor of eight, are
  round numbers chosen before the count was known. On this corpus they flag 48
  models. The numbers are in the report precisely so nobody has to trust the
  ratio.

## Consequences

48 models that were absent are now present, and `--report` grew from 0.6 MB to
2.0 MB, most of that 0011 rather than this.

A model can be `parsed`, `unchecked` and `thin` at once: read successfully,
nothing to check it against, and documented as something much larger. All three
are true and they answer different questions.
