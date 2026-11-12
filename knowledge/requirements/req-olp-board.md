---
kind: requirement
id: REQ-OLP-BOARD
title: "OLP 结构化追加黑板与可恢复状态投影"
status: accepted
liveness: auto
tags: [olp, protocol, blackboard, observability]
---

## Problem

现有 OLP 黑板以文字标题和 ACK 的相对位置表达归属，写者并发、ACK 乱序或进程重启后只能靠文本启发式恢复状态。系统需要一个保持旧文字可读、默认不改变旧项目语义的结构化扩展，用同一锁域把规范文字与可验证事件成对追加，并从追加账本投影待收、执行中、待审和升级状态。账本必须在坏行、半截写入和手写变体面前保持可用：问题以带字节证据的 DRIFT 暴露并阻断自动派单，再由只追加的补录或隔离事件显式收口，而不是让整块板失效。

## Requirements

[REQ-OLP-BOARD-APPEND] 系统 MUST 只使用预先存在的规范板文件及其独立普通 `.lock` 文件，在一次独占锁持有期间完成 UTF-8 正文校验、offset/prev/id/UTC 时间生成、追加、fsync 和固定区间回读；失败 MUST 报告 `may_have_appended`，MUST NOT 自动重建锁、截断板或盲目重试。板以无 LF 的半行结尾时，普通写入 MUST 拒绝，只有以该半行为目标的 `void` MAY 先补 LF 再追加。符号链接形式的板路径，以及有多个硬链接（`st_nlink` 大于 1）的板，MUST 被拒绝：每个路径各锁自己的 `<路径>.lock`，同一块板会分裂成两个锁域。链接数 MUST 在解析路径时检查，并在取得锁、打开板之后对打开的板再检查一次；锁由路径派生，无法抵御与写入并发的 link/unlink，这是非对抗文件系统的前提。独立 `ts=` 时间行（行尾只允许可选 CR）MUST 被拒绝，围栏内的示例也不例外，示例须缩进或改写。

[REQ-OLP-BOARD-SCHEMA] 结构化事件 MUST 使用 `schema="olp-board/v1"` 与 item、receive、ack、review、withdraw、resolve、void 七种事件，事件行固定以 `> OLP-EVENT ` 开头，共用严格 canonical JSON、32 位十六进制 id、prev 链、actor、UTC ts 和同板 source；item 与 ack MUST 含必需但可空的 `recovery`，void MUST 含 `target` 字节证据；未知字段、重复 JSON key、非有限数、bool 冒充整数、坏链、坏证据和语义不匹配正文 MUST NOT 进入账本。行 MUST 只以 LF 切分，裸 CR 是普通内容。事件 source MUST 从行首开始（仅 void 收口半行时可以补上的 LF 开头），且 MUST NOT 与已配对或已被 recovery、void 消费的字节重叠，否则该事件行是 `malformed_event`。

[REQ-OLP-BOARD-LIFECYCLE] 每个 item MUST 至多 receive 一次、终态 ack 一次、review 一次；receive 与 ack actor MUST 匹配 item.to，review、withdraw、resolve actor MUST 匹配 item.actor；wontdo MUST NOT return，return 与 blocked 恢复 MUST 使用新 item；未被 receive 的 item MAY 被作者 withdraw 一次，已 receive 的 item MUST NOT withdraw；escalate MUST 保持待人工处置，直到作者以 resolve 关闭并可指向另一个既有后续 item。

[REQ-OLP-BOARD-STATE] 状态查询 MUST 区分 unreceived、received_pending、unreviewed_ack、escalated，已 withdraw 的 item 与已 resolve 的升级 MUST 离开这些集合；各集合 MUST 保持账本出现顺序；`execution_authorized` MUST 为 false，received_pending MUST NOT 成为自动重执行队列；无事件且无 DRIFT 的板 MUST 返回 legacy。读取 MUST 在共享锁下只复制字节，回放在锁外进行；回放 MUST 与板大小近似线性。

[REQ-OLP-BOARD-DRIFT] 回放 MUST NOT 因单条事件行失败：任何不合法的事件行 MUST 作为 `malformed_event` DRIFT 以该行内容（不含 LF）的 offset/length/sha256 报出并排除出账本，后续写者从最后一个有效 head 续链。opt-in（第一条有效事件）之后，未配对的规范 item 标题（`unpaired_item`）、符合 v1 语法（允许前导空白、全角冒号与 CRLF 行尾）的规范 ACK（`unpaired_ack`）、以及引用、列表、标题、强调、行内 `ACK(` 或旧 `ACK:` 形式的疑似手写 ACK（`suspected_ack`）MUST 标 DRIFT；opt-in 前历史、已配对 source 与已闭合围栏内示例 MUST NOT 标 DRIFT。opt-in 后未闭合的反引号或波浪线围栏 MUST 以 opener 字节证据报 `unclosed_fence`，只有同种且不短于 opener 的闭合行 MUST 解除。任何 DRIFT MUST 令 mode=mixed 并阻止自动派发。判定规范 item、规范 ACK 与疑似 ACK 时 MUST 先去掉 NUL 字节（与 bash 4 及以上的 `read` 一致），NUL 拆开关键字的手写 ACK 同样标 DRIFT 并阻止派发；harvest 为某行出卡时 state MUST NOT 仍报该行干净（bash 3.2 的 `read` 在 NUL 处截断，此时 harvest 在 recovery 前可能不为该行出卡而 state 仍阻断，偏保守）。末尾无 LF 的半行 MUST 以 `partial_tail` 证据暴露；非事件半行本身不是 DRIFT。DRIFT 的原因文字 MUST 在任何输入下都能作为 UTF-8 JSON 输出（例如以孤立代理项转义作重复键的事件行），单条事件行 MUST NOT 使 state、snapshot、inbox、sentinel 或 harvest 失败。

[REQ-OLP-BOARD-RECONCILE] 人工收口 MUST 只追加：同类型正式 item/ack 的 `recovery` MUST 精确引用 opt-in 后、恢复事件前、未配对且未消费的完整规范行或疑似 ACK 行；item 编号/标题 MUST 匹配（规范 item 标题行在第一个 `. ` 处拆分编号与标题；写入端 MUST 拒绝含 `. ` 的编号，使工具自己写出的标题行总能以原编号和标题 recovery），ACK outcome MUST 与行中 `ACK(outcome)` 匹配（与 DRIFT 判定一样不计 NUL），无法解析 outcome 的疑似 ACK MUST NOT 被恢复。`void` MUST 精确引用一条尚未收口的 `malformed_event`、未配对规范行、疑似 ACK 或 `partial_tail`；板以半行结尾时 MUST 先 void 该半行；若补 LF 后该半行打开或位于未闭合围栏，void 会被隐藏而拒绝，此时 MUST 只能由普通追加器以显式 `--terminate-partial-line` 追加闭合围栏。每条原文至多被 recovery 或 void 消费一次，原字节 MUST 保留，`recovery_evidence` 与 `quarantine_evidence` MUST 记录事件 ID 与字节证据；已收口区间 MUST 从 DRIFT 移除。

[REQ-OLP-BOARD-INBOX] inbox MUST 支持 runtime/outer 两种查询和等待，单次未命中 MUST 退出 0，查询错误 MUST 退出 2，等待超时 MUST 退出 3；`--since-head <event>` MUST 只把触发事件位于该事件之后的 received_pending、unreviewed_ack、escalated 计入 `messages` 与 `matched`，runtime 尚未 receive 的 item MUST 始终计入，未知事件 MUST 退出 2；ready-file MUST 在首次成功查询后独占创建且每个进程只尝试一次，时间参数 MUST 为有限数，账本损坏 MUST NOT 被当成没有待办。

[REQ-OLP-BOARD-SENTINEL] sentinel MUST 启动即补读账本（给出 `--since-head` 时只补读该事件之后的可处理状态），只扫描启动基线后的文字，并用固定 `LEDGER-SIGNAL`/`BOARD-SIGNAL`/`DRIFT`/`ERROR`/`TIMEOUT` 前缀区分信号；普通等待和尾部未完成行两条超时路径 MUST 输出 `TIMEOUT` JSON 行并退出 3；板缩短或替换 MUST 输出 ERROR。基线后的文字 MUST 按字节匹配 token，含非 UTF-8 字节的行 MUST 照常命中（展示时替换无法解码的字节），MUST NOT 使 sentinel 失败。

[REQ-OLP-BOARD-HARVEST] 进化采集 MUST 只在板含 `> OLP-EVENT ` 行且回放出至少一条有效事件时走结构化路径；不含此类行的板（包括带旧 shell `.lock` 的 legacy 板）MUST 不调用 Python、不取板锁，输出与 legacy 扫描器逐字节一致；含此类行但无有效事件的板 MUST 回落到 legacy 扫描器；含此类行而事件工具或板 `.lock` 缺失时 MUST 明确非零失败，MUST NOT 静默回落 legacy。结构化路径 MUST 在共享锁下只读取一次板字节，回放、legacy 扫描与结构化扫描都基于这一快照，快照之后的追加留给下一轮；同一 ACK 行在连续各轮 MUST 得到同一个 identity。结构化路径 MUST 按字节处理候选行，旧 shell 写入的非 UTF-8 旧触发行 MUST 照常出卡；legacy 候选行 MUST 按行号换算出快照中的真实字节 offset 再与回放区间比对（bash `read` 遇到 NUL 时，4 及以上版本丢弃该字节、3.2 在该处截断，两者行数都不变，但累加的 offset 不可靠），带 recovery 的 ACK MUST 沿用 legacy 扫描器同一轮为被恢复行给出的 identity（该行未出卡时才自行计算），使结果与运行 harvest 的 bash 版本无关，板上的 NUL 字节 MUST NOT 让同一 ACK 出两张卡；采集任何失败 MUST 以 `error:` 行非零退出，且 MUST 清理自己的临时目录。结构化路径 MUST 复用事件回放器和 item ID 取得展示编号，使乱序 ACK 归属原 item，行字段中的 `|` MUST 被转义，给定的或解析后的板路径含 `|` 或换行（行协议无法承载）时结构化采集 MUST 只以一个物理行的 `error:` 非零退出（路径转义后显示），带 recovery 的 ACK MUST 沿用被恢复原行的 legacy identity；MUST 保留 legacy ACK 与签名 override/R2 触发，只抑制围栏区间、已 void 区间、任何正式事件的 source（事件自己的正文，不论其中引用了什么）以及 ACK recovery 原文的重复卡。opt-in 后未闭合围栏 MUST 令采集明确非零失败且 MUST NOT 推进采集游标；legacy 采集函数 MUST 保持原行为。

[REQ-OLP-BOARD-COMPAT] 旧 shell watcher MUST 保持运行行为不变；旧 shell 追加器对 legacy 板 MUST 保持不变，只在板已含事件行或 `<!-- olp-board/v1 -->` 标记时拒绝以 `> OLP-EVENT ` 开头的正文行与独立 `ts=` 时间行（判定与 Python 追加器一致，行尾只允许可选 CR），也拒绝不以 LF 结尾的正文，并在板以无 LF 的半行结尾时拒绝追加（与 Python 追加器一致，分两次追加拼不出事件行），并在这类板经符号链接路径调用或有多个硬链接时拒绝，MUST NOT 在链接旁另建锁；板路径交给任何外部命令时 MUST 始终被当作路径（相对路径前加 `./`），以 `-`、`!` 或 `(` 开头的板名不得被读成选项或表达式而让上述判定失效。四个 Python CLI MUST 相邻部署、仅依赖标准库并与旧 shell 共用同一锁文件。

[REQ-OLP-BOARD-DEPLOY] `olp-init.sh` MUST 默认保留 legacy 模板，仅在 `OLP_BOARD_MODE=structured` 且 Python 3 可用时为新项目生成 receive→执行→ack 模板、带 `<!-- olp-board/v1 -->` 标记且无裸 ACK 占位的新板、独立普通锁，并把该锁加入 `.gitignore`；结构化循环 MUST 按账本事件顺序选择最早的 `unreceived` item。四个 Python 工具 MUST 以原文件名相邻安装且 MUST NOT 覆盖既有副本；既有项目文件 MUST NOT 被覆盖，只能输出迁移指引；无 Python 时 MUST 明示结构化能力不可用并保留旧 shell 工具。

[REQ-OLP-BOARD-TEST] 每个合约场景 MUST 绑定一个真实 Rust 集成测试函数，由测试直接调用生产 Python CLI、真实临时文件、真实旧 shell 锁和真实 harvest；故障注入 MUST 只存在于测试进程，生产 CLI MUST NOT 暴露故障后门。inbox 与 sentinel 的机器输出（结果、信号行、错误行与 ready 文件）在命令行参数含非 UTF-8 字节时 MUST 仍是合法 UTF-8 JSON，这类字节以转义形式显示，sentinel 命中后 MUST NOT 因输出失败退出；所有写入口失败（包括嵌套过深的 JSON 输入引发的递归错误与命令行参数解析错误）MUST 只输出一个机器 JSON，写前失败为 `may_have_appended=false`，write/fsync/readback/回执输出边界后的失败为 true。

## Scenarios

Scenario: 四事件全流程投影
  Given 一块已有板与独立锁
  When 依次运行 item、receive、ack、review 生产 CLI
  Then 四类待办按生命周期变化并在 accept 后清空

Scenario: 非法生命周期被拒绝
  Given 一个已发布的结构化 item
  When 错误 actor、重复 receive、重复 ack 或非法 wontdo return 被提交
  Then 命令退出 2 且账本 head 不因拒绝而推进

Scenario: 坏事件行被隔离而非致命
  Given 篡改 source、prev、时间边界或残缺的保留记录
  When 运行 state
  Then state 退出 0，该行以 malformed_event 字节证据进入 DRIFT，事件不进入账本且调度阻塞

Scenario: 隔离事件收口坏行与半截写入
  Given 中途被注入的事件行、以半截事件行结尾的板或以非事件半行结尾的板
  When 查询 state、尝试普通写入并以 void 精确引用 DRIFT 或 partial_tail
  Then 旧回执仍可复验、写入从最后有效 head 续链，普通写入在半行存在时被拒绝，void 后 DRIFT 清空且旧字节不变

Scenario: 旧 shell 在已 opt-in 板上拒绝事件行
  Given legacy 板、带 opt-in 标记的板和已有事件的板
  When 旧 shell 追加以 `> OLP-EVENT ` 开头的正文、以可选 CR 结尾的独立 `ts=` 行，以及行尾带空格的 `ts=` 行
  Then legacy 板照旧写入，另两块板对事件行与 ts 行退出 2 且字节不变，行尾带空格的 ts 行与普通文字仍可追加，与 Python 追加器判定一致

Scenario: 越过行首或与已配对原文重叠的 source 被隔离
  Given 一条已配对的 item 记录，以及手工写入的两个 item 事件：一个 source 从板头开始、覆盖该记录和一行引用 ACK，一个 source 从行中间开始
  When 运行 state，并对被覆盖的引用 ACK 行运行 void
  Then 两个手工事件都以 malformed_event 进入 DRIFT，账本只有原 item；引用 ACK 行只能被 void 消费一次

Scenario: legacy 与 mixed 漂移
  Given opt-in 前旧 ACK、opt-in 后围栏示例、引用标题和新的未配对 item/ACK 与引用 ACK
  When 查询结构化状态
  Then 只把新的未配对 item、规范 ACK 与疑似 ACK 标为 DRIFT

Scenario: 手写 ACK 变体阻断并精确收口
  Given opt-in 后逐一追加缩进、全角冒号、引用、列表、标题、强调、行内与旧式 ACK
  When 查询 state 并以 ack recovery 或 void 收口
  Then 每种都成为阻断 DRIFT，含 outcome 的只接受相同 outcome 的 recovery，无 outcome 的只能 void

Scenario: 未闭合围栏阻塞并可追加恢复
  Given opt-in 后分别追加未闭合的反引号与波浪线围栏
  When 查询 state/inbox/sentinel、运行 harvest 并依次追加错误种类、过短和匹配闭合行
  Then opener 字节证据可见且调度与采集阻塞,只有匹配闭合行恢复结构化写入且采集游标未推进

Scenario: 精确恢复并保留原文
  Given opt-in 后一条未配对规范 item 或 ACK
  When 同类型正式事件以 recovery 引用其精确字节区间
  Then DRIFT 列表清空且旧字节与 recovery_evidence 均保留

Scenario: 无效恢复零写入
  Given opt-in 前、已配对、已消费、坏区间/摘要或语义不匹配的证据
  When item/ack 或通用 record 尝试补录
  Then 命令退出 2 且板字节和 head 不变

Scenario: 示例文字不可成为恢复对象
  Given 围栏中的 item/ACK 示例或引用中的 item 标题位于 opt-in 后
  When item/ack 记录以其字节区间作为 recovery，或 replay 遇到被篡改为指向它们的 recovery
  Then 记录命令退出 2 且板字节不变，被篡改的事件以 malformed_event 进入 DRIFT

Scenario: 撤回与关闭升级
  Given 投错收件人的 item 与一条已 escalate 的 review
  When 非作者或作者执行 withdraw 与 resolve
  Then 只有作者成功，已撤回 item 与已关闭升级离开待办集合，重复或非法关闭退出 2

Scenario: 基线只对新增状态报信号
  Given 一条已知的 escalation 与一个已 receive 的 item
  When inbox 与 sentinel 以 --since-head 等待并随后写入新 ACK
  Then 已知 escalation 不再唤醒，新 ACK 输出 LEDGER-SIGNAL 且只含该 ACK

Scenario: 读者共享锁
  Given 另一进程持有共享锁或独占锁
  When 运行 state 与 item
  Then 共享锁下 state 成功而写者等待超时，独占锁下 state 等待超时

Scenario: 只以 LF 切行
  Given 含裸 CR 的普通追加正文
  When 回放并对 CR 后的 ACK 文字做 recovery
  Then 事件前缀不能借 CR 混入，CR 后 ACK 是可恢复的疑似 ACK

Scenario: 并发与旧锁兼容
  Given 多个新 CLI 写者和旧 shell 写者共享同一板锁
  When 它们并发追加
  Then 每次正文保持连续且事件 prev 链可完整回放

Scenario: inbox 与 sentinel 信号
  Given 无待办、待收 item、已收未完 item 和启动后的文字 token
  When 运行 inbox 等待与 sentinel
  Then ready-file 只创建一次且 timeout、LEDGER-SIGNAL 和 BOARD-SIGNAL 可区分

Scenario: sentinel 超时输出机器行
  Given 普通空板等待或监视期间出现尾部未完成行
  When sentinel 到达等待期限
  Then 两条路径均输出 TIMEOUT JSON 行并退出 3

Scenario: 任意编号保持账本投递顺序
  Given 展示编号依次为 4N、2、A 的三个 item
  When runtime inbox 查询 unreceived
  Then 返回顺序仍为 4N、2、A

Scenario: 乱序完成保持待审和升级事件顺序
  Given 按 A、B、C、D 派单，完成与升级事件以不同顺序到达
  When 查询 state 和 outer inbox（包括 since-head 过滤）
  Then unreviewed_ack 按 ACK 事件顺序，escalated 按 review 事件顺序，outer messages 按两类事件的共同账本顺序返回

Scenario: legacy 板采集逐字节不变
  Given 带旧 shell 锁文件的 legacy 板、被他人持有的板锁、会记录调用的 python3 替身和一行引用事件文字
  When 运行真实进化采集脚本
  Then 输出与 legacy 扫描器一致，既不调用 python3 也不等锁

Scenario: 结构化板缺事件工具或板锁时采集明确失败
  Given 一块已有结构化事件的板
  When 分别在事件工具缺失与板锁缺失时运行真实进化采集脚本
  Then 两次都非零退出并指明缺失项，不输出卡片；工具与锁齐全时恰有一张含 R8-1 的卡

Scenario: 非 UTF-8 旧触发行照常采集且失败不留临时目录
  Given 已 opt-in 的板上有一条经旧 shell 追加、含非 UTF-8 字节的 blocked ACK 行
  When 运行真实进化采集脚本，并在事件工具返回损坏状态时再运行一次
  Then 第一次恰有一张卡，identity 与旧扫描器对同一行给出的一致；第二次以 error 行非零退出；两次都不在临时目录留下工作目录

Scenario: 嵌套过深的 JSON 输入只得到一个机器错误
  Given 一个嵌套二十万层的 JSON 文件
  When 把它作为 record 的事件文件、verify 的回执文件、item 的 recovery 文件和 void 的 target 文件
  Then 每次都退出 2，stderr 恰为一个 JSON 对象，板字节不变

Scenario: 构造的 JSON 键不会让整板读不出来
  Given 已 opt-in 的板上被追加一行事件，其 JSON 以孤立代理项转义 `\ud800` 作重复键
  When 运行 state、runtime inbox、sentinel 与真实进化采集脚本
  Then state 与 inbox 退出 0 并把该行报为 malformed_event，sentinel 输出 DRIFT，采集退出 0

Scenario: sentinel 按字节匹配非 UTF-8 文字中的 token
  Given sentinel 已在结构化板上记下基线并等待 token
  When 旧 shell 追加一行含该 token 与非 UTF-8 字节的文字
  Then sentinel 输出 BOARD-SIGNAL 并退出 0

Scenario: 板上的 NUL 字节不让同一 ACK 出两张卡
  Given 已 opt-in 的板上一条经旧 shell 追加、含 NUL 字节的手写 blocked ACK，本机上的每个 bash（含 macOS 自带的 3.2）各测一遍
  When 运行真实进化采集脚本，再以正式 ACK recovery 该行并追加一行签名 R2 记档后再运行一次
  Then 两次都只有该 ACK 的一张卡且 identity 相同，R2 记档卡的 offset 等于该行的真实字节 offset

Scenario: 旧 shell 在已 opt-in 板上不写半行
  Given 一块 legacy 板和一块已有事件的板
  When 旧 shell 追加不以 LF 结尾的正文、在以半行结尾的板上追加，并分两次追加 `> OLP-` 与 `EVENT {...}`
  Then legacy 板照旧写入；已 opt-in 的板对前两种退出 2 且字节不变，分两次追加拼不出事件行

Scenario: NUL 拆开关键字的手写 ACK 阻止派发且只出一张卡
  Given 已 opt-in 的板上一条经旧 shell 追加的 `A<NUL>CK(blocked): ...` 行，本机上的每个 bash 各测一遍
  When 运行 state 与真实进化采集脚本，再以正式 ACK recovery 该行后各运行一次
  Then state 把它报为 unpaired_ack 并阻止派发；recovery 前采集至多一张卡，recovery 后恰有一张卡，前后若都有卡则 identity 相同

Scenario: 标题含 `. ` 的 item 能以原编号和标题 recovery
  Given opt-in 后一行手写的 `### 9. Fix. the thing with dots`
  When 以编号 9 与标题 `Fix. the thing with dots` recovery，并尝试写入编号含 `. ` 的 item
  Then recovery 被接受且 DRIFT 清空；编号含 `. ` 的写入退出 2，板字节不变

Scenario: 参数含非 UTF-8 字节时各 CLI 的机器输出仍可解析
  Given 一块结构化板
  When sentinel 以含非 UTF-8 字节的 token 与 actor 命中一行，inbox 以含非 UTF-8 字节的 actor 查询，sentinel 与 inbox 各以含非 UTF-8 字节的未知 --since-head 启动
  Then 前两者退出 0，输出与 ready 文件都是可解析的 JSON（该字节以转义显示）；后两者各以一条可解析的错误行退出 2，都没有 traceback

Scenario: 有多个硬链接的板被拒绝
  Given 一块已 opt-in 的板和指向它的硬链接，其中一个名字只是一个换行
  When 通过任一路径运行 item、state、普通追加与旧 shell 追加，再删除硬链接后重试
  Then 有硬链接时都退出 2 且板字节不变，删除后照常工作

Scenario: 旧 shell 把以 `-` 开头的板名当作路径
  Given 一块名为 `-x` 的已 opt-in 板，以及另一目录下指向它、同样名为 `-x` 的硬链接
  When 在各自目录里以相对名 `-x` 调用旧 shell，追加事件行、普通文字
  Then 有硬链接时两边都以硬链接为由退出 2；删去硬链接后事件行仍被拒绝、普通文字照常追加

Scenario: 等锁期间出现的硬链接在取得锁后被发现
  Given 写者与读者已通过路径检查，在等锁期间板多出一个名字
  When 它们取得锁
  Then 读、普通追加、事件写入与旧 shell 追加都拒绝，板字节不变

Scenario: 正式事件正文里引用的触发行不出卡
  Given 一条正式 item 的正文里引用了 `ACK(blocked): ...` 与被 NUL 拆开的同类行
  When 运行 state 与真实进化采集脚本
  Then state 干净且可派发，采集不出卡；正文之外的同类手写行照常出卡并报 DRIFT

Scenario: 板路径含行分隔符时结构化采集明确失败
  Given 路径含 `|`、中间含换行或以换行结尾的三块有 blocked ACK 的结构化板
  When 分别运行真实进化采集脚本
  Then 都非零退出，stderr 只有一个物理行的 error 且没有 traceback，不输出卡片

Scenario: 写入口的参数错误只得到一个机器错误
  Given 两个写入口
  When 传入非法数值、非法选项值或缺少必需参数
  Then 每次都退出 2，stderr 恰为一个 may_have_appended=false 的 JSON 对象，板字节不变

Scenario: 采集只读一次快照
  Given 编号为 R8-1 的 item 已被 receive，其 blocked ACK 恰在采集取得快照之后追加
  When 连续运行两轮真实进化采集脚本
  Then 第一轮不为该 ACK 出卡，第二轮以含 R8-1 的结构化 identity 出卡，两轮合计只有一个 identity

Scenario: 乱序 ACK 精确采集
  Given legacy/签名触发与 item 1、item 2 的终态 ACK 以 2 后 1 的顺序并存
  When 运行真实进化采集脚本
  Then 恰有四张卡，结构化 identity 分别含 #2#blocked 与 #1#wontdo，legacy 与签名触发各一张

Scenario: 编号中的竖线不破坏采集行
  Given 编号为 A|1 与 A|2 的两个 blocked ACK
  When 运行真实进化采集脚本
  Then 两张卡的 identity 分别含 A¦1 与 A¦2 且 envelope 三个字段均可解析

Scenario: 恢复前已采集的手写 ACK 不重复落卡
  Given 一条手写 blocked ACK 已被真实 harvest 落卡
  When 正式 ACK 以 recovery 补录该行后再次 harvest
  Then 进化黑板仍只有一张卡

Scenario: 已恢复 ACK 只采集一次
  Given blocked 或 wontdo 的旧文字 ACK 已由正式结构化 ACK 精确恢复且另有无关 legacy/签名触发
  When 运行真实进化采集脚本
  Then 每个 outcome 恰有三张卡，恢复对只占一张

Scenario: 写入故障机器可判定
  Given 写前校验或 write/fsync/readback/回执输出边界失败
  When 运行任一写入口
  Then stderr 只有一个 JSON 且 may_have_appended 精确区分写前与写后风险

Scenario: 符号链接板被拒绝
  Given 指向真实板的符号链接路径
  When 通过它运行 item、state、普通追加，以及对已 opt-in 板的旧 shell 追加
  Then 四者都退出 2，板字节不变，链接旁不生成锁

Scenario: 结构化部署可实际运行且幂等
  Given 临时 HOME、新建临时 git 项目，以及 PATH 前放置的 octoscode/octos 空桩（结果不取决于机器上是否全局安装）
  When 以 structured 模式运行 init 并调用安装后的四个生产工具完成生命周期
  Then 新模板含 opt-in 标记、锁被 git 忽略，重复 init 不覆盖项目文件、锁或已安装工具

Scenario: legacy 与无 Python 降级保持可用
  Given 默认 init（PATH 前放置 octoscode/octos 空桩）或 PATH 中没有 Python 的临时项目
  When 运行真实 init
  Then 默认模板仍走旧 ACK 契约且无 Python 时只禁用结构化能力并继续安装旧 shell 工具

## Dependencies

- REQ-OLP-PROTO(ACK v1 定式与 R1/R5 文字协议)
- REQ-OLP-EVO(阶段 0 采集的来源、卡片与游标契约)

## Source Trace

- operator 2026-09-27 要求按上游需求与 spec 规范交付。
- PR #668 评审发现：坏行使账本不可用、手写 ACK 变体漏检、升级无法关闭、旧板采集行为改变、CR 切行不一致与回放规模。
- PR #668 维护者评审（2026-09-28）：init 测试依赖全局安装的 octoscode、source 区间可越过行首与已配对区间重叠、采集静默降级改变 ACK identity 且快照与扫描之间有竞态、围栏内 ts 行与事件行处理不对称、旧 shell 的符号链接与 ts 判定与 Python 工具不一致。
- PR #668 维护者复审（2026-09-28 17:14Z，基于 `4c5e0ef`）：非 UTF-8 旧触发行让结构化采集崩溃、不出卡并遗留临时目录；嵌套过深的 JSON 让事件 CLI 输出原始 traceback 而不是机器 JSON。
- 作者自查（2026-09-29，基于 `58d2277`）：以孤立代理项转义作重复键的事件行让 state、inbox、sentinel 与采集整体失败；sentinel 遇到旧 shell 写入的非 UTF-8 行即报 ERROR 退出，重挂后该行的 token 永久漏报；bash `read` 丢弃 NUL 使 legacy offset 偏移，同一 ACK 出两张卡；旧 shell 可在已 opt-in 板的半行上续写，分两次拼出事件行。
- 异构审查（codex，2026-09-29，基于 `a1e48c7`）：NUL 拆开 ACK 关键字时 state 报板干净而 harvest 出 blocked 卡；硬链接让同一块板分裂成两个锁域；板路径含 `|` 时采集行协议被击穿并输出 traceback；写入口的参数解析错误输出 usage 文本而不是机器 JSON。
- 异构复审（codex，2026-09-29，基于 `9c09017`）：正式 item 正文里引用的 ACK 被 harvest 当作手写触发出卡；旧 shell 的硬链接检查可被换行文件名绕过，且各入口只在取锁前检查；换行路径的错误信息破成两行，结尾换行被命令替换吞掉后绕过检查。EVENTS/MCP 路径含 `|` 时字段错位是本 PR 之前就有的 legacy 行为，不在本需求范围。
- 异构复审第三轮（codex，2026-09-29，基于 `3735e95`）：以 `-` 开头的相对板名被旧 shell 交给 `find`/`grep` 当作选项，opt-in 与硬链接判定都失效，事件行守卫与锁域保护一并被绕过。
- PR #668 维护者第三次复审（2026-09-29 17:11Z，基于 `b5a69d4`）：NUL 处理默认了 bash 4 及以上的 `read`，macOS 自带的 bash 3.2 在 NUL 处截断，一条 ACK 在 recovery 前后得到两个 identity，两个 NUL 测试在 3.2 下失败；标题含 `. ` 的 item 不能以原编号和标题 recovery；sentinel 的 token 含非 UTF-8 字节时命中后在输出处退出。

## Open Questions

- `olp-board/v1` 目前是 opt-in 实验扩展；合并前维护者需决定何时以及是否把 receive 语义纳入 OLP 主协议版本。
- 追加投影不物理移动历史，与现行 R5「移入历史区」的差异需由维护者决定未来主协议表述；本扩展仅冻结复制整板并在无在途项时另起新链。
- 写入时仍对整板回放两次（线性）；是否需要按 offset 的增量缓存，留待真实板规模数据决定。

## Next

Single exit: compile this requirement into `specs/task-req-olp-board.spec.md`.
