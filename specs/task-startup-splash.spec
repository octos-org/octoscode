spec: task
name: "启动动画：ttfx 渲染 OCTOS logo splash"
inherits: project
tags: [tui, startup, splash, ttfx, branding]
estimate: 1d
---

## 意图

octoscode 启动时（`backend_ensure` 之后、`event_loop::run` 接管终端之前）在主屏播放
一段 OCTOS ASCII logo 动画，用 [ttfx](https://github.com/omacom-io/ttfx) 引擎的公开
原语（`Effect::build`/`next_frame` + `Terminal` 帧原语）在 octos-tui 侧自建帧循环渲染。
每次启动从精选效果列表随机抽一个并**自然播完**（精选成员以自然时长 ~1.7–4.5s 为准入
标准），按键可随时跳过，8000ms 仅作防挂安全网；结束时（无论跑完还是截断）在原地留下
完整 logo + 版本号作为 banner，自然跑完后停顿 450ms 再进入 TUI（按键可打断停顿）。
动画是纯装饰，任何失败都静默跳过，绝不阻断启动。

## 已定决策

- **依赖**：`Cargo.toml` 新增 `ttfx = { git = "https://github.com/omacom-io/ttfx", rev = "<pin>" }`
  （pin 到当前 main tip），`.cargo/config.toml.example` 补一段 patch 到本地
  `../consult/ttfx` 的示例——与 `octos-core` 的 git-dep + 本地 patch 模式一致。
  新增依赖的正当理由：本任务即为集成该引擎；ttfx 自身仅依赖 clap + terminal_size。
- **挂载点**：`src/main.rs` 中 `backend_ensure::ensure_octos_backend` 之后、
  `event_loop::run(cli)` 之前调用 `splash::play(&cli)`；`update`/`doctor` 在更早的
  `cmd::dispatch` 已退出，天然不播。
- **门控**：`splash::should_play(inputs) -> bool` 为纯函数（入参打包 no_splash 标志、
  `OCTOSCODE_NO_SPLASH` 环境变量、stdout `IsTerminal`、`CI` 环境变量、终端宽高与 logo
  尺寸），任一跳过条件命中即返回 false。CLI 新增 `--no-splash` 标志。
- **内容**：`OCTOS` figlet 风格 ASCII art（const 字符串，约 40 列宽）+ 尾行
  `octoscode v{CARGO_PKG_VERSION}`。
- **随机效果**：从精选列表 `SPLASH_EFFECTS`（ttfx CLI 参数列表形式，支持按效果调参）
  随机抽取；准入标准为在本 logo 输入上 60fps 自然时长 ~1.7–4.5s（虚拟时钟实测；如
  `decrypt` 12.4s 被淘汰）。现行成员：`beams`、`sweep`、`wipe`、`rain`、`slide`、
  `scattered`、`middleout`、`highlight`，以及墙钟驱动、经 `--rain-time 1` 等参数
  调短的 `matrix`（release 约 3s）。随机种子取 `SystemTime` 纳秒，选择函数
  `pick_effect_args(seed)` 可用固定种子单测。`OCTOSCODE_SPLASH_EFFECT=<name>` 可按名
  钉选精选条目（保留其调参；仅限精选列表，未知名回落随机——时长保证不被绕过）。
- **帧循环**：`run_splash(effect, ctx, out, should_stop)` 使用 ttfx 公开原语
  `prep_canvas` → 循环 { `should_stop()` 为真即中断；`next_frame` → `print_frame` →
  `enforce_framerate` } → `restore_cursor`；`out: &mut impl Write` 与 `Clock`（real /
  virtual）均可注入，测试用虚拟时钟 + `Vec<u8>` 收帧、不真实 sleep。生产路径的
  `should_stop` = 超过 8000ms 安全网 deadline ∨ `crossterm::event::poll(0)` 读到任意
  按键 ∨ 终端 resize；自然跑完后在终态 logo 上停顿 450ms（`SPLASH_HOLD`，按键打断）
  再返回。播放期间临时开 raw mode（防按键回显），RAII guard 保证异常路径也关闭
  raw mode 并恢复光标。
- **终态**：循环结束后统一原样打印完整 logo 文本（默认前景色），使截断与跑完的
  视觉终态一致；banner 留在 scrollback，与 inline scrollback 模型不冲突。
- **失败静默**：`splash::play` 返回 `()`，内部 `run` 的任何 `Err`（引擎错误、IO 错误、
  终端探测失败）都被吞掉，启动继续。
- **块居中（ANSI 免疫）**：`block_pad` 在 `SplashSession::new()` 里按**最终文本**宽度
  （`text_dimensions(text)`）一次算定并存字段；`paint()` 只用 `self.block_pad`、绝不
  按 frame 测宽——ttfx 帧携带 SGR 颜色序列，其可打印字符会把 `UnicodeWidthStr::width`
  撑到数百列、使 pad 坍缩为 0。块内窄行保持左对齐，对齐主 banner 的 figlet 居中
  （`{art:<fig_w$}` + `centered()`）。
- **平滑斜接（inline viewport 坐标系）**：不使用 `MoveTo(0,0)`——TUI 是 inline
  viewport，从**当前光标行**开始，不是屏幕第 0 行。`run()` 结束后光标**上移回画布
  顶行**（`\x1b[{rows-1}A\r`）而非 park 在画布下方，使 launch banner 的第一帧原地
  覆盖 splash 终态帧。
- **LOGO 与 banner 一致**：`LOGO` 常量与 `app::ONBOARDING_LOGO_ART` 运行时逐字节相等
  （`\` 行延续吃掉下一行全部前导空白的行为两边一致）；`splash_text()` 的版本行无硬编码
  缩进（居中由 `paint()` 统一处理，硬编码缩进会重复计算偏移）。

## 边界

### Allowed Changes
- Cargo.toml
- Cargo.lock
- .cargo/config.toml.example
- src/main.rs
- src/lib.rs
- src/splash.rs
- src/cli.rs
- src/transport.rs
- docs/ARCHITECTURE.md
- README.md
- tests/splash_contract.rs
- specs/**

### Forbidden
- 不改变 `event_loop` 接管终端的时序与方式；splash 不进入 alternate screen。
- splash 的任何失败不得使进程退出或延迟启动超出 deadline 本身。
- 除 `ttfx` 外不新增 crate 依赖。
- 不在本 crate 引入 unsafe（信号处理留在 ttfx 内部，本任务不调用其信号 API）。

## 完成条件

场景: 非 TTY 时自动跳过
  测试: should_play_false_when_stdout_not_tty
  假设 门控入参 stdout_is_tty 为 false 且其余条件均允许播放
  当 调用 should_play
  那么 返回 false

场景: --no-splash 与环境变量关闭
  测试: should_play_false_on_flag_or_env
  假设 门控入参 no_splash 标志为 true 或 OCTOSCODE_NO_SPLASH 已设置
  当 调用 should_play
  那么 返回 false

场景: CI 环境自动跳过
  测试: should_play_false_in_ci
  假设 门控入参 ci 为 true 且其余条件均允许播放
  当 调用 should_play
  那么 返回 false

场景: 终端过窄跳过
  测试: should_play_false_when_terminal_narrower_than_logo
  假设 终端宽度小于 logo 宽度
  当 调用 should_play
  那么 返回 false

场景: 条件齐备时播放
  测试: should_play_true_when_interactive_and_wide_enough
  假设 stdout 为 TTY、无关闭标志与环境变量、非 CI、终端宽度足够
  当 调用 should_play
  那么 返回 true

场景: 钉选效果仅解析精选名
  测试: effect_pin_resolves_curated_names_only
  假设 OCTOSCODE_SPLASH_EFFECT 语义由 effect_args_for 实现
  当 以 "matrix" 与非精选名 "decrypt" 分别查询
  那么 "matrix" 返回含调参的精选条目、"decrypt" 返回 None

场景: 随机效果取自精选列表
  测试: pick_effect_stays_in_curated_list
  假设 任意 64 个连续种子值
  当 逐一调用 pick_effect
  那么 每次返回的效果名都属于 SPLASH_EFFECTS 列表

场景: 精选效果虚拟时钟冒烟
  测试: curated_effects_produce_frames_on_virtual_clock
  假设 SPLASH_EFFECTS 中的每个效果与固定种子、虚拟时钟、Vec 写入器
  当 运行 run_splash 且 should_stop 恒为 false
  那么 每个效果产出至少 1 帧且 run_splash 返回 Ok

场景: --no-splash 标志可被 CLI 解析
  测试: cli_parses_no_splash_flag
  当 以 --no-splash 参数解析 Cli
  那么 解析结果的 no_splash 字段为 true

场景: 截断后终态为完整 logo
  测试: truncated_run_ends_with_full_logo
  假设 should_stop 在第 3 帧后返回 true
  当 运行 run_splash 并执行终态打印
  那么 写入器末尾内容包含完整 logo 文本与版本号行

场景: 引擎错误静默不阻断
  测试: play_swallows_engine_errors
  假设 以空输入文本构造 splash 使引擎构建失败
  当 调用内部 run 路径
  那么 返回错误被吞掉且函数正常返回、不 panic

场景: 画布按终端宽度块居中
  测试: paint_centers_block_to_terminal
  假设 10 列终端与一段 2 列宽、2 行高的文本
  当 调用 paint 绘制该文本
  那么 每一行在清行序列后带 4 个空格的左填充
  并且 窄于块宽的行在块内保持左对齐

场景: 居中偏移对 ANSI 色码免疫
  测试: paint_block_pad_is_ansi_immune
  假设 帧内容被 SGR 颜色序列包裹（模拟 ttfx 真实输出）
  当 调用 paint 绘制该帧
  那么 左填充仍为按最终文本宽度算出的 4 列而非 0

场景: run 结束时光标回到画布顶行
  测试: run_leaves_cursor_on_canvas_top_row
  假设 一个 2 行文本的 splash 会话跑到自然结束
  当 run 返回
  那么 输出尾部是上移一行加回车（\x1b[1A\r）而非换行 park
  并且 后续 inline viewport 的首帧可原地覆盖 splash 终态

<!-- lint-ack: decision-coverage — ttfx git 依赖由 cargo build 本身机械验证，无需场景 -->
<!-- lint-ack: precedence-fallback-coverage — 门控为无序 OR 组合而非优先级链，五个单条件场景已穷举 -->
<!-- lint-ack: boundary-entry-point — main.rs 挂载点时序（backend_ensure 之后、event_loop 之前）靠人工真机验证，无法在集成测试中机械绑定 -->
<!-- lint-ack: bdd-implementation-detail-step — 本任务交付物即终端渲染输出，断言写入器内容是行为本身 -->
