"""Attribute each column without an edge on a BigQuery project to a first cause.
usage: python3 -I bq_causes.py DUMP_DIR REPORT CLASSES_JSON"""
import json, os, sys, collections
dump_dir, report_path = sys.argv[1:3]
rep = {m["unique_id"]: m for m in json.load(open(report_path))["models"]}
first = collections.Counter(); models = collections.defaultdict(collections.Counter); deg = collections.Counter()
for f in sorted(os.listdir(dump_dir)):
    d = json.load(open(os.path.join(dump_dir, f)))
    produced = {e["to_col"] for e in d.get("published") or []}
    roots = set(d.get("roots") or [])
    wh, decl, plan = d.get("warehouse"), d.get("declared"), d.get("plan")
    kept = plan == "parsed" and d.get("outputs") and not (d.get("unbacked") and (wh or decl))
    reference = wh if wh else (d.get("outputs") if kept else (decl or []))
    nested_parent = any("." in c for v in d.get("visible") or [] for c in v.get("columns", []))
    merged = bool(d.get("merged_scopes"))
    lost = {l["column"]: sorted({e["reason"] for e in l["ends"]}) for l in d.get("lost_ends") or []}
    if d.get("agreement") == "degraded" or (isinstance(d.get("agreement"), dict) and "degraded" in json.dumps(d["agreement"])):
        wh_nested = wh and any("." in c for c in wh)
        deg["nested fields in its own table" if wh_nested else "other"] += 1
    for c in reference:
        if c in produced or c in roots: continue
        if "." in c: k = "catalog nested field taken for a column"
        elif d.get("parse_error"): k = "parse failed"
        elif merged and plan != "parsed": k = "two scopes of one name (0018)"
        elif c in lost and "unresolved" in lost[c] and nested_parent: k = "lost, unresolved, a parent has STRUCTs"
        elif c in lost: k = "lost: " + "+".join(lost[c])
        elif plan == "inferred": k = "inferred, no parent has the name"
        elif c not in (d.get("outputs") or []): k = f"table only ({plan})"
        else: k = f"other ({plan})"
        first[k] += 1; models[k][d["name"]] += 1
print("gaps", sum(first.values()))
for k, v in first.most_common(): print(f"{v:6} {k:45} " + ", ".join(f"{n} {c}" for n, c in models[k].most_common(3)))
print("degraded:", dict(deg))
