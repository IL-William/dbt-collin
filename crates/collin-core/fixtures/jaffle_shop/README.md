# Jaffle Shop

dbt Labs' example project
[jaffle_shop_duckdb](https://github.com/dbt-labs/jaffle_shop_duckdb) at commit
`20cc904`, under the Apache License 2.0 in [LICENSE](LICENSE). Its models,
seeds, `dbt_project.yml` and `profiles.yml` are copied unchanged. Nothing else
of that repository is here.

- `target/` is what dbt wrote for it, by `regenerate.sh`.
- `expected/` holds the caches collin gives for it, with and without
  `--indirect`, less `generated_at` and `producer`.

`lineage::tests::jaffle_shop_gives_the_caches_committed_beside_it` fails when
either moves. A change meant to move them is accepted with:

```
COLLIN_BLESS=1 cargo test --release jaffle_shop
```

Why it is here, and what it does not catch:
[0038](../../../../docs/decisions/0038-a-public-example-project-is-a-fixture.md).
