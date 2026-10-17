# AGENTS.md — inox-notifications

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

通知守护进程：`org.freedesktop.Notifications` 协议契约、通知栈（标识符/寿命/上限/策略）、勿扰与锁屏脱敏、弹窗栈界面。所属层级：**L3**（见根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`；默认级别 `info`，不得每帧或每条通知打印（拒绝路径用 `debug!`）。
- **D-Bus 服务不在本 crate 内**（PRD §13 属 M5）。`update` 不做 I/O：它把结果放进 `contract::Outgoing` 队列，`State::take_outgoing` 是传输层的接缝，`Message::Request` 是它要喂进来的调用。不要为了「让通知真的到达」而在此引入 `zbus`/`dbus` 依赖——那需要先改 PRD §13 与 D06 的排期，并另开 PR。
- **正文只经 `markup::sanitize` 进入模型**：`contract::NotifyRequest::draft` 是唯一的 wire → `queue::Draft` 通道，它负责清理正文、展平摘要、解析 hints 与动作。任何新的入队路径都必须同样清理；`Span::Link` 与 `Span::Image` 只允许由 `markup` 构造。
- **标识符、寿命与上限只由 `queue::Queue` 决定**：不得在别处铸造 `NotificationId`、不得在别处判断过期、不得绕过 `QueueConfig::max_pending`。每条通知**恰好一次**终态（expired / dismissed / closed / coalesced / dropped），且每次都要发 `NotificationClosed`——`Counters` 的 `created == live + settled` 是测试断言的不变量。
- **锁屏脱敏是类型，不是布尔检查**：`policy::PopupContent::Withheld` 没有正文、应用名与图标字段。不要在 `view.rs` 里新增读取原始 `Notification` 的路径；视图只画视图模型。
- **勿扰不夺焦点**：`placement::KeyboardInteractivity` 永远为 `None`，视图模型的弹窗栈**没有** focus order。这条有测试（角落 × 全部配置），不要靠注释维持。
- **颜色/尺寸/间距/圆角只来自令牌**（`inox_config::Theme`）：`src/theme.rs` 是唯一把令牌变成颜色与样式的地方，`view.rs` 只调用它。唯一的裸像素是 1px 的 hairline 边框宽度（`BORDER_WIDTH`，令牌集没有边框宽度）与 iced 默认字号乘以 `typography.scale`（令牌集没有绝对字号）。
- **文案只来自 Fluent**：只能出现消息 id，不得在代码里写用户可见字符串（产品名与组件名除外）。新增 id 必须同时进 `en-US` 与 `zh-CN` 并登记到 `i18n::MESSAGES`；`tests/catalog.rs` 双向断言。应用名、摘要、正文与动作标签是**数据**，不是文案。
- **摆放策略是数据，不是已生效的行为**：`src/placement.rs` 计算锚点/层级/exclusive zone/键盘交互，但当前没有 layer-shell 宿主可应用它（ADR 0018）。启动时只允许**一条** `warn!` 说明表面是普通窗口，不得声称摆放已生效，也不得引入 `iced_layershell`。
- **启动必须自检**：没有可用的 Wayland/X11 显示时，`main` 必须以可读错误与非零退出码结束（ADR 0018）。`zwlr_layer_shell_v1` 的存在性查询尚未实现：本 crate 不与合成器建立连接，该自检随 layer-shell 宿主一并落地。
- 新增依赖前先确认依赖方向合法（根级 §2）：本 crate 只依赖 L1（`inox-common`、`inox-config`），不得引入同层或合成器依赖。

## 主要依赖

- 上游 Inox crate：`inox-common`（Fluent 加载器）、`inox-config`（`notifications.toml` 模型与设计令牌）。
- 第三方：`iced` 0.14（`tiny-skia` 渲染器）、`thiserror`、`tracing`、`tracing-subscriber`、`anyhow`（仅 `main`）。
- 测试：`iced_core` / `iced_runtime` / `iced_tiny_skia`（离屏渲染）、`image`（截图产物）、`proptest`、`tempfile`。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。以下四条即本 crate 的自检。

| 目的 | 命令 |
|------|------|
| 格式化 | `cargo fmt -p inox-notifications` |
| 本 crate 测试 | `cargo test -p inox-notifications --all-features` |
| 静态检查 | `cargo clippy -p inox-notifications --all-targets --all-features -- -D warnings` |
| 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-notifications --no-deps --all-features` |
| 渲染截图 | `INOX_SCREENSHOT_DIR=<dir> cargo test -p inox-notifications --test render` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 纯逻辑（`protocol` / `markup` / `actions` / `queue` / `policy` / `contract` / `placement` / `view_model` / `app`）必须有单元测试，且**拒绝路径与错误路径**同样覆盖：不合法动作数组、未知动作、重复触发、未知/已消失的标识符、显示范围外的时间、未知 hint、同一调用重复投递。
- 属性测试每项 ≥1000 样本（`ProptestConfig::with_cases(1000)`）：
  - 清理器：纯文本逐字符往返、任何输入都不产生可执行 scheme、清理不放大文本、输出不超上限、确定性；
  - 队列：不超过 `max_pending`、每条通知恰好一次终态且不静默丢失、活动标识符互不相同、替换保留标识符；
  - 调用：去重记忆只保留最近若干条、未被记录的调用没有答案；
  - 守护进程：任何消息序列下每条通知恰好关闭一次、同一通知的动作至多触发一次、不变量 `created == live + settled`。
- `tests/catalog.rs`：`compare_catalogs` 的 id 与占位符一致性，加上 `MESSAGES` ↔ 目录的双向检查。
- `tests/render.rs`：浅色、深色、空态、锁屏脱敏、溢出、勿扰、禁用动作与角落摆放等场景，在 1280×720 下经真实组件树渲染并断言像素；不得依赖字体度量（断言控件**背景/边框**令牌与「有无墨迹」，而非字形）。含一次点击动作按钮的交互测试：**恰好一个** `ActionInvoked` 消息与一个信号，且弹窗随之关闭。
- 测试代码不受「禁止 unwrap/expect/panic」约束。

## 目录结构

```
crates/inox-notifications/
  Cargo.toml    工作区继承：字段一律 *.workspace = true
  src/          protocol / markup / actions / queue / policy / contract /
                placement / view_model / app / view / theme / i18n / main
  i18n/         en-US / zh-CN 的 Fluent 目录
  tests/        catalog.rs、render.rs、support/（离屏渲染夹具）
  README.md     本 crate 说明与当前状态
  AGENTS.md     本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `notifications`。
