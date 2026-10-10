#!/bin/sh
# Rebuilds target/manifest.json and target/catalog.json from Fivetran's Shopify
# package at the tag below: its integration tests, seeded and run on DuckDB
# with the dbt pinned here (0038). Needs git and uv. Fetches the package from
# GitHub, its dependencies from dbt's hub at the versions package-lock.yml
# names, and dbt from PyPI. No warehouse: DuckDB runs in process.
#
# It does not touch expected/. Run the tests after it: a new dbt or a new tag
# that moves an edge fails them, and is accepted the way any change is, with
# COLLIN_BLESS=1.
set -eu

TAG=v1.10.0
COMMIT=03e91d7aeb83151f968105f8580d13110f5375e4
DBT_CORE=1.12.5
DBT_DUCKDB=1.11.0

here=$(cd "$(dirname "$0")" && pwd)
# A fixed directory, not a temporary one: dbt writes the directory it ran in
# into the manifest, as each seed's root_path, and a random name would move the
# manifest on every run.
work=/tmp/collin-fivetran-shopify
rm -rf "$work"
git -c advice.detachedHead=false clone -q --depth 1 --branch "$TAG" \
    https://github.com/fivetran/dbt_shopify "$work"
test "$(git -C "$work" rev-parse HEAD)" = "$COMMIT"
cp "$here/profiles.yml" "$here/package-lock.yml" "$work/integration_tests/"

export DBT_SEND_ANONYMOUS_USAGE_STATS=false
cd "$work/integration_tests"
dbt() {
    uv run --isolated --no-project --python 3.12 \
        --with "dbt-core==$DBT_CORE" --with "dbt-duckdb==$DBT_DUCKDB" \
        dbt "$@" --project-dir "$work/integration_tests" --profiles-dir "$work/integration_tests"
}
dbt deps
dbt seed --full-refresh
dbt run
dbt docs generate

cp target/manifest.json target/catalog.json "$here/target/"
rm -rf "$work"
