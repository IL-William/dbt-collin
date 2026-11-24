# 0058. A release ships a Windows exe with no version in its name

Date: 2026-11-24 · Status: accepted · Amends 0052

**Trigger:** read before changing the files the Windows leg of the release
workflow uploads, the name of the Windows asset, or what is sent to VirusTotal.

## Context

The Windows asset is `collin-v<version>-x86_64-pc-windows-msvc.zip`. dbt-edith
wants to download collin for a person on Windows, where `cargo install` cannot
build it: `psm`, under `sqlparser`, compiles C, and many Windows machines have no C
compiler. The command dbt-edith types into the terminal needs a URL that names no
version, because it cannot know which one is the latest: GitHub's
`releases/latest/download/<asset>` redirects to the newest release's asset, but
only when `<asset>` is the same name in every release. dbt-edith's own
`dbt-edith.exe` is published that way for the same reason. The zip also holds a
folder, so a command that fetches it has to unzip and move a file.

## Decision

- **The Windows leg also uploads `collin-x86_64-pc-windows-msvc.exe`**, the same
  `collin.exe` that is inside the zip, copied to the top level beside it. So
  `https://github.com/IL-William/dbt-collin/releases/latest/download/collin-x86_64-pc-windows-msvc.exe`
  always resolves. The zip stays, with its README and LICENSE.
- **The release job needs no change.** It already attests, sums and publishes
  every `collin-*` file: the exe has a build provenance and a line in
  `SHA256SUMS`, which ends in ` collin-x86_64-pc-windows-msvc.exe`. That line is
  what dbt-edith checks the download against.
- **VirusTotal scans it too.** It is the file a machine downloads and runs, so
  it is sent with the zip, under 0052's rules: a flag on either blocks the
  release, the table in the notes lists both, and the badge shows the one with
  the most flags. Each release now costs two files of a quota that is small
  (0052), not one.
- **A pull request's build makes it too**, under the same name: nothing in its
  name carries the version, so `check` builds package it the same way, and a
  break in that step shows before a bump merges.
- **0.12.2.** The bump is what makes a release due, so the asset exists as soon
  as this merges. The code is the same as 0.12.1; only the version it reports
  changes.

## Rejected

- **A versioned exe beside the zip.** `latest/download` needs the same name
  every time; a name with the version cannot be typed without looking up the
  latest release first, a second request from a tool that has no need for the
  number.
- **dbt-edith resolving the latest version through the API, then downloading the
  zip.** An API call and a JSON read in the command a person is shown, then an
  unzip, to reach one file.
- **Sending only the zip to VirusTotal.** The exe is what runs where the engine
  sits; a flag on it that the zip's compression hides would be missed.
- **Replacing the zip.** People who want the README and LICENSE with the binary,
  and the links of earlier releases, still use it.

## Consequences

*Amends 0052*: two files of the Windows leg are checked, not one. The releases
of 0.12.1 and before have no such asset, so dbt-edith's download works from
0.12.2 on, and `releases/latest/download/collin-x86_64-pc-windows-msvc.exe`
answers 404 until then. A release is immutable, so a wrong exe cannot be
replaced: the next version fixes it.
