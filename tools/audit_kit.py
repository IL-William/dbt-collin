"""Draw a sample of output columns for a blind hand audit, and write the kit.

usage: python3 -I audit_kit.py MANIFEST DUMP_DIR OUT_DIR SEED
Private: model names stay in OUT_DIR, never in the collin repository.
"""
import json, os, random, sys, collections
manifest, dump_dir, out, seed = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
m = json.load(open(manifest))
nodes = {**m["nodes"], **m["sources"]}
rnd = random.Random(seed)
plain = {"passthrough", "rename", "cast"}
uniform, complex_, roots = [], [], []
for f in sorted(os.listdir(dump_dir)):
    d = json.load(open(os.path.join(dump_dir, f)))
    if d.get("plan") != "parsed":
        continue
    kinds = collections.defaultdict(set)
    for e in d.get("published") or []:
        kinds[e["to_col"]].add(e["kind"])
    for col, ks in kinds.items():
        item = (d["uid"], col)
        uniform.append(item)
        if ks - plain:
            complex_.append(item)
    for r in d.get("roots") or []:
        roots.append((d["uid"], r if isinstance(r, str) else r.get("column")))
pick = []
seen = set()
def draw(pool, n, stratum):
    pool = [p for p in pool if p not in seen]
    for p in rnd.sample(pool, n):
        seen.add(p)
        pick.append({"stratum": stratum, "uid": p[0], "column": p[1]})
draw(uniform, 30, "uniform")
draw(complex_, 30, "complex")
draw(roots, 10, "root")
rnd.shuffle(pick)
os.makedirs(os.path.join(out, "sql"), exist_ok=True)
for i, p in enumerate(pick):
    n = nodes[p["uid"]]
    d = json.load(open(os.path.join(dump_dir, n["name"] + ".json")))
    sqlf = os.path.join(out, "sql", n["name"] + ".sql")
    if not os.path.exists(sqlf):
        open(sqlf, "w").write(n.get("compiled_code") or "")
        json.dump({"model": n["name"], "own_relation": n.get("relation_name"),
                   "parents": [{"relation": v["relation"], "columns": v["columns"]} for v in d.get("visible") or []]},
                  open(os.path.join(out, "sql", n["name"] + ".parents.json"), "w"), indent=1)
    p["id"] = i + 1
    p["model"] = n["name"]
json.dump(pick, open(os.path.join(out, "sample.json"), "w"), indent=1)

# Four batches, one reader each, a model's columns kept together and the SQL
# to read spread evenly.
groups = {}
for p in pick:
    groups.setdefault(p["model"], []).append(p)
size = {m: os.path.getsize(os.path.join(out, "sql", m + ".sql")) for m in groups}
batches, load = [[] for _ in range(4)], [0] * 4
for m in sorted(groups, key=lambda m: -size[m] * len(groups[m])):
    k = min(range(4), key=lambda k: (len(batches[k]), load[k]))
    batches[k] += groups[m]
    load[k] += size[m]
for k, b in enumerate(batches):
    json.dump([{"id": p["id"], "model": p["model"], "column": p["column"]} for p in b],
              open(os.path.join(out, f"batch-{k + 1}.json"), "w"), indent=1)
print(len(uniform), "columns with an edge,", len(complex_), "with a non plain kind,", len(roots), "roots; drew", len(pick))
