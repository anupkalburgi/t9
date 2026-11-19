#!/usr/bin/env python3
"""Generate a synthetic t9 vault for benchmarks.

usage: gen_vault.py VAULT ROOT_DIR --items N --days D --per-day M
Creates project BIG (slug `big`) whose root is ROOT_DIR, with N items and D journal files of M entries each.
"""
import argparse, json, os, random

p = argparse.ArgumentParser()
p.add_argument("vault"); p.add_argument("root")
p.add_argument("--items", type=int, default=2000)
p.add_argument("--days", type=int, default=50)
p.add_argument("--per-day", type=int, default=1000)
a = p.parse_args()
random.seed(9)
d = os.path.join(a.vault, "projects", "big")
for sub in ("items", "journal", "sessions"):
    os.makedirs(os.path.join(d, sub), exist_ok=True)
with open(os.path.join(d, "project.md"), "w") as f:
    f.write(f"---\nkey: BIG\nname: Big\nroots:\n- {a.root}\ncreated: 2026-10-01T00:00:00Z\n---\n\n# Big\n")
words = "invoice refund webhook currency tax payment ledger retry customer export provider rounding".split()
kinds = ["task", "bug", "revisit", "feature", "decision"]
for i in range(1, a.items + 1):
    k = random.choice(kinds)
    st = "accepted" if k == "decision" else random.choice(["open", "done", "done", "active"])
    with open(os.path.join(d, "items", f"BIG-{i}-x.md"), "w") as f:
        f.write(f"---\nid: BIG-{i}\nkind: {k}\ntitle: {' '.join(random.sample(words, 4))}\nstatus: {st}\n"
                f"created: 2026-10-01T00:00:00Z\nupdated: 2026-10-01T00:00:00Z\nrev: 1\n---\n\n"
                f"{' '.join(random.choices(words, k=40))}\n")
n = 0
for day in range(a.days):
    date = f"2026-{9 + (day // 28):02d}-{1 + day % 28:02d}"
    out = [f"# Journal · Big · {date}\n\n"]
    for j in range(a.per_day):
        n += 1
        m = {"id": f"01M0000000000000{n:010d}", "kind": "progress", "at": f"{date}T12:{j // 60 % 60:02d}:{j % 60:02d}Z",
             "item": f"BIG-{random.randint(1, a.items)}", "agent": "bench"}
        out.append(f"<!-- t9 {json.dumps(m)} -->\n### 12:00 · progress\n{' '.join(random.choices(words, k=25))}\n\n")
    with open(os.path.join(d, "journal", f"{date}.md"), "w") as f:
        f.write("".join(out))
print(f"generated {a.items} items, {n} entries")
