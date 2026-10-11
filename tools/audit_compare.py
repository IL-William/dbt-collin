"""Compare the blind reading of each audited column with collin's and sqlglot's.

usage: python3 -I audit_compare.py MANIFEST DUMP_DIR PRECISION_JSON AUDIT_DIR

PRECISION_JSON is what precision.py wrote for the same dump. It lists only the
models where sqlglot and collin disagree or sqlglot could not read an output,
so a parsed model it does not list is one where sqlglot reads every edge as
collin does.
Private: names stay in AUDIT_DIR, never in the collin repository.
"""
import collections
import glob
import json
import os
import sys

manifest, dump_dir, prec_path, audit = sys.argv[1:5]
m = json.load(open(manifest))
nodes = {**m["nodes"], **m["sources"]}


def norm(rel):
    return ".".join(p.strip().strip('"').strip("`").upper() for p in (rel or "").split("."))


rel_of = {uid: norm(n.get("relation_name")) for uid, n in nodes.items() if n.get("relation_name")}
sample = {p["id"]: p for p in json.load(open(os.path.join(audit, "sample.json")))}
answers = {}
for f in sorted(glob.glob(os.path.join(audit, "answers-*.json"))):
    for a in json.load(open(f)):
        answers[a["id"]] = a
disagreeing = {mm["name"]: mm for mm in json.load(open(prec_path)).get("models") or []}

rows, tally = [], collections.Counter()
for i, p in sorted(sample.items()):
    a = answers.get(i)
    if a is None:
        continue
    d = json.load(open(os.path.join(dump_dir, p["model"] + ".json")))
    collin = {
        (rel_of.get(e["from"], e["from"].upper()), e["from_col"].lower())
        for e in d.get("published") or []
        if e["to_col"] == p["column"]
    }
    kinds = sorted({e["kind"] for e in d.get("published") or [] if e["to_col"] == p["column"]})
    hand = {(norm(x["relation"]), x["column"].lower()) for x in a.get("inputs") or []}

    # sqlglot's reading, rebuilt from where it departs from collin's.
    g = disagreeing.get(p["model"])
    if g is None:
        sqlglot = collin if d.get("plan") == "parsed" else None
    elif p["column"] in (g.get("unread") or []):
        sqlglot = None
    else:
        c_only = {(norm(r), fc.lower()) for r, fc, tc in g.get("collin_only") or [] if tc == p["column"]}
        g_only = {(norm(r), fc.lower()) for r, fc, tc in g.get("sqlglot_only") or [] if tc == p["column"]}
        sqlglot = (collin - c_only) | g_only

    verdict = "agree" if collin == hand else "differ"
    tally[(p["stratum"], verdict)] += 1
    rows.append({
        "id": i, "stratum": p["stratum"], "model": p["model"], "column": p["column"],
        "verdict": verdict, "unsure": a.get("unsure"), "kinds": kinds,
        "collin_not_hand": sorted(collin - hand), "hand_not_collin": sorted(hand - collin),
        "sqlglot_equals_hand": None if sqlglot is None else sqlglot == hand,
        "note": a.get("note"),
    })
json.dump(rows, open(os.path.join(audit, "compare.json"), "w"), indent=1)
for k, v in sorted(tally.items()):
    print(k, v)
print("sqlglot equals the blind reading:", dict(collections.Counter(r["sqlglot_equals_hand"] for r in rows)))
print("collin differs on:", [r["id"] for r in rows if r["verdict"] == "differ"])
