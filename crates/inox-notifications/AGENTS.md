# AGENTS.md — inox-notifications

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

通知守护进程：`org.freedesktop.Notifications` 协议契约、通知栈（标识符/寿命/上限/策略）、勿扰与锁屏脱敏、弹窗栈界面。所属层级：**L3**（见根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`；默认级别 `info`，不得每帧或每条通知打印（拒绝路径用 `debug!`）。
- **D-Bus 服务在本 crate 内**（ADR 0022 决策 5 授权引入 `zbus`；该决策同时固定了线程模型）。`update` 仍然不做 I/O：它把结果放进 `contract::Outgoing` 队列，`State::take_outgoing` 是传输层的唯一出口，`app::Message::Request` 是传输要喂进来的调用，`app::Inbox` 是它进门的那扇门。**传输线程永远不碰 `State`**（它持有 `Rc<Localizer>`，不是 `Send`），`State` 也永远不进传输线程。
- **一次只服务一个调用**：`src/bus.rs` 的 `bus_loop` 在等待当前调用的答复期间不开始下一个，等待有上限（`REPLY_DEADLINE`），超时回 D-Bus 错误。这是 `app::State` 的出站队列有界且丢最旧造成的：流水线化会让某个客户端的**回复**被丢掉并永远等下去。不要在没有同时改掉那两处之前放宽这条。
- **`CloseNotification` 在线上的回复是空的**：`protocol::CLOSE_NOTIFICATION` 声明无返回值，真实客户端的代理会按签名校验回复。`ReplyBody::Closed` 的布尔值只存在于契约层，不上线——不要「顺手」把它发出去。
- **调用身份只来自消息 serial**（`Header::primary().serial_num()`）：不要在传输层另造计数器或 UUID，去重记忆认的就是它。
- **hint 表有上限、有损且可观察**：超过 `bus::MAX_HINTS` 的键被丢弃并计入 `Counts::hints_dropped`，无法用 `HintValue` 表示的 D-Bus 取值（double、64 位整数、嵌套 variant…）被丢弃并计入 `Counts::hints_unsupported`。**不要为了「别丢 hint」而放宽 `HintValue` 或悄悄强转**。
- **锁状态不是 capability**：`src/lock_state.rs` 只把合成器说的话转达给 `app::State`。没有合成器 / 没有 `inox_ipc` v2 / 连接断开时它如实报 `Tracking::Disabled` 并说明原因，**不得**把未知状态读成 `Unlocked`（`SessionState` 的未知取值一律 `None`），**不得**因此让守护进程退出。锁状态由合成器拥有（ADR 0015）。
- **正文只经 `markup::sanitize` 进入模型**：`contract::NotifyRequest::draft` 是唯一的 wire → `queue::Draft` 通道，它负责清理正文、展平摘要、解析 hints 与动作。任何新的入队路径都必须同样清理；`Span::Link` 与 `Span::Image` 只允许由 `markup` 构造。
- **标识符、寿命与上限只由 `queue::Queue` 决定**：不得在别处铸造 `NotificationId`、不得在别处判断过期、不得绕过 `QueueConfig::max_pending`。每条通知**恰好一次**终态（expired / dismissed / closed / coalesced / dropped），且每次都要发 `NotificationClosed`——`Counters` 的 `created == live + settled` 是测试断言的不变量。
- **锁屏脱敏是类型，不是布尔检查**：`policy::PopupContent::Withheld` 没有正文、应用名与图标字段。不要在 `view.rs` 里新增读取原始 `Notification` 的路径；视图只画视图模型。
- **勿扰不夺焦点**：`placement::KeyboardInteractivity` 永远为 `None`，视图模型的弹窗栈**没有** focus order。这条有测试（角落 × 全部配置），不要靠注释维持。
- **颜色/尺寸/间距/圆角只来自令牌**（`inox_config::Theme`）：`src/theme.rs` 是唯一把令牌变成颜色与样式的地方，`view.rs` 只调用它。没有**设计色与设计尺寸**的字面量（结构性哨兵如 `Color::TRANSPARENT`、边框宽 `0.0` 除外）；令牌集仍没有绝对字号与边框宽度，因此 hairline 取 `spacing.xs / 2`、字号四角色由 `spacing.lg × typography.scale` 按 1.5 / 1.25 / 1.0 / 0.875 派生，都是 `theme.rs` 里的具名派生（`docs/quality/ui-guidelines.md` §1–§2）。
- **文案只来自 Fluent**：只能出现消息 id，不得在代码里写用户可见字符串（产品名与组件名除外）。新增 id 必须同时进 `en-US` 与 `zh-CN` 并登记到 `i18n::MESSAGES`；`tests/catalog.rs` 双向断言。应用名、摘要、正文与动作标签是**数据**，不是文案。
- **摆放策略是数据，不是已生效的行为**：`src/placement.rs` 计算锚点/层级/exclusive zone/键盘交互，但当前没有 layer-shell 宿主可应用它（ADR 0018）。启动时只允许**一条** `warn!` 说明表面是普通窗口，不得声称摆放已生效，也不得引入 `iced_layershell`。
- **启动必须自检**：没有可用的 Wayland/X11 显示时，以及总线名已被别的守护进程占用时，`main` 必须以可读错误与非零退出码结束（ADR 0018、ADR 0022 决策 5）；锁状态读不到**不**属于这一类，它是 `warn!` 加一个诚实的 `Disabled` 状态。`zwlr_layer_shell_v1` 的存在性查询仍未实现（`src/lock_state.rs` 的连接只用于读锁状态，不查询 layer-shell）。
- 新增依赖前先确认依赖方向合法（根级 §2）：本 crate 依赖 L1（`inox-common`、`inox-config`）与 **L2**（`inox-protocols`，用于读 `inox_ipc` 的锁状态事件）——这是一条 **L3 → L2** 的向下边，ADR 0002 规则 1 允许，ADR 0022 §7 已登记；**不得**引入同层依赖或到 `inox-compositor` 的依赖。

## 主要依赖

- 上游 Inox crate：`inox-common`（Fluent 加载器）、`inox-config`（`notifications.toml` 模型与设计令牌）。
- 第三方：`iced` 0.14（`tiny-skia` 渲染器）、`zbus` 6.0（异步 API 由传输线程通过 `block_on` 驱动，p2p 用于真实往返测试；ADR 0027 记录迁移与 MSRV）、`wayland-client` 0.31（读 `inox_ipc`）、`thiserror`、`tracing`、`tracing-subscriber`、`anyhow`（仅 `main`）。
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
- `tests/bus.rs`（D06）：点对点 D-Bus 连接 + 真实 `app::State`。断言 Notify 返回队列分配的标识符、替换保留标识符且不发 `NotificationClosed`、关闭返回规范允许的空回复并恰好一个带正确原因的 `NotificationClosed`、关闭未知 id 不报错、`GetCapabilities` / `GetServerInformation` 与 `protocol` 常量逐字一致、不合法动作数组得到本文档记录的错误名、到期由 sweep 以 reason 1 关闭、动作恰好一次、勿扰只抑制呈现、锁屏时弹窗模型里没有任何可以读出正文的字段、每秒 100 条持续 10 s 后队列不越界且 `created == held + settled`、超时回 `Timeout` 错误、无人读取回 `Unavailable`，以及两条信号在**线上**的字面内容。**该文件保留一把进程内锁串行执行 p2p 用例**（最初用于隔离 zbus 5/async-io 的并发丢唤醒；本次运行时迁移不同时放宽测试并发），并在文件头说明原因。
- `tests/render.rs`：浅色、深色、空态、锁屏脱敏、溢出、勿扰、禁用动作与角落摆放等场景，在 1280×720 下经真实组件树渲染并断言像素；不得依赖字体度量（断言控件**背景/边框**令牌与「有无墨迹」，而非字形）。含一次点击动作按钮的交互测试：**恰好一个** `ActionInvoked` 消息与一个信号，且弹窗随之关闭。
- 测试代码不受「禁止 unwrap/expect/panic」约束。

## 目录结构

```
crates/inox-notifications/
  Cargo.toml    工作区继承：字段一律 *.workspace = true
  src/          protocol / markup / actions / queue / policy / contract /
                bus / lock_state / placement / view_model / app / view /
                theme / i18n / main
  i18n/         en-US / zh-CN 的 Fluent 目录
  tests/        catalog.rs、render.rs、bus.rs、support/（离屏渲染夹具）
  README.md     本 crate 说明与当前状态
  AGENTS.md     本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `notifications`。
