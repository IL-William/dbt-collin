"""Second, harder audit sample: deep models, many inputs, loud kinds.

usage: python3 -I audit_kit_hard.py MANIFEST DUMP_DIR OUT_DIR SEED
"""
import json, os, random, sys, re, collections
manifest, dump_dir, out, seed = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
m = json.load(open(manifest))
nodes = {**m["nodes"], **m["sources"]}
rnd = random.Random(seed)
prev = json.load(open(os.path.join(out, "sample.json")))
seen = {(p["uid"], p["column"]) for p in prev}
deep, multi, loud = [], [], []
for f in sorted(os.listdir(dump_dir)):
    d = json.load(open(os.path.join(dump_dir, f)))
    if d.get("plan") != "parsed":
        continue
    sql = nodes[d["uid"]].get("compiled_code") or ""
    ctes = len(re.findall(r"\bas\s*\(", sql, re.I))
    joins = len(re.findall(r"\bjoin\b", sql, re.I))
    inputs = collections.defaultdict(set)
    kinds = collections.defaultdict(set)
    for e in d.get("published") or []:
        inputs[e["to_col"]].add((e["from"], e["from_col"]))
        kinds[e["to_col"]].add(e["kind"])
    for col in d.get("outputs") or []:
        item = (d["uid"], col)
        if item in seen:
            continue
        if ctes >= 4 and joins >= 1:
            deep.append(item)
        if len(inputs[col]) >= 3:
            multi.append(item)
        if kinds[col] & {"window", "aggregate", "hash"}:
            loud.append(item)
pick = []
def draw(pool, n, stratum):
    pool = sorted(set(p for p in pool if p not in seen))
    for p in rnd.sample(pool, n):
        seen.add(p)
        pick.append({"stratum": stratum, "uid": p[0], "column": p[1]})
draw(deep, 20, "deep")
draw(multi, 10, "multi")
draw(loud, 15, "loud")
rnd.shuffle(pick)
for i, p in enumerate(pick):
    n = nodes[p["uid"]]
    d = json.load(open(os.path.join(dump_dir, n["name"] + ".json")))
    sqlf = os.path.join(out, "sql", n["name"] + ".sql")
    if not os.path.exists(sqlf):
        open(sqlf, "w").write(n.get("compiled_code") or "")
        json.dump({"model": n["name"], "own_relation": n.get("relation_name"),
                   "parents": [{"relation": v["relation"], "columns": v["columns"]} for v in d.get("visible") or []]},
                  open(os.path.join(out, "sql", n["name"] + ".parents.json"), "w"), indent=1)
    p["id"] = 101 + i
    p["model"] = n["name"]
json.dump(prev + pick, open(os.path.join(out, "sample.json"), "w"), indent=1)
groups = {}
for p in pick:
    groups.setdefault(p["model"], []).append(p)
batches = [[] for _ in range(3)]
for g in sorted(groups.values(), key=lambda g: -len(g)):
    min(batches, key=len).extend(g)
for k, b in enumerate(batches):
    json.dump([{"id": p["id"], "model": p["model"], "column": p["column"]} for p in b],
              open(os.path.join(out, f"batch-{5 + k}.json"), "w"), indent=1)
    print(5 + k, len(b))
print(len(deep), "deep,", len(multi), "multi,", len(loud), "loud")
