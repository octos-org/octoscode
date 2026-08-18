# 代码审查:`/loop list` 全局查询修复(d466c29)

**审查对象**:`fix/loop-list-global-query` 分支,commit `d466c29`
**审查日期**:2026-08-04
**结论**:**不通过** —— 闸门 1 修复正确;闸门 2 移除了客户端阻塞,但发出的载荷会被服务端过滤为空,用户仍然看不到 loop。

---

## 一、被审查的主张

提交信息声称:

> `/loop list` 被两道闸门挡住:
> 1. registry 层:`SessionRequirement::Open` → 没 session 直接判定命令不可用
> 2. dispatch 层:`active_autonomy_session_id()?` → 没 session 直接返回 None
>
> 两道都修了……Now `/loop list` is a global query that works with or without an
> active session, **showing all loop IDs as intended**.

以下逐条核对最后一句是否成立。

---

## 二、闸门 1(registry):✅ 修复正确

`CommandAvailability::app_ui_read` 的默认值确实是 `SessionRequirement::Open`:

```rust
// src/menu/availability.rs:59-68
pub fn app_ui_read(required_methods: &'static [&'static str]) -> Self {
    Self {
        runtime: RuntimeRequirement::Protocol,
        connection: ConnectionRequirement::Connected,
        session: SessionRequirement::Open,   // ← 默认要求已开会话
        ...
    }
}
```

改为 `.with_session(SessionRequirement::Any)` 是真实有效的:无会话时 `/loop`
不再从 `/` 命令弹窗中消失。**这一条成立,应予保留。**

---

## 三、闸门 2(dispatch):❌ 载荷不符合服务端契约

### 服务端的"查询全部"契约

`crates/octos-cli/src/api/agent_orchestrator.rs:5198`(octos 仓库):

```rust
fn list_loops(&self, request: LoopListRequest) -> Result<Value, RpcError> {
    let loops = state.loops.values()
        .filter(|rec| rec.status != "deleted")
        .filter(|rec| rec.profile_id == request.profile_id)          // ← 过滤 A
        .filter(|rec| {
            request.session_id.as_ref().is_none_or(|session_id| {    // ← 过滤 B
                session_controls_target(session_id, &rec.session_id)
            })
        })
        ...
}
```

过滤 B 的语义是明确的:

| 传入 | `is_none_or` 结果 | 效果 |
|---|---|---|
| `session_id: None` | 短路为 `true` | **不过滤 → 返回全部 loop** ✅ 这才是"全局查询" |
| `session_id: Some(SessionKey(""))` | 进入闭包比较 | 见下 |

### 本次修复实际发出的是空字符串,不是 `None`

```rust
// src/store.rs,本次改动
let session_id = self
    .active_session()
    .map(|session| session.id.clone())
    .unwrap_or_else(|| SessionKey("".into()));   // ← 空串,不是 None
```

而参数结构体的字段是**非 Option 且无 skip 属性**:

```rust
// src/model.rs:554
pub struct LoopListParams {
    pub session_id: SessionKey,                                 // 总是被序列化
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
}
```

因此线上载荷是 `"session_id": ""`,服务端解析为 `Some(SessionKey(""))`,进入过滤 B 的闭包:

```rust
// agent_orchestrator.rs:6057
fn session_controls_target(requested: &SessionKey, target: &SessionKey) -> bool {
    requested == target || requested.base_key() == target.base_key()
}
```

以真实数据代入(用户的 loop 位于 `kimi:local:tui#coding`):

- `"" == "kimi:local:tui#coding"` → `false`
- `"".base_key()`(= `""`)vs `"kimi:local:tui"` → `false`

**每个 loop 都被滤掉,返回空列表。** 用户看到的结果与修复前一致,只是换了条代码路径。

### 第二重失败:profile 也会解析错

`crates/octos-cli/src/api/ui_protocol.rs:14468`:

```rust
fn resolve_autonomy_profile_id(session_id: Option<&SessionKey>, ...) -> Result<String, RpcError> {
    ...
    if let Some(session_id) = session_id {          // Some("") 命中此分支
        return Ok(validate_session_scope(...)?
            .or_else(|| session_id.profile_id().map(ToOwned::to_owned))  // "".profile_id() → None
            .or_else(|| connection_profile_id.map(ToOwned::to_owned))
            .unwrap_or_else(|| MAIN_PROFILE_ID.to_owned()));             // 兜底 "main"
    }
```

`SessionKey("")` 解析不出 profile(`split_base_key().0` 对空串返回 `None`),
最终兜底为 `MAIN_PROFILE_ID`(`"main"`)。而用户的 loop 属于 `kimi` profile,
**过滤 A(`rec.profile_id == request.profile_id`)会再清零一次**。

即便过滤 B 被绕过,过滤 A 依然会返回空。

---

## 四、其他问题

### 1. `unreachable!()` 位于生产路径

```rust
LoopCommand::List => {
    unreachable!("LoopCommand::List handled above");
}
```

当前逻辑上确实到不了(前置 `matches!` 早返回保证)。但这是**重构地雷**:
将来任何人调整早返回条件,这里就从"死代码"变成"panic"。TUI 中的 panic 会直接
终止用户会话。建议改为返回 `None`,或降级为 `debug_assert!` + 优雅返回。

### 2. 测试恰好绕开了缺陷所在

```rust
assert!(matches!(
    store.compose_command(),
    Some(AppUiCommand::ListLoops(_))    // ← 只断言"发出了某个命令"
));
```

缺陷正在 payload 内部(`session_id` 是空串还是 `None`),而断言用 `_` 忽略了
payload。这个测试无法捕获本文指出的任一问题;若补上 `params.session_id` 的
断言,反而会把错误行为**固化为预期**。

测试还有两处次要问题:
- `store.state.sessions.clear()` 后设 `selected_session = 0`,依赖
  `active_session()` 的越界返回 `None`(当前实现安全,但属隐式依赖)。
- 它验证的是客户端是否发出请求,而非用户能否看到 loop —— 与合约意图脱节。

### 3. 根因归属存疑

用户报告的现象发生在**存在活跃会话**时:截图显示状态栏 `101 msgs`,且
`Loop list refreshed: 2 loop(s)` 正常返回。因此"无会话导致命令被拒"很可能
不是用户当次的病因。本修复触及的是一个**真实但不同**的潜在缺陷。

用户当次看不到 loop 的实际原因(据服务端日志)是:
```
2026-08-02T20:49:45Z INFO solo boot: restored loop parked as paused loop_id=loop_01 session_id=kimi:local:tui#coding
2026-08-02T21:02:10Z INFO solo boot: restored loop parked as paused loop_id=loop_02 session_id=kimi:local:tui#coding
```
即 server 反复重启导致 loop 被 park 为 paused,随后旧实例的 loop 记录消失。

---

## 五、建议的正确改法

让参数结构体表达真正的"可选",使无会话时发出 `None` 而非空串:

```rust
// src/model.rs
pub struct LoopListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
}
```

```rust
// src/store.rs — dispatch 处
return Some(AppUiCommand::ListLoops(LoopListParams {
    session_id: self.active_session().map(|s| s.id.clone()),   // 无会话 → None
    profile_id: self.active_session_profile_id(),               // 显式带上,勿让服务端兜底 main
}));
```

两点缺一不可:
1. `session_id: None` 才能命中服务端 `is_none_or` 的短路,跳过过滤 B。
2. `profile_id` 必须显式传递,否则服务端兜底为 `main`,过滤 A 依然清零。

配套调整:
- 移除 `unreachable!()`,改为优雅返回。
- 测试断言 payload 形状(无会话时 `session_id` 为 `None`、`profile_id` 为 `Some`),
  而非仅断言命令类型。
- 检查 `LoopListResult.session_id` 的解码侧是否也需容忍缺省。

---

## 六、审查方法说明

本审查未采信提交信息的自述,全部结论来自源码与运行时证据:

| 结论 | 依据 |
|---|---|
| `app_ui_read` 默认 `Open` | `src/menu/availability.rs:63` |
| 服务端以 `None` 表示"全部" | `agent_orchestrator.rs:5205` 的 `is_none_or` |
| 空串会被过滤掉 | `session_controls_target()` @ `agent_orchestrator.rs:6057` |
| 空串导致 profile 兜底 `main` | `resolve_autonomy_profile_id()` @ `ui_protocol.rs:14476-14484` |
| 参数字段总是被序列化 | `src/model.rs:554`(无 `skip_serializing_if`) |
| 用户当次故障的真实原因 | `~/.octos/logs/serve.2026-08-02.log` 的 solo boot 记录 |

测试状态:`cargo test --lib` 1685 通过 —— 但如上所述,现有测试无法覆盖本文
指出的缺陷,绿灯不构成该修复有效的证据。


---

# 第二轮审查:`1f6f8d4`(2026-08-04)

**结论**:**仍不通过** —— 请求侧已按建议改对,但**回包解码会失败**,用户依然
看不到列表;另有 profile 兜底问题未处理。

## 一、已修正的部分 ✅

| 上轮问题 | 本次处理 | 判定 |
|---|---|---|
| 发送空串而非 `None` | `LoopListParams.session_id` 改为 `Option<SessionKey>` + `skip_serializing_if` | ✅ 正确 |
| dispatch 处构造空串 | 改为 `self.active_session().map(\|s\| s.id.clone())` | ✅ 正确 |
| `unreachable!()` 在生产路径 | 改为返回 `None` | ✅ 正确 |
| 测试忽略 payload | 补上 `params.session_id.is_none()` 断言 | ✅ 正确 |
| 其他调用点(hydrate 路径) | 同步改为 `Some(session_id.clone())` | ✅ 未遗漏 |

请求侧的契约现在与服务端一致。

## 二、新发现的阻断性缺陷:回包无法解码 ❌

上轮审查末尾曾提示「检查 `LoopListResult.session_id` 的解码侧是否也需容忍缺省」,
该项未被处理。

服务端把请求里的 `session_id` **原样回显**(`agent_orchestrator.rs:5212`):

```rust
Ok(json!({
    "session_id": request.session_id,   // Option<SessionKey> → None 序列化为 null
    "profile_id": request.profile_id,
    "loops": loops
}))
```

而 TUI 的结果结构体仍是非 Option(`src/model.rs:561`):

```rust
pub struct LoopListResult {
    pub session_id: SessionKey,   // ← 无 Option、无 default
    #[serde(default)]
    pub loops: Vec<UiLoopRecord>,
}
```

**实证**(以服务端真实回包形状喂给解码器):

```rust
let wire = json!({"session_id": null, "profile_id": "kimi", "loops": []});
serde_json::from_value::<LoopListResult>(wire)
// → DECODE FAILED: invalid type: null, expected a string
```

因此全局查询(无会话)的完整链路是:

1. 客户端正确发出 `session_id` 缺省 ✅
2. 服务端正确返回全部 loop ✅
3. **客户端解码回包失败 ❌ → 列表永远为空**

这与 2026-08-03 的 `session/hydrate: missing field payload` 属同一类故障:
请求方向修好了,响应方向没跟上。

### 修法

```rust
pub struct LoopListResult {
    #[serde(default)]
    pub session_id: Option<SessionKey>,
    #[serde(default)]
    pub loops: Vec<octos_core::ui_protocol::UiLoopRecord>,
}
```

并同步处理消费点 `apply_autonomy_result` 中的
`set_session_loops(&result.session_id, result.loops)` —— 当 `session_id` 为
`None` 时,这些 loop 属于「全局查询结果」,不能塞进某个会话的镜像;需决定是
仅渲染到转录(推荐),还是按每条 loop 自身的 `record.session_id` 分发入镜像。

## 三、未处理的遗留问题:profile 仍会兜底成 `main` ⚠️

上轮审查明确指出「`profile_id` 必须显式传递,否则服务端兜底为 `main`」。
本次仍使用:

```rust
let profile_id = self.active_session_profile_id();
// src/store.rs:2368
fn active_session_profile_id(&self) -> Option<String> {
    self.active_session().and_then(|session| session.profile_id.clone())
}
```

**无会话时该函数必然返回 `None`** —— 正是全局查询的场景。此时服务端
`resolve_autonomy_profile_id(None, None, connection_profile_id)` 走连接兜底,
若连接未携带 profile 则落到 `MAIN_PROFILE_ID`(`"main"`),而用户的 loop 属于
`kimi` profile,会被过滤 A(`rec.profile_id == request.profile_id`)清零。

是否实际触发,取决于 stdio 连接是否携带 profile。建议:
- 无会话时改用启动 profile(`cli.profile_id` / `onboarding.launch_profile_id`)
  作为回退,而不是直接放弃;
- 或在文档/合约中明确「全局查询以连接 profile 为准」,并验证 stdio 连接确实
  携带了它。

## 四、测试为何仍未拦住

`loop_list_works_without_active_session` 只覆盖**请求出站方向**。本轮两个问题
分别位于**响应入站方向**与**服务端过滤语义**,单元测试的边界之外。

建议补一条契约测试,直接以服务端真实回包形状驱动解码:

```rust
#[test]
fn loop_list_result_decodes_a_global_query_response() {
    let wire = serde_json::json!({
        "session_id": serde_json::Value::Null,
        "profile_id": "kimi",
        "loops": []
    });
    serde_json::from_value::<LoopListResult>(wire)
        .expect("server echoes null session_id for a global query");
}
```

## 五、本轮审查依据

| 结论 | 依据 |
|---|---|
| 请求侧已改对 | `git show 1f6f8d4 -- src/model.rs src/store.rs` |
| 服务端回显 `session_id` | `agent_orchestrator.rs:5212` |
| 结果结构体仍非 Option | `src/model.rs:561` |
| 解码确实失败 | 以真实回包形状运行解码器,得 `invalid type: null, expected a string` |
| 无会话时 profile 为 None | `src/store.rs:2368` `active_session_profile_id` |

测试状态:`cargo test --lib` 1685 通过 —— 与上轮同理,绿灯未覆盖上述路径,
不构成修复有效的证据。
