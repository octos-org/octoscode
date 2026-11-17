# OctoLoop:内环用 Claude Code 或 Codex

> 内环不一定是 octoscode。单元上的内环也可以是 Claude Code 或 Codex,黑板、派卡、
> 唤醒、提问、推送、负哨这些都照常工作。这一页说明它们各自要怎么接,以及哪里和
> octoscode 不一样。下面的结论来自 2026-11-14 在一台 Mac 上的实测:Claude Code
> 2.1.319、Codex 0.167.1,跑在 herdr 0.9.5 的窗格里,单机部署(hub 就是本机)。

## 1. 一览

| | octoscode | Claude Code | Codex |
|---|---|---|---|
| herdr 的种类 | `octoscode` | `claude` | `codex` |
| 读项目里的 `AGENTS.md` 上岗卡 | 读 | 读(实测) | 读(实测) |
| 提问工具 `ask_outer` 怎么挂 | profile 里的 `mcp_servers` | `--mcp-config <文件>` 或 `claude mcp add` | `-c mcp_servers.olp.…` 或 `codex mcp add`,**另要放行审批**(§3) |
| 没人回答时 50 秒拿到降级指引 | 是 | 是(实测) | 是(实测) |
| 能不能提交 | 看权限档 | 能(实测,用户的权限模式下) | 工作区沙箱默认不让写 `.git`,要放开(§4) |
| 单元健康检查 | `agent`(默认) | `agent`(默认) | `agent`(默认) |
| 负哨守护 | 带 ledger | 不带 ledger、不带后端 pid | 不带 ledger、不带后端 pid |

`agent` 检查、守护不带 ledger、单机部署分别见 `docs/OLP_CLUSTER.md` 的 §2、§4c、§8。

## 2. 启动

在 herdr 的窗格里启动,种类写对,herdr 才认得出它、hub 才唤醒得到:

```bash
herdr agent start inner --kind claude --pane <窗格号> -- --mcp-config <mcp 配置文件>
herdr agent start inner --kind codex  --pane <窗格号> -- --no-daemon <Codex 的 -c 参数,见 §3、§4>
```

Codex 要带 `--no-daemon`。不带时它的会话在共享的后台服务(`codex app-server
daemon`)里,窗格里的程序只是客户端:关掉窗格或杀掉它,工作照常继续("Any running
work continues"),执行的命令挂在后台服务下面;负哨守护盯着的进程也就不是真正干活
的那个。

第一次在某个目录里启动,两者都会问"信任这个目录吗",要人确认一次(之后记住)。
Codex 有新版本时还会先问要不要升级。这些提示出现时 herdr 报 `blocked`,
`herdr agent start` 返回 `agent_not_ready`;确认之后状态变为 `idle`。

## 3. 挂上提问工具 `ask_outer`

MCP 服务是 `octoscode olp-mcp-serve`。单元的配置(`unit.conf`)不在默认位置时,把
环境变量也传给它,它要据此把提问的审计行写到 hub 的黑板。

**Claude Code**:写一个配置文件,启动时 `--mcp-config` 指向它(或者用 `claude mcp add`
写进项目或用户配置):

```json
{"mcpServers": {"olp": {"command": "octoscode", "args": ["olp-mcp-serve"]}}}
```

工具名是 `mcp__olp__ask_outer`。实测在用户的自动权限模式下直接可用。

**Codex**:用 `-c` 在启动时传(或者用 `codex mcp add` 写进 `~/.codex/config.toml`):

```bash
-c 'mcp_servers.olp.command="octoscode"' \
-c 'mcp_servers.olp.args=["olp-mcp-serve"]' \
-c 'mcp_servers.olp.default_tools_approval_mode="approve"'
```

**最后一行不能少。** Codex 的审批策略是 `never` 时,需要审批的 MCP 调用会被直接拒绝,
内环拿到的是"MCP tool call requires approval, but approval policy is never",提问
根本发不出去。实测里内环随后按规程写了 `ACK(blocked)`。`default_tools_approval_mode`
只放行这一个 MCP 服务的工具。

**等待时间**:`ask_outer` 等 50 秒。实测两者都在第 50 秒拿到降级指引原文,没有被客户端
提前中止(Codex 对 MCP 工具调用的上限默认是 60 秒;改小了的话要不小于 55 秒)。

## 4. Codex 要能写 `.git`

Codex 的工作区沙箱(`workspace-write`)默认不让写 `.git`,内环建不了分支、提交不了,
实测报 `Unable to create '…/.git/index.lock'`。两种放开方式,由 operator 决定:

- 只放开这个仓库的 `.git`(实测可行,范围最小):

  ```bash
  -c 'sandbox_workspace_write.writable_roots=["<仓库>/.git"]'
  ```
- 完全访问:`--sandbox danger-full-access`(心跳的权限一列会显示"完全访问")。

另外,Codex 在沙箱里执行 `olp-board-remote.py ack` 时要能写 hub 的黑板。实测里黑板在
`/tmp` 下,沙箱允许;单机部署而黑板在别处时,还要把黑板所在目录加进
`writable_roots`。这一点没有实测。

Claude Code 的权限在它自己的权限模式里决定;实测用的是用户的自动模式,建分支、
提交、运行脚本都没有被拦。

## 5. 守护

内环是 Claude Code 或 Codex 时,负哨守护不给 `--backend-pid` 和 ledger 三项:

```bash
pid=$(herdr pane process-info --pane <窗格号> | python3 -c 'import json,sys; print([p["pid"] for p in json.load(sys.stdin)["result"]["process_info"]["foreground_processes"] if p["name"] in ("claude","codex")][0])')
octoscode olp-runtime-watch --agent inner --agent-pid "$pid" --repo <仓库> \
    --task <N> --progress-file <本卡要改的文件> --stall-seconds 900 \
  | scripts/olp-alert-forward.py
```

没有 turn 一层,不会报 `turn_error`;停滞提示靠 herdr 的 `working` 状态区分"在忙但
文件不动"和"什么都没做"。

## 6. 实测记录(2026-11-14,单机)

| 卡 | 内环 | 结果 |
|---|---|---|
| #2 | Claude Code | 唤醒后同步镜像、读卡、提问;hub 16 秒后回答;原样写入 README、在 `task/2` 提交、`ACK(done)`;复验推送、采认、推 GitHub |
| #3 | Codex | 提问被 Codex 的审批策略拒绝 → `ACK(blocked)`(§3 的那一行补上后解决) |
| #4 | Codex | 第 50 秒拿到降级指引;沙箱不让写 `.git` → `ACK(blocked)`(§4) |
| #5 | Codex | 第 50 秒拿到降级指引,按指引完成、提交、`ACK(done)`;复验推送、采认、推 GitHub |

另外:Claude Code 单独的超时测试也在第 50 秒拿到降级指引。归档之后采集拿到了全部
信号(负哨、提问超时、两次 `ACK(blocked)`、两次改判)。

没有实测的:多机部署下 Claude Code/Codex 当内环(各条信道与 octoscode 内环相同,
已在 Linux 容器里用 octoscode 内环测过);以完全访问参数启动它们(本机的自动权限检查
拒绝了这样启动);Codex 写不在 `/tmp` 下的黑板。
