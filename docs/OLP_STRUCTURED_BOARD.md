# OLP 结构化黑板（实验性可选扩展）

`schema="olp-board/v1"` 为现有文字 OLP 黑板增加可验证的追加账本。它是
**显式 opt-in 的实验扩展**，不升级或替代 `olp/v2` 主协议；没有事件行的项目
继续按 legacy 流程工作。本仓库跟踪的 `.octos/loop.md` 不会自动迁移。
英文版：[`OLP_STRUCTURED_BOARD.en.md`](OLP_STRUCTURED_BOARD.en.md)。

合并前仍需维护者裁定两项协议选择：R1 是否正式把“消费”与“执行完成”分开；
R5 是否接受“历史不移动、状态由回放投影”取代把 ACK 条目物理移入历史区。

## 安装与选择模式

依赖 Python 3.9+、POSIX 文件语义和 `flock(2)`。在**尚未创建 OLP 文件的新项目**中，
下面两种模式必须二选一，不能先运行默认模式再尝试 structured：

```bash
# 方案 A：默认保持原有文字黑板和手写 ACK 契约
bash /path/to/octoscode/scripts/olp-init.sh
```

或者在第一次 init 前选择方案 B：

```bash
# 方案 B：替代方案 A，生成结构化 loop、无裸 ACK 占位的新板和独立普通锁
OLP_BOARD_MODE=structured bash /path/to/octoscode/scripts/olp-init.sh
```

脚本把以下四个原文件名相邻安装到 `~/.octos/outer/`：

- `olp-board-append.py`
- `olp-board-event.py`
- `olp-board-inbox.py`
- `olp-board-sentinel.py`

重复运行不会覆盖已有 loop、黑板、锁、上岗卡或已安装工具；结构化模式遇到
既有项目只打印迁移提示。没有 Python 时，脚本明确报告结构化能力不可用，旧的
`board-append.sh` 与 `watch-board.sh` 仍照常安装和使用。

不要把已有项目仅靠重跑 init 改成结构化模式。先清空在途项、冻结复制整板并保留
回执，再人工更新 loop；新链必须使用一块新板和新锁。

新生成的 structured 板在第一条事件写入前，`state` 仍会按“没有事件”返回
`mode=legacy`；此时以 init 生成的 `Structured maintenance loop` 和板头单独一行的
`<!-- olp-board/v1 -->` 标记为采用依据，第一条派单必须用 `item`。第一条有效
事件之后才由账本可靠区分 structured/mixed。普通旧板没有这两处显式声明，始终
走 legacy。

## 固定路径与写入边界

下文用同一个规范路径，避免别名路径把旧 shell 与新工具分裂到不同锁域：

```bash
BOARD="$PWD/.octos/OUTER_LOOP_REVIEW.md"
TOOLS="$HOME/.octos/outer"
```

板和 `<板>.lock` 必须事先存在且都是普通文件；板路径本身是符号链接时工具直接
拒绝（旧 shell 锁的是 `<链接>.lock`，新工具若跟随链接会锁 `<目标>.lock`）；板有
多个硬链接时同样拒绝（每个名字各锁自己的 `<名字>.lock`）；旧 `olp-board-append.sh`
在已 opt-in 的板上经符号链接调用或板有多个硬链接时同样拒绝，不会在链接旁建锁。
事件工具在同一次独占持锁期间追加可读正文、`> OLP-EVENT <canonical-json>` 和
UTC `ts=` 行，随后 fsync 并回读固定区间。读者（state、inbox、sentinel、verify）
只在共享锁下复制字节，释放锁后再回放，互不阻塞。这个保证只覆盖使用同一锁的
合作写者；绕过工具的写者和 I/O 中断仍可能留下部分记录，它们会以 DRIFT 暴露
（见下文），不会让 state 失败。错误 JSON 中 `may_have_appended=true` 时先查
`state` 和现场字节，禁止盲目重试、截断或“回滚”板。两个写入口的命令行参数错误
也只输出一个 `may_have_appended=false` 的机器 JSON。

板以没有 LF 的半行结尾时，普通写入一律拒绝，`state` 用 `partial_tail` 给出
这段字节的 offset/length/sha256（不论内容；未进围栏的半截事件行同时是
`malformed_event`）；人工确认后用 `void` 隔离它（见下文）。若补上 LF 后这段半行
会打开代码围栏（或它本就在未闭合围栏里），void 自身会被围栏隐藏而被拒绝，这时
改用 `olp-board-append.py append --terminate-partial-line`，正文以 LF 开头并补一行
闭合围栏。

旧 `olp-board-append.sh` 在 legacy 板上行为不变；板里已有事件行或
`<!-- olp-board/v1 -->` 标记时，它拒绝以 `> OLP-EVENT ` 开头的正文行和独立的 `ts=` 时间行（与 `olp-board-append.py`
同一判定：时间戳后只允许一个可选的 CR）；它也和 `olp-board-append.py` 一样拒绝
不以 LF 结尾的正文，并在板以半行结尾时拒绝追加（先按上文 void 该半行），两次追加
拼不成同一行。其余文字照常追加。

逻辑 `actor` 不是 OS 身份认证，也不校验 `outer-duty` 的 R7 lease。调用者仍须按
OLP 的权限与主审纪律选择 actor。

## 七种事件与四个待办集合

生命周期事件有 `item`、`receive`、`ack`、`review` 四种；收口事件有 `withdraw`、
`resolve`、`void` 三种。显示编号只是给人看的字符串，关联一律使用 32 位十六进制
事件 ID。

`state` 与 inbox 投影四个集合：

| 集合 | 含义 | 可否自动执行 |
|---|---|---|
| `unreceived` | item 已发布，目标 runtime 尚未记录消费 | 只有当前 loop 按顺序消费后才执行 |
| `received_pending` | 已消费但没有终态 ACK，常用于重启对账 | **否**；`execution_authorized=false`，不得据此重复执行 |
| `unreviewed_ack` | runtime 已终态交付，outer 尚未 review | 只允许 outer 审查 |
| `escalated` | review 已升级给人工裁定，尚未 `resolve` | 不是批准，保持待人工处置 |

每个 item 最多一次 receive、一次终态 ACK、一次 review。`return` 和解除
`blocked` 都必须开新 item，不能重放旧项；`wontdo` 只能 `accept` 或
`escalate`，不能 `return`。

| 收口事件 | 谁可写 | 作用 |
|---|---|---|
| `withdraw --item` | item 作者 | 撤回尚未 receive 的 item（例如收件人写错）；已 receive 的只能由收件人 ACK |
| `resolve --review [--next]` | item 作者 | 人工裁定后关闭一次 `escalate`，可指向另一个既有后续 item |
| `void --target-file` | 人工核对后的外环（约定而非强制：actor 不认证，任何 actor 都能写 void） | 把一段 DRIFT 或 `partial_tail` 字节隔离为“不是账本记录” |

## 完整生命周期

所有正文文件均为 UTF-8；item 正文必须以 LF 结尾，ACK/review 说明必须是非空
单行。下面的 stdout 是应保存的机器回执，包含 `offset`、`length`、摘要、UTC
时间和 `verified=true`。

```bash
cat > /tmp/olp-item.txt <<'EOF'
Implement the requested change and run the repository gate.
EOF

python3 -B "$TOOLS/olp-board-event.py" item \
  --board "$BOARD" --actor outer --number 1 \
  --title "First structured item" --to runtime \
  --body-file /tmp/olp-item.txt > /tmp/item-receipt.json

# Read the `event` field from item-receipt.json and keep it as ITEM_ID.
python3 -B "$TOOLS/olp-board-event.py" receive \
  --board "$BOARD" --actor runtime --item "$ITEM_ID" \
  > /tmp/receive-receipt.json

cat > /tmp/olp-ack.txt <<'EOF'
Completed in the recorded commit; full verification command is attached.
EOF
python3 -B "$TOOLS/olp-board-event.py" ack \
  --board "$BOARD" --actor runtime --item "$ITEM_ID" \
  --outcome done --commit 0123456789abcdef0123456789abcdef01234567 \
  --r2 verified --body-file /tmp/olp-ack.txt \
  > /tmp/ack-receipt.json

# Read the `event` field from ack-receipt.json and keep it as ACK_ID.
cat > /tmp/olp-review.txt <<'EOF'
Reproduced the declared verification in an isolated worktree.
EOF
python3 -B "$TOOLS/olp-board-event.py" review \
  --board "$BOARD" --actor outer --ack "$ACK_ID" \
  --decision accept --body-file /tmp/olp-review.txt \
  > /tmp/review-receipt.json
```

正常 item 与 ack 的 `recovery` 为 `null`。`ack --r2` 接受
`verified`、`partially-verified` 或 `unverified`，必须与实际验证相符。

收口事件的说明文件同样是非空单行：

```bash
# 撤回投错收件人的 item（只能在它被 receive 之前）
python3 -B "$TOOLS/olp-board-event.py" withdraw \
  --board "$BOARD" --actor outer --item "$ITEM_ID" --body-file /tmp/why.txt

# 人工裁定后关闭升级，并指向新开的后续 item（没有后续时省略 --next）
python3 -B "$TOOLS/olp-board-event.py" resolve \
  --board "$BOARD" --actor outer --review "$REVIEW_ID" --next "$NEXT_ITEM_ID" \
  --body-file /tmp/decision.txt
```

## 回执复验与普通追加

事件回执可在后续追加之后复验：

```bash
python3 -B "$TOOLS/olp-board-event.py" verify \
  --receipt-file /tmp/item-receipt.json
```

非协议正文可通过追加工具写入并保留回执：

```bash
python3 -B "$TOOLS/olp-board-append.py" append \
  --board "$BOARD" --body-file /tmp/note.txt > /tmp/note-receipt.json

python3 -B "$TOOLS/olp-board-append.py" verify \
  --board "$BOARD" --body-file /tmp/note.txt \
  --offset "$OFFSET" --ts "$UTC_TS"
```

`OFFSET` 与 `UTC_TS` 取自保存的追加回执。普通追加器拒绝事件前缀和调用者
伪造的独立 `ts=` 行，已闭合围栏里的示例也不例外；示例中的时间行请缩进或改写（如 `ts: …`）。

## 查询与观察

单次 inbox 未命中仍退出 0 并返回 `matched=false`；错误退出 2；`--wait`
超时退出 3。

```bash
python3 -B "$TOOLS/olp-board-inbox.py" \
  --board "$BOARD" --for runtime --actor runtime

python3 -B "$TOOLS/olp-board-inbox.py" \
  --board "$BOARD" --for outer --wait --interval 2 --timeout 1800 \
  --ready-file /tmp/olp-inbox-ready.json

python3 -B "$TOOLS/olp-board-sentinel.py" \
  --board "$BOARD" --token 'ACK(' --for outer \
  --skip-signature '外环(example)' --interval 2 --timeout 1800
```

`--since-head <事件 ID>` 让 inbox 与 sentinel 只把触发事件位于该事件之后的
`received_pending`、`unreviewed_ack`、`escalated` 计入 `messages`/`matched`；
runtime 尚未 receive 的 item 始终计入。基线要取自某次 inbox/sentinel 输出的
`head`，并且那次输出里的**每条** messages 都已处理或有意暂留——不要用自己写入
回执里的 head。这样外环不会被自己暂留的 ACK 或升级反复唤醒，runtime 也不会被
自己在途的条目唤醒。基线未知时退出 2。不带该参数时行为
同前：启动即补读全部可处理状态。state、inbox、sentinel 都接受
`--lock-timeout`。

sentinel 启动时先补读账本，再从启动字节基线之后扫描文字。输出前缀分别是
`LEDGER-SIGNAL`（启动补读或后续账本变化）、`BOARD-SIGNAL`（文字只负责唤醒）、
`DRIFT`、`ERROR` 和 `TIMEOUT`。文字按字节匹配 token，旧 shell 写入的非 UTF-8
行照常命中（展示时替换无法解码的字节）；token、actor 或 `--since-head` 参数里的非
UTF-8 字节在 sentinel 与 inbox 的输出和 ready 文件里以转义形式显示。普通等待超时及尾部未完成行持续到期限都会在
stdout 输出 `TIMEOUT: {...}` 并退出 3；监视方必须同时检查前缀和退出码。正信号的完成判定始终以账本为准；原有
`events.jsonl` 的 `goal_transition blocked`/`escalation` 负信号哨继续挂载。
sentinel 不会自动开 turn、派单或执行，也不替代
`olp-watch-board.sh --harvest` 的既有采集集成。

## legacy、mixed 与 DRIFT 收口

无事件行的板返回 `mode=legacy`，工具不会把“账本无待办”解释成“文字无待办”。
任何 DRIFT 都令 `mode=mixed`、`dispatch_blocked=true`。每条 DRIFT 都带
`kind`、`offset`、`length`、`sha256` 和 `reason`：

| kind | 何时出现 | 收口方式 |
|---|---|---|
| `malformed_event` | 任意位置一条未进围栏的 `> OLP-EVENT ` 行未通过校验（坏 JSON、非 canonical、source 不相邻、不从行首开始、与已配对或已消费的文字重叠，或字节被改、ts 边界不对、坏链、非法生命周期、坏 recovery/void）；证据是该行不含 LF 的字节 | `void` |
| `unpaired_item` | opt-in 后未与事件配对的 `### 编号. 标题` 行（在第一个 `. ` 处拆分，写入端拒绝含 `. ` 的编号，标题可以含 `. `） | 同编号标题的 `item --recovery-file`，或 `void` |
| `unpaired_ack` | opt-in 后未配对、符合 v1 语法的 ACK 行（允许前导空白、全角冒号与 CRLF） | outcome 相同的 `ack --recovery-file`，或 `void` |
| `suspected_ack` | opt-in 后未配对的疑似手写 ACK：`> ACK(`、`- ACK(`、`### ACK`、`**ACK(...)**`、行内 `ACK(`、旧式 `ACK:`（仅以 ACK 一词开头的普通文字不算，但任何 `ACK(` 都算） | 行内有 `ACK(outcome)` 时可用同 outcome 的 ack recovery，否则只能 `void` |
| `unclosed_fence` | opt-in 后直到文件末尾仍未闭合的代码围栏 | 追加同种且不短于 opener 的闭合行 |

坏事件行不会让回放失败：它被排除出账本，后续写者从最后一个有效 head 续链，
旧回执照常可以复验；它只是以 DRIFT 阻断自动派单，直到有人收口。opt-in 前的
规范文字不追溯，但坏事件行在任何位置都会报出；在还没有有效事件的板上 void
它，这条 void 就成为第一条事件，板随之 opt-in。**示例文字必须放进已闭合的代码围栏**；引用块里的 ACK 也会被当作
疑似手写 ACK（上游规模化实战中车道确实会写 `> ACK(...)`、`### ACK` 这类变体，
它们必须被看见，不能静默漏掉）。判定规范行和疑似 ACK 时先去掉 NUL 字节（与 bash 4
及以上读行时丢掉 NUL 一致），NUL 拆开关键字也藏不住手写 ACK。macOS 自带的 bash 3.2 读
行时在 NUL 处截断，这时进化采集在 recovery 之前可能不为这一行出卡，但 state 照样报
DRIFT、阻止派发。

反引号与波浪线围栏都必须由同种类、且长度不短于 opener 的闭合行结束；错误种类
或更短的行仍属于围栏正文。未闭合围栏会以 opener 行的字节证据报
`unclosed fence after structured opt-in`，inbox 停止自动派发，sentinel 输出
`DRIFT`，harvest 明确非零失败且不推进采集游标。人工核对后，只能追加匹配的
闭合行；工具不删除、移动或重写旧字节。补闭合后结构化写入恢复，但围栏内文字
仍是示例，不能成为 recovery 或 void 的对象。

进化采集（`olp-evo-harvest.sh`）对含事件行的板走结构化路径：它经
`olp-board-event.py snapshot` 在共享锁下只读一次板字节，回放、旧扫描器和结构化
扫描都用这份快照，采集期间新追加的文字整条留到下一轮，同一条 ACK 在各轮只有一个
identity。因此板旁的 `.lock` 和相邻安装的事件工具是结构化采集的前提：板里已有事件行
而二者缺一时，harvest 明确非零失败并指明缺什么，不会静默改用旧扫描器（那样同一条 ACK
会换一个 identity 再出一张卡）。不含事件行的 legacy 板不受影响。旧 shell 追加器不校验
UTF-8，含其他字节的旧触发行在结构化路径上照常出卡；bash 读行遇到 NUL 时，4 及以上
版本丢掉该字节、3.2 在该处截断，行数都不变，结构化路径因此按行号在快照上定位旧扫描器的
每一行；带 recovery 的 ACK 沿用旧扫描器同一轮给被恢复行的 identity，所以不论哪个版本的
bash 在跑，NUL 都不会让同一条 ACK 出两张卡；任何正式
事件的正文（source）里引用的触发行都不出卡，与回放把它当作事件自己的文字一致；候选行以
`|` 分隔、identity 含板路径，给定的或解析后的板路径含 `|` 或换行时，结构化采集只以一行
`error:` 退出；采集任何失败都以 `error:` 行退出并
清理自己的临时目录。

收口时从 `state` 输出取得该条 DRIFT（或 `partial_tail`）的精确 `offset`、
`length`、`sha256`，只把这三个字段写入证据文件：

```json
{"offset":1234,"length":42,"sha256":"<64-lowercase-hex>"}
```

- **recovery**：这段文字确实是某个 item 或 ACK 的记录，由同类型正式事件补录。
  item 编号/标题或 ACK outcome 必须一致，每段原文只能被消费一次。
- **void**：这段文字不是账本记录（误贴的事件行、半截写入、无法补录的疑似
  ACK）。`void` 追加一条只追加的隔离事件，原字节保留，证据进入
  `quarantine_evidence`：

```bash
python3 -B "$TOOLS/olp-board-event.py" void \
  --board "$BOARD" --actor outer --target-file /tmp/drift.json \
  --body-file /tmp/why.txt
```

板以半行结尾时，必须**先** void `partial_tail`（工具会先补一个 LF 再写隔离
事件；会打开围栏的半行见上文），其余 DRIFT 之后再逐条处理。一次中断的写入通常留下两条 DRIFT：半截事件行
（`malformed_event`）和它前面已写入的标题（`unpaired_item`），先 void 前者再
void 后者，然后重新发 item。

下面是在独立临时板上可直接运行的完整演示。它新建并 receive 四个 item，实际
覆盖恢复成功、重复 recovery、outcome 不同和 opt-in 前引用四种结果；拒绝路径
都在调用前后用 `cmp` 核板字节未变。`TOOLS` 沿用上文安装目录。

```bash
DEMO=$(mktemp -d)
BOARD="$DEMO/BOARD.md"
EVENT="$TOOLS/olp-board-event.py"
APPEND="$TOOLS/olp-board-append.py"

# 这行先于第一条事件，稍后用于证明 opt-in 前引用会被拒绝。
printf 'ACK(done): pre-opt-in completion\n' > "$BOARD"
: > "$BOARD.lock"
printf 'Backfilled the exact old ACK.\n' > "$DEMO/recovery-note.txt"

# 成功：新 item/receive → 旧规范文字 → state 导出三字段 → 正式 recovery ACK。
printf 'Execute the recovery success case.\n' > "$DEMO/item-1.txt"
python3 -B "$EVENT" item --board "$BOARD" --actor outer --number R8-1 \
  --title 'Recovery success' --to runtime --body-file "$DEMO/item-1.txt" \
  > "$DEMO/item-1.json"
ITEM_1=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["event"])' "$DEMO/item-1.json")
python3 -B "$EVENT" receive --board "$BOARD" --actor runtime --item "$ITEM_1" \
  > "$DEMO/receive-1.json"
printf 'ACK(done): completed by an older writer\n' > "$DEMO/legacy-done.txt"
python3 -B "$APPEND" append --board "$BOARD" --body-file "$DEMO/legacy-done.txt" \
  > "$DEMO/append-done.json"
python3 -B "$EVENT" state --board "$BOARD" > "$DEMO/state-before.json"
python3 -c 'import json,sys; d=json.load(open(sys.argv[1]))["drift"][0]; print(json.dumps({k:d[k] for k in ("offset","length","sha256")},separators=(",",":")))' \
  "$DEMO/state-before.json" > "$DEMO/recovery-done.json"
python3 -B "$EVENT" ack --board "$BOARD" --actor runtime --item "$ITEM_1" \
  --outcome done --r2 unverified --body-file "$DEMO/recovery-note.txt" \
  --recovery-file "$DEMO/recovery-done.json" > "$DEMO/recovery-success.json"
python3 -B "$EVENT" state --board "$BOARD" > "$DEMO/state-after.json"

# 重复：另一个已 receive item 复用同一 evidence，退出 2 且板不变。
printf 'Exercise duplicate rejection.\n' > "$DEMO/item-2.txt"
python3 -B "$EVENT" item --board "$BOARD" --actor outer --number R8-2 \
  --title 'Duplicate rejection' --to runtime --body-file "$DEMO/item-2.txt" > "$DEMO/item-2.json"
ITEM_2=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["event"])' "$DEMO/item-2.json")
python3 -B "$EVENT" receive --board "$BOARD" --actor runtime --item "$ITEM_2" > "$DEMO/receive-2.json"
cp "$BOARD" "$DEMO/before-duplicate.md"
python3 -B "$EVENT" ack --board "$BOARD" --actor runtime --item "$ITEM_2" \
  --outcome done --r2 unverified --body-file "$DEMO/recovery-note.txt" \
  --recovery-file "$DEMO/recovery-done.json" 2> "$DEMO/duplicate-error.json"; test $? -eq 2
cmp -s "$BOARD" "$DEMO/before-duplicate.md"

# outcome 不同：旧 blocked 行不能恢复为 done，退出 2 且板不变。
printf 'Exercise outcome mismatch.\n' > "$DEMO/item-3.txt"
python3 -B "$EVENT" item --board "$BOARD" --actor outer --number R8-3 \
  --title 'Outcome mismatch' --to runtime --body-file "$DEMO/item-3.txt" > "$DEMO/item-3.json"
ITEM_3=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["event"])' "$DEMO/item-3.json")
python3 -B "$EVENT" receive --board "$BOARD" --actor runtime --item "$ITEM_3" > "$DEMO/receive-3.json"
printf 'ACK(blocked): dependency unavailable\n' > "$DEMO/legacy-blocked.txt"
python3 -B "$APPEND" append --board "$BOARD" --body-file "$DEMO/legacy-blocked.txt" > "$DEMO/append-blocked.json"
python3 -B "$EVENT" state --board "$BOARD" > "$DEMO/state-blocked.json"
python3 -c 'import json,sys; d=json.load(open(sys.argv[1]))["drift"][0]; print(json.dumps({k:d[k] for k in ("offset","length","sha256")},separators=(",",":")))' \
  "$DEMO/state-blocked.json" > "$DEMO/recovery-blocked.json"
cp "$BOARD" "$DEMO/before-outcome.md"
python3 -B "$EVENT" ack --board "$BOARD" --actor runtime --item "$ITEM_3" \
  --outcome done --r2 unverified --body-file "$DEMO/recovery-note.txt" \
  --recovery-file "$DEMO/recovery-blocked.json" 2> "$DEMO/outcome-error.json"; test $? -eq 2
cmp -s "$BOARD" "$DEMO/before-outcome.md"

# opt-in 前引用：offset 0 的旧 ACK 即使摘要正确也退出 2 且板不变。
printf 'Exercise pre-opt-in rejection.\n' > "$DEMO/item-4.txt"
python3 -B "$EVENT" item --board "$BOARD" --actor outer --number R8-4 \
  --title 'Pre opt-in rejection' --to runtime --body-file "$DEMO/item-4.txt" > "$DEMO/item-4.json"
ITEM_4=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["event"])' "$DEMO/item-4.json")
python3 -B "$EVENT" receive --board "$BOARD" --actor runtime --item "$ITEM_4" > "$DEMO/receive-4.json"
printf 'ACK(done): pre-opt-in completion\n' > "$DEMO/pre-opt-in.txt"
python3 -c 'import hashlib,json,sys; b=open(sys.argv[1],"rb").read(); print(json.dumps({"offset":0,"length":len(b),"sha256":hashlib.sha256(b).hexdigest()},separators=(",",":")))' \
  "$DEMO/pre-opt-in.txt" > "$DEMO/recovery-pre-opt-in.json"
cp "$BOARD" "$DEMO/before-pre-opt.md"
python3 -B "$EVENT" ack --board "$BOARD" --actor runtime --item "$ITEM_4" \
  --outcome done --r2 unverified --body-file "$DEMO/recovery-note.txt" \
  --recovery-file "$DEMO/recovery-pre-opt-in.json" 2> "$DEMO/pre-opt-error.json"; test $? -eq 2
cmp -s "$BOARD" "$DEMO/before-pre-opt.md"
```

一次真实运行的关键结果如下；三个拒绝回执的 `may_have_appended=false`，且上面的
`cmp` 均为 0：

```text
success: exit 0, verified=true, drift=0, recovery_evidence=1
duplicate: exit 2, Recovery source was already consumed
outcome: exit 2, ACK recovery outcome does not match
pre-opt-in: exit 2, Recovery must reference normative text after structured opt-in
```

成功后原字节不动，旧行从未解决 `drift` 消失，并进入
`recovery_evidence`（含补录事件 ID 和字节证据）。item 漂移同理使用
`item --recovery-file`，且显示 number/title 必须完全一致。已配对区间、围栏
示例、引用标题或错误摘要/区间也会被生产校验拒绝。

DRIFT 未收口前禁止重新执行受影响项；“人工忽略”必须落成一条 `void`，不能只
停留在口头。

## 历史、归档与限制

结构化板永久追加，完成状态来自回放；不物理移动 Active/历史字节。归档只能冻结
复制整板，并核对摘要；只有四个待办集合均为空、且没有未收口 DRIFT 时才能创建
新板、新锁和新链（`withdraw`、`resolve`、`void` 与闭合围栏让这一条件可以达成）。
旧板、回执及 recovery/quarantine evidence 都要保留。

已知边界：

- 只约束使用规范板路径和同一锁的合作写者；符号链接板路径和有多个硬链接的板会被
  拒绝（链接数在解析路径时和取得锁后各查一次），不经工具、另持他锁的写者仍不受约束。
  锁由路径派生，挡不住与写入同时发生的建链、删链，前提是没有人对板路径做对抗性操作。
- 不提供自动身份认证或 R7 lease enforcement。
- `received_pending` 是恢复对账证据，不是重执行队列。
- `may_have_appended=true` 代表写后结果不确定，必须先读现场再决定下一步。
- 每次写入在独占锁内对整板回放两次（线性）；读者只在共享锁下复制字节。
- `schema="olp-board/v1"` 仍为实验性 opt-in，主协议版本与 R1/R5 选择待维护者决定。

## 验证

Rust 集成测试直接调用生产 CLI、真实临时文件、安装后的工具和旧 shell 锁：

```bash
cargo test --test olp_board_protocol
```

仓库 CI 的 `cargo test --all-targets` 会逐场景执行这些测试。提交前仍需运行项目的
完整 `scripts/verify.sh`（含 fmt 与 clippy）；聚焦测试不能代替完整门禁。测试范围
覆盖协议状态机、损坏/并发/部分写、mixed/DRIFT/recovery/void、withdraw/resolve、
`--since-head`、共享读锁、旧 shell 守卫、legacy 采集逐字节不变、采集归属和 init
的 structured/legacy 双路径，不覆盖 OS 身份认证、R7 lease 或网络安装回退。
