# OctoLoop 的外部策略

OctoLoop 里有些判定是固定规则,但规则本身会随着使用变:什么算停滞、每张卡该说它在等谁。
外部策略让这类规则可以改而**不用重新构建和安装 octoscode**:工具收集事实,交给一个外部程序
判定,再校验它的回答。

工具始终是主导的一方:

- 事实由工具收集,策略只读 stdin 里的那份 JSON,看不到日志、锁和关口,也不该去碰它们。
- 回答必须在约定范围内,越界的整份不用。
- 策略起不来、失败、超时或回答不合规时,工具改用内置规则,并在 stderr 说明原因。命令照常成功。

目前接入的判定只有一个:`octoscode olp board stall`(集群是否整体静默)。

## 两种给法

| 给法 | 适用 | 超时 |
|---|---|---|
| `--policy <命令>`,或环境变量 `OLP_STALL_POLICY` | 任何可执行程序;命令行按 shell 的规则拆词,但不经过 shell | 20 秒 |
| `--policy-crate <目录>` | 策略是一个 Rust 包,由 [Relax](https://github.com/makepad/relax) 从源码直接运行 | 180 秒(首次运行要编译) |

`--policy-crate` 需要带 `relax` feature 的构建:

```sh
cargo build --release --features relax
```

这个 feature 默认关闭,不增加任何依赖:Relax 是运行时找到的外部程序(环境变量 `OLP_RELAX`,
否则 `PATH` 里的 `relax`),octoscode 在包目录里执行 `relax run --unattended --quiet <包名>`。
没带 feature 的构建遇到 `--policy-crate` 会以退出码 2 拒绝并说明。Relax 自称是实验性的开发工具,
所以它只用来跑策略;日志、锁、推送关口都不经过它。

## `board stall` 的约定

stdin(一份 JSON):

| 字段 | 含义 |
|---|---|
| `quiet` | `--quiet` 的秒数 |
| `silent_seconds` | 日志最后一条事件距今的秒数 |
| `working_units` | 心跳里有内环处于 `working` 的单元 |
| `unknown_units` | 看不到的单元(未登记或心跳失联) |
| `cards[]` | 每张未了结的卡:`number`、`unit`、`partition`(`inbox` 等外环 / `working`)、`reason`、`claimed`、`last_seq`、`last_ts`、`last_kind`、`silent_seconds`、`agents`(该单元各内环的状态:`working`/`idle`/`blocked`/`done`,看不到时为空) |

stdout:最后一行以 `{` 开头的内容当作回答。

```json
{"stalled": true, "alerts": [{"number": 2, "waits_on": "unit", "cause": "m08 的内环停在审批框上"}]}
```

- `stalled`:是否算静默。为 false 时不报任何卡。
- `alerts[]`:要报的卡。`number` 必须是 `cards` 里的卡且不重复;`waits_on` 只能是 `unit` 或 `outer`;
  `cause` 是 1 到 200 字的一行。
- 单元、起始事件与时间由工具填,策略说了不算。"同一段沉默最多报三次"的限制也仍由工具执行。

## 一个策略包

下面这个包先照搬内置规则,再加一条内置规则没有的:内环停在审批框上时直说。

`Cargo.toml`:

```toml
[package]
name = "stall-policy"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[workspace]
```

`src/main.rs`:

```rust
//! The stall rule of OctoLoop as a policy: facts in on stdin, a decision out on stdout.
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Facts {
    quiet: u64,
    silent_seconds: i64,
    working_units: Vec<String>,
    cards: Vec<Card>,
}

#[derive(Deserialize)]
struct Card {
    number: u64,
    unit: String,
    partition: String,
    reason: String,
    claimed: bool,
    #[serde(default)]
    agents: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq)]
struct Verdict {
    number: u64,
    waits_on: &'static str,
    cause: String,
}

#[derive(Serialize)]
struct Decision {
    stalled: bool,
    alerts: Vec<Verdict>,
}

fn verdict(c: &Card) -> Verdict {
    let (waits_on, cause) = if c.partition == "inbox" {
        ("outer", format!("等外环:{}", c.reason))
    } else if c.agents.iter().any(|a| a == "blocked") {
        ("unit", format!("{} 的内环停在审批框上,要有人去窗格里批或拒", c.unit))
    } else if !c.claimed {
        ("unit", format!("派出后没有领取,{} 上没有内环在工作", c.unit))
    } else {
        ("unit", format!("领取后没有回执,{} 上没有内环在工作", c.unit))
    };
    Verdict { number: c.number, waits_on, cause }
}

fn decide(f: &Facts) -> Decision {
    let stalled = !f.cards.is_empty() && f.working_units.is_empty() && f.silent_seconds >= f.quiet as i64;
    Decision { stalled, alerts: f.cards.iter().map(verdict).collect() }
}

fn main() {
    let facts: Facts = serde_json::from_reader(std::io::stdin()).expect("facts");
    println!("{}", serde_json::to_string(&decide(&facts)).unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;
    fn card(claimed: bool, agents: &[&str]) -> Card {
        Card { number: 2, unit: "m08".into(), partition: "working".into(), reason: String::new(), claimed, agents: agents.iter().map(|s| s.to_string()).collect() }
    }
    #[test]
    fn a_blocked_inner_loop_is_named() {
        assert!(verdict(&card(true, &["blocked"])).cause.contains("审批框"));
        assert!(verdict(&card(true, &["idle"])).cause.contains("没有回执"));
    }
    #[test]
    fn an_unclaimed_card_waits_on_its_unit() {
        assert!(verdict(&card(false, &["idle"])).cause.contains("没有领取"));
    }
}
```

改规则的一轮:

```sh
cd stall-policy
relax test stall-policy          # 策略自己的测试
octoscode olp board stall <日志目录> --policy-crate "$PWD"
```

## 实测(2026-11-25,M 系列 MacBook)

| | 耗时 |
|---|---|
| 内置规则 | 0.01 秒 |
| 策略包经 Relax,已运行过 | 0.10 秒 |
| 策略包首次运行(含编译 serde) | 约 25 秒 |
| 改一条规则:编辑 → `relax test` → 输出变化 | 5.6 秒 |
| 对照:改 octoscode 后增量构建 | 15 到 60 秒(debug),之后还要重装到每台机器 |

## 限制

- 策略只能在给它的事实里做文章。事实不够时要先改 octoscode 把它加进去。
- 每个策略包在自己的 `target/relax` 下有约 300M 的缓存;新机器或清过缓存后第一次运行慢,
  定时任务上线前先手动跑一次。
- Relax 目前支持 x86-64 与 AArch64 的 Linux 和 macOS。
