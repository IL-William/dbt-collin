#!/bin/sh
# Rebuilds target/manifest.json and target/catalog.json from this project with
# the dbt pinned here (0038). Needs uv, which fetches dbt from PyPI. No
# warehouse: DuckDB runs in process.
#
# It does not touch expected/. Run the tests after it: a new dbt that moves an
# edge fails them, and is accepted the way any change is, with COLLIN_BLESS=1.
set -eu

DBT_CORE=1.12.5
DBT_DUCKDB=1.11.0

here=$(cd "$(dirname "$0")" && pwd)
# A fixed directory, not a temporary one: dbt writes the directory it ran in
# into the manifest, as each seed's root_path, and a random name would move the
# manifest on every run.
work=/tmp/collin-jaffle-shop
rm -rf "$work"
mkdir -p "$work"
cp -R "$here/dbt_project.yml" "$here/profiles.yml" "$here/models" "$here/seeds" "$work/"

export DBT_SEND_ANONYMOUS_USAGE_STATS=false
cd "$work"
dbt() {
    uv run --isolated --no-project --python 3.12 \
        --with "dbt-core==$DBT_CORE" --with "dbt-duckdb==$DBT_DUCKDB" \
        dbt "$@" --project-dir "$work" --profiles-dir "$work"
}
dbt build
dbt docs generate

cp "$work/target/manifest.json" "$work/target/catalog.json" "$here/target/"
rm -rf "$work"
