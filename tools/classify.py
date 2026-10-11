"""Bucket every column without an edge by cause, from a collin dump and its manifest.

usage: python3 -I classify.py MANIFEST DUMP_DIR OUT_JSON
Private: model names stay in OUT_JSON, never in the collin repository.
"""
import collections
import json
import os
import re
import sys

import sqlglot
from sqlglot import exp

manifest_path, dump_dir, out_path = sys.argv[1:4]
m = json.load(open(manifest_path))
sql_of = {u: n.get("compiled_code") or "" for u, n in m["nodes"].items() if n["resource_type"] == "model"}

NILADIC = {"current_timestamp", "current_date", "current_time", "sysdate", "getdate", "localtimestamp",
           "uuid_string", "current_user", "current_role", "current_database", "current_schema", "seq4", "seq8",
           "random", "current_warehouse", "current_session", "convert_timezone", "to_timestamp", "to_date"}


def root_kind(sql, col):
    """literal, niladic, reads_name, or unknown: what the root's expression is made of."""
    try:
        tree = sqlglot.parse_one(sql, read=os.environ.get("DIALECT", "snowflake"))
    except Exception:
        return "unknown_unparsed"
    ctes = {}
    for c in tree.find_all(exp.CTE):
        ctes.setdefault(c.alias_or_name.lower(), c.this)

    def projections(select_like):
        if isinstance(select_like, exp.Union):
            return projections(select_like.left) + projections(select_like.right)
        if isinstance(select_like, exp.Subquery):
            return projections(select_like.this)
        if isinstance(select_like, exp.Select):
            return [select_like]
        return []

    def find(select_like, name, depth):
        if depth > 12:
            return []
        found = []
        for s in projections(select_like):
            hit = None
            for p in s.expressions:
                if p.alias_or_name.lower() == name:
                    hit = p
                    break
            if hit is None:
                # through a star: look in the sources of this select
                if any(isinstance(p, exp.Star) or (isinstance(p, exp.Column) and isinstance(p.this, exp.Star)) for p in s.expressions):
                    for t in s.find_all(exp.Table):
                        if t.name.lower() in ctes:
                            found += find(ctes[t.name.lower()], name, depth + 1)
                continue
            inner = hit.this if isinstance(hit, exp.Alias) else hit
            if isinstance(inner, exp.Column):
                tbl = inner.table.lower()
                srcs = [t for t in s.find_all(exp.Table) if (t.alias_or_name.lower() == tbl or not tbl)]
                followed = False
                for t in srcs:
                    if t.name.lower() in ctes:
                        found += find(ctes[t.name.lower()], inner.name.lower(), depth + 1)
                        followed = True
                if not followed:
                    found.append(inner)
            else:
                found.append(inner)
        return found

    body = tree
    exprs = find(body, col, 0)
    if not exprs:
        return "unknown_unfound"
    kinds = set()
    for e in exprs:
        cols = [c for c in e.find_all(exp.Column) if not isinstance(c.this, exp.Star)]
        cols = [c for c in cols if c.name.lower() not in NILADIC]
        if cols:
            kinds.add("reads_name")
            continue
        funcs = {type(f).__name__.lower() for f in e.find_all(exp.Func)}
        anon = {f.name.lower() for f in e.find_all(exp.Anonymous)}
        if funcs - {"cast", "trycast", "null", "anonymous"} or anon:
            kinds.add("niladic_or_function_of_literals")
        else:
            kinds.add("literal")
    for k in ("reads_name", "niladic_or_function_of_literals", "literal"):
        if k in kinds:
            return k
    return "unknown"


causes = collections.Counter()
causes_ind = collections.Counter()
by_model = collections.defaultdict(collections.Counter)
samples = collections.defaultdict(list)
for f in sorted(os.listdir(dump_dir)):
    d = json.load(open(os.path.join(dump_dir, f)))
    uid = d["uid"]
    produced = {e["to_col"] for e in d.get("published") or []}
    outputs = set(d.get("outputs") or [])
    roots = set(d.get("roots") or [])
    lost = {l["column"]: l["ends"] for l in d.get("lost_ends") or []}
    wh = d.get("warehouse")
    decl = d.get("declared")
    plan = d.get("plan")
    # As Store::visible: the warehouse, else the compile when the plan kept it,
    # else the YAML.
    kept = plan == "parsed" and d.get("outputs") and not (d.get("unbacked") and (wh or decl))
    reference = wh if wh else (d.get("outputs") if kept else (decl or []))
    sql = sql_of.get(uid, "")
    root_kinds = {}

    def cause(c):
        if d.get("parse_error"):
            return "parse_failed"
        if c in roots:
            if c not in root_kinds:
                root_kinds[c] = root_kind(sql, c)
            return "root:" + root_kinds[c]
        if c in lost:
            reasons = sorted({e["reason"] for e in lost[c]})
            return "lost:" + "+".join(reasons)
        if plan == "inferred":
            return "inferred_no_parent_has_name"
        if c not in outputs:
            return f"table_only:{plan}"
        return f"other:{plan}"

    for c in reference:
        if c in produced:
            continue
        k = cause(c)
        causes[k] += 1
        by_model[k][d["name"]] += 1
        if len(samples[k]) < 8:
            samples[k].append([d["name"], c])
    independent = wh if wh else decl
    for c in independent or []:
        if c in produced or c in roots:
            continue
        k = cause(c)
        if wh is None and c not in outputs and not d.get("parse_error"):
            k = "yaml_only"
        causes_ind[k] += 1

out = {
    "reference": dict(causes.most_common()),
    "independent": dict(causes_ind.most_common()),
    "top_models": {k: v.most_common(6) for k, v in by_model.items()},
    "samples": samples,
}
json.dump(out, open(out_path, "w"), indent=1)
print("reference gaps", sum(causes.values()))
for k, v in causes.most_common():
    print(f"  {v:6d} {k}")
print("independent gaps (roots excluded)", sum(causes_ind.values()))
for k, v in causes_ind.most_common():
    print(f"  {v:6d} {k}")
