# AGENTS.md — inox-notifications

本 crate 遵循根仓库的**全部硬性规则**（[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)），本文件只补充本 crate 特有内容，不放宽任何硬性规则。

## 职责

通知守护进程：`org.freedesktop.Notifications` 协议契约、通知栈（标识符/寿命/上限/策略）、勿扰与锁屏脱敏、弹窗栈界面。所属层级：**L3**（见根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

表面模式：**普通窗口**。`main` 引导 `QGuiApplication` + `QQmlApplicationEngine` 加载内嵌的 `qml/Main.qml`（`Window` 根）；QML 只渲染 Rust 的 JSON 投影并通过一个窄动作通道回报。

**卡片是共享玻璃组件**：每张卡片的根是 L3 的 `inox-glass` 提供的 `GlassPane`（ADR 0044，扩展 ADR 0040 的组件库），材质值（tint / highlight / shade / hairline / 圆角）由 `src/theme.rs` 从 `tokens.glass` 取出后经投影传入。窗口清屏色是 `"transparent"`（结构性哨兵，不是设计色）：玻璃只有在背后能透出东西时才是玻璃。**这不代表通知栈有模糊**——见「已知缺口」。

## crate 特有约束

- **`unsafe` 例外仅限 cxx-qt 生成的胶水**：`[lints.rust] unsafe_code = "allow"` 覆盖工作区的 `forbid`，因为 `#[cxx_qt::bridge]` 宏必然生成 `unsafe extern "C++"`。Inox 手写的 Rust 仍不写 `unsafe`。
- **不引入私有 ABI**：本 crate 只用 cxx-qt / Qt 的公开 API（`QGuiApplication`、`QQmlApplicationEngine`、`QUrl`、`QString`）。需要 Qt 私有头或私有 Qt Quick 接口时先改 ADR 记录范围，再动代码。
- **bin-only**：没有 `src/lib.rs`，所有模块挂在 `main.rs` 的 `mod` 上。cxx-qt 0.10 把 bridge 的 `cxx_qt_init_*` 定义嵌进声明 bridge 的 target，独立库 target / `tests/` 集成测试会以 `undefined reference` 链接失败（KDAB/cxx-qt#770，契约 §2.1）。所以证据写在 `#[cfg(test)] mod headless_tests` 与 `#[cfg(test)] mod bus_tests` 里。
- **QML↔Rust 只有一条数据通道和一条动作通道**：`model_json`（整份投影，变化时整体重发）与 `dispatch(json)`；不要为每个字段加 `#[qproperty]`，也不要在 QML 里做业务决策（契约 §3）。
- **QML 里没有颜色、像素或用户可见文案**：`qml/Main.qml` 只绑定 `vm.*`（契约 §4）。新增视觉值先加到 `view.rs` 的投影。Qt 6.12 的 `Accessible` 附加类型没有「不可用」状态，禁用动作把原因放进 `Accessible.description`。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`；默认级别 `info`，不得每帧或每条通知打印（拒绝路径用 `debug!`）。
- **D-Bus 服务在本 crate 内**。`update` 仍然不做 I/O：它把结果放进 `contract::Outgoing` 队列，`State::take_outgoing` 是传输层的唯一出口，`app::Message::Request` 是传输要喂进来的调用，`app::Inbox` 是它进门的那扇门。**传输线程永远不碰 `State`**（它持有 `Rc<Localizer>`，不是 `Send`），`State` 也永远不进传输线程。
- **一次只服务一个调用**：`src/bus.rs` 的 `bus_loop` 在等待当前调用的答复期间不开始下一个，等待有上限（`REPLY_DEADLINE`），超时回 D-Bus 错误。这是 `app::State` 的出站队列有界且丢最旧造成的：流水线化会让某个客户端的**回复**被丢掉并永远等下去。不要在没有同时改掉那两处之前放宽这条。
- **传输里没有任何一处等待是没有上界的，而且上界到了必须说出来**：客户端调用受 `REPLY_DEADLINE`、从 inbox 取件受 `deliver_within` 的 grace、**读连接**受 `bus::READ_TICK`（250 ms，见 `next_message_within`）。`READ_TICK` 不是超时——空闲的守护进程本来就该一直等客户端——它是**上界**：每一 tick 循环都会醒来一次，因此既能看到 `Service` 被 drop（`Reading::Stopping`），也不会有任何交错能把这个线程停在一个没人能观察的地方（`Reading::Idle` / `Closed`）。**不要再引入没有上界的 `recv()` / `join()` / `block_on`**：CI 曾因为一处没有上界的等待僵住 40 分钟直到被取消，而有上界的失败几分钟就带着名字报出来。测试侧同理：`bus_tests` 的串行锁有期限并点名持有者、握手与守护线程的 `join` 有上界、客户端连接带 `method_timeout`、每个驱动守护进程的用例都挂了一只 `Watchdog`。
- **`CloseNotification` 在线上的回复是空的**：`protocol::CLOSE_NOTIFICATION` 声明无返回值，真实客户端的代理会按签名校验回复。`ReplyBody::Closed` 的布尔值只存在于契约层，不上线——不要「顺手」把它发出去。
- **调用身份只来自消息 serial**（`Header::primary().serial_num()`）：不要在传输层另造计数器或 UUID，去重记忆认的就是它。
- **hint 表有上限、有损且可观察**：超过 `bus::MAX_HINTS` 的键被丢弃并计入 `Counts::hints_dropped`，无法用 `HintValue` 表示的 D-Bus 取值（double、64 位整数、嵌套 variant…）被丢弃并计入 `Counts::hints_unsupported`。**不要为了「别丢 hint」而放宽 `HintValue` 或悄悄强转**。
- **锁状态不是 capability**：`src/lock_state.rs` 只把合成器说的话转达给 `app::State`。没有合成器 / 没有 `inox_ipc` v2 / 连接断开时它如实报 `Tracking::Disabled` 并说明原因，**不得**把未知状态读成 `Unlocked`（`SessionState` 的未知取值一律 `None`），**不得**因此让守护进程退出。锁状态由合成器拥有。
- **正文只经 `markup::sanitize` 进入模型**：`contract::NotifyRequest::draft` 是唯一的 wire → `queue::Draft` 通道，它负责清理正文、展平摘要、解析 hints 与动作。任何新的入队路径都必须同样清理；`Span::Link` 与 `Span::Image` 只允许由 `markup` 构造。
- **标识符、寿命与上限只由 `queue::Queue` 决定**：不得在别处铸造 `NotificationId`、不得在别处判断过期、不得绕过 `QueueConfig::max_pending`。每条通知**恰好一次**终态（expired / dismissed / closed / coalesced / dropped），且每次都要发 `NotificationClosed`——`Counters` 的 `created == live + settled` 是测试断言的不变量。
- **锁屏脱敏是类型，不是布尔检查**：`policy::PopupContent::Withheld` 没有正文、应用名与图标字段。不要在 `view.rs` 里新增读取原始 `Notification` 的路径；视图只画视图模型。
- **勿扰不夺焦点**：`placement::KeyboardInteractivity` 永远为 `None`，视图模型的弹窗栈**没有** focus order。这条有测试（角落 × 全部配置），不要靠注释维持。
- **颜色/尺寸/间距/圆角只来自令牌**（`inox_config::Theme`）：`src/theme.rs` 是唯一把令牌变成 QML 颜色与尺寸的地方，`view.rs` 只调用它。没有**设计色与设计尺寸**的字面量；令牌集仍没有绝对字号与边框宽度，因此 hairline 取 `spacing.xs / 2`、字号四角色由 `spacing.lg × typography.scale` 按 1.5 / 1.25 / 1.0 / 0.875 派生，都是 `theme.rs` 里的具名派生（`docs/quality/ui-guidelines.md` §1–§2）。
- **文案只来自 Fluent**：只能出现消息 id，不得在代码里写用户可见字符串（产品名与组件名除外）。新增 id 必须同时进 `en-US` 与 `zh-CN` 并登记到 `i18n::MESSAGES`；`headless_tests` 双向断言。应用名、摘要、正文与动作标签是**数据**，不是文案。
- **摆放策略是数据，不是已生效的行为**：`src/placement.rs` 计算锚点/层级/exclusive zone/键盘交互，但当前没有 layer-shell 宿主可应用它。启动时只允许**一条** `warn!` 说明表面是普通窗口，不得声称摆放已生效，也不得引入任何 layer-shell 绑定 crate。
- **启动必须自检**：没有可用的 Wayland/X11 显示时，以及总线名已被别的守护进程占用时，`main` 必须以可读错误与非零退出码结束；锁状态读不到**不**属于这一类，它是 `warn!` 加一个诚实的 `Disabled` 状态。`zwlr_layer_shell_v1` 的存在性查询仍未实现（`src/lock_state.rs` 的连接只用于读锁状态，不查询 layer-shell）。
- 新增依赖前先确认依赖方向合法（根级 §2）：本 crate 依赖 L1（`inox-common`、`inox-config`）与 **L2**（`inox-protocols`，用于读 `inox_ipc` 的锁状态事件）——这是一条 **L3 → L2** 的向下边，分层规则 1 允许。**唯一允许的同层边是 build 期的 `inox-glass`**：`[build-dependencies]` 里的 `inox-glass` + `qt-build-utils` 只服务于 `build.rs` 把 QML 资产编进本模块（ADR 0044 授权，登记在根 `Cargo.toml` 的 `same_layer_exceptions`；删除该边时登记必须一起删）。运行期**不得**出现 `use inox_glass`，也不得创建 `inox_glass_v1` 对象。除此之外不得引入同层依赖或到 `inox-compositor` 的依赖。

## 主要依赖

- 上游 Inox crate：`inox-common`（Fluent 加载器）、`inox-config`（`notifications.toml` 模型与设计令牌）、`inox-protocols`（`inox_ipc` 锁状态事件），以及**构建期**的 `inox-glass`（QML 资产；ADR 0044 的 L3 → L3 边，运行期不链接）。
- 第三方（UI）：`cxx`、`cxx-qt`、`cxx-qt-lib`，构建期 `cxx-qt-build` 注册 `com.inox.notifications.qml` 模块，构建期 `qt-build-utils` 用于把 `inox-glass` 的 QML 以显式别名编进本模块（`cxx-qt-build` 没有再导出 `QResourceFile`）。
- 第三方（其余）：`zbus` 6.0、`wayland-client` 0.31（读 `inox_ipc`）、`serde_json`（投影与动作通道）、`calloop`（SIGHUP 遮罩）、`thiserror`、`tracing`、`tracing-subscriber`、`anyhow`（仅 `main`）。
- 测试：`proptest`、`tempfile`；没有离屏渲染/screenshot harness（见「已知缺口」）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。`CARGO_HOME` 指向仓库内缓存，`--offline` 避免联网。

| 目的 | 命令 |
|------|------|
| 格式化 | `cargo fmt -p inox-notifications` |
| 本 crate 测试 | `CARGO_HOME=<repo>/.cargo-home QT_QPA_PLATFORM=offscreen cargo test -p inox-notifications --all-features --offline` |
| 静态检查 | `CARGO_HOME=<repo>/.cargo-home cargo clippy -p inox-notifications --all-targets --all-features --offline -- -D warnings` |
| 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-notifications --no-deps --all-features` |
| 只跑 QML 表面自检 | `cargo test -p inox-notifications --all-features headless_tests::the_qml_surface_loads_offscreen` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 纯逻辑（`protocol` / `markup` / `actions` / `queue` / `policy` / `contract` / `placement` / `view_model` / `app` / `runtime`）必须有单元测试，且**拒绝路径与错误路径**同样覆盖：不合法动作数组、未知动作、重复触发、未知/已消失的标识符、显示范围外的时间、未知 hint、同一调用重复投递。
- 属性测试每项 ≥1000 样本（`ProptestConfig::with_cases(1000)`）：
  - 清理器：纯文本逐字符往返、任何输入都不产生可执行 scheme、清理不放大文本、输出不超上限、确定性；
  - 队列：不超过 `max_pending`、每条通知恰好一次终态且不静默丢失、活动标识符互不相同、替换保留标识符；
  - 调用：去重记忆只保留最近若干条、未被记录的调用没有答案；
  - 守护进程：任何消息序列下每条通知恰好关闭一次、同一通知的动作至多触发一次、不变量 `created == live + settled`。
- `headless_tests`：目录一致性（`compare_catalogs` + `MESSAGES` 双向）、中英两份投影（窗口标题、空态文案）、CLI 行为（`--help` 成功、无显示时以可读错误非零退出、未知参数被拒），以及**离屏 QML 加载**——子进程带 `QT_QPA_PLATFORM=offscreen` 与临时 HOME/XDG，走真实的 `run(["--self-check"])`，等 `reportReady` 确认标题、弹窗数量与语义树。
- `bus_tests`（D06）：点对点 D-Bus 连接 + 真实 `app::State`/`Runtime`。断言 Notify 返回队列分配的标识符、替换保留标识符且不发 `NotificationClosed`、关闭返回规范允许的空回复并恰好一个带正确原因的 `NotificationClosed`、关闭未知 id 不报错、`GetCapabilities` / `GetServerInformation` 与 `protocol` 常量逐字一致、不合法动作数组得到本文档记录的错误名、到期由 sweep 以 reason 1 关闭、动作恰好一次、勿扰只抑制呈现、锁屏时弹窗模型里没有任何可以读出正文的字段、每秒 100 条持续 10 s 后队列不越界且 `created == held + settled`、超时回 `Timeout` 错误、无人读取回 `Unavailable`，以及两条信号在**线上**的字面内容。**该文件保留一把进程内锁串行执行 p2p 用例**（最初用于隔离 zbus 5/async-io 的并发丢唤醒；本次运行时迁移不同时放宽测试并发），并在文件头说明原因。这把锁的等待**有期限**（`SERIAL_DEADLINE`）并点名当前持有者：一个僵住的用例必须变成一条指着它的失败，而不是三个排在它后面的用例一起挂到 CI 被取消。同一理由下，该文件的每一次等待都有上界（客户端的 `method_timeout`、握手与守护线程的有界 `join`、pump 确认的 `pause`），并由 `Watchdog` 兜底：任何东西在 `WEDGED` 之后仍在跑，进程会打印该用例名字并以非零码结束——**这是兜底，不是替代品**。
- 测试代码不受「禁止 unwrap/expect/panic」约束。

## 已知缺口（诚实记录）

- **没有像素级证据**：Iced 的 `tests/render.rs`（tiny-skia 离屏渲染 + `INOX_SCREENSHOT_DIR` 截图工件）已随迁移删除；现在断言的是投影值与「QML 模块能在 offscreen 下加载并回报 ready」。Q02 的截图证据需要 waydriver + 真实 Qt 平台，工作区还没有这样的宿主。
- **语义树只被「存在性/数量」断言**：`reportReady` 检查标题、弹窗卡片数量与 `Accessible` 控件数下限；离屏平台没有 AT-SPI 桥，screen reader 路径未验证。
- **会话总线上的名字所有权**：服务会声明 `org.freedesktop.Notifications`，别的守护进程已持有时拒绝启动；这条拒绝的映射有单元测试，而「两条守护进程争一个名字」的真实场景需要一个会话总线，crate 的 CI 分片没有。`bus_tests` 因此走点对点连接：线上协议、回复与信号都是真的，但 p2p 不携带 well-known name。
- **能真正被问锁状态的合成器**：`src/lock_state.rs` 已接线且它的判断有测试，但本 crate 没有任何测试绑定真实的 `inox_ipc` 全局：工作区里没有可以充当假合成器的服务端，另一端由 `inox-compositor` 的测试负责。
- **layer 表面**：弹窗栈目前是普通窗口，`src/placement.rs` 的策略没有被应用（合成器**已经**广告 `zwlr_layer_shell_v1` v4，缺的是本 crate 的宿主）。
- **卡片上的模糊与折射**：卡片用共享玻璃组件画材质（ADR 0044），但本 crate 不创建 `inox_glass_v1` 对象，弹窗栈也没有 layer 角色——合成器只对有 layer 角色的表面广告非零能力。因此真实会话里卡片是**纯半透明**，不是磨砂。「用同一套材质」不等于「有玻璃效果」。
- **窗口透明（E4）**：`Window.color` 是 `"transparent"`，它只表示 QML 不清屏；能否真的透出背后的桌面取决于 Qt 平台插件的 ARGB visual 与会话合成，离屏测试证明不了。
- **图标解码与正文图片**：`icon-static` / `body-images` capability 故意不声明；带图标的弹窗画图标将占用的空位，正文 `<img>` 画 alt 文本。
- **声音 / 持久化**：不声明 `sound` / `persistence` capability；`sound-name`、`suppress-sound`、`resident` 会被读取并记录。

## 目录结构

```
crates/inox-notifications/
  Cargo.toml        工作区继承：字段一律 *.workspace = true
  build.rs          cxx-qt：注册 com.inox.notifications.qml 模块并编译 bridge；把 inox-glass 的 QML 资产以显式别名编进本模块
  qml/Main.qml      声明式 UI（Window 根），只绑定 vm.*
  src/main.rs       入口、Qt 宿主、--self-check；模块声明（bin-only）
  src/cxxqt_object.rs  #[cxx_qt::bridge]：model_json + pump / dispatch / reportReady
  src/runtime.rs    Runtime（pump 的唯一实现）、Task、Reply/Completer、UiAction
  src/reload.rs     SIGHUP + org.inox.Notifications 的 org.inox.Configuration1 重载桥
  src/view.rs       状态 + 令牌 + Fluent → JSON 投影
  src/theme.rs      令牌 → QML 颜色与尺寸（纯函数；含卡片玻璃材质的三个颜色）
  src/view_model.rs 状态 → 弹窗栈语义
  src/protocol.rs / markup.rs / actions.rs / queue.rs / policy.rs /
  contract.rs / bus.rs / lock_state.rs / placement.rs / app.rs / i18n.rs
  src/headless_tests.rs  目录、投影、CLI 与 QML 加载证据（#[cfg(test)]）
  src/bus_tests.rs       点对点 D-Bus + 真实运行时证据（#[cfg(test)]）
  i18n/{en-US,zh-CN}/notifications.ftl
  README.md         本 crate 说明与当前状态
  AGENTS.md         本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `notifications`。
