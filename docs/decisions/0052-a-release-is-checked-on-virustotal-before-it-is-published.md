# 0052. A release is checked on VirusTotal before it is published

Date: 2026-11-08 · Status: accepted · Amends 0035

**Trigger:** read before changing what the release workflow does between the
build and the publication, the `VIRUSTOTAL_API_KEY` secret, or the override that
publishes a flagged release.

## Context

0035 made merging a version bump the release: the workflow builds, attests and
publishes with nothing between the build and the tag. A release is immutable:
once its tag exists the files cannot be replaced, so a flag found afterwards is
mended by the next release, and the flagged file stays on the page. Endpoint
protection has flagged dbt-edith's Windows binary on a Windows machine (its
0123); nothing records a collin release being flagged, and nothing here looked
at the file CI publishes before the tag was made. dbt-edith made that check for its own release in
[its 0152](https://github.com/IL-William/dbt-edith/blob/main/docs/decisions/0152-a-release-is-checked-on-virustotal-before-it-is-published.md);
this is the same check on collin's Windows archive.

## Decision

- **A `virustotal` job sits between `build` and `release`**, and `release`
  needs it. It downloads the artifact of the Windows target, the one
  `release` uploads, and looks `collin-v<version>-x86_64-pc-windows-msvc.zip` up
  on VirusTotal by its SHA-256. A file it already has is not sent again; one it
  does not have is uploaded, and the job waits for the analysis, 30 minutes at
  most. "Re-run failed jobs" after an outage or a slow analysis therefore scans
  the same artifact and uploads nothing twice.
- **Only the Windows archive is sent.** The Linux and macOS archives are not:
  the Windows .zip is what runs on the Windows machines where that engine sits,
  and dbt-edith's binary is the one it flagged there. Each file sent costs
  requests of a quota that is small.
- **Only a release that is due is checked**, on main, never on a pull request.
  A pull request's build is not the file that ships, and a report on it would
  say nothing about the release and would spend the quota.
- **No per-file check against a `.sha256`.** dbt-edith's job verifies the
  download against files its build wrote. Collin writes `SHA256SUMS` in the
  `release` job, after this one, so there is nothing to compare with; the
  artifact is the one `build` uploaded under the target's name.
- **A flag blocks the release.** Flagged is `malicious` plus `suspicious` in
  the report's stats, by any of the engines in it. The job fails, naming the
  engines, and the run's summary gives the file with its SHA-256, flagged over
  engines, a link to its report, and for a flagged file the verdict of each
  engine that flagged it.
- **A run started by hand can publish anyway.** `workflow_dispatch` has one box,
  "Publish even if a VirusTotal engine flags the file". Ticked, a flag is a
  warning and the job passes. That run builds again, so its file is a new one
  and is checked afresh, but a flag on it is only a warning: it is published
  before anyone has read the report. There is no approval of a given SHA-256.
- **The free key sets the pace.** Its quotas are 4 requests a minute, 500 a day
  and 15 500 a month. Every call waits 15 seconds first, and a `429` waits a
  minute and asks again, five times at most. The key is in the repository secret
  `VIRUSTOTAL_API_KEY`, read from an environment variable, written to a header
  file for curl and never printed. Without the secret every release blocks, with
  an error naming it.

## Rejected

- **Sending all three archives.** They are not what runs on those
  Windows machines, and they would triple the requests of each release.
- **Checking pull-request builds.** Not the shipped bytes, and a daily quota
  spent on files that are thrown away.
- **Scanning after the release.** The tag and the files are immutable by then.
- **Only warning.** A warning on a green run is read by nobody; the override is
  the explicit way to say "I looked".
- **Treating a clean report as a guarantee.** See the consequences.

## Consequences

*Amends 0035*: the release is no longer the build's end; it waits for a report.
**VirusTotal is not an endpoint's on-device engine**: a clean report there does
not show what a sensor on someone's machine says, and a flag there may be one
engine's reputation guess. That is why the override exists.

A file sent to VirusTotal is kept there and shared with its partners. The
sources are public anyway, and a build the check blocked is still kept and
shared by VirusTotal, though it is never released.

A release now waits for the analysis, a few minutes for a file VirusTotal has
not seen. A report already on VirusTotal for the same bytes is used as it
stands, however old. If the key expires or the quota is spent, the job fails
with VirusTotal's error code and releases wait until it is fixed or the failed
jobs are re-run. The secret `VIRUSTOTAL_API_KEY` has to be added to this
repository before the next version bump merges.
