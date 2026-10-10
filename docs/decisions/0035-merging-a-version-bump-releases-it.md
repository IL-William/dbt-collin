# 0035. Merging a version bump releases it

Date: 2026-10-10 · Status: accepted

**Trigger:** read before changing how a release is started, what makes the tag,
or the install commands in the README.

## Context

A release took three steps by hand: bump the version in `Cargo.toml`, point
the README's install commands at it, push a signed tag. The tag started the
release workflow. None of the three checked the others, and the README pointed
at a v0.1.0 that was never tagged: the repository had no tag and no release at
all when this was written.

## Decision

- **A version on main with no tag is released.** The release workflow runs on
  every push to main that touches `Cargo.toml`, reads the version, and builds,
  attests and publishes it when `v<version>` does not exist yet. The tag is
  created with the release, at the commit that was built. A release is a pull
  request that bumps the version, and nothing after its merge.
- **The version still moves only when dbt-lens relies on a change** (0034).
  This decides how a bump becomes a release, not when to bump.
- **The README must install the version `Cargo.toml` names**, checked in the
  `rust` job, which main requires. The bump and the README land together or not
  at all.
- **The notes are GitHub's list of the pull requests merged since the last
  release**, not a commit message: a retry from `workflow_dispatch`, or a push
  that only fixes the workflow, would otherwise publish its own message.

## Rejected

- **Tagging on a schedule.** A tag with nothing dbt-lens needs behind it is a
  version with nothing to say. dbt-lens installs from main with
  `cargo install --git` anyway, so a scheduled tag would reach no one who needs
  it.
- **A tag pushed by hand, kept as the trigger.** It is the step that was
  forgotten. And a tag pushed from a workflow with its own token starts no
  other workflow, so automating only the tag would still leave the release
  unbuilt.
- **A release bot (release-please and the like).** It writes the bump pull
  request from commit prefixes this repository does not use, and it is a third
  party action with write access to the tags, which cannot be moved or deleted
  here.

## Consequences

The tag is no longer signed by hand. What vouches for a release is the signed,
reviewed commit on main it was built from, and the provenance attestation of
each archive, which names this workflow and that commit.

A release that fails leaves its version untagged, so the next run releases it.
When nothing in the repository needs to change for that, `workflow_dispatch`
on main starts it.
