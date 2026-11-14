#!/usr/bin/env python3
"""OLP 单元登记与健康识别(多机部署)。

一个"单元"是一台跑内环车道的机器(例如一台 Mac mini)。hub 是放黑板、
跑大外环的那台机器。本工具回答两个问题:这台机器是不是登记过的那个
单元;它现在能不能接卡。

子命令
  hub 上:
    register <单元号> --hostname H --mac M [--hostkey FP] [--serial S] [--note T]
        登记或更新一个单元。单元号、主机名、网卡地址、主机密钥各自唯一。
    set-state <单元号> online|offline
        人工标记离线(不派卡)或恢复。
    receive [--unit <单元号>]
        从 stdin 读一条心跳并落盘。--unit 由 hub 的 authorized_keys 强制
        命令给出时,心跳自报的单元号必须与它一致。
    status [--json] [--stale-after 秒]
        汇总每个单元的状态。
    check <单元号> [--stale-after 秒]
        派卡前的检查:只有"正常"退出 0。
    where <单元号>
        输出 local(登记的主机名就是本机,即单机部署)或 remote。派卡的
        唤醒与回答提问据此决定走本机还是 ssh。
  单元上:
    heartbeat --unit <单元号> [--require serve,disk] [--disk-min-free 百分比]
        做本机巡检,把一条心跳(JSON)打印到 stdout。发送由调用方完成:
            olp-unit.py heartbeat --unit m07 | ssh hub olp-unit.py receive

状态(由 hub 判定)
    healthy   正常  心跳按时,身份相符,巡检全过            → 可以派卡
    degraded  降级  心跳按时,但巡检有项不过或身份不符      → 不派新卡
    stale     失联  没有心跳,或超过 --stale-after(默认 180 秒)→ 不派卡,告警
    offline   离线  人工标记                                → 不派卡

身份
    - 网卡地址:登记的地址必须出现在心跳上报的本机网卡地址里。它用于
      DHCP 保留和认机器,**不是**安全凭据(可以伪造)。
    - 主机名:心跳上报的主机名(第一个标签、小写)必须等于登记值。
    - 信任锚是 ssh:hub 的 authorized_keys 给每个单元的密钥配强制命令
      `olp-unit.py receive --unit m07`,这样一个单元无法冒充另一个。
      登记的主机密钥指纹用于 hub → 单元方向(known_hosts 固定),本工具
      只保证它在登记文件里唯一。
    - 是否失联按 hub 收到心跳的时间算,不看单元自己的时钟。

内环是谁
    内环可以是 octoscode(octos)、Claude Code 或 Codex。单元上的内环跑在
    herdr 的窗格里,心跳以 `herdr agent list` 为准报告有哪些内环、各自
    状态(字段 agents)。herdr 不在 PATH 上时用环境变量 OLP_HERDR 指定。

启动权限
    心跳带上本机内环的启动方式,hub 在 status 里显示:
        full       完全访问:每个内环都以完全访问启动
        sandboxed  沙箱:都没有
        mixed      混合:有的有有的没有
        none       没有内环在跑
        unknown    看不出来:读不到进程表;或者有 Claude Code / Codex 内环
                   没带完全访问的启动参数——它们的权限可能来自各自的配置
                   文件,本工具不读配置
    完全访问的判定只看启动参数:octos 后端的 --danger-full-access;
    Claude Code 的 --dangerously-skip-permissions 或
    --permission-mode bypassPermissions;Codex 的
    --dangerously-bypass-approvals-and-sandbox 或
    --sandbox danger-full-access。命令行的其余部分不离开单元。这是单元
    **自报**的,用来让大外环知道各台机器的实际情况,不是安全边界,也不
    参与健康判定。

巡检项
    内置:agent(herdr 里至少有一个内环,任何一种)、serve(本机有
    `octos serve` 进程,只适用于 octos 内环)、disk(HOME 所在磁盘的空闲
    比例不低于阈值)。--require 选择必须通过的内置项,默认 agent,disk。
    扩展:${OLP_UNIT_CHECKS_DIR:-~/.octos/outer/unit-checks.d} 下的每个
    可执行文件是一项检查——退出 0 为通过,其余为不通过,stdout 第一行作
    为说明,10 秒超时。项目专用的检查(设备、循环是否暂停等)放在这里。

文件(hub)
    ${OLP_UNITS_DIR:-~/.octos/outer/units}/registry.tsv
        每行:单元号 主机名 网卡地址 主机密钥指纹 序列号 enabled 备注(制表符分隔)
    ${OLP_UNITS_DIR}/heartbeat/<单元号>.json
        该单元最近一次心跳,加上 hub 写入的 received_at。

退出码:0 成功;1 check 未通过;2 用法或输入错误;3 心跳被拒绝(未登记、
单元号与强制命令不符)。
"""

import argparse
import datetime
import fcntl
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time

UNIT_RE = re.compile(r"^[a-z][a-z0-9-]{0,31}$")
MAC_RE = re.compile(r"^[0-9a-f]{2}(:[0-9a-f]{2}){5}$")
FIELDS = ["unit", "hostname", "mac", "hostkey", "serial", "enabled", "note"]
UNIQUE = ["hostname", "mac", "hostkey"]
DEFAULT_STALE = 180
MAX_HEARTBEAT_BYTES = 256 * 1024
EXIT_USAGE = 2
EXIT_REJECTED = 3


class Fail(Exception):
    def __init__(self, code, msg):
        super().__init__(msg)
        self.code = code


def units_dir():
    return os.environ.get("OLP_UNITS_DIR") or os.path.join(
        os.path.expanduser("~"), ".octos", "outer", "units"
    )


def registry_path():
    return os.path.join(units_dir(), "registry.tsv")


def heartbeat_path(unit):
    return os.path.join(units_dir(), "heartbeat", unit + ".json")


def norm_host(name):
    return name.strip().split(".")[0].lower()


def norm_mac(mac):
    m = mac.strip().lower().replace("-", ":")
    if not MAC_RE.match(m):
        raise Fail(EXIT_USAGE, f"网卡地址格式不对: {mac}(应为 aa:bb:cc:dd:ee:ff)")
    return m


def clean_field(name, value):
    if any(c in value for c in "\t\r\n"):
        raise Fail(EXIT_USAGE, f"{name} 不能含制表符或换行")
    return value.strip()


def check_unit_name(unit):
    if not UNIT_RE.match(unit):
        raise Fail(EXIT_USAGE, f"单元号不合法: {unit}(小写字母开头,只含 [a-z0-9-],例如 m07)")
    return unit


def load_registry():
    rows = {}
    path = registry_path()
    if not os.path.isfile(path):
        return rows
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\r\n")
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            parts += [""] * (len(FIELDS) - len(parts))
            row = dict(zip(FIELDS, parts))
            if UNIT_RE.match(row["unit"]):
                rows[row["unit"]] = row
    return rows


def write_atomic(path, text):
    directory = os.path.dirname(path)
    os.makedirs(directory, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=".olp-unit-", dir=directory)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(text)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, path)
    except BaseException:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise


def save_registry(rows):
    lines = ["# unit\thostname\tmac\thostkey\tserial\tenabled\tnote\n"]
    for unit in sorted(rows):
        lines.append("\t".join(rows[unit][k] for k in FIELDS) + "\n")
    write_atomic(registry_path(), "".join(lines))


class RegistryLock:
    def __enter__(self):
        os.makedirs(units_dir(), exist_ok=True)
        self.fd = os.open(registry_path() + ".lock", os.O_WRONLY | os.O_CREAT, 0o644)
        fcntl.flock(self.fd, fcntl.LOCK_EX)
        return self

    def __exit__(self, *exc):
        os.close(self.fd)


# --- hub: register / set-state ------------------------------------------


def cmd_register(args):
    unit = check_unit_name(args.unit)
    row = {
        "unit": unit,
        "hostname": norm_host(clean_field("--hostname", args.hostname)),
        "mac": norm_mac(args.mac),
        "hostkey": clean_field("--hostkey", args.hostkey or ""),
        "serial": clean_field("--serial", args.serial or ""),
        "enabled": "yes",
        "note": clean_field("--note", args.note or ""),
    }
    if not row["hostname"]:
        raise Fail(EXIT_USAGE, "--hostname 不能为空")
    with RegistryLock():
        rows = load_registry()
        for other, existing in rows.items():
            if other == unit:
                continue
            for key in UNIQUE:
                if row[key] and row[key] == existing[key]:
                    raise Fail(
                        EXIT_USAGE,
                        f"{key} {row[key]} 已登记给单元 {other},一个值只能属于一个单元",
                    )
        if unit in rows:
            # 重新登记不改变人工标记的离线状态。
            row["enabled"] = rows[unit]["enabled"] or "yes"
        rows[unit] = row
        save_registry(rows)
    print(f"registered {unit} hostname={row['hostname']} mac={row['mac']}")
    return 0


def cmd_set_state(args):
    unit = check_unit_name(args.unit)
    with RegistryLock():
        rows = load_registry()
        if unit not in rows:
            raise Fail(EXIT_USAGE, f"单元未登记: {unit}")
        rows[unit]["enabled"] = "yes" if args.state == "online" else "no"
        save_registry(rows)
    print(f"{unit} {args.state}")
    return 0


# --- unit: heartbeat ----------------------------------------------------


def local_macs():
    macs = set()
    sysfs = "/sys/class/net"
    if os.path.isdir(sysfs):
        for iface in os.listdir(sysfs):
            if iface == "lo":
                continue
            try:
                with open(os.path.join(sysfs, iface, "address"), "r", encoding="ascii") as f:
                    mac = f.read().strip().lower()
            except OSError:
                continue
            if MAC_RE.match(mac) and mac != "00:00:00:00:00:00":
                macs.add(mac)
    ifconfig = shutil.which("ifconfig") or "/sbin/ifconfig"
    if not macs and os.path.exists(ifconfig):
        try:
            out = subprocess.run(
                [ifconfig, "-a"], capture_output=True, text=True, timeout=10
            ).stdout
        except (OSError, subprocess.SubprocessError):
            out = ""
        for m in re.finditer(r"\bether\s+([0-9a-fA-F:]{17})\b", out):
            macs.add(m.group(1).lower())
    return sorted(macs)


def check_serve():
    try:
        r = subprocess.run(
            ["pgrep", "-f", "octos serve"], capture_output=True, text=True, timeout=10
        )
    except (OSError, subprocess.SubprocessError) as e:
        return False, f"pgrep 不可用: {e}"
    pids = [p for p in r.stdout.split() if p.isdigit() and int(p) != os.getpid()]
    if pids:
        return True, f"{len(pids)} 个 octos serve 进程"
    return False, "没有 octos serve 进程"


def herdr_bin():
    return os.environ.get("OLP_HERDR") or shutil.which("herdr") or "herdr"


def herdr_json(args):
    """Run a herdr command that prints JSON. Returns (result, error text)."""
    try:
        r = subprocess.run(
            [herdr_bin(), *args], capture_output=True, text=True, timeout=10
        )
    except (OSError, subprocess.SubprocessError) as e:
        return None, f"无法运行 herdr: {e}"
    try:
        doc = json.loads(r.stdout or "{}")
    except ValueError:
        return None, (r.stderr.strip() or "herdr 的输出不是 JSON")[:200]
    if isinstance(doc, dict) and doc.get("error"):
        err = doc["error"]
        return None, str(err.get("message") or err.get("code") or err)[:200]
    if r.returncode != 0 or not isinstance(doc, dict):
        return None, (r.stderr.strip() or f"herdr 退出码 {r.returncode}")[:200]
    return doc.get("result") or {}, ""


def inner_agents():
    """The inner loops herdr knows on this machine: [{name, kind, status, pane}].
    None (with a reason) when herdr cannot be asked."""
    result, err = herdr_json(["agent", "list"])
    if result is None:
        return None, err
    agents = []
    for a in result.get("agents") or []:
        if not isinstance(a, dict):
            continue
        agents.append(
            {
                "name": str(a.get("name") or ""),
                "kind": str(a.get("agent") or "unknown"),
                "status": str(a.get("agent_status") or "unknown"),
                "pane": str(a.get("pane_id") or ""),
            }
        )
    return agents, ""


def check_agent(agents, why):
    if agents is None:
        return False, f"读不到 herdr 里的内环:{why}"
    if not agents:
        return False, "herdr 里没有内环"
    shown = "、".join(f"{a['kind']}({a['status']})" for a in agents)
    return True, f"{len(agents)} 个内环:{shown}"


def full_access_flags(kind, argv):
    """True / False from the launch flags of a Claude Code or Codex process;
    None when the flags say nothing (the agent's config file decides)."""
    if kind == "claude":
        if "--dangerously-skip-permissions" in argv:
            return True
        for i, a in enumerate(argv):
            if a == "--permission-mode" and i + 1 < len(argv):
                return argv[i + 1] == "bypassPermissions"
            if a.startswith("--permission-mode="):
                return a.split("=", 1)[1] == "bypassPermissions"
        return None
    if kind == "codex":
        if "--dangerously-bypass-approvals-and-sandbox" in argv or "--yolo" in argv:
            return True
        for i, a in enumerate(argv):
            if a in ("-s", "--sandbox") and i + 1 < len(argv):
                return argv[i + 1] == "danger-full-access"
            if a.startswith("--sandbox="):
                return a.split("=", 1)[1] == "danger-full-access"
        return None
    return None


def agent_permissions(agents):
    """Launch permission of each Claude Code / Codex inner loop, read from the
    argv of the pane's foreground process (herdr pane process-info)."""
    out = []
    for a in agents or []:
        if a["kind"] not in ("claude", "codex") or not a["pane"]:
            continue
        result, _ = herdr_json(["pane", "process-info", "--pane", a["pane"]])
        procs = ((result or {}).get("process_info") or {}).get("foreground_processes") or []
        argv = next(
            (p.get("argv") or [] for p in procs if p.get("name") == a["kind"] or p.get("argv0") == a["kind"]),
            None,
        )
        full = full_access_flags(a["kind"], argv) if argv is not None else None
        out.append({"kind": a["kind"], "pane": a["pane"], "full_access": full})
    return out


def combine_permission(octos_permission, others):
    """Fold the octos backends' permission and the other inner loops' into one."""
    if octos_permission == "unknown" or any(o["full_access"] is None for o in others):
        return "unknown"
    values = [o["full_access"] for o in others]
    if octos_permission == "full":
        values.append(True)
    elif octos_permission == "sandboxed":
        values.append(False)
    elif octos_permission == "mixed":
        values += [True, False]
    if not values:
        return "none"
    if all(values):
        return "full"
    if not any(values):
        return "sandboxed"
    return "mixed"


SERVE_ARGV0 = re.compile(r"^octos(-[a-z0-9]+)?$")
PERMISSIONS = ("full", "sandboxed", "mixed", "none", "unknown")


def serve_permission(ps_output_file=None):
    """How the inner-loop backends on this machine were launched.

    Reads the process table and looks at every `octos serve` backend (the
    TUI that wraps it carries the same words inside one argument and is not
    counted). Only two launch flags are read — `--danger-full-access` and
    `--solo`; the rest of the command line never leaves the machine.

    Returns (permission, backends): permission is "full" when every backend
    has --danger-full-access, "sandboxed" when none has, "mixed" otherwise,
    "none" when no backend runs, "unknown" when the process table cannot be
    read."""
    if ps_output_file:
        try:
            with open(ps_output_file, "r", encoding="utf-8", errors="replace") as f:
                text = f.read()
        except OSError:
            return "unknown", []
    else:
        try:
            r = subprocess.run(
                ["ps", "-axo", "pid=,command="], capture_output=True, text=True, timeout=10
            )
        except (OSError, subprocess.SubprocessError):
            return "unknown", []
        if r.returncode != 0:
            return "unknown", []
        text = r.stdout
    backends = []
    for line in text.splitlines():
        parts = line.split()
        if len(parts) < 3 or not parts[0].isdigit():
            continue
        argv = parts[1:]
        if not SERVE_ARGV0.match(os.path.basename(argv[0])) or argv[1] != "serve":
            continue
        backends.append(
            {
                "pid": int(parts[0]),
                "full_access": "--danger-full-access" in argv,
                "solo": "--solo" in argv,
            }
        )
    if not backends:
        return "none", []
    full = [b["full_access"] for b in backends]
    if all(full):
        return "full", backends
    if not any(full):
        return "sandboxed", backends
    return "mixed", backends


def check_disk(min_free):
    usage = shutil.disk_usage(os.path.expanduser("~"))
    free = usage.free * 100.0 / usage.total if usage.total else 0.0
    return free >= min_free, f"空闲 {free:.1f}%(阈值 {min_free:g}%)"


def run_plugin_checks():
    directory = os.environ.get("OLP_UNIT_CHECKS_DIR") or os.path.join(
        os.path.expanduser("~"), ".octos", "outer", "unit-checks.d"
    )
    results = []
    if not os.path.isdir(directory):
        return results
    for name in sorted(os.listdir(directory)):
        path = os.path.join(directory, name)
        if name.startswith(".") or not os.path.isfile(path) or not os.access(path, os.X_OK):
            continue
        try:
            r = subprocess.run([path], capture_output=True, text=True, timeout=10)
            ok = r.returncode == 0
            detail = (r.stdout.strip().splitlines() or [""])[0][:200]
            if not ok and not detail:
                detail = f"exit {r.returncode}"
        except subprocess.TimeoutExpired:
            ok, detail = False, "超时(10 秒)"
        except OSError as e:
            ok, detail = False, f"无法执行: {e}"
        results.append({"name": name, "ok": ok, "detail": detail})
    return results


def cmd_heartbeat(args):
    unit = check_unit_name(args.unit)
    agents, agents_why = inner_agents()
    builtin = {
        "agent": lambda: check_agent(agents, agents_why),
        "serve": check_serve,
        "disk": lambda: check_disk(args.disk_min_free),
    }
    required = [r for r in (args.require or "").split(",") if r]
    for r in required:
        if r not in builtin:
            raise Fail(EXIT_USAGE, f"未知的内置巡检项: {r}(可选 agent、serve、disk)")
    checks = []
    for name in required:
        ok, detail = builtin[name]()
        checks.append({"name": name, "ok": ok, "detail": detail})
    checks.extend(run_plugin_checks())
    beat = {
        "v": 1,
        "unit": unit,
        "hostname": norm_host(socket.gethostname()),
        "macs": local_macs(),
        "sent_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "checks": checks,
    }
    octos_permission, backends = serve_permission(args.ps_output)
    others = agent_permissions(agents)
    permission = combine_permission(octos_permission, others)
    if agents is None and permission == "none":
        # No octos backend, and herdr could not say whether another kind of
        # inner loop runs: that is not "none".
        permission = "unknown"
    beat["permission"] = permission
    beat["backends"] = backends
    beat["agents"] = [
        {"kind": a["kind"], "status": a["status"], "pane": a["pane"]} for a in agents or []
    ]
    beat["agent_permissions"] = others
    json.dump(beat, sys.stdout, ensure_ascii=False)
    sys.stdout.write("\n")
    return 0


# --- hub: receive -------------------------------------------------------


def cmd_receive(args):
    raw = sys.stdin.buffer.read(MAX_HEARTBEAT_BYTES + 1)
    if len(raw) > MAX_HEARTBEAT_BYTES:
        raise Fail(EXIT_USAGE, "心跳超过 256 KiB,拒收")
    try:
        beat = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        raise Fail(EXIT_USAGE, "心跳不是合法的 JSON")
    if not isinstance(beat, dict) or not isinstance(beat.get("unit"), str):
        raise Fail(EXIT_USAGE, "心跳缺少 unit 字段")
    unit = beat["unit"]
    if not UNIT_RE.match(unit):
        raise Fail(EXIT_USAGE, "心跳里的单元号不合法")
    if args.unit and args.unit != unit:
        raise Fail(
            EXIT_REJECTED,
            f"心跳自报单元 {unit},但这个连接只允许 {args.unit},拒收",
        )
    if unit not in load_registry():
        raise Fail(EXIT_REJECTED, f"单元未登记: {unit},拒收")

    checks = []
    for c in beat.get("checks") if isinstance(beat.get("checks"), list) else []:
        if isinstance(c, dict) and isinstance(c.get("name"), str):
            checks.append(
                {
                    "name": c["name"][:64],
                    "ok": c.get("ok") is True,
                    "detail": str(c.get("detail", ""))[:200],
                }
            )
    macs = [
        m.lower()
        for m in (beat.get("macs") if isinstance(beat.get("macs"), list) else [])
        if isinstance(m, str) and MAC_RE.match(m.lower())
    ]
    permission = beat.get("permission")
    if permission not in PERMISSIONS:
        permission = "unknown"
    # Inner loops as herdr reported them: only kind, status and pane, each a
    # short plain token, so nothing else the unit sends reaches the hub's files.
    agents = []
    for a in beat.get("agents") if isinstance(beat.get("agents"), list) else []:
        if not isinstance(a, dict):
            continue
        fields = {k: a.get(k) for k in ("kind", "status", "pane")}
        if all(isinstance(v, str) and re.match(r"^[A-Za-z0-9:_.-]{1,40}$", v) for v in fields.values()):
            agents.append(fields)
    backends = []
    for b in beat.get("backends") if isinstance(beat.get("backends"), list) else []:
        if isinstance(b, dict) and isinstance(b.get("pid"), int) and not isinstance(b.get("pid"), bool):
            backends.append(
                {
                    "pid": b["pid"],
                    "full_access": b.get("full_access") is True,
                    "solo": b.get("solo") is True,
                }
            )
    stored = {
        "v": 1,
        "unit": unit,
        "permission": permission,
        "backends": backends[:64],
        "agents": agents[:64],
        "hostname": norm_host(str(beat.get("hostname", ""))),
        "macs": macs[:64],
        "sent_at": str(beat.get("sent_at", ""))[:40],
        "checks": checks[:200],
        # hub 的时钟:是否失联只按它算。
        "received_at": time.time(),
    }
    write_atomic(heartbeat_path(unit), json.dumps(stored, ensure_ascii=False) + "\n")
    print(f"received {unit}")
    return 0


# --- hub: status / check ------------------------------------------------


def evaluate(row, now, stale_after):
    """Return (state, reasons, heartbeat-or-None) for one registered unit."""
    unit = row["unit"]
    beat = None
    try:
        with open(heartbeat_path(unit), "r", encoding="utf-8") as f:
            beat = json.load(f)
        if not isinstance(beat, dict) or not isinstance(beat.get("received_at"), (int, float)):
            beat = None
    except (OSError, ValueError):
        beat = None

    if row["enabled"] == "no":
        return "offline", ["人工标记离线"], beat
    if beat is None:
        return "stale", ["没有收到过心跳"], None
    age = now - beat["received_at"]
    if age > stale_after:
        return "stale", [f"最近一次心跳在 {int(age)} 秒前(上限 {stale_after} 秒)"], beat

    reasons = []
    if beat.get("hostname") != row["hostname"]:
        reasons.append(f"主机名不符:登记 {row['hostname']},上报 {beat.get('hostname') or '空'}")
    if row["mac"] not in (beat.get("macs") or []):
        reasons.append(f"网卡地址不符:登记的 {row['mac']} 不在上报的地址里")
    for c in beat.get("checks") or []:
        if not c.get("ok"):
            detail = f":{c['detail']}" if c.get("detail") else ""
            reasons.append(f"巡检未过 {c.get('name')}{detail}")
    return ("degraded" if reasons else "healthy"), reasons, beat


STATE_ZH = {"healthy": "正常", "degraded": "降级", "stale": "失联", "offline": "离线"}
PERMISSION_ZH = {
    "full": "完全访问",
    "sandboxed": "沙箱",
    "mixed": "混合",
    "none": "无内环",
    "unknown": "未知",
}


def cmd_status(args):
    rows = load_registry()
    now = time.time()
    report = []
    for unit in sorted(rows):
        state, reasons, beat = evaluate(rows[unit], now, args.stale_after)
        report.append(
            {
                "unit": unit,
                "state": state,
                "dispatchable": state == "healthy",
                "reasons": reasons,
                "hostname": rows[unit]["hostname"],
                "mac": rows[unit]["mac"],
                "heartbeat_age_seconds": int(now - beat["received_at"]) if beat else None,
                # 单元自报的启动权限;只是告知,不参与健康判定。
                "permission": (beat or {}).get("permission", "unknown") if beat else None,
                "backends": (beat or {}).get("backends", []) if beat else [],
                "agents": (beat or {}).get("agents", []) if beat else [],
            }
        )
    if args.json:
        json.dump({"units": report}, sys.stdout, ensure_ascii=False, indent=2)
        sys.stdout.write("\n")
        return 0
    if not report:
        print("没有登记任何单元")
        return 0
    for r in report:
        age = "-" if r["heartbeat_age_seconds"] is None else f"{r['heartbeat_age_seconds']}s"
        permission = PERMISSION_ZH.get(r["permission"], "-")
        kinds = ",".join(sorted({a.get("kind", "?") for a in r["agents"]})) or "-"
        line = (
            f"{r['unit']}\t{STATE_ZH[r['state']]}({r['state']})\t心跳 {age}"
            f"\t内环 {kinds}\t权限 {permission}\t{r['hostname']}"
        )
        if r["reasons"]:
            line += "\t" + ";".join(r["reasons"])
        print(line)
    return 0


def is_local_unit(unit):
    """A unit registered with this machine's hostname is this machine."""
    row = load_registry().get(unit)
    return bool(row) and row["hostname"] == norm_host(socket.gethostname())


def cmd_where(args):
    unit = check_unit_name(args.unit)
    if unit not in load_registry():
        raise Fail(EXIT_USAGE, f"单元未登记: {unit}")
    print("local" if is_local_unit(unit) else "remote")
    return 0


def cmd_check(args):
    unit = check_unit_name(args.unit)
    rows = load_registry()
    if unit not in rows:
        raise Fail(EXIT_USAGE, f"单元未登记: {unit}")
    state, reasons, _ = evaluate(rows[unit], time.time(), args.stale_after)
    if state == "healthy":
        print(f"{unit} 正常,可以派卡")
        return 0
    print(f"{unit} {STATE_ZH[state]},不要派卡:{';'.join(reasons)}", file=sys.stderr)
    return 1


def positive_int(text):
    value = int(text)
    if value <= 0:
        raise argparse.ArgumentTypeError("必须是正整数")
    return value


def build_parser():
    ap = argparse.ArgumentParser(prog="olp-unit.py", description="OLP 单元登记与健康识别")
    sub = ap.add_subparsers(dest="cmd")

    p = sub.add_parser("register", help="(hub)登记或更新一个单元")
    p.add_argument("unit")
    p.add_argument("--hostname", required=True)
    p.add_argument("--mac", required=True)
    p.add_argument("--hostkey")
    p.add_argument("--serial")
    p.add_argument("--note")
    p.set_defaults(fn=cmd_register)

    p = sub.add_parser("set-state", help="(hub)人工标记离线或恢复")
    p.add_argument("unit")
    p.add_argument("state", choices=["online", "offline"])
    p.set_defaults(fn=cmd_set_state)

    p = sub.add_parser("heartbeat", help="(单元)巡检并把心跳打印到 stdout")
    p.add_argument("--unit", required=True)
    p.add_argument("--require", default="agent,disk")
    p.add_argument("--disk-min-free", type=float, default=10.0)
    p.add_argument("--ps-output", help=argparse.SUPPRESS)
    p.set_defaults(fn=cmd_heartbeat)

    p = sub.add_parser("where", help="(hub)单元就是本机(local)还是另一台机器(remote)")
    p.add_argument("unit")
    p.set_defaults(fn=cmd_where)

    p = sub.add_parser("receive", help="(hub)从 stdin 接收一条心跳")
    p.add_argument("--unit")
    p.set_defaults(fn=cmd_receive)

    p = sub.add_parser("status", help="(hub)汇总各单元状态")
    p.add_argument("--json", action="store_true")
    p.add_argument("--stale-after", type=positive_int, default=DEFAULT_STALE)
    p.set_defaults(fn=cmd_status)

    p = sub.add_parser("check", help="(hub)派卡前检查,只有正常退出 0")
    p.add_argument("unit")
    p.add_argument("--stale-after", type=positive_int, default=DEFAULT_STALE)
    p.set_defaults(fn=cmd_check)
    return ap


def main():
    ap = build_parser()
    try:
        args = ap.parse_args()
    except SystemExit as exc:
        return EXIT_USAGE if exc.code not in (0, None) else 0
    if not getattr(args, "fn", None):
        ap.print_usage(sys.stderr)
        return EXIT_USAGE
    try:
        return args.fn(args)
    except Fail as e:
        print(f"olp-unit: {e}", file=sys.stderr)
        return e.code


if __name__ == "__main__":
    sys.exit(main())
