# inox-notifications

Inox 桌面的通知守护进程：`org.freedesktop.Notifications` 背后的进程。属于 Inox 桌面环境的 **L3** 层（PRD §3.1；层级成员的唯一事实来源是根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

它**拥有**通知栈本身：标识符、寿命、队列上限、勿扰状态，以及锁屏时能显示什么。它不拥有窗口、不拥有会话的锁定状态；锁状态是合成器经 `inox-ipc` 告诉它的。

弹窗栈是 **QML + Qt Quick 的普通窗口**，经 cxx-qt 绑定到 Rust（[迁移契约](../../docs/qml-migration-contract.md)）。守护进程的领域逻辑、D-Bus 服务、队列、策略与锁屏脱敏全在 Rust；QML 只渲染一份 JSON 投影并把点击发回来。

## 运行结构

```
              zbus（自己的线程） ── org.freedesktop.Notifications ── 客户端
                        │ contract::Request / Outgoing
                        ▼
  Inbox（有界 mpsc） ──► Runtime::pump（Qt 线程，QML Timer 驱动）──► app::update
                        ▲                                        │
  lock_state（自己的 wayland-client 连接，inox_ipc 事件）          ├─► contract::Outgoing ──► bus 线程
  reload（SIGHUP + org.inox.Configuration1）                      └─► view.rs 投影 ──► modelJson ──► QML

  其他：QGuiApplication + QQmlApplicationEngine → qrc:/…/Main.qml (Window)
```

- **一条数据通道**：`view.rs` 把「弹窗栈 + 令牌 + Fluent 文案」投影成一份 JSON，桥作为只读属性 `modelJson` 交给 QML；变化时整体重发（契约 §3 的取舍）。
- **一条动作通道**：`dispatch(json)` 解析成 `runtime::UiAction` → `app::Message`；不合法动作只 `warn!`。
- **定时器**：QML `Timer` 调 `pump()`，排空 Inbox 与重载队列、跑 `update`、必要时重发投影。传输线程永远不碰 `State`（它持有 `Rc<Localizer>`，不是 `Send`），也永远不碰 QML。

## 状态

当前证据等级 **E1 + 一条 E2 腿**：编译、lint、文档与纯逻辑测试通过；`bus_tests` 用**真实 D-Bus 连接**驱动真实状态机（点对点连接，因为 CI 没有会话总线），覆盖 D06 要求的 Notify / 替换 ID / 关闭与关闭原因 / 超时 / 动作至多一次 / 勿扰 / 每秒 100 条持续 10 s 的有界性 / 锁屏脱敏，以及本守护进程自己的三种拒绝错误；`headless_tests` 在离屏平台上加载 QML 表面并确认它读到了投影。

已交付：

- `src/protocol.rs` —— freedesktop 通知契约**作为数据**：总线名、对象路径、接口、四个方法各自的完整参数表与签名（`Notify` = `susssasa{sv}iu`）、两个信号、Inox 声明的 capability 列表、关闭原因（1 expired / 2 dismissed / 3 CloseNotification / 4 undefined）、紧急程度（0/1/2）、标准 hint 键（含已废弃的 `image_path` 拼写）、`expire_timeout` 的三种含义。每个方法的签名由参数表拼出来自检，拼错就失败。
- `src/markup.rs` —— 正文清理器：只识别 `<b> <i> <u> <a href> <img src alt>`，其余标签与裸 `<`、`&`、引号**原样保留为文本**（输出是结构化 span 列表，不是 HTML 字符串，所以没有第二个解释器可以出错）；`<a href>` 只接受 http/https/mailto，`<img src>` 只接受 http(s) URL 或路径，`javascript:` / `data:` / `file:` 一律按文本处理。
- `src/actions.rs` —— 动作身份与**至多一次**：动作键唯一、数量有上限、`default` 动作只作为「点击通知本身」的语义；一次动作被触发后再次触发会被**拒绝并报告**，而不是静默重复。
- `src/queue.rs` —— 守护进程拥有的模型：单调分配、替换（`replaces_id`）在原位更新并重开计时、关闭原因、寿命（`TimeoutConfig` 的 `0` = 永不过期，**关键通知永不自过期**）、有界队列（`QueueConfig::max_pending` + `DropOldest` / `CoalesceByApp`）与**可观察计数器**（admitted/created/replaced/dropped/coalesced/expired/dismissed/closed_by_call）。
- `src/policy.rs` —— 勿扰（只抑制呈现，不抑制投递；守护进程持有运行期状态，`notifications.toml` 只是启动初值）与**锁屏脱敏**：`PopupContent::Withheld` **没有**正文、应用名与图标字段，因此不存在「忘记检查」的绘制路径。
- `src/contract.rs` —— 调用契约：`Request`（四个方法）/ `Reply` / `Signal` / `Outgoing`，以及按调用标识去重的有界记忆（重复调用按原答案回答，不产生第二条通知）。
- `src/placement.rs` —— layer-shell 摆放策略（overlay 层、角落锚点、margin、尺寸、**键盘交互永远为 None**、逐输出分配），**纯数据**。
- `src/view_model.rs`、`src/view.rs`、`src/theme.rs` —— 投影链：状态 → 弹窗栈语义 → 一份 JSON（颜色是令牌转换的 `#AARRGGBB`、尺寸与字号是令牌派生、文案是 Fluent 的完整句子）。QML 里没有第二种颜色/尺寸/文案来源。
- `src/app.rs`、`src/runtime.rs`、`src/reload.rs` —— 状态机（Elm）、Qt 宿主的 pump（`Runtime`）、本 crate 自己的 `Task` / `Reply` / 一次性通道，以及 SIGHUP 与 `org.inox.Configuration1` 的配置重载桥（失败的重新加载保留旧配置）。
- `src/cxxqt_object.rs`、`qml/Main.qml`、`src/main.rs` —— cxx-qt 桥（`modelJson` + `pump`/`dispatch`/`reportReady`）、声明式弹窗栈、Qt 宿主与 `--self-check` 表面自检。
- `i18n/en-US/notifications.ftl`、`i18n/zh-CN/notifications.ftl` —— 守护进程自己的全部文案；`headless_tests` 断言两个目录 id 与占位符一致，且与 `i18n::MESSAGES` 双向一致。
- `src/bus.rs` —— `org.freedesktop.Notifications` 服务本体：在**自己的线程**上持有 `zbus::Connection`（zbus 6 的异步操作由 `zbus::block_on` 驱动），把方法调用翻译成 `contract::Request` 交给应用，再把 `Outgoing` 变成回复与 `NotificationClosed` / `ActionInvoked`；信号由第二条线程发出，否则一次到期没有客户端调用就带不出总线。**一次只服务一个调用**并带**上限**等待答复（超时返回 D-Bus 错误，迟到的答复被去重记忆吸收）；调用身份取自消息 serial；hint 表有上限、无法用 `HintValue` 表示的取值会被**计数后丢弃**；另外服务 `Introspectable` / `Properties.GetAll` / `Peer.Ping`，因为真实客户端的代理会先问这些。
- `src/lock_state.rs` —— 锁状态的**观察者**：在自己的线程上用自己的 `wayland-client` 连接绑定 `inox_ipc`，把 `session_lock_state` 事件作为 `Message::SessionLockChanged` 交给应用。它不是 capability：没有合成器、没有 v2 全局或连接断开时，它**如实报告 Disabled 并说明原因**，不猜「未锁定」，也不让守护进程退出。
- `src/main.rs` —— 日志、配置、目录、**传输的启动与自检**：先声明总线名，再接上锁状态与重载，最后引导 Qt/QML。没有可用的 Wayland/X11 显示时同样以可读错误与非零退出结束；`--self-check` 是唯一不需要显示的路径，它只验证表面。

未交付，且不声称：

| 项 | 原因 |
|----|------|
| **像素级 UI 证据** | Iced 时代的 `tests/render.rs`（tiny-skia 离屏渲染 + `INOX_SCREENSHOT_DIR` 截图工件）已随迁移删除。现在断言投影值与「QML 模块在 `QT_QPA_PLATFORM=offscreen` 下加载并回报 ready」；截图证据需要 waydriver + 真实 Qt 平台，工作区还没有宿主。 |
| **会话总线上的名字所有权** | 服务会声明 `org.freedesktop.Notifications`，别的守护进程已持有时拒绝启动；这条拒绝的映射有单元测试，而「两条守护进程争一个名字」的真实场景需要一个会话总线，crate 的 CI 分片没有。`bus_tests` 因此走点对点连接：线上协议、回复与信号都是真的，但 p2p 不携带 well-known name。 |
| **能真正被问锁状态的合成器** | `src/lock_state.rs` 已接线且它的判断有测试，但本 crate 没有任何测试绑定真实的 `inox_ipc` 全局：工作区里没有可以充当假合成器的服务端，另一端由 `inox-compositor` 的测试负责。 |
| **layer 表面**：弹窗栈目前是普通窗口 | 合成器尚未广告 `zwlr_layer_shell_v1`；启动时输出**恰好一条** `tracing::warn!` 说明这一点并点名 L3 组件架构，`src/placement.rs` 的策略**没有被应用**。|
| **AT-SPI 语义导出** | QML 用 `Accessible.role`/`Accessible.name`/`description` 标注控件，`reportReady` 只断言语义树存在（数量下限）；离屏平台没有 AT-SPI 桥，screen reader 路径未验证。Qt 6.12 的 `Accessible` 也没有「不可用」状态，禁用动作把原因写进 description。 |
| **图标解码**：`icon-static` 与 `body-images` capability 故意不声明 | 解析主题图标或正文图片需要在交互循环之外做文件查找与解码。带图标的弹窗画出图标将占用的**空位**（按 `display.icon_size`），正文 `<img>` 画 alt 文本。 |
| **声音**：不声明 `sound` capability | `sound-name` / `suppress-sound` 会被记录，但 Inox 目前不播放任何声音。 |
| **持久化**：不声明 `persistence` capability | 该 capability 承诺的是 `resident` hint 的行为——动作被触发后保留通知；本守护进程在动作触发时即关闭通知。`resident` 仍会被读取并记录（`protocol::Hints`）。 |
| `<u>` 的下划线 | 清理器保留 `underline` 强调，但 QML 的 `Text` 富文本路径尚未实现它（模型里有，画面上没有）。 |
| `<b>`/`<i>` 在正文里生效，在 **summary** 里被展平为纯文本 | 标题只画一行文字，标记在那里没有视觉意义；展平还保证摘要里的 `javascript:` 链接不会变成可点的东西。 |
| **E2/E3 GUI 证据** | 需要支持 layer-shell 的宿主与真实会话总线，CI 目前都没有。 |

## 结构

| 文件 | 内容 |
|------|------|
| `src/protocol.rs` | 通知契约（常量、方法表、能力、关闭原因、hint） |
| `src/markup.rs` | 正文清理器（纯逻辑，属性测试） |
| `src/actions.rs` | 动作身份与至多一次（纯逻辑） |
| `src/queue.rs` | 标识符、寿命、上限与计数器（纯逻辑，属性测试） |
| `src/policy.rs` | 勿扰与锁屏脱敏（纯逻辑） |
| `src/contract.rs` | 调用、回复、信号与去重 |
| `src/bus.rs` | `org.freedesktop.Notifications` 的 D-Bus 传输（两条线程） |
| `src/lock_state.rs` | `inox-ipc` 锁状态观察者（自己的连接与线程） |
| `src/placement.rs` | layer-shell 摆放策略（数据，未应用） |
| `src/view_model.rs` | 状态 → 弹窗栈语义 |
| `src/view.rs` | 弹窗栈的 JSON 投影 |
| `src/theme.rs` | 设计令牌 → QML 颜色与尺寸 |
| `src/app.rs` | `State` / `Message` / `update`（Elm） |
| `src/runtime.rs` | `Runtime`（pump 的唯一实现）、`Task`、`Reply`、`UiAction` |
| `src/reload.rs` | SIGHUP + `org.inox.Configuration1` 重载桥 |
| `src/cxxqt_object.rs` | cxx-qt 桥（`modelJson` + 三个 invokable） |
| `qml/Main.qml` | 声明式弹窗栈（只绑定 `vm.*`） |
| `src/main.rs` | 入口、Qt 宿主、`--self-check` |
| `src/i18n.rs` | 内嵌的 Fluent 目录 |
| `i18n/<locale>/notifications.ftl` | 文案 |
| `src/headless_tests.rs` | 目录一致性、投影、CLI 与 QML 加载证据 |
| `src/bus_tests.rs` | 真实 D-Bus 连接 + 真实状态机（D06） |

`crates/<name>/AGENTS.md`（`CLAUDE.md` 是其符号链接）是本 crate 的约束文档。

## 开发约定

- 本项目遵循[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**。
- 颜色、尺寸、间距、圆角与动效时长只来自 `inox_config::Theme` 令牌；守护进程自己的文案只来自 Fluent 目录（应用名、摘要、正文与动作标签是随通知到达的**数据**，不翻译）；QML 里不允许出现字面量颜色、像素尺寸或用户可见英文串。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 使用 `notifications`。

| 目的 | 命令 |
|------|------|
| 测试 | `CARGO_HOME=<repo>/.cargo-home QT_QPA_PLATFORM=offscreen cargo test -p inox-notifications --all-features --offline` |
| 静态检查 | `CARGO_HOME=<repo>/.cargo-home cargo clippy -p inox-notifications --all-targets --all-features --offline -- -D warnings` |
| 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-notifications --no-deps --all-features` |
| 表面自检（有人看界面时） | `cargo run -p inox-notifications -- --self-check` |

## 许可证

LGPL-3.0，见仓库根的 [`LICENSE`](../../LICENSE)。
