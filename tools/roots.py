"""For each root collin named, follow every column its expression reads through the CTEs.

usage: python3 -I roots.py MANIFEST DUMP_DIR
A root is 'real' when every path ends at a literal; 'reads_table' when some path reaches
a column of a relation that is not a CTE: a candidate false root.
"""
import json, os, sys, collections
import sqlglot
from sqlglot import exp

m = json.load(open(sys.argv[1]))
sql_of = {u: n.get("compiled_code") or "" for u, n in m["nodes"].items() if n["resource_type"] == "model"}

def selects(node):
    if isinstance(node, exp.Union):
        return selects(node.left) + selects(node.right)
    if isinstance(node, (exp.Subquery, exp.Paren)):
        return selects(node.this)
    if isinstance(node, exp.Select):
        return [node]
    return []

def sources(sel):
    """alias -> table name, for the FROM and JOINs of this select only."""
    out = {}
    fr = sel.args.get("from_") or sel.args.get("from")
    items = []
    if fr is not None:
        items.append(fr.this)
    for j in sel.args.get("joins") or []:
        items.append(j.this)
    for t in items:
        if isinstance(t, exp.Table):
            out[t.alias_or_name.lower()] = ("table", t.name.lower())
        elif isinstance(t, exp.Subquery):
            out[t.alias_or_name.lower()] = ("sub", t.this)
    return out

def classify(sql, col):
    try:
        tree = sqlglot.parse_one(sql, read="snowflake")
    except Exception:
        return "unparsed", []
    ctes = {}
    for c in tree.find_all(exp.CTE):
        ctes.setdefault(c.alias_or_name.lower(), c.this)
    reads = []
    seen = set()

    def column_in(node, name, depth):
        key = (id(node), name)
        if depth > 25 or key in seen:
            return
        seen.add(key)
        for s in selects(node):
            srcs = sources(s)
            hit = None
            for p in s.expressions:
                if p.alias_or_name.lower() == name and not isinstance(p, exp.Star):
                    hit = p
                    break
            if hit is None:
                # through a star
                for alias, (kind, what) in srcs.items():
                    if kind == "table" and what in ctes:
                        column_in(ctes[what], name, depth + 1)
                    elif kind == "sub":
                        column_in(what, name, depth + 1)
                    elif kind == "table":
                        reads.append((what, name, "star"))
                continue
            expr = hit.this if isinstance(hit, exp.Alias) else hit
            for c in expr.find_all(exp.Column):
                if isinstance(c.this, exp.Star):
                    continue
                cname = c.name.lower()
                q = c.table.lower()
                cands = [srcs[q]] if q in srcs else list(srcs.values())
                for kind, what in cands:
                    if kind == "table" and what in ctes:
                        column_in(ctes[what], cname, depth + 1)
                    elif kind == "sub":
                        column_in(what, cname, depth + 1)
                    elif kind == "table":
                        reads.append((what, cname, "expr"))

    column_in(tree, col, 0)
    return ("reads_table" if reads else "literal_only"), reads

kinds = collections.Counter()
examples = []
for f in sorted(os.listdir(sys.argv[2])):
    d = json.load(open(os.path.join(sys.argv[2], f)))
    if not d.get("roots"):
        continue
    sql = sql_of.get(d["uid"], "")
    for c in d["roots"]:
        k, reads = classify(sql, c)
        kinds[k] += 1
        if k == "reads_table" and len(examples) < 40:
            examples.append((d["name"], c, reads[:3]))
print(kinds)
for e in examples:
    print(e)
