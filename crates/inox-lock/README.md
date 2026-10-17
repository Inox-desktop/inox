# inox-lock

会话锁屏。属于 Inox 桌面环境的 **L3** 层（[ADR 0018](../../docs/adr/0018-l3-component-architecture.md)）。

本 crate 的规范是 [ADR 0015](../../docs/adr/0015-session-lock-and-failure-boundaries.md)，它比 PRD 更严格，并直接决定了两件事：

- **锁屏不是遮罩。** ADR 0015 明确拒绝「顶层全屏窗口或 layer-shell 遮罩当锁屏」，因为两者都不能证明输入与其他层的隔离、也不能证明 UI 崩溃后会话仍被遮蔽。锁状态由合成器所有，`inox-lock` 是 `ext-session-lock-v1` 的可信客户端。因此本 crate **没有摆放策略**：没有锚点、没有 exclusive zone、没有层表面尺寸——没有层表面可摆。
- **认证是人审项，不是本里程碑的产物。** PAM 或其他认证机制、helper 权限、凭据传递与清理都是 ADR 0015 要求人类签字后才落地的条目，Inox 不得自研口令哈希/校验；**测试桩不算认证**。

## 状态

**已实现**（证据等级 E0/E1：`fmt`、`clippy -D warnings`、`cargo doc -D warnings`、`cargo test` 全绿；纯逻辑有单元测试与属性测试）：

| 模块 | 内容 | 验收 |
|------|------|------|
| `state` | ADR 0015 的锁状态机：`Unlocked / Locking / Locked / Covering / LockedWithoutUi / Unlocking`，输出覆盖门（`Locked → Covering → Locked`）、UI 崩溃与认证失败保持锁定、**`unlock()` 是通往 `Unlocked` 的唯一门**（`LockEvent` 里根本没有成功变体）。两条 ≥1000 样本的属性测试 | S02、S03 |
| `attempts` | 失败限流：滑动窗口内的有界尝试、等待上限正好一个窗口（不会永久锁死合法用户）、内部计时不出现在任何面向用户的文案里；边界与拒绝路径有测试 | S03 |
| `credential` | 凭据缓冲：基于 `zeroize`，`clear` 就地写零而非缩短长度，`Debug`/`Display` 只打印 `<redacted>`，退格按**字符**而非字节并清零尾部，不可 `Clone`、不可 `PartialEq` | S03 |
| `suspend` | 挂起顺序：先锁定 → 收到**当前请求**的锁完成确认 → 才允许挂起；确认带代次，旧代次的确认不授权；失败按 `LockFailure::{Refuse, Cancel}` 明确策略处理，确认超时是有界策略数据 | S03 |
| `contract` | ADR 0013 的边界词汇：请求代次、`Sent/Accepted/Answered/Unavailable`、答复只命中「在途代次」（更早的、从未发出的、以及已被答复的那一次都被丢弃）；`LockProtocol` 与 `ProtocolAbsence` 把「合成器没有这个协议」建模成值而不是错误 | D05 |
| `view_model`、`view`、`theme`、`i18n` | 状态到画面的纯投影（提示、验证中、失败与原因、限流等待、无可信 UI、不可用 + 重试；标题跟着画面走，画不出密码输入时不出现提示，也不出现「按回车解锁」）、Iced 组件树、令牌到 `iced::Theme` 的映射、Fluent 目录 | D05、Q01、Q02、Q03 |
| `app` | Elm 循环：`State`/`Message`/`update`，键入、提交、限流、挂起编排、按键绑定；凭据以**移动**而非拷贝的方式离开状态进入待发命令 | D05 |
| `render` | 界面真正落到像素上：`UserInterface` + tiny-skia 把 `app::view` 画到 configure 尺寸，写成 `wl_shm` 的 `Argb8888`（ADR 0025） | S02、Q02 |
| `session` | `ext-session-lock-v1` 客户端：认领角色、取锁、每个输出一块锁表面、按 configure 提交渲染帧、输出消失时显式销毁表面、信号不解锁 | S02、S03 |

**未实现 / 被阻塞**（不声称上表之外的任何等级）：

- **没有认证，因此没有解锁路径。** 没有 PAM 调用、没有 helper、没有特权进程、没有任何口令校验。`contract::AuthOutcome` 是答复将到达的形状；谁有权产生它，就是 ADR 0015 点名的那项评审。`--take-lock` 因此**不能释放**会话——释放靠注销，收到 `SIGTERM`/`SIGINT` 时退出而**不发 `unlock_and_destroy`**（合成器保持锁定）。默认（不带该开关）行为不变：以非零状态码退出并说明 ADR 0015 要求的审查尚未完成。见 `tests/protocol.rs` 与 `tests/cli.rs`。
- **界面在真实表面上只呈现它诚实的那个状态。** `--take-lock` 现在把 `app::view` 真正画到每个锁表面（ADR 0025），但本构建没有认证边界，状态里的事实就是 `ServiceFailure::NotProvided`，所以屏幕上出现的是不可用画面：它说这里没法校验任何东西，而不是请人输密码。锁表面的内容由 `tests/protocol.rs` 读回并断言。**键盘与指针都没有接**：屏幕上没有可操作的东西，输入路径会是不可能执行的代码，它与经过评审的认证边界一起落地。
- **没有语义树。** iced 0.14 没有 AT-SPI 桥，Q01 要求的 role/name/state/action 导出被该原型阻塞（与其余 L3 组件相同）。
- **凭据字段不是文本框。** 掩码由凭据的**长度**推出，按键以消息形式到达；密码明文因此从不进入控件树（也就没有截图可脱敏的地方）。代价是没有光标、没有选区、没有 IME、字段内没有复制粘贴——这些要连同焦点处理与可访问角色一起，在有了经过评审的认证边界之后再决定。

## 目录结构

```
src/lib.rs            crate 文档、模块声明与再导出
src/render.rs         离屏渲染：控件树 → configure 尺寸 → wl_shm 字节（ADR 0025）
src/session.rs        协议腿：取锁、覆盖每个输出、提交渲染帧、信号不解锁（ADR 0022 决策 6）
src/state.rs          锁状态机与不变量（纯）
src/attempts.rs       失败限流（纯）
src/credential.rs     会自我清零的凭据缓冲（纯）
src/suspend.rs        挂起前的锁定顺序（纯）
src/contract.rs       ADR 0013 的边界词汇
src/view_model.rs     状态 → 画面（纯投影）
src/app.rs            Elm：State / Message / update / subscription
src/view.rs           Iced 组件树
src/theme.rs          令牌 → iced::Theme 与样式闭包
src/i18n.rs           Fluent 目录装配
src/main.rs           日志、配置、目录；默认说明并退出，--take-lock 运行协议腿
i18n/{en-US,zh-CN}/lock.ftl
tests/{catalog.rs,cli.rs,protocol.rs,render.rs,support/mod.rs}
```

## 测试

```sh
cargo test -p inox-lock --all-features
```

- 单元测试：每条纯规则各一条，**含拒绝与错误路径**（重复覆盖、未知输出、重复锁定、无挂起在途却收到完成确认、陈旧答复、从未发出的代次、已被答复后重放的消息、超限尝试、退格与清零；取消、UI 退出与挂起都清零凭据）。
- 属性测试（每项 ≥1000 样本）：任意事件序列下「除认证成功外永不进入 `Unlocked`」，以及「输出覆盖簿记与独立模型一致、报锁定必然跟随覆盖」。
- `tests/catalog.rs`：`en-US` 与 `zh-CN` 的 id 与占位符一致；代码里出现的每个 `lock-…` id 都在目录里，目录里每个 id 都被代码用到。
- `tests/cli.rs`：真实子进程的默认路径——不带参数时非零退出、文案含 ADR 0015 与 `--take-lock`、**不含**密码提示；`--help` 不需要显示。
- `tests/protocol.rs`：手写的 `ext-session-lock-v1`/`wl_shm`/`inox_ipc` 服务端 peer 驱动真实二进制，除了认领、遮蔽与「信号不解锁」，还**读回客户端交出的帧**：字节数等于 configure 尺寸的 4 倍、每个 alpha 为 `0xff`、像素不止一种颜色、背景占多数。
- `tests/render.rs`：1280×720 的深色、浅色、已输入、限流、失败、无可信 UI、协议缺失、认证边界不可用（可重试）与 200% 字号等场景，用**真实的 Iced 组件树**经 `render::Interface`（二进制提交帧走的同一段代码）渲染后断言像素；一个交互测试用像素定位解锁按钮并断言一次点击只产生一次提交。

设置 `INOX_SCREENSHOT_DIR` 时，每个场景会额外落盘为 `inox-lock-<场景>.png`，供 CI 收集为工件（验收 Q02）。

## 开发约定

- 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径。**
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 用 `lock`。

## 许可证

BSD 3-Clause，见仓库根的 [`LICENSE`](../../LICENSE)。
