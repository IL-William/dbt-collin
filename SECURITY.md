# Security

collin reads a dbt project's files and writes a cache. It never runs dbt, never
reaches a warehouse and takes no credentials, so what it exposes is what it
parses: a manifest, a catalog and compiled SQL, none of which it can trust.

## Reporting a vulnerability

Report it privately, from the repository's Security tab, under "Report a
vulnerability". A public issue would describe the problem to everyone before it
is fixed.

Only the latest commit on `main` is supported.
