# AGENTS.md — inox-lock

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

会话锁屏。所属层级：**L3**（见根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

它是 ADR 0015 定下的**可信锁屏客户端**：锁状态由合成器所有，本 crate 拥有的是状态机、它的安全不变量、凭据卫生、限流策略、挂起前的锁定顺序，以及锁屏界面。它**不**拥有认证，也**不**拥有协议实现。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。`src/main.rs` 以 `std::process::exit` 结束，不是 panic。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- **只有 `state::LockMachine::unlock` 能把会话推向 `Unlocked`**，而且只接受 `AuthOutcome::Success`。`LockEvent` 里没有成功变体，这是刻意设计：任何人读类型就能核对这条规则。新增任何能改变锁阶段的代码路径前，先读 ADR 0015 与本模块文档。
- **来自认证边界的消息只对「在途代次」生效。** `app::update` 必须先问 `BoundaryState::is_current`，只有仍然在途的那一次尝试才接受 `AuthAccepted`/`AuthAnswered`；`is_stale` 只覆盖「更早的代次」，挡不住从未发出的代次与已被答复的那一次（重复/乱序消息会让界面永远停在验证中，或覆盖已记录的结果）。
- **不允许出现「看起来像锁屏」的东西。** ADR 0015 拒绝 layer-shell 遮罩方案，本 crate 因此没有 `placement` 模块、没有锚点/exclusive zone/表面尺寸；不得引入 `iced_layershell`，也不得让二进制在协议缺失时假装已经锁定。
- **认证不在本 crate 内。** 不得实现口令校验、不得调用 PAM、不得添加认证 helper 或任何特权路径——这些是 ADR 0015 的人类安全评审项。`contract::AuthOutcome` 只是答复的形状。
- **凭据不出安全边界。** 明文只能经 `credential::Secret::expose` 取出，其唯一调用方是发往认证边界的 `contract::AuthAttempt`；禁止把凭据写入日志、通知、剪贴板或截图，禁止给 `Secret` 加 `Clone`/`PartialEq`/`Deref`/`AsRef<str>`。凭据在**取消、UI 隐藏或退出、解锁与挂起**时清零（ADR 0015：UI 隐藏、取消、切换用户/认证器或挂起时清理敏感输入，不依赖动画结束）。改 `app::update` 时，新增任何会让会话离开屏幕的路径都必须先清零——`Message::SuspendRequested` 与 `Message::UiExited` 都是这么做的。
- **限流文案不含计时。** 「还剩几次」「还要等 37 秒」都是限流器的内部状态，不得出现在任何面向用户的字符串里；等待时长只属于 `attempts::AttemptLimiter`，供调度者使用。
- **用户可见文案只来自 Fluent**，`en-US` 与 `zh-CN` 同步；`tests/catalog.rs` 双向检查「代码提到的 id 都在目录里」与「目录里的 id 都被用到」，所以新增/删除文案必须两边一起改。
- **颜色、尺寸、间距、圆角只来自 `inox_config::Theme`。** `view.rs` 与 `theme.rs` 里没有**设计色与设计尺寸**的字面量（结构性哨兵如 `Color::TRANSPARENT`、边框宽 `0.0` 除外）；界面宽度用 `Length::FillPortion` 的比例（无量纲），字号四角色由 `spacing.lg × typography.scale` 派生，倍数为 1.5 / 1.25 / 1.0 / 0.875，hairline 为 `spacing.xs / 2`，焦点环为 `spacing.xs`（`docs/quality/ui-guidelines.md` §1–§2）。
- **掩码不是文本框。** 凭据字段画的是由**长度**推出的掩码，按键经 `app::key_binding` 以消息到达；不得把明文交给 iced 的 `text_input`（它持有本 crate 无法清零的 `String`，并提供复制粘贴）。
- 与工作区内其他 Inox crate 的依赖使用 **path 依赖**（根级 §8）。本 crate 依赖 L1：`inox-common`（i18n）、`inox-config`（设计令牌）；ADR 0022 决策 7 之后**另有一条向下的 L3 → L2 边**：`inox-protocols`（`ext-session-lock-v1` 与 `inox-ipc` 绑定）、`wayland-client` 与 `calloop`（协议腿的事件循环）、`rustix`（`memfd_create`，锁帧的共享内存）。**不得**新增 L3 依赖或依赖 `inox-compositor`（ADR 0013、ADR 0018 不变）；令牌映射与 Fluent 装配在每个 L3 组件里各有一份，这是 ADR 0018 明确接受的成本。
- 新增依赖前先确认依赖方向合法（根级 §2），否则 `cargo test -p inox-integration --test layering` 会失败。

- **协议腿没有解锁路径，这是刻意的。** `--take-lock` 取真实的会话锁、覆盖每个输出，然后一直保持；它**不得**发送 `unlock_and_destroy`，收到 `SIGTERM`/`SIGINT` 时也不得解锁——没有经过审查的认证边界，就没有任何东西可以构成一次解锁的凭据。凭据校验、提示与掩码字段都不在这条模式里：帧只是一层不透明底色。改动这条路径前先读 ADR 0022 决策 6 与 `src/session.rs` 的模块文档。
- **令牌只经 `INOX_LOCK_TOKEN` 进入本进程，且不得打印。** 它由 `inox-session` 生成、只投递给本组件与合成器；日志、错误信息与用户可见文案里都不得出现它的值。
- **协议错误不得被当成解锁。** 合成器发来 `finished` 表示拒绝或撤回：没有 `locked` 时它是拒绝（退出非零），有 `locked` 时是合成器自己释放；两种情况都不由本 crate 决定会话是否安全。

## 主要依赖

- 上游 Inox crate：`inox-common`（`i18n::Localizer`/`Locale`、`i18n::compare_catalogs`）、`inox-config`（`Theme` 令牌、`InoxConfig`/`ConfigPaths`）。
- 第三方：`iced` 0.14（`tiny-skia` 渲染器）、`zeroize`（凭据清零）、`thiserror`、`tracing`、`tracing-subscriber`、`anyhow`（仅 `main`）。
- 测试：`proptest`、`iced_core` / `iced_runtime` / `iced_tiny_skia`（无头渲染）、`image`（仅在设置 `INOX_SCREENSHOT_DIR` 时写 PNG 工件）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。以下四条即本 crate 的自检。

| 目的 | 命令 |
|------|------|
| 格式化 | `cargo fmt -p inox-lock` |
| 本 crate 测试 | `cargo test -p inox-lock --all-features` |
| 静态检查 | `cargo clippy -p inox-lock --all-targets --all-features -- -D warnings` |
| 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-lock --no-deps --all-features` |
| 渲染截图 | `INOX_SCREENSHOT_DIR=<dir> cargo test -p inox-lock --test render` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`），**含拒绝与错误路径**，不只覆盖成功路径。
- 属性测试每项 ≥1000 样本（`ProptestConfig::with_cases(1000)`）：
  - **除认证成功外，任何事件序列都不会让会话离开锁定**（UI 退出、认证失败、认证超时、新增/移除输出、挂起请求、挂起完成）；
  - **覆盖簿记与独立写出的模型一致**，且「报锁定」必然跟随「所有现存输出已被遮蔽」。
- 渲染测试必须包含浅色、深色、限流等待、失败、无可信 UI 与 200% 字号等场景（1280×720），并断言像素与「禁用控件与启用控件渲染不同」。测试夹具每帧先投递一次 `RedrawRequested`：iced 的控件只在处理它时确定交互状态，没有它每个控件都会被画成禁用（`iced_widget::button`）。
- `tests/catalog.rs` 与 `tests/render.rs` 之外，新增行为必须同时新增测试。
- 测试代码不受「禁止 unwrap/expect/panic」约束。

## 目录结构

```
crates/inox-lock/
  Cargo.toml    工作区继承：字段一律 *.workspace = true
  src/          源码（模块划分见 README）
  i18n/         en-US / zh-CN 的 Fluent 目录
  tests/        catalog.rs、render.rs、support/mod.rs
  README.md     本 crate 说明
  AGENTS.md     本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `lock`。
