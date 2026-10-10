# 0040. A finding says what to do

Date: 2026-10-10 · Status: accepted · Amends 0006

**Trigger:** read before adding or rewording a finding, changing who a finding
is for, or putting in a finding something the models' entries do not say.

## Context

0006 made the gaps a deliverable: every model with something to say gets an
entry, field by field. That serves a tool. A person reading 300 entries does
not see that 59 of them fail for one reason, nor what the reason usually is,
nor that the fix is a command in the project and not a change in collin.

On a 3507 model Snowflake project the difference was a day's work. A compile
for its target made without the target's environment variables read another
environment's databases: 59 compiles came out as invalid SQL, written by
macros that found no parent, and 149 tables had columns the code no longer
writes. Every one was in the report, model by model, and read as faults of the
project. dbt-edith, which runs collin and shows its report, has no place for a
project-wide list either.

## Decision

- **The report gains `findings`**, beside `models`: the models that share one
  cause, each finding with a stable `category`, who can act (`inputs`, the
  manifest and catalog collin was handed; `project`; `collin`), a `severity`,
  a title with the count, what collin saw and what it usually means, the
  action, the columns it costs where the report counts them, and each model
  with its own entry's words.
- **A finding adds no fact.** It is read off the report, so the two cannot
  disagree. Where the cause is a likelihood, such as a macro that found no
  parent or a target compiled against the wrong environment, the words say
  "usually".
- **Who can act orders them**: the inputs first, since they make every other
  figure wrong, then the project, then collin's own limits, each by severity.
- **The CLI prints the titles**, so a run says what to look at without opening
  the file.
- **`models` stays as it is.** dbt-edith reads it field by field, and a reader
  that does not know `findings` skips it.

## Rejected

- **Guessing the environment.** collin does not know which target a manifest
  was compiled for, and a database name ending `_DEV` is a convention, not a
  fact. The findings it causes say what to suspect instead.
- **Findings in place of `models`.** A tool needs the fields, a person the
  words: both read the same entries.
- **A finding per model.** That is `models` again, reworded.

## Consequences

On the same project, compiled on its own environment, three findings remain:
two views older than their code, the three models reading only their own
table, and the YAML that disagrees with 1282 tables. On the compile without its
environment there are ten, the first being 59 compiles that do not parse,
whose words point at the environment.

Writing them showed three models flagged as reading a name their CTE lacks
that read none: a `VALUES` list's `column1` and a FLATTEN's `INDEX`, which the
engine gives nothing to feed. 0019 no longer counts those, so neither does the
finding.
