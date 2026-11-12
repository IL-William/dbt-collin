# 0053. A release shows its VirusTotal result

Date: 2026-11-12 · Status: accepted · Amends 0052 · amended by 0057

**Trigger:** read before changing the VirusTotal table in a release's notes,
the `virustotal.json` asset, or the README badge that reads it.

## Context

0052 checks the Windows archive on VirusTotal before a release is published, but
the result only lived in the run's summary, which nobody reads once the release
is out. The check is only worth showing if a reader of the release, or of the
README, can see it. The scan runs after the merge, so the README cannot carry
the number itself without a commit per release, and a release is immutable: its
notes and files cannot be corrected afterwards. dbt-edith made the same change
for its own releases in
[its 0174](https://github.com/IL-William/dbt-edith/blob/main/docs/decisions/0174-a-release-shows-its-virustotal-result.md).

## Decision

- **The notes start with the report.** The `virustotal` job writes
  `virustotal.md`: a `### VirusTotal` heading, one sentence, the table of the
  run's summary (file, SHA-256, flagged over engines, link to the report) and,
  when something was flagged, the verdict of each engine that flagged it. The
  release is created with `--notes-file virustotal.md --generate-notes`, so
  GitHub's list of merged pull requests follows it.
- **The release carries `virustotal.json`**, in the endpoint format of
  shields.io: `{"schemaVersion":1,"label":"VirusTotal","message":"0/75 · v0.12.1",
  "color":"brightgreen"}`. The message is the flagged count over the engines
  for the file with the most flags, then the version. The colour follows the
  flags: 0 `brightgreen`, 1 `yellow`, 2 `orange`, 3 or more `red`. Only a run
  started by hand with the override can publish a non-zero one (0052). The file
  is written with `jq --arg`, never by joining strings.
- **Both files are written before a flag can end the step**, and uploaded as the
  artifact `virustotal` only when the step succeeds: a blocked run uploads
  nothing, a forced one does. The `release` job merges every artifact into
  `dist`, where neither name matches `collin-*`, so the checksums and the
  attestation do not take them in. `virustotal.json` is not attested: it is not
  a binary anyone runs.
- **The README badge points at
  `releases/latest/download/virustotal.json`**, which GitHub serves from the
  newest release, so the badge follows each release with no commit and no
  version bump of its own.

## Rejected

- **A bot committing the badge to the README on main.** It needs a push past
  the branch protection and adds one commit per release.
- **A static "scanned" badge.** It would say a scan exists, not what it found.
- **An SVG attached to the release.** GitHub's image proxy needs an image
  content type, and a release asset is served as `application/octet-stream`.

## Consequences

*Amends 0052*: the report is part of the release, not only of the run.

The badge depends on shields.io reading a GitHub release asset. It shows the
scan at publication, not later re-scans, so an engine that flags the file next
month changes nothing here; the link in the notes goes to the live report.
Releases before 0.12.1 have no `virustotal.json`: the badge reads the latest
release, which always has one once 0.12.1 is out.
