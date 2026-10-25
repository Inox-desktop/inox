# inox-panel

Inox 桌面的面板：任务栏、系统托盘与时钟。属于 Inox 桌面环境的 **L3** 层（PRD §3.1；层级成员的唯一事实来源是根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

面板不拥有任何状态：窗口属于合成器，托盘项属于各自的应用程序。面板订阅它们，并把自己看到的东西画出来。

## 表面模式：真实 layer 表面 + 离屏 QML

面板**没有自己的窗口**。它持有的是真实的 `zwlr_layer_shell_v1` 表面（`src/layer.rs` 创建、摆放并提交帧），界面由 QML 渲染，但渲染目标是**离屏**的：

```text
calloop 主循环（layer.rs）
  └─ 状态变化 → view.rs 投影成一份 JSON（矩形 + 文本 + 点击区域）
       └─ offscreen.rs → cxx-qt 桥（cxxqt_object.rs）→ C++ 宿主（offscreen.cpp）
            └─ QGuiApplication(offscreen) + QQmlApplicationEngine → QQuickWindow::grabWindow()
                 └─ RGBA8 预乘像素 → write_argb() → wl_shm 的 Argb8888 帧 → 真正的 layer 表面
```

因此：进程里**没有** Qt Wayland 客户端、没有 `LayerShellQt`、没有 `xdg_toplevel`，也没有 Qt Wayland Compositor。输入的归属也不变：`wl_pointer` 的事件仍由 Rust 翻译成 `Pointer`，在 `src/layer.rs` 里按场景自带的 `entries`（点击区域）命中，产生既有的 `app::Message`；QML 只画，不接输入、不持状态。

任务区最多显示 `panel.max_task_buttons` 个按钮（默认 12）；任务区是弹性区域，超长标题在任务区末端被裁掉，时钟与托盘从右边缘向内排布，因此不会被挤出表面（D05）。

## 状态

当前证据等级 **E0/E1**：编译、lint、文档与纯逻辑测试通过；每个表面投影出的场景（颜色、尺寸、句子、点击目标）都有逐值断言，另有一个测试经离屏 QML 宿主真实渲染一帧并断言像素。

**摆放、帧与玻璃请求是 E2**：`src/headless_tests/protocol.rs` 用真实 `wayland-server` 在 socket pair 上扮演合成器，驱动本测试二进制作为子进程，断言它真的创建了 `wlr-layer-shell` 表面、真的按 `placement.rs` 的锚点/exclusive zone/layer/margin/尺寸提交了预乘 `Argb8888` 共享内存帧、真的在 layer role 之后用钳制过的半径请求了磨砂，以及合成器不支持 layer-shell 时以可读错误非零退出。

已交付：

- `src/tasks.rs` —— 窗口与任务分组模型。窗口身份是**服务端分配的 `WindowId` + 会话代次**，绝不使用 PID、标题或列表下标；分组按服务端给出的 `app_id`；激活态由「哪个窗口被报告为 focused」推导，至多一个；顺序稳定，无关窗口更新不会让按钮重排；应用退出后其窗口与分组一并消失。
- `src/tray.rs` —— StatusNotifierItem / DBusMenu 模型：注册（总线名 + 对象路径）、标题/图标/状态/关注/菜单更新、注销与退出，以及**按快照重连**（重建集合，不产生重复条目）。
- `src/clock.rs` —— 展开 `inox_config::CLOCK_DIRECTIVES` 的每个指令；`%a/%A/%b/%B` 与 `%p` 取自本 crate 的 Fluent 目录，而不是 chrono 的语言数据或硬编码英文。
- `src/contract.rs` —— 交互契约的服务契约：快照/代次、`Connecting / Ready / Unavailable`、请求的 `Sent / Accepted / Applied / Rejected / TimedOut`，以及「同一窗口同时至多一个请求」的去重。
- `src/placement.rs` —— layer-shell 摆放策略（锚点、exclusive zone、margin、高度、键盘交互、逐输出分配），**纯数据**；`src/layer.rs` 是唯一把它应用到协议上的地方。
- `src/frost.rs` —— 磨砂玻璃的**纯决策**：`glass.blur == 0` 不创建对象；`capabilities` 的 `max_radius == 0` 时画纯半透明并只记一条 `info!`；否则 `radius = min(blur, max_radius)`、region 是窗格自身区域，尺寸变化时重发。
- `src/layer.rs` —— **协议腿**：`Connection::connect_to_env()` + `calloop` 事件循环，绑定 `wl_compositor` / `wl_shm` / `wl_seat` / `zwlr_layer_shell_v1`（必需，缺失即非零退出）与 `inox_glass_manager_v1`（可选，缺失只降级）；一帧 = `view.rs` 的投影经离屏 QML 渲染出的 `Argb8888` 像素（预乘 `B,G,R,A`）；`wl_pointer` 的 enter/leave/motion/button 翻译成 `Pointer` 并按场景的 `entries` 命中，产生的 `Message` 由 `app::update` 应用；`SIGINT`/`SIGTERM` 干净拆卸，`SIGHUP`/D-Bus 重载由 `src/reload.rs` 通过 ping 喂进来。信号源在任何线程（包括 Qt 宿主）创建**之前**装好掩码。
- `src/reload.rs` —— 会话 D-Bus（`org.inox.Panel`）与 `SIGHUP` 的重载主机；候选文档在另一个线程读取解析，经通道 + `calloop::ping` 交给 `layer.rs` 应用。`[panel] surface` 不采用候选值（那是另一个进程；`--surface` 已覆盖文件），保留运行中的表面并记一条 `warn!`，其余摆放字段热改并在下一个 commit 生效。
- `src/view.rs`、`src/view_model.rs`、`src/theme.rs`、`src/app.rs` —— 状态 → 视图模型 → **Qt-free 的 JSON 场景**（`{ font_family, background, items: [ ] }` + `entries`）、Elm 更新循环，以及设计令牌 → 场景颜色/尺寸的唯一转换处（`#AARRGGBB` 字符串与数值）。
- `qml/Main.qml` —— 纯渲染器：只读 `modelJson`，按 `items` 的绝对坐标画矩形与文本。没有颜色字面量、像素字面量或用户可见英文字符串（迁移契约 §4）。
- `src/cxxqt_object.rs`、`src/offscreen.rs`、`src/offscreen.cpp` —— 唯一的 QML↔Rust 边界：`load(url)` / `setModel(json)` / `render(w, h)`，以及 `QGuiApplication` + `QQmlApplicationEngine` + `QQuickWindow::grabWindow()` 的 C++ 薄宿主。**帧必须带真实 alpha**：Qt 6.8 之前的 offscreen 插件 backing store 是 `QImage::Format_RGB32`，单次抓帧必然不透明，所以宿主首帧探测一次，之后用黑/白两次清屏抓帧按 `a = 255 - max(white - black)` 重建 alpha（预乘颜色取黑底那一帧），两条路径输出同一帧；`INOX_PANEL_OFFSCREEN_NO_ALPHA_GRAB` 是强制走重建路径的测试钩子。
- `src/headless_tests/` —— 原本的 `tests/*.rs`（crate 只能有 bin target，见迁移契约 §2.1）：`binary`（子进程跑本面板入口）、`catalog`（目录一致性 Q03）、`projection`（投影逐值断言）、`protocol`（真实 `wayland-server` 扮演合成器，E2）、`qml`（经离屏宿主渲染一帧真实像素）。
- `i18n/en-US/panel.ftl`、`i18n/zh-CN/panel.ftl` —— 全部用户可见文案。
- `src/main.rs` —— 日志、配置与目录装配、`--surface <taskbar|menubar|dock>` 与 `--help`（经 `tracing` 输出），以及启动自检：没有可用的 Wayland 显示时以可读错误与非零退出结束，然后把控制权交给 `layer::run`。

未交付，且不声称：

| 项 | 原因 |
|----|------|
| **窗口服务与 SNI 传输** | `inox-ipc` / D-Bus 的协议腿不在本范围内。面板启动后停在 `Starting`，把意图放进 `app::OutgoingCommand` 队列；`app::Message` 就是未来传输层要喂进来的接口，`layer.rs` 把队列取出并丢弃（一次 `info!` 说明没有传输可送）。 |
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
| `src/tasks.rs` | 窗口 / 分组模型（纯逻辑，属性测试） |
| `src/tray.rs` | StatusNotifierItem 与 DBusMenu 模型（纯逻辑） |
| `src/clock.rs` | 时钟格式展开 |
| `src/contract.rs` | 服务状态、请求关联与去重 |
| `src/placement.rs` | layer-shell 摆放策略（数据，由 `layer.rs` 应用） |
| `src/frost.rs` | 磨砂玻璃的客户端决策（纯，含降级表） |
| `src/layer.rs` | 协议腿：layer 表面、ARGB 共享内存帧、指针输入 |
| `src/reload.rs` | `SIGHUP` 与 `org.inox.Panel` 的重载请求 |
| `src/view_model.rs` | 状态 → 视图的投影 |
| `src/view.rs` | Qt-free 的 JSON 场景（矩形 / 文本 / 点击区域） |
| `src/app.rs` | `State` / `Message` / `update`（Elm） |
| `src/theme.rs` | 设计令牌 → 场景的颜色与尺寸 |
| `src/cxxqt_object.rs` | cxx-qt 桥（唯一的 QML↔Rust 边界） |
| `src/offscreen.rs` / `src/offscreen.cpp` / `src/offscreen.h` | 离屏 QML 宿主的 Rust 面与 C++ 实现 |
| `qml/Main.qml` | QML 渲染器（无颜色/尺寸/文案字面量） |
| `src/main.rs` | 日志、自检、`--surface`/`--help`，然后 `layer::run` |
| `src/i18n.rs` | 内嵌的 Fluent 目录 |
| `i18n/<locale>/panel.ftl` | 文案 |
| `src/headless_tests/{binary,catalog,projection,protocol,qml,support}.rs` | bin 内测试（原 `tests/*.rs`） |
| `build.rs` | `CxxQtBuilder::new_qml_module`：QML 模块、桥与 C++ 宿主 |

`crates/<name>/AGENTS.md`（`CLAUDE.md` 是其符号链接）是本 crate 的约束文档。

## 开发约定

- 本项目遵循[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，本地命令只作自检。
- 颜色、尺寸、间距、圆角只来自 `inox_config::Theme` 令牌，并且只在 `src/theme.rs` 里转成场景值；QML 里不得出现颜色/像素字面量或用户可见文案。
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
