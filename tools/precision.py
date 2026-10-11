"""Compare collin's parsed edges with an independent reading of the SQL by sqlglot.

usage: python3 -I precision.py MANIFEST DUMP_DIR OUT_JSON [LIMIT]
Both readers get the same input: the compiled SQL and the column lists the engine
was handed (the dump's `visible`). An edge is (source relation, source column,
output column). Self reads are left out on both sides, as collin drops them.
Private: model names stay in OUT_JSON, never in the collin repository.
"""
import collections
import json
import os
import sys
import time

import sqlglot
from sqlglot import exp
from sqlglot.lineage import lineage
from sqlglot.optimizer.qualify import qualify
from sqlglot.optimizer.scope import build_scope

manifest_path, dump_dir, out_path = sys.argv[1:4]
limit = int(sys.argv[4]) if len(sys.argv) > 4 else None
m = json.load(open(manifest_path))
nodes = {**m["nodes"], **m["sources"]}


def norm(rel):
    return ".".join(p.strip().strip('"').upper() for p in rel.split("."))


def table_name(t):
    parts = [t.args.get("catalog"), t.args.get("db"), t.this]
    return ".".join(p.name.upper() for p in parts if p is not None and p.name)


totals = collections.Counter()
models_out = []
start = time.time()
files = sorted(os.listdir(dump_dir))
if limit:
    files = files[:: max(1, len(files) // limit)][:limit]
for f in files:
    d = json.load(open(os.path.join(dump_dir, f)))
    if d.get("parse_error") or d.get("plan") not in ("parsed", "per_column"):
        totals["models_skipped_not_parsed"] += 1
        continue
    uid = d["uid"]
    sql = m["nodes"][uid].get("compiled_code") or ""
    own = norm(m["nodes"][uid].get("relation_name") or "")
    uid_rel = {v["uid"]: norm(v["relation"]) for v in d.get("visible") or []}
    schema = {}
    for v in d.get("visible") or []:
        parts = norm(v["relation"]).split(".")
        cur = schema
        for p in parts[:-1]:
            cur = cur.setdefault(p, {})
        cur[parts[-1]] = {c.upper(): "VARCHAR" for c in v["columns"]}
    # collin: the parsed edges this model published, the withheld ones aside
    collin = set()
    for e in d.get("published") or []:
        if e["kind"] == "inferred":
            continue
        src = e["from"]
        rel = norm(src[4:]) if src.startswith("rel:") else uid_rel.get(src)
        if rel is None:
            rel = norm(nodes.get(src, {}).get("relation_name") or src)
        if rel == own:
            continue
        collin.add((rel, e["from_col"].lower(), e["to_col"].lower()))
    outputs = {e[2] for e in collin} | {c.lower() for c in d.get("outputs") or []}
    if d.get("plan") == "per_column" and d.get("warehouse"):
        outputs &= set(d["warehouse"])
    try:
        tree = sqlglot.parse_one(sql, read=os.environ.get("DIALECT", "snowflake"))
        # collin reads names case-insensitively, quoted or not: do the same, so a
        # difference is a reading and not a spelling.
        for ident in tree.find_all(exp.Identifier):
            ident.set("quoted", False)
        q = qualify(tree, schema=schema, dialect=os.environ.get("DIALECT", "snowflake"), validate_qualify_columns=False, identify=False)
        sc = build_scope(q)
    except Exception as ex:
        totals["models_sqlglot_failed"] += 1
        models_out.append({"name": d["name"], "sqlglot_error": str(ex)[:200]})
        continue
    names = {}
    for s in q.selects if isinstance(q, exp.Query) else []:
        names[s.alias_or_name.lower()] = s.alias_or_name
    glot = set()
    unread = []
    for col in sorted(outputs):
        target = names.get(col, col.upper())
        try:
            node = lineage(target, q, schema=schema, dialect=os.environ.get("DIALECT", "snowflake"), scope=sc, copy=False)
        except Exception:
            unread.append(col)
            continue
        for n in node.walk():
            if n.downstream or not isinstance(n.source, exp.Table):
                continue
            rel = table_name(n.source)
            if not rel or rel == own:
                continue
            glot.add((rel, n.name.split(".")[-1].strip('"').lower(), col))
    # judge only the outputs sqlglot could read
    glot_judged = {e for e in glot if e[2] not in unread}
    collin_judged = {e for e in collin if e[2] not in unread}
    both = collin_judged & glot_judged
    c_only = collin_judged - glot_judged
    g_only = glot_judged - collin_judged
    totals["models_compared"] += 1
    totals["outputs_unread_by_sqlglot"] += len(unread)
    totals["edges_both"] += len(both)
    totals["edges_collin_only"] += len(c_only)
    totals["edges_sqlglot_only"] += len(g_only)
    if c_only or g_only or unread:
        models_out.append({
            "name": d["name"],
            "plan": d["plan"],
            "collin_only": sorted(c_only),
            "sqlglot_only": sorted(g_only),
            "unread": unread,
        })

totals["seconds"] = round(time.time() - start)
b, c, g = totals["edges_both"], totals["edges_collin_only"], totals["edges_sqlglot_only"]
if b + c:
    totals["precision_pct_x100"] = round(10000 * b / (b + c))
if b + g:
    totals["recall_pct_x100"] = round(10000 * b / (b + g))
json.dump({"totals": totals, "models": models_out}, open(out_path, "w"), indent=1)
print(json.dumps(totals, indent=1))
