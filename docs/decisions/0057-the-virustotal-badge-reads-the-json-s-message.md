# 0057. The VirusTotal badge reads the JSON's message

Date: 2026-11-12 · Status: accepted · Amends 0053

**Trigger:** read before changing the README's VirusTotal badge, the service
that draws it, or the colour it shows.

## Context

0053 put shields.io's endpoint badge in the README, pointed at
`releases/latest/download/virustotal.json`. shields.io refuses github.com as an
endpoint host, so the badge drew "custom badge | domain is blocked" in red.
dbt-edith, which made the same change, hit it first and settled it in
[its 0179](https://github.com/IL-William/dbt-edith/blob/main/docs/decisions/0179-the-virustotal-badge-reads-the-json-s-message.md).

## Decision

- **The README's badge is shields.io's dynamic JSON badge**
  (`img.shields.io/badge/dynamic/json`), which accepts github.com. It reads the
  `message` of the same `virustotal.json` through the query `$.message`, with
  the label `VirusTotal`, and still links to `releases/latest`.
- **The colour is fixed, blue.** The dynamic badge cannot read the file's
  `color`, so the badge no longer goes from green to red with the flags. The
  count stays in the text (`0/75 · v<version>`).
- **The file is unchanged.** `virustotal.json` keeps the endpoint format and
  its colour, and the release workflow is not touched: the file stays valid for
  an endpoint host later.

## Rejected

- **Keeping the colours by pushing the JSON to a `badges` branch read through
  raw.githubusercontent.com**, a host the endpoint badge accepts. It costs a
  commit per release and gives the release workflow write access to a branch.
- **Dropping the badge.** The release's notes carry the table, but the README is
  where a person deciding to download looks first.

## Consequences

*Amends 0053*: only how the README reads the file changes. The badge no longer
turns orange or red for a flagged file; only a run started by hand with the
override can publish one (0052), and its text then shows the count. It still
depends on shields.io being up and on the latest release carrying the file.
