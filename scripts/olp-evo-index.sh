#!/usr/bin/env bash
# olp-evo-index.sh — FLAW 索引生成器(#44b, SDD contract v3).
#
# Usage: olp-evo-index.sh <repo-root>
# Writes knowledge/context/evolution/INDEX.md (content-identical reruns
# preserve mtime). retired_prose counts distinct `> 已记录:FLAW-NNN`
# references in docs/OUTER_LOOP_PROTOCOL.md.
set -euo pipefail

REPO="${1:?usage: olp-evo-index.sh <repo-root>}"
[ -d "$REPO" ] || { echo "error: repo-root not found: $REPO" >&2; exit 2; }

python3 -B - "$REPO" "$(dirname "$0")" <<'PY'
import importlib.util, os, re, sys

repo, script_dir = sys.argv[1], sys.argv[2]

spec = importlib.util.spec_from_file_location(
    "olp_evo_lib", os.path.join(script_dir, "olp-evo-lib.py")
)
lib = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lib)

flaw_dir = os.path.join(repo, "knowledge/context/evolution")
rows = []
for name in sorted(os.listdir(flaw_dir)):
    if not (name.startswith("FLAW-") and name.endswith(".md")):
        continue
    if name == "FLAW-template.md":
        continue
    path = os.path.join(flaw_dir, name)
    flaw = lib.parse_flaw(open(path, encoding="utf-8").read())
    fm = flaw["frontmatter"]
    # 44-r2: both present → "<issue> / <pr>"; one → that one.
    _i = (fm.get("issue", "") or "").strip()
    _p = (fm.get("pr", "") or "").strip()
    rows.append(
        (
            fm.get("id", name[:-3]),
            fm.get("status", "unknown"),
            (fm.get("layers", "") or "").strip("[]"),
            f"{_i} / {_p}" if _i and _p else (_i or _p or "—"),
        )
    )

# retired prose: `> 已记录:FLAW-NNN` lines in PROTOCOL, titled by the
# nearest preceding `#` heading
retired = {}
protocol = os.path.join(repo, "docs/OUTER_LOOP_PROTOCOL.md")
if os.path.isfile(protocol):
    heading = ""
    for line in open(protocol, encoding="utf-8"):
        if line.startswith("#"):
            heading = line.lstrip("#").strip()
        m = re.match(r"^>\s*已记录:FLAW-(\d+)", line)
        if m:
            fid = f"FLAW-{m.group(1)}"
            retired.setdefault(fid, heading or "PROTOCOL")

lines = ["# FLAW 索引(生成,勿手改)", ""]
lines.append("| FLAW | 状态 | 层 | issue / PR | 取代散文 |")
lines.append("|---|---|---|---|---|")
for fid, status, layers, issue in sorted(rows, key=lambda r: r[0]):
    prose = retired.get(fid, "—")
    lines.append(f"| {fid} | {status} | {layers} | {issue} | {prose} |")
lines.append("")
lines.append(f"retired_prose: {len(retired)}")
new = "\n".join(lines) + "\n"

out = os.path.join(flaw_dir, "INDEX.md")
if os.path.isfile(out) and open(out, encoding="utf-8").read() == new:
    sys.exit(0)  # identical → preserve mtime
with open(out, "w", encoding="utf-8") as f:
    f.write(new)
print(f"wrote {out}")
PY
