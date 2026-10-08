#!/usr/bin/env python3
"""Preflight: lay every sheet over the others before a build.

Lists
  - unfilled cells (a column a row is missing, or null / "" / "TODO"),
  - references (@sheet:id) that don't resolve,
  - cells marked unverified (need a check in the running game or the player's install).

Exit code 1 when anything is unfilled or broken (the build must not run);
unverified cells are reported but don't block a build: they block calling a row done.
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SHEETS = ROOT / "sheets"
REF = re.compile(r"^@([a-z_]+):([a-z0-9_]+)$")


def load():
    sheets = {}
    for p in sorted(SHEETS.glob("*.json")):
        d = json.loads(p.read_text())
        if d.get("sheet") != p.stem:
            raise SystemExit(f"{p.name}: 'sheet' must be {p.stem!r}")
        sheets[p.stem] = d
    return sheets


def walk(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, list):
        for v in value:
            yield from walk(v)
    elif isinstance(value, dict):
        for v in value.values():
            yield from walk(v)


def main():
    sheets = load()
    ids = {name: {r["id"] for r in d["rows"]} for name, d in sheets.items()}
    unfilled, broken, unverified, dupes = [], [], [], []
    total_cells = 0

    for name, d in sheets.items():
        cols = list(d["columns"])
        seen = set()
        for row in d["rows"]:
            rid = row.get("id", "?")
            if rid in seen:
                dupes.append(f"{name}:{rid}")
            seen.add(rid)
            for c in cols:
                total_cells += 1
                v = row.get(c)
                if v is None or v == "" or v == [] or (isinstance(v, str) and "TODO" in v):
                    unfilled.append(f"{name}:{rid}.{c}")
            extra = set(row) - set(cols)
            for c in sorted(extra):
                unfilled.append(f"{name}:{rid}.{c} (column not declared in sheet)")
            for s in walk(row):
                m = REF.match(s)
                if s.startswith("@") and not m:
                    broken.append(f"{name}:{rid} -> {s} (bad reference syntax)")
                elif m and (m.group(1) not in ids or m.group(2) not in ids[m.group(1)]):
                    broken.append(f"{name}:{rid} -> {s}")
        for rid, cols_u in d.get("_unverified", {}).items():
            if rid not in seen:
                broken.append(f"{name}: _unverified names unknown row {rid}")
            for c in cols_u:
                if c not in d["columns"]:
                    broken.append(f"{name}: _unverified names unknown column {rid}.{c}")
                unverified.append(f"{name}:{rid}.{c}")
        # sheet-level extras (hud.font)
        for k, v in d.items():
            if k in ("sheet", "about", "columns", "rows", "_unverified", "_unverified_note"):
                continue
            for s in walk(v):
                m = REF.match(s)
                if m and (m.group(1) not in ids or m.group(2) not in ids[m.group(1)]):
                    broken.append(f"{name}.{k} -> {s}")

    # fortnite assets that nothing uses are dead weight
    used = set()
    for name, d in sheets.items():
        if name == "fortnite_assets":
            continue
        for s in walk(d):
            m = REF.match(s)
            if m and m.group(1) == "fortnite_assets":
                used.add(m.group(2))
    orphans = sorted(ids.get("fortnite_assets", set()) - used - {"outfit", "pickaxe"})

    print(f"sheets: {len(sheets)}  rows: {sum(len(d['rows']) for d in sheets.values())}  cells: {total_cells}")
    for title, items in (("UNFILLED", unfilled), ("BROKEN REFERENCES", broken), ("DUPLICATE IDS", dupes),
                         ("UNUSED FORTNITE ASSETS", orphans)):
        print(f"\n{title}: {len(items)}")
        for i in items:
            print("  " + i)
    print(f"\nUNVERIFIED (need the player's game/install): {len(unverified)}")
    by_sheet = {}
    for u in unverified:
        by_sheet.setdefault(u.split(":")[0], []).append(u)
    for s, us in by_sheet.items():
        print(f"  {s}: {len(us)}  e.g. {us[0]}")
    clean = not (unfilled or broken or dupes or orphans)
    print("\nPREFLIGHT", "CLEAN (build allowed)" if clean else "FAILED (fix before building)")
    return 0 if clean else 1


if __name__ == "__main__":
    sys.exit(main())
