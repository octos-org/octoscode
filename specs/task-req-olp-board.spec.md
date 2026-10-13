spec: task
name: "OLP 结构化追加黑板与可恢复状态投影"
tags: [olp, protocol, blackboard, observability]
satisfies: [REQ-OLP-BOARD]
estimate: 3d
---

## 意图

为现有文字 OLP 黑板增加 opt-in 的 `olp-board/v1` 结构化扩展。规范文字继续供人和旧工具读取，item、receive、ack、review 及 withdraw、resolve、void 收口事件在同一锁域与文字成对追加；inbox、sentinel 和进化采集从真实回放状态判断待办、漂移和精确条目归属。坏行、半截写入与手写 ACK 变体只会成为带字节证据的阻断 DRIFT，不会让整块板失效。旧板默认保持 legacy，本任务不升 OLP 主版本。

## 已定决策

- 四个相邻标准库 Python CLI 分别负责字节追加、事件回放、待办查询和综合哨；旧 watcher 只增加英文指引；旧追加器只在已 opt-in（含事件行或 `<!-- olp-board/v1 -->` 标记）的板上拒绝 `> OLP-EVENT ` 与独立 `ts=` 正文行、不以 LF 结尾的正文以及在板尾半行上的续写（与 Python 追加器一致，分两次追加拼不出事件行），也拒绝经符号链接路径调用或有多个硬链接的板，legacy 板行为不变；独立 `ts=` 行的判定两边一致（行尾只允许可选 CR），围栏内也拒绝，示例须缩进或改写。
- 事件行固定以 `> OLP-EVENT ` 开头，共用 `schema/id/prev/type/actor/ts/source`；类型为 item、receive、ack、review、withdraw、resolve、void；item/ack 另有必需但可空的 `recovery`，void 带 `target` 字节证据。
- 回放逐行容错：任一事件行的 JSON、canonical、相邻 source、摘要、ts 边界、recovery/void 目标或生命周期校验失败，都成为 `malformed_event` DRIFT（证据为该行不含 LF 的字节），事件不入账，后续写者从最后有效 head 续链；state 本身不因坏行失败。DRIFT 原因总能作为 UTF-8 JSON 输出（重复键以 JSON 转义形式报出），单条事件行不会让 state、snapshot、inbox、sentinel 或 harvest 失败。
- 行只以 LF 切分；追加守卫、回放、recovery 行边界共用这一规则，裸 CR 是普通内容；规范 item/ACK 行允许以 CRLF 结尾。
- source 是事件行前紧邻的规范文字字节区间，必须从行首开始（void 收口半行时以补上的 LF 开头），且不得与已配对或已被 recovery/void 消费的区间重叠，否则该事件行成为 malformed_event；id、prev、ts、offset 在独占锁内生成，正文、事件、fsync 和回读属于一次追加；写前对候选板回放，新事件必须成为 head 且不得产生新 DRIFT。板以半行结尾时普通写入拒绝，只有以该半行为目标的 void 可先补 LF；半行会打开或位于未闭合围栏时 void 也会被隐藏，此时只能用普通追加器的 `--terminate-partial-line` 显式补一行闭合围栏。
- 读者以共享锁复制字节后在锁外回放；已配对、已消费区间用有序区间索引查询，回放近似线性。
- item 只有一次 receive、一次终态 ack、一次 review；return 或 blocked 恢复另开 item，wontdo 不得 return；作者可 withdraw 未 receive 的 item，可 resolve 已 escalate 的 review 并可指向另一个既有后续 item。
- 状态固定分为 unreceived、received_pending、unreviewed_ack、escalated；withdraw 与 resolve 使条目离开集合；runtime inbox 按 actor 显式返回前两类并保持账本出现顺序，但始终不授权执行；`--since-head` 只把触发事件位于基线之后的 received_pending、unreviewed_ack、escalated 计入 messages，runtime 尚未 receive 的 item 始终计入；基线应取自调用方已处理或有意暂留全部 messages 的那次输出的 head。
- opt-in 前历史不追溯；opt-in 后未配对规范 item（`unpaired_item`）、符合 v1 语法（前导空白、全角冒号）的 ACK（`unpaired_ack`）与引用/列表/标题/强调/行内/旧式疑似 ACK（`suspected_ack`）标 DRIFT 并阻止自动调度；已配对 source 与已闭合围栏示例不触发。判定规范行与疑似 ACK 前先去掉 NUL（与 bash 4 及以上的 `read` 一致），NUL 拆开关键字的手写 ACK 同样阻断派发；harvest 为某行出卡时 state 不会仍报该行干净（bash 3.2 的 `read` 在 NUL 处截断，这时 harvest 在 recovery 前可能不为该行出卡，state 仍阻断，偏保守）。未闭合围栏以 opener 字节证据阻塞，只有同种且不短于 opener 的闭合行恢复写入。末尾无 LF 的半行（无论内容）以 `partial_tail` 暴露，未进围栏的半截事件行同时是 malformed_event。
- 收口只追加：item/ack 的 recovery 精确消费一条未配对规范行或含 `ACK(outcome)` 的疑似 ACK，语义须匹配（同样不计 NUL；规范 item 标题行在第一个 `. ` 处拆分编号与标题，写入端拒绝含 `. ` 的编号，所以工具写出的标题行总能以原编号和标题恢复）；void 精确隔离一条 malformed_event、未配对行、疑似 ACK 或 partial_tail；每段原文至多消费一次，证据分别进入 recovery_evidence 与 quarantine_evidence。actor 是逻辑名，任何 actor 都可写 void，由外环人工核对是约定而非强制。
- sentinel 的固定输出前缀为 LEDGER-SIGNAL、BOARD-SIGNAL、DRIFT、ERROR、TIMEOUT；普通等待与尾部未完成行超时都输出 TIMEOUT JSON 行并退出 3。基线后的文字按字节匹配 token，含非 UTF-8 字节的行照常命中，展示时替换无法解码的字节。
- harvest 先无锁检查板中是否有 `> OLP-EVENT ` 行，没有就直接走原 legacy 扫描器，不调用 Python、不取板锁；有此类行时在共享锁下回放，回放出有效事件才走结构化路径，否则仍走 legacy 扫描器；有此类行而事件工具或板锁缺失时明确失败，不静默走 legacy（`.lock` 是结构化采集的前提）；结构化路径经 `snapshot` 在共享锁下只读一次字节，回放、legacy 扫描与结构化扫描都用这份快照，快照之后的追加留给下一轮，同一 ACK 跨轮只有一个 identity；候选行按字节合并，旧 shell 写入的非 UTF-8 旧触发行照常出卡，legacy 候选行按行号换算出快照中的真实 offset 后再与回放区间比对（bash `read` 遇到 NUL 时 4 及以上版本丢弃、3.2 截断，行数都不变，但累加的 offset 不可靠），带 recovery 的 ACK 沿用 legacy 扫描器同一轮为被恢复行给出的 identity（该行未出卡时才自行计算），结果与运行 harvest 的 bash 版本无关，采集在任何失败路径上都以 `error:` 行退出并清理临时目录；取得 ack.item、item.number（`|` 转义）与围栏/隔离区间；给定的或解析后的板路径含 `|` 或换行时行协议无法承载，结构化采集只以一个物理行的 `error:` 退出（路径转义后显示；给定路径也要查，因为命令替换会吞掉结尾换行）；带 recovery 的 ACK 沿用被恢复原行的 legacy identity，使恢复前已落的卡不重复；保留 legacy ACK 与签名 override/R2，抑制围栏示例、已隔离行、任何正式事件的 source（事件自己的正文，不论引用了什么）及 ACK recovery 原文的重复卡；临时文件用 mktemp。
- 板路径若是符号链接则拒绝，避免 Python 工具锁 `<目标>.lock` 而旧 shell 锁 `<链接>.lock`；板有多个硬链接（`st_nlink` 大于 1）时同样拒绝，否则每个路径各锁自己的 `.lock`；链接数在解析路径时查一次，取得锁、打开板后对打开的板再查一次（锁由路径派生，挡不住与写入并发的 link/unlink，是非对抗文件系统的前提）；旧 shell 按数值判断链接数，不从文件名文本推断；旧 shell 先把相对板路径改写成 `./<名字>`，以 `-`、`!`、`(` 开头的名字不会被 `find`/`grep` 读成选项或表达式；旧 shell 在已 opt-in 的板上同样拒绝，且不在链接旁建锁。
- inbox 与 sentinel 的机器输出（结果、信号行、错误行与 ready 文件）在命令行参数含非 UTF-8 字节时仍是合法 UTF-8 JSON（这类字节以转义显示），sentinel 命中后不会因输出失败退出。各 CLI 把嵌套过深的 JSON 引发的递归错误与其他输入错误一样，报成一个机器 JSON 并退出 2；两个写入口的命令行参数解析错误也是写前失败，同样只输出一个 `may_have_appended=false` 的机器 JSON。
- 依赖外部命令的 init 场景在 PATH 前放 `octoscode`/`octos` 空桩，测试结果不取决于开发机或 CI 是否全局安装这两个命令。
- init 默认保持 legacy；`OLP_BOARD_MODE=structured` 只为新文件生成 receive→执行→ack 模板、带 opt-in 标记且无裸 ACK 占位的新板和独立普通锁，并把锁加入 `.gitignore`；按账本事件顺序选择最早的 `unreceived` item。四个 Python 工具相邻安装且不覆盖，既有项目仅提示迁移，无 Python 时保留旧 shell 能力。
- 所有场景绑定 `tests/olp_board_protocol.rs` 中直接调用生产 CLI 的 Rust 集成测试，不用单一 Python suite wrapper 代替逐场景证据。

## 边界

### Allowed Changes
- scripts/olp-board-append.py
- scripts/olp-board-event.py
- scripts/olp-board-inbox.py
- scripts/olp-board-sentinel.py
- scripts/olp-board-append.sh
- scripts/olp-watch-board.sh
- scripts/olp-evo-harvest.sh
- scripts/olp-init.sh
- .claude/skills/octoloop/SKILL.md
- .claude/skills/olp-outer/SKILL.md
- docs/OUTER_LOOP_PROTOCOL.md
- docs/OLP_STRUCTURED_BOARD.md
- docs/OLP_STRUCTURED_BOARD.en.md
- docs/OLP_OUTER_BOOT.md
- docs/OLP_QUICKSTART.md
- docs/OLP_QUICKSTART.en.md
- docs/OCTOLOOP_AGENTS.md
- docs/OCTOLOOP_AGENTS.zh-CN.md
- tests/olp_board*.rs
- knowledge/requirements/req-olp-board.md
- specs/task-req-olp-board.spec.md

### Forbidden
- 不改 `.octos/loop.md`、`.octos/progress.md` 或 `docs/OUTER_LOOP_REVIEW.md` 冻结快照。
- 不改 Rust 运行时、Cargo 依赖或 CI workflow。
- 不自动迁移、重写、截断旧板，不自动创建或替换既有板锁。
- 不把逻辑 runtime actor 描述为 OS 身份认证或 R7 lease。
- 不在生产 CLI 暴露测试故障注入开关。

## 排除范围

- 自动迁移既有 loop、黑板、锁或仓库自身 tracked `.octos/loop.md`。
- OLP 主版本升级、R1/R5 最终规范裁定和仓库自身 tracked loop 切换。
- push、PR、issue 与外部项目部署。

## 完成条件

场景: 四事件全流程投影与回执复验(critical)
  标签: critical
  测试: olp_board_lifecycle_projects_actionable_states
  假设 一块已有普通板和已有独立普通锁
  当 依次运行 item、runtime inbox、receive、ack、outer inbox、review 与 verify
  那么 状态依次出现 unreceived、received_pending、unreviewed_ack 并在 accept 后清空
  并且 旧回执在后续追加后仍可复验

场景: 非法与重复状态转移零落盘(critical)
  标签: critical
  测试: olp_board_rejects_invalid_or_repeated_transitions
  假设 一个已发布或已终态的 item
  当 错误 actor、重复 receive、重复 ack、错误 review actor 或 wontdo return 被提交
  那么 命令退出 2 且拒绝动作不推进 head

场景: 通用 record 严格 JSON 且不能绕过正文配对
  测试: olp_board_record_is_strict_and_cannot_bypass_rendering
  假设 通用 record 读取事件文件和正文文件
  当 输入有效 receive、重复 JSON key 或 NaN
  那么 有效输入沿生产状态机追加,非法输入退出 2 且不追加

场景: 篡改证据、prev 或时间边界被隔离为 DRIFT(critical)
  标签: critical
  测试: olp_board_replay_quarantines_tampered_source_chain_and_timestamp
  假设 一块含有效结构化 item 的临时板副本
  当 source 字节、prev、source 类型或相邻 ts 行任一被修改后运行 state
  那么 state 退出 0,该事件以含对应完整性原因的 malformed_event 字节证据进入 DRIFT,账本无事件且调度阻塞

场景: legacy 历史豁免而新未配对 item/ACK 与引用 ACK 标 DRIFT
  测试: olp_board_mixed_mode_reports_only_post_opt_in_drift
  假设 opt-in 前旧 ACK、配对 source 内示例、opt-in 后围栏示例、引用标题,以及新的未配对 item、规范 ACK、引用 ACK 与 CRLF 结尾的 item 标题
  当 查询 state
  那么 只按 offset 顺序报出 unpaired_item、unpaired_ack、suspected_ack 与 unpaired_item,mode 为 mixed 且 dispatch_blocked 为 true

场景: 手写 ACK 变体阻断调度并只能精确收口(critical)
  标签: critical
  测试: olp_board_ack_variants_raise_drift_and_recover_exactly
  Level: integration
  Test Double: temporary boards only; production CLIs are not replaced
  Targets: shared normative and suspected ACK classification in olp-board-event.py
  假设 opt-in 且已 receive 的 item 之后逐一追加行首、缩进、全角冒号、CRLF、引用、列表、标题、强调、行内、无括号标题与旧式 `ACK:` 形式
  当 查询 state 并以 ack recovery 或 void 收口
  那么 前四种为 unpaired_ack、其余为 suspected_ack 且均阻断调度;含 `ACK(done)` 的只接受 done 的 recovery,其余被拒并提示 void,收口后 DRIFT 清空;仅以 ACK 一词开头的普通文字不报 DRIFT

场景: 回放与追加守卫只以 LF 切行
  测试: olp_board_replay_splits_lines_on_lf_only
  假设 普通追加器写入含裸 CR 的事件前缀文字和 CR 后的 ACK 文字
  当 回放并以 ack recovery 引用后者
  那么 前者不产生 DRIFT 且账本完整,后者是整行 suspected_ack 并可被精确恢复

场景: 未闭合围栏阻塞全部消费者且匹配闭合后恢复(critical)
  标签: critical
  测试: olp_board_unclosed_fence_blocks_consumers_and_matching_close_recovers
  Level: integration
  Test Double: temporary boards, harvest state, and short sentinel polling only; production parsers and CLIs are not replaced
  Targets: olp-board-event.py shared fence projection, inbox, sentinel, legacy append, and olp-evo-harvest.sh
  假设 opt-in 后分别追加四字符反引号或波浪线 opener、规范 ACK、错误种类闭合行与过短闭合行
  当 state、inbox、sentinel 与 harvest 读取该板,随后追加同种且长度不短于 opener 的闭合行并再次写 item
  那么 opener 的 offset/length/sha256 可见,mode 为 mixed 且调度阻塞,harvest 非零失败且不推进游标,补闭合后旧字节保留且写入恢复

场景: 越过行首或与已配对原文重叠的 source 被隔离(critical)
  标签: critical
  测试: olp_board_replay_quarantines_misaligned_and_overlapping_sources
  Level: integration
  Test Double: raw appends stand in for a writer that bypasses every guard
  Targets: olp-board-event.py replay source-range checks and void claiming
  假设 一条已配对的 item 记录,以及手工写入的两个 item 事件:一个 source 从板头开始、覆盖该记录和一行引用 ACK,一个 source 从行中间开始
  当 运行 state,并对被覆盖的引用 ACK 行运行两次 void
  那么 两个手工事件都以 malformed_event 进入 DRIFT,账本只有原 item;第一次 void 成功,第二次以 already consumed 被拒

场景: 保留前缀的残缺记录以 DRIFT 暴露且不可被掩盖(critical)
  标签: critical
  测试: olp_board_replay_quarantines_incomplete_reserved_records
  假设 缺失 ts、半截 ts、坏末条或损坏后又追加文字的保留前缀记录
  当 运行真实 state CLI 回放
  那么 全部退出 0 并以 malformed_event 报出坏行字节证据,后续追加不能使其消失

场景: 隔离事件收口注入行与半截写入(critical)
  标签: critical
  测试: olp_board_void_quarantines_injected_and_partial_event_lines
  Level: integration
  Test Double: raw appends stand in for a guard-bypassing writer and an interrupted write; production CLIs are not replaced
  Targets: olp-board-event.py replay, void, verify and item paths
  假设 一块中途被注入事件行的板、一块以半截事件行结尾的板、一块以围栏 opener 半行结尾的板和一块以非事件半行结尾的板
  当 查询 state、复验旧回执、尝试普通写入,并以 void 精确引用 DRIFT 或 partial_tail
  那么 旧回执仍验证成功,写入从最后有效 head 续链;半截事件行同时出现在 malformed_event 与 partial_tail;半行存在时普通写入与先 void 其他条目均被拒绝;围栏半行的 void 被拒并提示先闭合围栏,只有带 --terminate-partial-line 的闭合追加成功;收口后 DRIFT 与 partial_tail 清空、原字节不变且写入恢复

场景: 旧 shell 在已 opt-in 板上拒绝事件行
  测试: olp_board_legacy_shell_refuses_event_lines_on_opted_in_boards
  假设 一块 legacy 板、一块只带 opt-in 标记的板和一块已有事件的板
  当 旧 olp-board-append.sh 追加以 `> OLP-EVENT ` 开头的正文、以可选 CR 结尾的独立 `ts=` 行、行尾带空格的 `ts=` 行与普通文字,并以 Python 追加器对比同样的 ts 行
  那么 legacy 板照旧写入,另两块板对事件行与 ts 行退出 2 且字节不变,行尾带空格的 ts 行与普通文字仍可追加,两个追加器对每种 ts 行判定一致

场景: runtime inbox 按 actor 补读已收未完
  测试: olp_board_runtime_inbox_filters_and_recovers_received_pending
  假设 指向 worker 的 item 已 receive 但未 ack
  当 runtime inbox 以错误 actor 查询或进程重启后以 worker 查询
  那么 错误 actor 不命中,worker 的 received_pending 重复可见且 execution_authorized 为 false

场景: 作者撤回投错 item 并关闭升级
  测试: olp_board_withdraw_and_resolve_close_open_queues
  假设 一个投错收件人的 item、一个已 receive 并 escalate 的 item 和一个已 accept 的 review
  当 非作者与作者分别执行 withdraw、resolve,并对已 receive、已 resolve、非升级或自指的目标重复操作
  那么 只有作者的合法操作成功,撤回的 item 不可再 receive 且离开 unreceived,关闭的升级离开 escalated,其余均退出 2;64 位提交摘要被接受

场景: --since-head 只对新增可处理状态报信号
  测试: olp_board_since_head_signals_only_new_actionable_state
  Level: integration
  Test Double: temporary boards, ready files, and short polling intervals only
  Targets: olp-board-inbox.py and olp-board-sentinel.py --since-head filtering
  假设 一条已知 escalation、一个已 receive 的 item、记录下来的账本 head,以及一个排队 item 与一个 runtime 在途 item
  当 inbox 与 sentinel 以 --since-head 等待,随后 runtime 写入新 ACK
  那么 无基线查询仍命中,带基线的 inbox 等待超时退出 3,未知基线退出 2,sentinel 只以新 ACK 输出 LEDGER-SIGNAL;runtime 以在途 receive 为基线时 messages 只含排队 item

场景: 多个结构化写者同锁生成完整链
  测试: olp_board_concurrent_writers_share_one_lock_and_chain
  假设 八个 item CLI 共享一块板及锁
  当 八个进程并发追加
  那么 八个命令全部成功且 state 回放八个互不丢失的 item

场景: 旧 shell 与新写者共锁不交错
  测试: olp_board_old_shell_and_new_writer_do_not_interleave
  假设 旧追加器写入大正文且新 item CLI 同时启动
  当 两个生产入口共享同一 `.lock`
  那么 旧正文保持一个连续区间且结构化事件可回放

场景: 读者共享锁且写者仍互斥
  测试: olp_board_readers_share_the_lock_and_writers_wait
  假设 另一进程先后持有板的共享锁与独占锁
  当 运行 state 与 item 并给出短锁超时
  那么 共享锁下 state 成功而 item 报 Board lock unavailable,独占锁下 state 报 Board lock unavailable

场景: 符号链接板路径被拒绝
  测试: olp_board_tools_refuse_symlinked_boards
  假设 一个指向真实板的符号链接路径,真实板已 opt-in
  当 通过它运行 item、state、普通追加与旧 shell 追加
  那么 四者均退出 2 并报 symlink,真实板字节不变,链接旁不生成锁

场景: inbox 等待与 sentinel 信号可区分
  测试: olp_board_inbox_and_sentinel_expose_distinct_outcomes
  Level: integration
  Test Double: temporary boards and short polling intervals only; production state machines are not replaced
  Targets: olp-board-inbox.py and olp-board-sentinel.py production entry points
  假设 空账本、待收 item 和 sentinel 启动后追加的文字 token
  当 运行 inbox wait 与 sentinel
  那么 空账本超时退出 3,待收 item 输出 LEDGER-SIGNAL,新文字输出 BOARD-SIGNAL

场景: sentinel 两条超时路径输出机器行
  测试: olp_board_sentinel_timeouts_are_machine_readable
  Level: integration
  Test Double: temporary boards, ready files, and short polling intervals only; production sentinel is not replaced
  Targets: olp-board-sentinel.py ordinary deadline and incomplete trailing-line deadline
  假设 一块空板和一块监视启动后出现尾部未完成行的板
  当 两个 sentinel 分别到达等待期限
  那么 两者均输出 TIMEOUT JSON 行并退出 3,尾部路径明确报告 incomplete trailing line

场景: 任意展示编号不改变账本投递顺序
  测试: olp_board_inbox_preserves_ledger_order_for_arbitrary_numbers
  Level: integration
  Test Double: temporary board files only; production event and inbox CLIs are not replaced
  Targets: olp-board-event.py insertion order and olp-board-inbox.py runtime projection
  假设 三个 item 的展示编号按 4N、2、A 的顺序写入
  当 runtime inbox 查询 unreceived
  那么 返回顺序仍为 4N、2、A 且不按编号排序

场景: 乱序完成时待审与升级按自身事件顺序返回
  测试: olp_board_outer_queues_follow_ack_and_review_event_order
  Test Path Statement: Real-path regression through event, state and outer inbox CLIs.
  Test Double: Only temporary board files; no parser or ledger substitutes.
  Targets: olp-board-event.py projection and olp-board-inbox.py outer messages
  假设 A、B、C、D 依次派单,ACK 按 B、A、C 到达,升级按 C、B 到达,最后 D ACK
  当 查询 state、outer inbox 与带 since-head 的 outer inbox
  那么 待审 ACK 和升级各按其自身事件顺序排列,合并 messages 也按事件顺序排列

场景: sentinel 启动后动态账本事件使用冻结前缀
  测试: olp_board_sentinel_reports_dynamic_ledger_signals
  Level: integration
  Test Double: temporary boards, ready files, and short polling intervals only; production writers and sentinel are not replaced
  Targets: olp-board-sentinel.py polling loop plus olp-board-event.py item, receive, and ack paths
  假设 runtime 与 outer sentinel 均先在各自无匹配消息时完成首次查询
  当 启动后依次写入目标 runtime 的真实 item 和面向 outer 的真实 ACK
  那么 两次动态命中均输出 LEDGER-SIGNAL 且包含对应事件 ID

场景: 空板等待只创建一次 ready-file
  测试: olp_board_empty_wait_owns_ready_file_once
  Level: integration
  Test Double: temporary boards, ready files, and short polling intervals only
  Targets: olp-board-inbox.py wait and ready-file creation
  假设 空板等待分别走超时、新 item 唤醒和 ready-file 已存在三条路径
  当 运行真实 inbox wait
  那么 前两条只创建一次 ready-file 且按预期退出,已存在文件被拒绝且不覆盖

场景: sentinel 区分漂移与板替换错误
  测试: olp_board_sentinel_reports_drift_and_replacement_errors
  假设 一块含新未配对 ACK 的 mixed 板和另一块正被监视的结构化板
  当 sentinel 查询 mixed 板或监视中的板 inode 被替换
  那么 前者输出 DRIFT 并停止自动调度,后者输出 ERROR 且退出 2

场景: 混合采集保留旧触发并精确归属乱序 ACK(critical)
  标签: critical
  测试: olp_board_harvest_attributes_out_of_order_acks_to_event_items
  假设 legacy ACK、签名 R2、围栏示例以及 item 1 与 item 2 按 2 后 1 追加的 ACK 同时存在
  当 真实 harvest 以 dry-run 读取结构化板
  那么 恰有四张卡,包含 legacy 与签名触发且结构化 identity 分别含 #2#blocked 与 #1#wontdo

场景: legacy 板采集逐字节不变(critical)
  标签: critical
  测试: olp_board_harvest_keeps_legacy_boards_byte_identical
  Level: integration
  Test Double: a recording python3 shim, a lock-holder process and temporary boards; the harvest script is not replaced
  Targets: olp-evo-harvest.sh opt-in precheck and the unchanged legacy scanner
  假设 一块带旧 shell 锁文件的 legacy 板、被另一进程独占的板锁、会记录调用的 python3 替身,以及随后追加的一行引用事件文字
  当 以真实事件工具运行 harvest dry-run,并与不提供事件工具的 legacy 扫描器输出对比
  那么 持锁时 harvest 成功、不调用 python3、不等锁且输出(掩去时间戳)与 legacy 一致;引用事件文字不含有效事件时会被回放,但仍回落到与 legacy 一致的输出

场景: 结构化板缺事件工具或板锁时采集明确失败
  测试: olp_board_harvest_refuses_to_degrade_structured_boards
  Level: integration
  Test Double: temporary boards; the harvest script and event tool are not replaced
  Targets: olp-evo-harvest.sh opt-in gate
  假设 一块已有结构化事件且有编号为 R8-1 的 blocked ACK 的板
  当 分别在事件工具缺失与板锁缺失时运行真实 harvest dry-run
  那么 两次都非零退出并指明缺失项,不输出卡片;工具与锁齐全时恰有一张含 #R8-1#blocked# 的卡

场景: 非 UTF-8 旧触发行照常采集且失败不留临时目录
  测试: olp_board_harvest_keeps_non_utf8_triggers_and_cleans_up
  Level: integration
  Test Double: a temporary TMPDIR, and an event-tool wrapper that forwards to the real tool except for returning a corrupt snapshot state in the second run
  Targets: olp-evo-harvest.sh candidate merge and temporary-directory cleanup
  假设 已 opt-in 的板上有一条经旧 shell 追加、含非 UTF-8 字节的 blocked ACK 行
  当 运行真实 harvest dry-run,再在事件工具返回损坏的 snapshot 状态时运行一次
  那么 第一次退出 0 且恰有一张卡,identity 为旧扫描器对该行给出的 #-1#blocked#<行 sha256>;第二次非零退出并输出 error 行;两次都不在 TMPDIR 留下工作目录

场景: 嵌套过深的 JSON 输入只得到一个机器错误
  测试: olp_board_event_cli_reports_deep_json_as_one_machine_error
  Level: integration
  Test Double: a generated deeply nested JSON file
  Targets: olp-board-event.py record/verify/item/void input parsing and error reporting
  假设 一个嵌套二十万层的 JSON 文件
  当 把它作为 record 的事件文件、verify 的回执文件、item 的 recovery 文件和 void 的 target 文件
  那么 每次都退出 2,stderr 恰为一个 JSON 对象,板字节不变

场景: 构造的 JSON 键不会让整板读不出来(critical)
  标签: critical
  测试: olp_board_drift_reasons_stay_printable_for_crafted_json_keys
  Level: integration
  Test Double: a raw file append stands in for a writer that bypasses every guard; the consumers are not replaced
  Targets: olp-board-event.py malformed-event reasons and the state, inbox, sentinel and harvest consumers
  假设 已 opt-in 的板上被追加一行事件,其 JSON 以孤立代理项转义 `\ud800` 作重复键
  当 运行 state、runtime inbox、sentinel 与真实 harvest dry-run
  那么 state 与 inbox 退出 0 并把该行报为 malformed_event,原因中的键以 JSON 转义形式出现;sentinel 输出 DRIFT 并退出 0;harvest 退出 0

场景: sentinel 按字节匹配非 UTF-8 文字中的 token
  测试: olp_board_sentinel_matches_tokens_in_non_utf8_text
  Level: integration
  Test Double: temporary boards and a ready file only
  Targets: olp-board-sentinel.py post-baseline text scan
  假设 sentinel 已在结构化板上记下基线并等待 token
  当 旧 shell 追加一行含该 token 与非 UTF-8 字节的文字
  那么 sentinel 输出 BOARD-SIGNAL 并退出 0,命中行中无法解码的字节以替换字符展示

场景: 板上的 NUL 字节不让同一 ACK 出两张卡
  测试: olp_board_harvest_ignores_nul_bytes_like_the_legacy_scanner
  Level: integration
  Test Double: temporary boards only; the harvest script and event tool are not replaced
  Targets: olp-evo-harvest.sh candidate-merge offsets and the identity of recovered lines
  假设 已 opt-in 的板上一条经旧 shell 追加、含 NUL 字节的手写 blocked ACK,本机上的每个 bash(PATH 上的与 /bin/bash,macOS 上后者是 3.2)各测一遍
  当 运行真实 harvest dry-run,再以正式 ACK recovery 该行、追加一行签名 R2 记档后再运行一次
  那么 两次都只有该 ACK 的一张卡且 identity 相同(recovery 沿用旧扫描器给该行的 identity);第二次另有 R2 记档一张卡,其 envelope offset 等于该行在板上的真实字节 offset

场景: 旧 shell 在已 opt-in 板上不写半行
  测试: olp_board_legacy_shell_keeps_lines_whole_on_opted_in_boards
  Level: integration
  Test Double: temporary boards only
  Targets: olp-board-append.sh opted-in guard
  假设 一块 legacy 板和一块已有事件的板
  当 旧 shell 追加不以 LF 结尾的正文、在以半行结尾的板上追加,并分两次追加 `> OLP-` 与 `EVENT {...}`
  那么 legacy 板照旧写入;已 opt-in 的板对前两种退出 2 且字节不变,分两次追加拼不出事件行

场景: NUL 拆开关键字的手写 ACK 阻止派发且只出一张卡(critical)
  标签: critical
  测试: olp_board_nul_split_ack_blocks_dispatch_and_cards_once
  Level: integration
  Test Double: temporary boards only, run under every bash on the host (on macOS the system /bin/bash is 3.2); the harvest script and event tool are not replaced
  Targets: olp-board-event.py normative classification and recovery matching, olp-evo-harvest.sh legacy scanner and recovered identity
  假设 已 opt-in 的板上一条经旧 shell 追加的 `A<NUL>CK(blocked): ...` 行
  当 运行 state 与真实 harvest dry-run,再以正式 ACK recovery 该行后各运行一次
  那么 state 把它报为 unpaired_ack 并阻止派发;recovery 前 harvest 至多一张卡(bash 4 及以上一张,3.2 为零);recovery 后 DRIFT 清空且恰有一张卡,前后若都有卡则 identity 相同

场景: 标题含 `. ` 的 item 能以原编号和标题 recovery
  测试: olp_board_item_numbers_stay_parseable_for_titles_with_dot_space
  Level: integration
  Test Double: temporary boards; a raw append stands in for a hand-written heading
  Targets: olp-board-event.py ITEM_LINE, recovery matching and the item writer
  假设 opt-in 后一行手写的 `### 9. Fix. the thing with dots`
  当 以编号 9 与标题 `Fix. the thing with dots` recovery,并尝试写入编号为 `9. Fix` 的 item
  那么 recovery 被接受且 DRIFT 清空;编号含 `. ` 的写入退出 2 并指明原因,板字节不变

场景: 参数含非 UTF-8 字节时各 CLI 的机器输出仍可解析
  测试: olp_board_cli_output_survives_non_utf8_arguments
  Level: integration
  Test Double: temporary boards and non-UTF-8 command-line arguments only
  Targets: olp-board-sentinel.py emit and ready file, olp-board-inbox.py output, the shared printable helper
  假设 一块结构化板
  当 sentinel 以含非 UTF-8 字节的 token 与 actor 命中旧 shell 追加的一行,inbox 以含非 UTF-8 字节的 actor 查询,sentinel 与 inbox 各以含非 UTF-8 字节的未知 --since-head 启动
  那么 前两者退出 0,输出与 ready 文件都是可解析的 JSON(该字节以转义显示);后两者各以一条可解析的错误行退出 2(sentinel 在 stdout 输出 ERROR 行,inbox 在 stderr 输出错误 JSON),都没有 traceback

场景: 有多个硬链接的板被拒绝
  测试: olp_board_tools_refuse_hard_linked_boards
  Level: integration
  Test Double: temporary boards and a hard link only
  Targets: olp-board-append.py board path check, olp-board-append.sh opted-in guard
  假设 一块已 opt-in 的板和指向它的硬链接(各有 `.lock`),另有一个名字只是一个换行的硬链接
  当 通过任一路径运行 item、state、普通追加与旧 shell 追加,再删除硬链接后重试
  那么 有硬链接时都退出 2 且板字节不变,旧 shell 不在硬链接旁建锁;删除后照常工作

场景: 旧 shell 把以 `-` 开头的板名当作路径
  测试: olp_board_legacy_shell_treats_option_like_names_as_paths
  Level: integration
  Test Double: temporary boards named `-x` in two directories only
  Targets: olp-board-append.sh opted-in guard
  假设 一块名为 `-x` 的已 opt-in 板,以及另一目录下指向它、同样名为 `-x` 的硬链接
  当 在各自目录里以相对名 `-x` 调用旧 shell,追加事件行与普通文字
  那么 有硬链接时两边都以硬链接为由退出 2 且板字节不变;删去硬链接后事件行仍被拒绝,普通文字照常追加

场景: 等锁期间出现的硬链接在取得锁后被发现
  测试: olp_board_writers_recheck_links_after_taking_the_lock
  Level: integration
  Test Double: test-process wrappers only: the lock step of the production module, and a `flock` shim first on PATH for the legacy shell, each add a second name for the board right as the lock is taken
  Targets: olp-board-append.py read_snapshot/run/append_generated, olp-board-append.sh opted-in guard
  假设 读者、普通追加、事件写入和旧 shell 已通过路径检查
  当 它们等锁期间板多出一个名字,随后取得锁
  那么 四者都拒绝并指明硬链接,写入口报 may_have_appended=false,板字节不变

场景: 正式事件正文里引用的触发行不出卡
  测试: olp_board_harvest_ignores_triggers_inside_event_sources
  Level: integration
  Test Double: temporary boards only; the harvest script and event tool are not replaced
  Targets: olp-evo-harvest.sh candidate merge
  假设 一条正式 item 的正文引用了 `ACK(blocked): ...`、被 NUL 拆开的同类行和一行签名 R2 记档
  当 运行 state 与真实 harvest dry-run,再在正文之外追加一条手写 blocked ACK 后各运行一次
  那么 第一次 state 干净可派发、harvest 不出卡;第二次 state 报 unpaired_ack、harvest 恰为那一行出一张卡

场景: 板路径含行分隔符时结构化采集明确失败
  测试: olp_board_harvest_refuses_board_paths_with_row_separators
  Level: integration
  Test Double: temporary boards whose paths contain `|`, an inner newline or a trailing newline
  Targets: olp-evo-harvest.sh structured path
  假设 路径含 `|`、中间含换行或以换行结尾的三块有 blocked ACK 的结构化板
  当 分别运行真实 harvest dry-run
  那么 都非零退出,stderr 只有一个物理行的 error 且没有 traceback,stdout 不输出卡片

场景: 写入口的参数错误只得到一个机器错误
  测试: olp_board_write_clis_report_argument_errors_as_one_machine_json
  Level: integration
  Test Double: temporary boards only
  Targets: olp-board-event.py and olp-board-append.py argument parsing
  假设 两个写入口
  当 传入非法数值、非法选项值、缺少必需参数或未知子命令
  那么 每次都退出 2,stderr 恰为一个 may_have_appended=false 的 JSON 对象,板字节不变

场景: 采集只读一次快照
  测试: olp_board_harvest_reads_one_snapshot_per_run
  Level: integration
  Test Double: an event-tool wrapper that forwards every call to the real tool and, once, appends a real ACK right after its second call returns
  Targets: olp-evo-harvest.sh structured path and olp-board-event.py snapshot
  假设 编号为 R8-1 的 item 已被 receive,其 blocked ACK 恰在采集取得快照之后追加
  当 连续运行两轮真实 harvest dry-run
  那么 第一轮不为该 ACK 出卡,第二轮以含 #R8-1#blocked# 的结构化 identity 出卡,两轮合计只有一个 identity

场景: 编号含竖线时采集行保持完整
  测试: olp_board_harvest_escapes_pipes_in_item_numbers
  假设 编号为 A|1 与 A|2 的两个 blocked ACK
  当 以真实 harvest 执行 dry-run
  那么 恰有两张卡,identity 分别含 #A¦1#blocked# 与 #A¦2#blocked#,每个 envelope 的 line、offset、ts 三个字段均可解析

场景: append 回执不受后续追加影响
  测试: olp_board_append_receipt_survives_later_appends
  假设 append CLI 已返回 verified 回执
  当 旧 shell 再追加正文后按原 offset 与 ts 运行 verify
  那么 原固定区间仍验证成功

场景: 短写与追加后故障如实报告
  测试: olp_board_test_process_faults_report_partial_write_truthfully
  假设 测试进程在生产 append 函数的 write、fsync 或 readback 系统边界注入故障
  当 发生可恢复短写、部分写后错误、fsync 错误或回读错误
  那么 短写被 write_all 完成,其余错误均报告 may_have_appended 为 true

场景: event 写入口故障只输出机器错误
  测试: olp_board_event_cli_reports_write_boundary_failures_as_json
  Level: integration
  Test Double: OS and output boundaries replaced inside the test process only
  Targets: olp-board-event.py main write and receipt paths
  假设 测试进程在 event CLI 的 write、fsync、readback 或回执输出边界注入故障
  当 运行真实 event main
  那么 每条路径退出 2 且 stderr 恰为一个 verified=false、may_have_appended=true 的 JSON,写前拒绝则为 false

场景: 精确恢复 item 与 ACK 漂移并保留审计证据(critical)
  标签: critical
  测试: olp_board_recovery_clears_item_and_ack_drift_with_audit_evidence
  假设 opt-in 后各有一条未配对 item 与 ACK 规范行
  当 ACK CLI 与通用 record 分别用 recovery 字节证据补录正式事件
  那么 未解决 drift 清空,旧字节不变且 recovery_evidence 保留两个事件 ID 与证据

场景: 恢复证据不可复用且语义必须匹配
  测试: olp_board_recovery_rejects_duplicate_and_wrong_semantics_without_writes
  假设 一条已消费 ACK 和一条 outcome 不同的未配对 ACK
  当 再次引用前者或用错误 outcome 引用后者
  那么 两次均退出 2 且 head 与板字节不变

场景: 恢复只接受 opt-in 后未配对的精确区间
  测试: olp_board_recovery_rejects_pre_opt_in_bad_hash_and_paired_sources
  假设 opt-in 前文字、坏 hash/区间和已配对 source
  当 item CLI 尝试用它们作为 recovery
  那么 全部退出 2 且零写入

场景: 围栏示例与引用标题不可恢复,篡改后被隔离
  测试: olp_board_recovery_rejects_non_normative_examples
  Level: integration
  Test Double: temporary boards only; production item, ack, and state CLIs are not replaced
  Targets: olp-board-event.py record and replay paths through the shared normative projection
  假设 opt-in 后围栏中的 item/ACK 示例、引用中的 item 标题以及一条可合法恢复的同语义规范行
  当 item/ack CLI 直接引用示例,或把合法事件的 recovery 篡改为示例后运行 state
  那么 直接引用退出 2 且板字节不变,被篡改的事件以 malformed_event 进入 DRIFT 且调度阻塞

场景: 已恢复 blocked/wontdo ACK 只产一张采集卡
  测试: olp_board_harvest_deduplicates_recovered_acks
  Level: integration
  Test Double: temporary boards and absent optional MCP source only; production harvest is not replaced
  Targets: olp-evo-harvest.sh combined legacy and structured collection
  假设 blocked 或 wontdo 旧文字 ACK 已由正式事件 recovery 且另有无关 legacy ACK、签名 R2 与围栏示例
  当 真实 harvest 以 dry-run 读取 structured/mixed 板
  那么 每个 outcome 恰有三张卡,恢复对只占一张且无关 legacy 与签名触发保留

场景: 恢复前已采集的手写 ACK 恢复后不重复落卡
  测试: olp_board_harvest_keeps_card_identity_after_recovery
  Level: integration
  Test Double: temporary boards, state directory and an absent MCP source only; the harvest script commits for real
  Targets: olp-evo-harvest.sh structured identity for ACK events that carry recovery evidence
  假设 opt-in 后一条手写 `ACK(blocked)` 已被一次真实 harvest 落卡
  当 正式 ACK 以 recovery 补录该行后再次运行真实 harvest
  那么 进化黑板仍只有一张卡

场景: 生产 CLI 不含故障注入后门
  测试: olp_board_public_tools_contain_no_production_fault_switches
  假设 四个公开 Python CLI
  当 测试检查生产入口
  那么 不存在公开测试故障环境变量或 fault-inject 开关

场景: structured init 安装后可运行且重复执行不覆盖
  测试: olp_board_init_installs_structured_mode_idempotently
  Level: integration
  Test Double: temporary HOME and git repository, plus no-op octoscode/octos stubs first on PATH so the dependency check does not depend on the machine; installed production scripts execute the lifecycle
  Targets: olp-init.sh and the four installed olp-board Python entry points
  假设 临时 HOME 与新建 git 项目没有 OLP 文件
  当 以 `OLP_BOARD_MODE=structured` 运行真实 init,用安装后的四个工具完成 item→receive→ack→review 并再次运行 init
  那么 模板带 opt-in 标记且无裸 ACK 占位,独立普通锁存在并被 git 忽略,生命周期完成且项目文件、锁与本地工具副本均不被覆盖

场景: 默认 legacy 且无 Python 时旧工具仍可用
  测试: olp_board_init_preserves_legacy_fallback_without_python
  Level: integration
  Test Double: temporary HOME/repositories, no-op octoscode/octos stubs first on PATH, and a curated executable PATH without Python
  Targets: olp-init.sh legacy template and shell-tool installation paths
  假设 一个默认环境和一个 PATH 不含 Python 的临时项目
  当 分别运行真实 init
  那么 默认 loop 保留旧 ACK 契约,无 Python 运行明确报告结构化能力不可用且仍安装旧 watcher 与 append shell
