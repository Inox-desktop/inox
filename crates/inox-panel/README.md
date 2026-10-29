# inox-panel

Inox 桌面的面板：任务栏、系统托盘与时钟。属于 Inox 桌面环境的 **L3** 层（PRD §3.1；层级成员的唯一事实来源是根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

面板不拥有任何状态：窗口属于合成器，托盘项属于各自的应用程序。面板订阅它们，并把自己看到的东西画出来。

## 表面模式：真实 layer 表面 + 离屏 QML

面板**没有自己的窗口**。它持有的是真实的 `zwlr_layer_shell_v1` 表面（`src/layer.rs` 创建、摆放并提交帧），界面由 QML 渲染，但渲染目标是**离屏**的：

> **这一段是过渡态。** [ADR 0046](../../docs/adr/0046-shell-surfaces-and-native-qt.md) 把终点定在
> 「Qt 原生 wayland 表面 + 私有协议 `inox-shell-v1`」：离屏帧没有 `wl_surface.frame` 时间线、没有
> 第二个表面，因此面板长成菜单、菜单从按钮里钻出这类跨表面过渡，以及子菜单与键盘导航的菜单，
> 在离屏路径上做不到；它也没有输入与 AT-SPI。P3 落地前，下面的管线仍是真实情况。

```text
calloop 主循环（layer.rs）
  └─ 状态变化 → view.rs 投影成一份 JSON（矩形 + 文本 + 点击区域）
       └─ offscreen.rs → cxx-qt 桥（cxxqt_object.rs）→ C++ 宿主（offscreen.cpp）
            └─ QGuiApplication(offscreen) + QQmlApplicationEngine → QQuickWindow::grabWindow()
                 └─ RGBA8 预乘像素 → write_argb() → wl_shm 的 Argb8888 帧 → 真正的 layer 表面
```

因此：进程里**没有** Qt Wayland 客户端、没有 `LayerShellQt`、没有 `xdg_toplevel`，也没有 Qt Wayland Compositor。输入的归属也不变：`wl_pointer` 的事件仍由 Rust 翻译成 `Pointer`，在 `src/layer.rs` 里按场景自带的 `entries`（点击区域）命中，产生既有的 `app::Message`；QML 只画，不接输入、不持状态。

任务区最多显示 `panel.max_task_buttons` 个按钮（默认 12）；任务区是弹性区域，超长标题在任务区末端被裁掉，时钟与托盘从右边缘向内排布，因此不会被挤出表面（D05）。

## 窗口服务：任务栏背后的传输

面板的窗口状态来自合成器的私有 **`inox-window-manager-v1`**（ADR 0045 §4，`proposed`）。`src/window_service.rs` 就是那段传输，它**不新增窗口模型**：把 `inox_window_list_v1` 的事件翻译成既有的 `app::Message`，把既有的 `app::OutgoingCommand` 队列翻译成协议请求，`src/layer.rs` 的循环负责泵送。

| 协议事件 | 既有 `app::Message` |
|----------|---------------------|
| `session_generation` | `ServiceReady { generation }` |
| `workspace_state` | `WorkspacesChanged { active, count }` |
| `window` | `WindowUpdated { key, window }`（快照期间同一条路径） |
| `window_removed` | `WindowRemoved { key }` |
| `snapshot_done` | `Snapshot { generation, windows }` |
| `request_failed`（窗口） | `CommandSettled { request, outcome }`，`Rejected { reason }` |
| `request_failed`（保留的无窗口句柄） | `WorkspaceSwitchFailed { reason }` |

`request_error` 的三个取值一一对应既有的 `contract::RejectionReason::{UnknownWindow, NotPermitted}`，`unsupported` 落进 `Protocol { detail }`——**没有第四个错误变体**。反向的六个请求是 `activate` / `minimize` / `close` / `set_mode` / `move_to_workspace` / `switch_workspace`；`OutgoingCommand::Snapshot` 创建一个新的 `inox_window_list_v1`（列表**就是**快照，重连等于换观察者）。

**一条请求由状态事件确认，不由应答确认**：腿不产生 `CommandState::Applied`，`window` / `window_removed` / `workspace_state` 才是确认，`Rejected` 只来自 `request_failed`。**句柄只在它的 `session_generation` 内有意义**：每个 `WindowKey` 都把 wire 句柄与列表宣告的代次配成一对。工作区切换是唯一不命名窗口的请求，它占 `PendingRequests` 的独立槽位（至多一个在途），失败落在 `WorkspaceSwitchFailed` 而不是某个窗口的 `CommandSettled`。

点击语义：左键沿用既有定义（未聚焦=激活、已聚焦=最小化），**中键=关闭**（`TaskClosed` → `close`，是发给客户端的**请求**，客户端可以保存、弹窗或拒绝；面板只在收到 `window_removed` 之后移除按钮），**右键=模式循环** `tiled → floating → maximized → fullscreen`（`TaskModeCycled`，循环顺序由 `tasks::WindowMode::cycled` 一处定义），**任务区左侧的工作区指示器点击=切换工作区**（`WorkspaceSwitched`；`WorkspaceView::is_shown` 只在 `count > 1` 时画它——单工作区的控件没有可改变的东西）。

**服务不可用是设计状态，不是崩溃**：合成器不广告该全局时，面板以一次 `info!` 说明，停在既有的 `ServiceState::Unavailable`，画出既有的「窗口服务不可用」句子与重试按钮（D05），**不**非零退出——没有窗口服务，面板仍是一个可用的时钟与托盘。`Message::Retry` 会重新排一次快照。

## 状态

当前证据等级 **E0/E1**：编译、lint、文档与纯逻辑测试通过；每个表面投影出的场景（颜色、尺寸、句子、点击目标）都有逐值断言，另有一个测试经离屏 QML 宿主真实渲染一帧并断言像素。

**摆放、帧与玻璃请求是 E2**：`src/headless_tests/protocol.rs` 用真实 `wayland-server` 在 socket pair 上扮演合成器，驱动本测试二进制作为子进程，断言它真的创建了 `wlr-layer-shell` 表面、真的按 `placement.rs` 的锚点/exclusive zone/layer/margin/尺寸提交了预乘 `Argb8888` 共享内存帧、真的在 layer role 之后以 `inox-glass-v1` **v2** 请求了液态玻璃（钳制过的模糊半径、窗格自身区域、四角半径与折射带；v1 合成器只收到半径与区域），以及合成器不支持 layer-shell 时以可读错误非零退出。

**窗口服务的传输已接线，证据只到 E1，且当前测试未全绿**：`src/window_service.rs` 的事件→消息表与失败映射由纯函数单元测试逐行断言（不需要合成器）。两处必须说清的边界：一是这条腿**还没有**进程级 peer 证据——`headless_tests/protocol.rs` 目前只扮演 layer-shell 与 `inox-glass-v1` 的合成器，尚未加入 `inox_window_manager_v1` 的 peer，「全局缺失时停在 `Unavailable` 且不退出」因此只有代码路径、没有进程级断言；二是最近一次本地 `cargo test -p inox-panel --all-features` 是 **344 通过 / 1 失败**（`contract::tests::a_workspace_switch_settles_only_its_own_identifier` 的断言写错了期望值），在修好并通过 CI 之前**不得**把这条腿读作 E1 已取得。**不得**把这条腿读作 D03 通过：D03 还要求真实客户端与 D-Bus 断言，E3 仍缺。

已交付：

- `src/tasks.rs` —— 窗口与任务分组模型，外加窗口模式与工作区视图。窗口身份是**服务端分配的 `WindowId` + 会话代次**，绝不使用 PID、标题或列表下标；分组按服务端给出的 `app_id`；激活态由「哪个窗口被报告为 focused」推导，至多一个；顺序稳定，无关窗口更新不会让按钮重排；应用退出后其窗口与分组一并消失。`WindowMode::{Tiled, Floating, Maximized, Fullscreen}` 与协议枚举一一对应（`from_protocol` / `code`），右键循环的顺序由 `cycled` 一处定义；`WorkspaceView::{Unknown, Known}` 只在服务端**报告过** `workspace_state` 之后才有值，`is_shown` 只在 `count > 1` 时为真。
- `src/window_service.rs` —— **窗口服务的传输腿**（`inox-window-manager-v1`）：协议事件 → 既有 `app::Message`、`OutgoingCommand` → 协议请求（映射表见上文）。纯半 `ListState` / `rejection` / `Failure` 有单元测试；`WindowService` 只是持有 `wayland_client` proxy 的那层薄壳，并在合成器没有该全局时提供一个**只记一次**的 `info!` 与 `ServiceState::Unavailable` 消息。
- `src/contract.rs` —— 交互契约的服务契约：快照/代次、`Connecting / Ready / Unavailable`、请求的 `Sent / Accepted / Applied / Rejected / TimedOut`，以及「同一窗口同时至多一个请求」的去重。工作区切换不命名窗口，因此占**独立的一个槽位**（`issue` / `pending_workspace` / `settle_workspace`），「至多一个切换在途」与「每个窗口至多一个请求」由两处互不干扰的簿记保证；`RejectionReason` 仍是四个变体。
- `src/tray.rs` —— StatusNotifierItem / DBusMenu 模型：注册（总线名 + 对象路径）、标题/图标/状态/关注/菜单更新、注销与退出，以及**按快照重连**（重建集合，不产生重复条目）。
- `src/clock.rs` —— 展开 `inox_config::CLOCK_DIRECTIVES` 的每个指令；`%a/%A/%b/%B` 与 `%p` 取自本 crate 的 Fluent 目录，而不是 chrono 的语言数据或硬编码英文。
- `src/placement.rs` —— layer-shell 摆放策略（锚点、exclusive zone、margin、高度、键盘交互、逐输出分配），**纯数据**；`src/layer.rs` 是唯一把它应用到协议上的地方。
- `src/frost.rs` —— 液态玻璃的**纯决策**，架在 `inox-glass` 之上：面板只管「哪些表面是窗格」（菜单栏与 Dock 是，任务栏不是）与「材质是否要效果」；只要模糊 **或** 折射之一被请求就创建对象。clamp、v1/v2 协商与降级理由都在 `inox_glass::decide`。合成器报 `max_radius = 0` 且折射上限也为 0 时画纯半透明并只记一条 `info!`；否则请求 `radius = min(blur, max_radius)`、窗格自身区域、`radii.md` 的四角半径与被钳制的折射带，尺寸或令牌变化时重发。
- `src/layer.rs` —— **协议腿**：`Connection::connect_to_env()` + `calloop` 事件循环，绑定 `wl_compositor` / `wl_shm` / `wl_seat` / `zwlr_layer_shell_v1`（必需，缺失即非零退出）、`inox_glass_manager_v1`（可选，缺失只降级；绑定版本是 `min(advertised, 2)`）与 `inox_window_manager_v1`（可选：缺失即 `ServiceState::Unavailable` + 重试，绝不非零退出）。`capabilities`（v1）与 `liquid_capabilities`（v2）都归一到 `inox_glass::Capabilities`，后者优先；请求经 `Request::apply` 发到线上，v1 对象自动跳过 `since=2` 的请求。一帧 = `view.rs` 的投影经离屏 QML 渲染出的 `Argb8888` 像素（预乘 `B,G,R,A`）；`wl_pointer` 的 enter/leave/motion/button 翻译成 `Pointer`、按场景的 `entries` 命中（左键激活/最小化、中键关闭、右键模式循环、工作区指示器切换），产生的 `Message` 由 `app::update` 应用；`app::State` 取出的 `OutgoingCommand` 由这里交给 `window_service` 发到线上（**不是**取出并丢弃）。`SIGINT`/`SIGTERM` 干净拆卸，`SIGHUP`/D-Bus 重载由 `src/reload.rs` 通过 ping 喂进来。信号源在任何线程（包括 Qt 宿主）创建**之前**装好掩码。
- `src/reload.rs` —— 会话 D-Bus（`org.inox.Panel`）与 `SIGHUP` 的重载主机；候选文档在另一个线程读取解析，经通道 + `calloop::ping` 交给 `layer.rs` 应用。`[panel] surface` 不采用候选值（那是另一个进程；`--surface` 已覆盖文件），保留运行中的表面并记一条 `warn!`，其余摆放字段热改并在下一个 commit 生效。
- `src/view.rs`、`src/view_model.rs`、`src/theme.rs`、`src/app.rs` —— 状态 → 视图模型 → **Qt-free 的 JSON 场景**（`{ font_family, background, items: [ ] }` + `entries`）、Elm 更新循环，以及设计令牌 → 场景颜色/尺寸的唯一转换处（`#AARRGGBB` 字符串与数值）。
- `qml/Main.qml` —— 纯渲染器：只读 `modelJson`，按 `items` 的绝对坐标画矩形、玻璃窗格（`GlassPane`）与文本。没有颜色字面量、像素字面量或用户可见英文字符串（迁移契约 §4）。
- **共享玻璃组件库**（`inox-glass` 的 `qml/`，ADR 0040 §3）：五个无状态组件（`GlassPane` / `GlassButton` / `GlassToggle` / `GlassSlider` / `GlassTabBar`）与目录页 `CatalogWindow.qml` 不是独立的 QML 模块，而是由本 crate 的 `build.rs` 用显式 `qml/<name>` 别名编进**自己的** qrc（跨 crate 的静态模块注册不可靠，KDAB/cxx-qt#770），因此它们和 `Main.qml` 同目录、可作为隐式类型使用。
  这些组件**用在生产界面上，不只是测试**：菜单栏与 Dock 的窗格在投影里是 `kind: "pane"` 的图元，`qml/Main.qml` 用它实例化 `GlassPane`（`tint` / `highlight` / `shade` / `hairline` / `radius` 全部来自 `theme::pane_style`），所以面板不再自己把窗格铺成一叠色带。
  离屏测试另外把组件目录页渲染成一帧并断言：窗格中心就是 `glass.tint` 令牌、圆角外完全透明、勾选开关的轨道是 accent 令牌、滑杆填充在 `value` 处结束——这是组件库的像素证据，`inox-glass` 自己不链接 Qt。
- `src/cxxqt_object.rs`、`src/offscreen.rs`、`src/offscreen.cpp` —— 唯一的 QML↔Rust 边界：`load(url)` / `setModel(json)` / `render(w, h)`，以及 `QGuiApplication` + `QQmlApplicationEngine` + `QQuickWindow::grabWindow()` 的 C++ 薄宿主。**帧必须带真实 alpha**：Qt 6.8 之前的 offscreen 插件 backing store 是 `QImage::Format_RGB32`，单次抓帧必然不透明，所以宿主首帧探测一次，之后用黑/白两次清屏抓帧按 `a = 255 - max(white - black)` 重建 alpha（预乘颜色取黑底那一帧），两条路径输出同一帧；`INOX_PANEL_OFFSCREEN_NO_ALPHA_GRAB` 是强制走重建路径的测试钩子。
- `src/headless_tests/` —— 原本的 `tests/*.rs`（crate 只能有 bin target，见迁移契约 §2.1）：`binary`（子进程跑本面板入口）、`catalog`（目录一致性 Q03）、`projection`（投影逐值断言）、`protocol`（真实 `wayland-server` 扮演合成器，E2，含 v2 请求与 v1 回退；窗口管理协议的 peer 尚未加入）、`qml`（经离屏宿主渲染真实像素：任务栏、Dock，以及 `inox-glass` 的组件目录页）。
- `i18n/en-US/panel.ftl`、`i18n/zh-CN/panel.ftl` —— 全部用户可见文案。
- `src/main.rs` —— 日志、配置与目录装配、`--surface <taskbar|menubar|dock>` 与 `--help`（经 `tracing` 输出），以及启动自检：没有可用的 Wayland 显示时以可读错误与非零退出结束，然后把控制权交给 `layer::run`。

未交付，且不声称：

| 项 | 原因 |
|----|------|
| **SNI（托盘）传输** | 窗口侧已由 `src/window_service.rs` 接到 `inox-window-manager-v1`，**托盘侧仍然没有传输**：`src/tray.rs` 的模型（注册、更新、注销、按快照重连）已落地并有测试，但没有任何进程把 StatusNotifierItem 事件喂进来，D-Bus 服务本身也不在本 crate 内。`app::Message` 的 `TraySnapshot` / `TrayRegistered` / `TrayUpdated` / `TrayUnregistered` 就是那条传输将来要喂的接口。 |
| **每个输出一个面板** | 协议腿只创建一个表面，并把输出交给合成器选择（`wlr-layer-shell` 的 null output）。一个输出一个面板需要枚举 `wl_output` 并按 `placement::assign` 建表面；当前由会话按输出各起一个进程代替。 |
| **E3 GUI 证据** | 需要真实硬件与支持 layer-shell 的合成器；CI 有 socket pair，因此 E2 可跑，E3 不能。 |
| **AT-SPI 语义导出** | 面板的 Qt 应用跑在 `offscreen` 平台上，没有 QPA 无障碍桥。QML 只画矩形与文本，语义树没有承载者；要导出 role/name/state/action，得先在 Rust 侧建模。|
| `%Z` 的时区缩写 | `clock` feature 下拿不到 `tzname`，因此 `%Z` 渲染为 `UTC` / `UTC+08:00` 这类 UTC 偏移（对没有缩写的时区也正确）。 |
| 时钟按格式降频 | 目前固定每秒一次唤醒（`%S` 必须准）。等 P03 的空闲预算说明这次唤醒有代价时再做。 |
| 任务区溢出指示 | 任务区是弹性区域，超出的按钮被裁掉；没有滚动、没有溢出指示。 |
| 图标渲染、托盘菜单弹出 | `tray` 已保留 `icon` 与菜单树，但面板目前只画文字；托盘菜单是后续的弹出层工作。 |
| tooltip | 视图模型带着 hover 文本（含禁用原因的说明，D05），但投影只画矩形与文本，没有承载 tooltip 的 popup 表面；离屏渲染器也收不到指针。tooltip 是另一件事，不声称已实现。 |

## 结构

| 文件 | 内容 |
|------|------|
| `src/tasks.rs` | 窗口 / 分组模型与窗口模式、工作区视图（纯逻辑，属性测试） |
| `src/tray.rs` | StatusNotifierItem 与 DBusMenu 模型（纯逻辑） |
| `src/clock.rs` | 时钟格式展开 |
| `src/contract.rs` | 服务状态、请求关联与去重（工作区切换占独立槽位） |
| `src/window_service.rs` | 窗口服务的传输腿：`inox-window-manager-v1` 事件 → `app::Message`、`OutgoingCommand` → 协议请求 |
| `src/placement.rs` | layer-shell 摆放策略（数据，由 `layer.rs` 应用） |
| `src/frost.rs` | 液态玻璃的客户端决策（纯，含降级表；clamp 与 v1/v2 协商在 `inox-glass`） |
| `src/layer.rs` | 协议腿：layer 表面、ARGB 共享内存帧、指针输入、`inox-glass-v1` v2 |
| `src/reload.rs` | `SIGHUP` 与 `org.inox.Panel` 的重载请求 |
| `src/view_model.rs` | 状态 → 视图的投影 |
| `src/view.rs` | Qt-free 的 JSON 场景（矩形 / 玻璃窗格 / 文本 / 点击区域） |
| `src/app.rs` | `State` / `Message` / `update`（Elm） |
| `src/theme.rs` | 设计令牌 → 场景的颜色与尺寸（玻璃窗格由 `pane_style` 给出 `tint` / `highlight` / `shade` / `hairline` / `radius` / 两面发丝开关） |
| `src/cxxqt_object.rs` | cxx-qt 桥（唯一的 QML↔Rust 边界） |
| `src/offscreen.rs` / `src/offscreen.cpp` / `src/offscreen.h` | 离屏 QML 宿主的 Rust 面与 C++ 实现 |
| `qml/Main.qml` | QML 渲染器（无颜色/尺寸/文案字面量；`kind: "pane"` 的图元实例化 `inox-glass` 的 `GlassPane`） |
| `src/main.rs` | 日志、自检、`--surface`/`--help`，然后 `layer::run` |
| `src/i18n.rs` | 内嵌的 Fluent 目录 |
| `i18n/<locale>/panel.ftl` | 文案 |
| `src/headless_tests/{binary,catalog,projection,protocol,qml,support}.rs` | bin 内测试（原 `tests/*.rs`） |
| `build.rs` | `CxxQtBuilder::new_qml_module`：QML 模块、桥、C++ 宿主，以及 `inox-glass` 的玻璃组件（显式 qrc 别名） |

`crates/<name>/AGENTS.md`（`CLAUDE.md` 是其符号链接）是本 crate 的约束文档。

## 开发约定

- 本项目遵循[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，本地命令只作自检。
- 颜色、尺寸、间距、圆角只来自 `inox_config::Theme` 令牌，并且只在 `src/theme.rs` 里转成场景值；QML 里不得出现颜色/像素字面量或用户可见文案。
- `inox-panel -> inox-glass` 是本 crate 唯一的同层依赖，由 ADR 0040 授权并登记在根 `Cargo.toml` 的 `same_layer_exceptions`：玻璃材质的 clamp、降级与 v1/v2 协商只在 `inox-glass` 实现一次。仓库里另有 `inox-notifications -> inox-glass`（ADR 0044），那是通知栈自己的边，与本 crate 无关。改这条边必须同时改那份登记，否则 `cargo test -p inox-integration --test layering` 变红。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 使用 `panel`。

| 目的 | 命令 |
|------|------|
| 静态检查（自检） | `CARGO_HOME=$PWD/.cargo-home cargo check -p inox-panel --all-targets --all-features --offline` |
| 静态检查（零警告） | `CARGO_HOME=$PWD/.cargo-home cargo clippy -p inox-panel --all-targets --all-features --offline -- -D warnings` |
| 测试 | `CARGO_HOME=$PWD/.cargo-home QT_QPA_PLATFORM=offscreen cargo test -p inox-panel --all-features --offline` |
| 协议腿（E2） | `QT_QPA_PLATFORM=offscreen cargo test -p inox-panel --all-features headless_tests::protocol` |
| 真实一帧（离屏 QML） | `QT_QPA_PLATFORM=offscreen cargo test -p inox-panel --all-features headless_tests::qml` |
| 格式化 | `cargo fmt -p inox-panel` |
| 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-panel --no-deps --all-features` |

## 许可证

LGPL-3.0，见仓库根的 [`LICENSE`](../../LICENSE)。
