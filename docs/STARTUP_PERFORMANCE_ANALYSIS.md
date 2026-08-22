# octoscode 启动性能分析

## 启动流程（src/main.rs）

```
main()
  ├─ color_eyre::install()              <1ms
  ├─ install_terminal_restoring_panic_hook()  <1ms
  ├─ cmd::dispatch()                    <1ms（拦截 update/doctor）
  ├─ Cli::parse()                       <1ms
  ├─ backend_ensure::ensure_octos_backend()  可能慢（下载 octos server）
  ├─ splash::play()                     明显瓶颈（2.15-4.15s）
  └─ event_loop::run()                  取决于 backend 连接速度
```

## 测量结果

### 代码分析

**splash 动画时长**（`src/splash.rs`）：

```rust
pub const SPLASH_EFFECTS: [&[&str]; 9] = [
    &["beams"],     // 3.5s
    &["sweep"],     // 3.7s
    &["wipe"],      // 2.3s
    &["rain"],      // 2.8s
    &["slide"],     // 1.9s
    &["scattered"], // 1.8s
    &["middleout"], // 1.7s
    &["highlight"], // 2.1s
    &["matrix", ...], // ~3s release / ~5s debug
];

pub const SPLASH_HOLD: Duration = Duration::from_millis(450);
```

- 动画时长：1.7-3.7s（取决于效果）
- Hold：450ms
- **总计**：2.15-4.15s

**backend_ensure 下载**（`src/backend_ensure.rs`）：

- 如果 octos server 不在 PATH 或版本不匹配，会从 GitHub 下载（慢，取决于网络）
- 有 fast path：PATH 上可用就直接返回，不探测 legacy dir
- 有 opt-out：`OCTOSCODE_NO_AUTO_INSTALL=1` 跳过下载

## 瓶颈排序

1. **splash 动画**——2.15-4.15s，明显瓶颈
2. **backend_ensure 下载**——如果需要下载，取决于网络（可能几十秒）
3. **event_loop::run()**——TUI 启动，取决于 backend 连接速度

## 优化建议（按优先级）

### 1. splash 动画（优先级：高）

**问题**：2.15-4.15s 的动画时长对启动速度影响明显。

**方案**：
- **方案 A**：默认跳过 splash（`--no-splash` 变成默认，需要 `--splash` 才播放）
- **方案 B**：缩短动画时长（比如只播 1s，或去掉 450ms hold）
- **方案 C**：第一次启动时播放，后续启动跳过（需要持久化状态）

**推荐**：方案 A——默认跳过，需要时手动开启。

### 2. backend_ensure 下载（优先级：中）

**问题**：如果 octos server 不在 PATH 或版本不匹配，会从 GitHub 下载（慢）。

**现状**：已有 fast path（PATH 上可用就直接返回）和 opt-out（`OCTOSCODE_NO_AUTO_INSTALL=1`）。

**方案**：
- 缓存版本检查结果（比如 1 小时内不重复检查）
- 后台下载（不阻塞启动，下载完成后再提示重启）

**推荐**：暂不改——fast path 已经足够快，下载是必要的一次性操作。

### 3. event_loop::run()（优先级：低）

**问题**：TUI 启动，取决于 backend 连接速度。

**方案**：
- 本地 stdio 连接应该很快（<100ms）
- WebSocket 连接可能慢（取决于网络）

**推荐**：暂不改——本地连接已经足够快。

## 结论

**最大瓶颈是 splash 动画**（2.15-4.15s）。建议默认跳过 splash，需要时手动开启（`--splash`）。

**实施**：
1. 把 `--no-splash` 改成 `--splash`（默认不播放）
2. 更新 spec 和文档
3. 更新测试

**预期效果**：启动时间从 ~5s 降到 ~2s（mock 模式）或更短（protocol 模式）。
