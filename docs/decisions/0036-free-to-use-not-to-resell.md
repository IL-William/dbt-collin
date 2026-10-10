# 0036. Free to use, not to resell

Date: 2026-10-10 · Status: accepted

**Trigger:** read before changing the license, the copyright line, or the
`license` field of `Cargo.toml`, and before linking code whose license would
have to be weighed against this one.

## Context

MIT would let anyone sell collin, or a service built on it, as long as the
notice stays. The intent is not that: collin is meant to be free for whoever
uses it, a company and its consultants included, but not something a third
party can sell as its own product. dbt-edith, which runs collin, is under the
same license.

## Decision

- **The license is FSL-1.1-MIT**, the Functional Source License 1.1 with an MIT
  future license, as published at fsl.software. Any purpose is allowed except a
  competing use: making the software available to others in a commercial
  product or service that substitutes for it, or offers substantially the same
  features. Internal use, and professional services for a client, are allowed
  by name.
- **Each version becomes MIT two years after it is published.** That is part of
  the license, not a promise kept by hand.
- **The licensor is Datadorelix**: `Copyright 2026 Datadorelix` in `LICENSE`.

## Rejected

- **MIT.** It allows exactly the resale this is meant to stop.
- **PolyForm Noncommercial.** It forbids any commercial use, so a company could
  not run collin internally, nor a consultant on a client's project.
- **PolyForm Shield.** It also forbids competing with any product the licensor
  provides using the software, which reaches other consultants, and it never
  ends.
- **Business Source License and Elastic License 2.0.** BUSL needs an
  additional use grant and a change date written by hand; Elastic is written
  for hosted services and keeps its restriction forever.

## Consequences

collin is source-available, not open source in the OSI sense. `Cargo.toml`
carries the SPDX identifier `FSL-1.1-MIT` for both crates. The engine,
flowscope-core, is Apache-2.0, and every crate linked is MIT, Apache-2.0 or
similarly permissive, which allow being shipped under this license. The
release archives carry `LICENSE` and the README; the linked crates' own
license texts are not in them yet, which Apache-2.0 asks of a binary
distribution and is the next thing to add to the archive.
