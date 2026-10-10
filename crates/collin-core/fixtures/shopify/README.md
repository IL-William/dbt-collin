# Shopify

Fivetran's [dbt_shopify](https://github.com/fivetran/dbt_shopify) package at
`v1.10.0` (`03e91d7`), under the Apache License 2.0 in [LICENSE](LICENSE). The
manifest also carries the macros of the packages it installs, dbt_utils,
fivetran_utils and spark_utils, under the same license.

Only what collin reads is here, not the package:

- `target/` is what dbt wrote for the package's integration tests, seeded and
  run on DuckDB as Fivetran's own CI does, by `regenerate.sh`. The script
  fetches the tag, checks its commit, installs the dependencies
  `package-lock.yml` pins and runs with `profiles.yml`.
- `expected/` holds the caches collin gives for it, with and without
  `--indirect`, less `generated_at` and `producer`, one edge a line.

They record what collin gives, which is not everything that is right:

- 16 marts are set aside, and their 630 edges inferred. Ephemeral models are
  inlined as CTEs, each with its own `WITH`, and two of them name a CTE alike
  (0018).
- Of the 4245 parsed edges, sqlglot reads 4244 the same way. The other is
  `country as country_name` beside `country_code as country`: the SQL reads the
  table's `country`, and collin credits the alias.

A fix to either moves them, and the diff of `expected/` shows by how much.

`lineage::tests::shopify_gives_the_caches_committed_beside_it` fails when
either cache moves. A change meant to move them is accepted with:

```
COLLIN_BLESS=1 cargo test --release committed_beside
```

Why it is here: [0038](../../../../docs/decisions/0038-a-public-example-project-is-a-fixture.md).
