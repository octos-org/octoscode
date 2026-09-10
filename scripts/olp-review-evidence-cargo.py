#!/usr/bin/env python3
"""olp-review-evidence-cargo — 生产 Cargo harness adapter(真实执行入口).

行为证据的"执行为本"合约(specs/task-evo-review-evidence.spec.md):
- 只在真实 Cargo repo 上执行 `cargo test --test <target> <selector>`;
- 真实捕获退出码 + 完整 stdout/stderr(先解析后截断展示);
- 绑定 before/after git HEAD、运行目录、测试 target、selector、manifest 路径、
  manifest SHA256、编译单元 target SHA256 与输出摘要;
- 环境键白名单(如 OLP_ADAPTER_FIXTURE),入 receipt 供复核;
- 结构化 JSON receipt(--receipt 落盘)或错误信封(stdout,exit != 0)。

输入形状与外层 ../prepared-replay-slots.json 的 slot 兼容:
每 slot = 独立 PR 树(repo + manifest + selectors 列表 + pinned HEAD);
重放执行由外层完成,本 adapter 是外层调用的生产入口。

外层实测两漏洞(../outer-adapter-probe-results.json,本 SHA 719e69ab 修复):
- zero-tests-must-reject: selector 未匹配仍 exit0/"pass" → 现在要求 exact
  匹配且非 ignored,且 PASS/FAIL 需 cargo exit code 与 summary 一致;
- foreign-manifest-must-reject: --repo A --manifest B 跨仓库误绑定 → 现在
  manifest 必须属于真实执行 repo(同一 git worktree 内),HEAD before/after
  与 git status 树状全部绑定进 receipt。
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

PASS_SUMMARY = re.compile(r"test result: ok\.", re.M)
FAIL_SUMMARY = re.compile(r"test result: FAILED\.", re.M)
PANIC = re.compile(r"panicked at [^\s:]+:\d+:\d+")
TEST_LINE = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored)", re.M)
# rustc 编译错误形态: "error[E0308]:" 或 "error: <msg>" 紧跟换行/EOF
# (排除 libtest 尾注 "error: test failed, to rerun pass ...")
COMPILE_ERR = re.compile(r"^error(?:\[[A-Z0-9]+\])?: .*$", re.M)
# cargo test --list 行: "<qualified-name>: test" / "<qualified-name>: benchmark"
LIST_LINE = re.compile(r"^(\S+): (test|benchmark)\s*$", re.M)
RUST_IDENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)*$")

# 环境键白名单:只有这些键会注入 cargo 子进程并入 receipt。
DEFAULT_ENV_KEYS = ("OLP_ADAPTER_FIXTURE",)


def sha256_file(path: Path) -> str | None:
    try:
        h = hashlib.sha256()
        with open(path, "rb") as fh:
            for chunk in iter(lambda: fh.read(65536), b""):
                h.update(chunk)
        return h.hexdigest()
    except OSError:
        return None


def emit_json(obj: dict) -> None:
    json.dump(obj, sys.stdout, ensure_ascii=False, indent=1)
    sys.stdout.write("\n")


def fail(code: str, message: str, detail: dict | None = None) -> int:
    err = {"code": code, "message": message}
    if detail is not None:
        err["detail"] = detail
    emit_json({"error": err})
    return 1


def git_head(repo: Path) -> str | None:
    try:
        out = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=str(repo),
            capture_output=True,
            text=True,
            timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    head = out.stdout.strip()
    return head if out.returncode == 0 and re.fullmatch(r"[0-9a-f]{40}", head) else None


def git_worktree_root(repo: Path) -> Path | None:
    """真实执行 repo 的 git worktree 顶层(绝对,realpath);非 git 树返回 None。"""
    try:
        out = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            cwd=str(repo),
            capture_output=True,
            text=True,
            timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    top = out.stdout.strip()
    return Path(top).resolve() if out.returncode == 0 and top else None


def git_status_porcelain(repo: Path) -> str | None:
    """绑定执行前后工作树状态(source/tree pin 的一部分)。"""
    try:
        out = subprocess.run(
            ["git", "status", "--porcelain"],
            cwd=str(repo),
            capture_output=True,
            text=True,
            timeout=30,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    return out.stdout if out.returncode == 0 else None


def tracked_modified_lines(porcelain: str | None) -> list[str]:
    """porcelain 输出中的 tracked 修改行(排除 untracked '??')。

    ROOT-1 #5 采纳 K3 设计: untracked(注入 probe/artifact/target)允许;
    tracked 生产源被改(staged/unstaged/renamed/deleted)→ 拒 —— 绑定
    "编译的 tracked 源 == HEAD 树"。probe 源本身由 test_target_sha256
    hash 锚,两门互补。
    """
    if not porcelain:
        return []
    out = []
    for line in porcelain.splitlines():
        if not line.strip():
            continue
        if line.startswith("??"):
            continue
        out.append(line)
    return out


def fail_if_tracked_modified(repo: Path, phase: str) -> int:
    """执行前后同门: tracked 修改 → 拒;git status 查询失败 ≠ clean → 拒。

    统一错误码 tracked-source-modified(ROOT-2: 合并一套,两种失败共用;
    查询失败不得放行为"无改动"——那等于绕过 tracked 绑定门)。
    返回 0=通过 / 1=已 emit 结构化错误(调用方 return)。
    """
    porcelain = git_status_porcelain(repo)
    if porcelain is None:
        emit_json({"error": {
            "code": "tracked-source-modified",
            "message": f"{phase}: git status 查询失败(不得当作 clean)",
        }})
        return 1
    lines = tracked_modified_lines(porcelain)
    if lines:
        emit_json({
            "error": {
                "code": "tracked-source-modified",
                "message": (
                    f"{phase}: repo 存在 tracked 修改(untracked 探针/工件"
                    f"允许,tracked 生产源须等于 HEAD): {lines[:8]}"
                ),
            }
        })
        return 1
    return 0


def infer_test_target(repo: Path, selector: str) -> str | None:
    """在 tests/*.rs 中定位 selector 定义的集成测试 target 名。"""
    tests_dir = repo / "tests"
    if not tests_dir.is_dir():
        return None
    needle = re.compile(r"fn\s+" + re.escape(selector) + r"\s*\(")
    for f in sorted(tests_dir.glob("*.rs")):
        try:
            if needle.search(f.read_text(encoding="utf-8", errors="replace")):
                return f.stem
        except OSError:
            continue
    return None


def infer_lib_unit_target(repo: Path, selector: str) -> str | None:
    """在 src/**/*.rs 中定位 --lib 单元测试 selector,返回定义文件相对路径。

    selector 可为裸名或全限定路径(mod::name);匹配任一段落即可。
    """
    src_dir = repo / "src"
    if not src_dir.is_dir():
        return None
    last = selector.rsplit("::", maxsplit=1)[-1]
    needle = re.compile(r"fn\s+" + re.escape(last) + r"\s*\(")
    for f in sorted(src_dir.rglob("*.rs")):
        try:
            if needle.search(f.read_text(encoding="utf-8", errors="replace")):
                return str(f.relative_to(repo))
        except OSError:
            continue
    return None


def verify_selector_exact(
    cmd: list[str], cwd: Path, env: dict, selector: str, timeout: int
) -> tuple[bool, str | None]:
    """用 `cargo test -- --list` 验证 selector 精确命中一个真实测试。

    返回 (verified, qualified_name)。verified=False 表示编译失败或
    无任何以 selector 结尾的全限定条目 — 用于阻断外层反例 a
    (no_such_test 0 matched 不得走到执行阶段再判 pass)。
    """
    list_cmd = cmd + ["--", "--list", "--format", "terse"]
    try:
        proc = subprocess.run(
            list_cmd, cwd=str(cwd), capture_output=True, text=True,
            timeout=timeout, env=env,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False, None
    if proc.returncode != 0:
        return False, None
    candidates = []
    for m in LIST_LINE.finditer(proc.stdout or ""):
        name = m.group(1)
        if name == selector or name.endswith("::" + selector):
            candidates.append(name)
    if len(candidates) != 1:
        return False, None
    return True, candidates[0]


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="olp-review-evidence-cargo")
    p.add_argument("--repo", required=True, help="真实 Cargo repo 路径(只读执行)")
    p.add_argument("--manifest", help="Cargo.toml 路径(默认 <repo>/Cargo.toml)")
    p.add_argument("--test-target", help="集成测试 target(默认按 selector 在 tests/*.rs 定位)")
    p.add_argument("--selector", required=True, help="精确测试名过滤(裸名或全限定)")
    p.add_argument("--expect", choices=["pass", "fail"], required=True)
    p.add_argument("--lib", action="store_true",
                   help="执行 --lib 单元测试(src/ 内测试),不经 tests/*.rs 定位")
    p.add_argument("--artifact-dir",
                   help="完整 stdout/stderr 工件留存目录(默认与 --receipt 同目录)")
    p.add_argument("--receipt", help="receipt JSON 输出路径")
    p.add_argument("--timeout", type=int, default=600)
    p.add_argument("--env-key", action="append", default=[],
                   help="额外允许注入的环境键(白名单追加)")
    p.add_argument("--jobs", default=os.environ.get("CARGO_BUILD_JOBS", "4"))
    p.add_argument("--target-dir", default=os.environ.get("CARGO_TARGET_DIR"))
    args = p.parse_args(argv)

    repo = Path(args.repo).resolve()
    manifest = Path(args.manifest).resolve() if args.manifest else repo / "Cargo.toml"
    if not repo.is_dir():
        return fail("repo-missing", f"repo 不存在: {repo}")
    if not manifest.is_file():
        return fail("manifest-missing", f"Cargo manifest 不存在: {manifest}")
    if not RUST_IDENT.fullmatch(args.selector):
        return fail("selector-invalid", f"selector 非合法 Rust 路径: {args.selector}")

    # 外层反例 b: --repo A --manifest B 跨仓库误绑定 — manifest 必须归属
    # 真实执行 repo 的同一 git worktree。
    worktree = git_worktree_root(repo)
    if worktree is None:
        return fail("head-unresolvable", f"repo 非 git 工作树: {repo}")
    try:
        manifest.relative_to(worktree)
    except ValueError:
        return fail(
            "foreign-manifest",
            f"manifest {manifest} 不属于真实执行 repo 的 git worktree {worktree}"
            "(跨仓库绑定拒绝)",
        )

    head_before = git_head(repo)
    if head_before is None:
        return fail("head-unresolvable", f"无法解析 repo 真实 HEAD: {repo}")
    if fail_if_tracked_modified(repo, "执行前"):
        return 1
    status_before = git_status_porcelain(repo)
    if args.lib:
        target = "--lib"
        unit_source = infer_lib_unit_target(repo, args.selector)
        if unit_source is None:
            return fail(
                "no-tests-matched",
                f"selector {args.selector} 在 {repo}/src/**/*.rs 无定义",
            )
        target_source_path = repo / unit_source
    else:
        unit_source = None
        target = args.test_target or infer_test_target(repo, args.selector)
        if target is None:
            return fail(
                "no-tests-matched",
                f"selector {args.selector} 在 {repo}/tests/*.rs 无定义",
            )
        target_source_path = repo / "tests" / f"{target}.rs"

    env = os.environ.copy()
    env["CARGO_BUILD_JOBS"] = str(args.jobs)
    if args.target_dir:
        env["CARGO_TARGET_DIR"] = str(args.target_dir)
    allowed_keys = list(DEFAULT_ENV_KEYS) + list(args.env_key or [])
    env_receipt = {k: os.environ[k] for k in allowed_keys if k in os.environ}

    cmd = ["cargo", "test", "--manifest-path", str(manifest)]
    if args.lib:
        cmd.append("--lib")
    else:
        cmd += ["--test", target]
    cmd.append(args.selector)

    # 外层反例 a 前置闸: cargo --list 定位全限定名,必须恰好命中一个
    verified, qualified = verify_selector_exact(
        cmd, repo, env, args.selector, min(args.timeout, 600)
    )
    if not verified:
        return fail(
            "no-tests-matched",
            f"selector {args.selector} 经 cargo --list 无法唯一定位"
            "(编译失败或零/多重匹配),不得执行",
        )
    # 定位到的全限定名用于 --exact 执行,杜绝子串误匹配
    exec_cmd = cmd[:-1] + [qualified, "--", "--exact", "--test-threads=1", "--nocapture"]
    started = time.time()
    try:
        proc = subprocess.run(
            exec_cmd, cwd=str(repo), capture_output=True, text=True,
            timeout=args.timeout, env=env,
        )
    except FileNotFoundError:
        return fail("cargo-missing", "cargo 不在 PATH")
    except subprocess.TimeoutExpired:
        return fail("execution-timeout", f"cargo 执行超时(>{args.timeout}s)",
                    {"selector": args.selector})
    elapsed = round(time.time() - started, 3)

    head_after = git_head(repo)
    status_after = git_status_porcelain(repo)
    if fail_if_tracked_modified(repo, "执行后"):
        return 1
    stdout_full = proc.stdout or ""
    stderr_full = proc.stderr or ""
    combined = stdout_full + "\n" + stderr_full

    matched = TEST_LINE.findall(stdout_full)
    ran_statuses = {status for name, status in matched if name == qualified}
    ran_ok = "ok" in ran_statuses
    ran_failed = "FAILED" in ran_statuses
    ran_ignored = "ignored" in ran_statuses and not (ran_ok or ran_failed)

    observed: str
    detail_code: str | None = None
    if not ran_statuses:
        # 外层反例 a: 0 matched(全 filtered)不得判 pass
        observed = "no-tests-matched"
        detail_code = "no-tests-matched"
    elif ran_ignored:
        observed = "ignored-not-run"
        detail_code = "ignored-not-run"
    elif (
        ran_ok
        and not ran_failed
        and PASS_SUMMARY.search(stdout_full)
        and proc.returncode == 0
    ):
        # pass 需: 真实执行 + ok 汇总 + cargo exit 0(一致性绑定)
        observed = "pass"
    elif (
        ran_failed
        and not ran_ok
        and FAIL_SUMMARY.search(stdout_full)
        and proc.returncode != 0
        and PANIC.search(combined)
    ):
        # fail 需: 真实执行 + FAILED 汇总 + cargo exit != 0 + panic 锚
        observed = "fail"
    elif ran_failed and FAIL_SUMMARY.search(stdout_full) and proc.returncode != 0:
        observed = "no-assertion-failure"
        detail_code = "no-assertion-failure"
    elif COMPILE_ERR.search(stderr_full):
        observed = "compile-error"
        detail_code = "compile-error"
    else:
        observed = "inconsistent-exit"
        detail_code = "inconsistent-exit"

    # 完整 stdout/stderr 工件留存(评审级证据,非仅尾部)
    artifact_dir = (
        Path(args.artifact_dir).resolve()
        if args.artifact_dir
        else (Path(args.receipt).resolve().parent if args.receipt else None)
    )
    artifacts: dict[str, str] = {}
    if artifact_dir is not None:
        artifact_dir.mkdir(parents=True, exist_ok=True)
        # 审计尾项: 同 selector 多轮重跑不得覆盖 stdout/stderr。文件名用
        # mkstemp 唯一创建(内核保证唯一,同进程同秒重跑/多进程并发同
        # artifact-dir 均不碰撞;秒+pid 不够)。旧 receipt 的 hash 与文件
        # 仍可验证(旧文件不再被覆盖);receipt 引用工件绝对路径,重跑
        # 产生新文件而非改写。
        base = qualified.replace("::", "_")
        out_fd, out_name = tempfile.mkstemp(
            prefix=f"{base}.stdout.", suffix=".log", dir=str(artifact_dir)
        )
        err_fd, err_name = tempfile.mkstemp(
            prefix=f"{base}.stderr.", suffix=".log", dir=str(artifact_dir)
        )
        os.close(out_fd)
        os.close(err_fd)
        out_p = Path(out_name)
        err_p = Path(err_name)
        out_p.write_text(stdout_full, encoding="utf-8")
        err_p.write_text(stderr_full, encoding="utf-8")
        artifacts = {"stdout": str(out_p), "stderr": str(err_p)}

    receipt = {
        "receipt_kind": "cargo-test-execution",
        "adapter": "scripts/olp-review-evidence-cargo.py",
        "argv": exec_cmd,
        "cwd": str(repo),
        "repo": str(repo),
        "worktree_root": str(worktree),
        "manifest": str(manifest),
        "manifest_sha256": sha256_file(manifest),
        "test_target": target,
        "test_target_sha256": sha256_file(target_source_path),
        "test_source": str(target_source_path),
        "selector": args.selector,
        "selector_qualified": qualified,
        "expect": args.expect,
        "env": env_receipt,
        "exit_code": proc.returncode,
        "observed": observed,
        "matched_tests": [name for name, _ in matched],
        "head_before": head_before,
        "head_after": head_after,
        "status_before": status_before,
        "status_after": status_after,
        "artifacts": artifacts,
        "elapsed_seconds": elapsed,
        "stdout_sha256": hashlib.sha256(stdout_full.encode()).hexdigest(),
        "stderr_sha256": hashlib.sha256(stderr_full.encode()).hexdigest(),
        "stdout_tail": stdout_full[-4000:],
        "stderr_tail": stderr_full[-2000:],
        "executed_unix": int(time.time()),
    }

    if args.receipt:
        Path(args.receipt).write_text(
            json.dumps(receipt, ensure_ascii=False, indent=1) + "\n",
            encoding="utf-8",
        )

    if detail_code is not None:
        return fail(
            detail_code,
            f"selector {args.selector} 真实执行形态={observed},非可信终态",
            {"observed": observed, "exit_code": proc.returncode,
             "receipt": args.receipt},
        )
    if observed != args.expect:
        return fail(
            "expectation-violated",
            f"期望 {args.expect} 但真实执行为 {observed}",
            {"observed": observed, "expect": args.expect,
             "exit_code": proc.returncode, "receipt": args.receipt},
        )
    emit_json({"ok": True, "observed": observed, "receipt": args.receipt})
    return 0


if __name__ == "__main__":
    sys.exit(main())
