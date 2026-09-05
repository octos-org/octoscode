#!/usr/bin/env bash
# olp-evo-spec-skeleton.sh — FLAW record → agent-spec contract skeleton
# (#44b, SDD contract v3 specs/task-req-olp-evo-p3.spec.md).
#
# Usage: olp-evo-spec-skeleton.sh <FLAW-NNN.md> [--out <file>]
# Default: skeleton to stdout. --out may only target outside the repo or
# under specs/drafts/ (anything else → exit 2).
set -euo pipefail

SRC="${1:?usage: olp-evo-spec-skeleton.sh <FLAW-NNN.md> [--out <file>]}"
shift || true
OUT=""
while [ $# -gt 0 ]; do
    case "$1" in
        --out) OUT="${2:?--out needs a path}"; shift 2 ;;
        *) echo "error: unknown flag $1" >&2; exit 2 ;;
    esac
done
[ -f "$SRC" ] || { echo "error: not a file: $SRC" >&2; exit 2; }

# 44-r1: --out protection must NOT depend on the caller's cwd — derive
# the target's owning repo from the TARGET's directory, not from cwd.
if [ -n "$OUT" ]; then
    OUT_REAL="$(realpath -m "$OUT")"
    OUT_DIR="$(dirname "$OUT_REAL")"
    mkdir -p "$OUT_DIR"
    TARGET_REPO="$(git -C "$OUT_DIR" rev-parse --show-toplevel 2>/dev/null || true)"
    if [ -n "$TARGET_REPO" ]; then
        case "$OUT_REAL" in
            "$TARGET_REPO"/specs/drafts/*) : ;;
            "$TARGET_REPO"/specs/*)
                echo "refusing to write outside specs/drafts/" >&2
                exit 2
                ;;
        esac
    fi
fi

python3 -B - "$SRC" "$OUT" "$(dirname "$0")" <<'PY'
import importlib.util, os, re, sys

src, out, script_dir = sys.argv[1], sys.argv[2], sys.argv[3]

spec = importlib.util.spec_from_file_location(
    "olp_evo_lib", os.path.join(script_dir, "olp-evo-lib.py")
)
lib = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lib)

flaw = lib.parse_flaw(open(src, encoding="utf-8").read())
fm = flaw["frontmatter"]
sections = flaw["sections"]
paths = flaw["paths"]

req = fm.get("req", "") or fm.get("satisfies", "") or fm.get("requirement", "")
flaw_id = fm.get("id", os.path.basename(src))

# decision-source section alias: 修复 else 结案
decision_sec = sections.get("修复") or sections.get("结案") or ""
# forbidden-source alias: 预防 else 保护门
forbidden_sec = sections.get("预防") or sections.get("保护门") or ""

def list_items(body):
    items = [l[2:].strip() for l in body.split("\n") if l.startswith("- ")]
    if not items and body.strip():
        items = [body.strip()]
    return items

decisions = list_items(decision_sec)
forbiddens = list_items(forbidden_sec)

def slugify(text, n):
    parts = re.findall(r"[A-Za-z0-9_]+", text)
    slug = "_".join(p.lower() for p in parts)
    if not slug:
        return f"item_{n}"
    # 44b-r1: cap the slug at 48 chars, cutting only at segment
    # boundaries; if the first segment alone exceeds it, hard-cut.
    if len(slug) > 48:
        out = ""
        for seg in slug.split("_"):
            cand = f"{out}_{seg}" if out else seg
            if len(cand) > 48:
                break
            out = cand
        slug = out or slug[:48]
        if not slug:
            return f"pending_item_{n}"
    return slug

lines = []
lines.append(f"spec: task")
lines.append(f'name: "骨架(来自 {flaw_id},待人工补全)"')
lines.append(f"satisfies: [{req}]" if req else "satisfies: []")
lines.append("tags: [evo-skeleton]")
lines.append("estimate: TODO")
lines.append("---")
lines.append("")
lines.append("## 意图")
lines.append("")
# 44-r2 (#10): EITHER missing section keeps its own positional TODO
# placeholder — 症状 first, then 根因.
intent = sections.get("症状", "").strip() or "<!-- TODO: 症状 -->"
root_cause = sections.get("根因", "").strip() or "<!-- TODO: 根因 -->"
lines.append(f"{intent}\n\n根因:{root_cause}")
lines.append("")
lines.append("## 已定决策")
lines.append("")
if not decisions:
    lines.append("<!-- TODO: 已定决策(来源段:修复/结案) -->")
for d in decisions:
    lines.append(f"- {d}")
lines.append("")
lines.append("## 边界")
lines.append("")
lines.append("### Allowed Changes")
if not paths:
    lines.append("<!-- TODO: Allowed(来源段:责任步/锚点 paths) -->")
for p in paths:
    lines.append(f"- {p}")
lines.append("")
lines.append("### Forbidden")
if not forbiddens:
    lines.append("<!-- TODO: Forbidden(来源段:预防/保护门) -->")
for f in forbiddens:
    lines.append(f"- {f}")
lines.append("")
lines.append("## 排除范围")
lines.append("")
lines.append("<!-- TODO: 排除范围 -->")
lines.append("")
lines.append("## 完成条件")
lines.append("")
if not decisions:
    lines.append("<!-- TODO: 完成条件 -->")
n = 1
for d in decisions:
    lines.append(f"场景: {d[:40]}")
    lines.append(f"  测试: pending_{slugify(d, n)}")
    lines.append("  假设 TODO")
    lines.append("  当 TODO")
    lines.append("  那么 TODO")
    lines.append("")
    n += 1
lines.append("## 问题")
lines.append("")
if not req:
    lines.append("- 未绑定需求")

text = "\n".join(lines).rstrip() + "\n"
if out:
    with open(out, "w", encoding="utf-8") as f:
        f.write(text)
    print(f"wrote {out}")
else:
    sys.stdout.write(text)
PY
