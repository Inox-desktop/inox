# AGENTS.md — inox-panel

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

面板：任务栏、菜单栏与 Dock，以及系统托盘与时钟（PRD §3.1）。所属层级：**L3**。它不拥有窗口状态、也不拥有托盘状态：窗口状态来自合成器，托盘项来自各自的应用程序，面板只订阅并呈现（ADR 0013）。

## crate 特有约束

- **`unsafe` 的唯一例外是 cxx-qt 生成的胶水。** `#[cxx_qt::bridge]`（`src/cxxqt_object.rs`）会展开出 `unsafe extern "C++"`，无法在 `forbid(unsafe_code)` 下编译，因此本 crate 的 `[lints.rust]` 把它放宽为 `allow`（ADR 0035 的例外）。**Inox 手写的 Rust 仍然不写 `unsafe`**；本 crate 不使用 Qt 私有头文件或私有 Qt Quick API。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`；默认级别 `info`，不得每帧打印。
- **crate 只能有 bin target。** cxx-qt 0.10 把桥的初始化符号放进「声明桥的那个 target」，被 `--gc-sections` 丢掉的静态库会链接失败（KDAB/cxx-qt#770）。因此没有 `src/lib.rs`：模块全挂在 `src/main.rs`，原本的 `tests/*.rs` 迁到 `src/headless_tests/` 的 `#[cfg(test)]` 模块（迁移契约 §2.1、§2.2）。测试要驱动进程时，就把**测试二进制自己**作为子进程再跑一遍（`headless_tests::binary`）。
- **窗口身份只来自服务端**：`WindowKey` = 服务端分配的 `WindowId` + `SessionGeneration`。禁止用 PID、标题或列表下标识别窗口，也禁止用它们做分组（ADR 0013 决策 1）。分组只按服务端给出的 `app_id`。
- **代次不匹配的状态一律丢弃**：跨代次的窗口状态返回 `Applied::Stale`，代次变化必须先 `TaskList::resync`，不得把两个会话拼在一起。
- **颜色/尺寸/圆角/动效只来自令牌**（`inox_config::Theme`）：`src/theme.rs` 是唯一把令牌变成场景值的地方（QML 颜色用 `#AARRGGBB`，尺寸/圆角/字号是数值），`src/view.rs` 只调用它，`qml/Main.qml` 只绑定。文字尺寸由面板高度与 `typography.scale` 推导（令牌集没有字号），不得写死像素。
- **QML 是纯渲染器。** `qml/Main.qml` 只读 `bridge.modelJson`：没有颜色字面量、没有像素字面量、没有用户可见英文字符串（迁移契约 §4）；它不接输入、不持状态、不做决策。`wm` 之外的边界只有 `load` / `setModel` / `render` 三个调用。
- **投影是 Qt-free 的 JSON。** `src/view.rs` 把「状态 + 令牌 + Fluent 文案」投影成 `{ font_family, background, items: [ … ] }` 加上可点击的 `entries`，每次状态变化整体重发（迁移契约 §3）。不得为每个字段加 `#[qproperty]`，也不得让 QML 自己拼句子或挑颜色。
- **离屏宿主不得变成窗口。** `src/offscreen.cpp` 只创建 `QGuiApplication`（**offscreen** 平台、**software** 场景图）+ `QQmlApplicationEngine` + `QQuickWindow`，再用 `grabWindow()` 取帧；不打开 Qt Wayland 连接、不创建 `xdg_toplevel`、不引入 `LayerShellQt`、不引入 Qt Wayland Compositor（GPL-only，ADR 0033）。宿主起不来是可读错误 + 非零退出，**没有**退回普通窗口的路径。
- **三种表面是同一份数据的三种呈现**：`[panel] surface` 决定进程画哪一种（`taskbar` / `menubar` / `dock`），三者共用 `view_model` 的投影与 `tasks` / `tray` / `clock` 模型；菜单栏与 Dock 只各自新增几何与排布（`src/view/menubar.rs`、`src/view/dock.rs`），不得新增窗口/托盘/时钟数据。运行中不得改变 `surface`：那是另一个窗口，重载**不得采用候选文档里的值**，必须保留运行中的表面并记一条 `warn!`（`Message::ConfigReloaded` 同理），而不是让整次重载失败。
- **菜单栏没有应用菜单可画**：Inox 没有应用菜单协议，菜单栏只显示焦点应用名（`show_app_name`）、面板自身的状态与重试、托盘项与时钟；**禁止**编造 "File"/"Edit"/"View" 之类的菜单标题。这条必须同时留在 `src/menubar.rs` 的模块文档里。
- **玻璃材质只来自 `glass` 令牌**（`tint` / `tint_strong` / `highlight` / `shade` / `blur`）：`src/theme.rs` 是唯一把它们变成绘制值的地方（`glass_stops` 分档、`glass_tint` / `glass_highlight` / `glass_shade`），组件不得自选颜色或透明度。窗格是「一条 highlight 发丝 + tint 主体（+ 一条 shade 发丝）」的分档矩形，不是渲染器里的渐变；两道发丝边是窗格自身的一部分，**不得**用占满宽度的子控件去画。背景模糊由合成器绘制，`glass_blur` 只是请求值：面板不得声称自己画了模糊。
- **Dock 的图块是徽标，不是图标**：面板没有图标主题解码器，也不得为此新增依赖；图块用应用名首字符，没有名字用 `panel-dock-icon-placeholder`（与启动器 `row_widget` 同类的降级）。图块是条目（`button` 语义），消息与任务栏一致（`TaskActivated` / `TaskToggled`），数量受既有 `panel.max_task_buttons` 限制，溢出不滚动、不加溢出指示。图块的点击区域覆盖徽标与其下方活动标记槽位，不得只盖住其中一半。
- **玻璃表面的背景是结构性透明**：三个表面的根背景都是 `theme::TRANSPARENT`（`#00000000`，结构性哨兵，不是设计颜色）；菜单栏与 Dock 的玻璃窗格画在它上面，任务栏则自己画不透明的 `surface` 填充，其投影不得因 `glass` 令牌改变一个值（有测试断言）。
- **Dock 的表面宽度必须覆盖它会画的一切**：`DockContent { tiles, tray_width, notice_width }` 是宽度的唯一输入（`src/dock.rs` → `DockGeometry::surface_width` → `PanelPlacement::dock(config, tokens, content)`），托盘宽度与句子宽度都是**字符估算**（`AVERAGE_ADVANCE_RATIO`，与启动器 `label_budget` 同一类做法；**不得**为此引入字体度量依赖），句子另有 `minimum_width` 保底槽位，因此没有窗口服务时空 dock 也必须是一个可读的居中胶囊。窗格宽度必须等于表面宽度：合成器磨砂的是整个 configure 区域（`headless_tests::protocol` 断言 `set_region == (0,0,width,height)`）。字符估算是估算而非度量：CJK 托盘标签可能被低估、更宽的字体可能裁掉最后的字形，这两条是已知的、被记录的残余误差。
- **文案只来自 Fluent**：只能出现消息 id，不得在代码里写用户可见字符串（产品名与组件名除外）。新增 id 必须同时进 `en-US` 与 `zh-CN`，占位符保持一致（`headless_tests::catalog` 断言）。
- **摆放策略是数据，但 `src/layer.rs` 会把它应用到协议上**：`src/placement.rs` 计算锚点/layer/exclusive zone/margin/尺寸/键盘交互，`src/layer.rs` 是唯一把这些值变成 `zwlr_layer_surface_v1` 请求的地方（`anchor_bits` / `layer_of` / `keyboard_of` / `surface_size`）。**禁止**在别处再算一遍锚点或尺寸。
- **没有 layer 宿主就是失败，不是降级**：`wl_compositor`、`wl_shm`、`zwlr_layer_shell_v1` 三者缺一，`layer::run` 返回可读的 `LegError::MissingGlobal`，`main` 记录并以非零码退出（ADR 0018）。`inox_glass_manager_v1` 可选：拿不到就一次 `info!` 说明，画纯半透明；没有 `wl_seat` 或该 seat 没有 pointer capability 同理（一次 `info!`，继续画、不接受点击，**绝不 panic**）。
- **协议腿自己建连、自己画帧、自己收指针**：`src/layer.rs` 用 `Connection::connect_to_env()` + `calloop` 事件循环（与 `inox-lock/src/session.rs` 同一套模式：`wl_registry` 收集 globals、`memfd_create` + `wl_shm` 帧、`wl_buffer.release` 记账、`MAX_FRAMES_IN_FLIGHT` 上限、`Generic` 连接源）。帧是离屏 QML 在 configure 尺寸下渲染、再经 `write_argb` 换过通道的像素，格式**必须是 `Argb8888`**（窗格半透明；`Xrgb8888` 会画成不透明方块），字节序为**预乘** `B, G, R, A`（Qt 的 `Format_RGBA8888_Premultiplied` 已经是预乘；写成非预乘会在圆角与字形周围出现光晕）。根背景必须是结构性透明，否则圆角外会是一整块颜色。
- **只提交与 configure 同尺寸的帧**：ack 必须在第一次 attach 之前；`set_size` 之后要等下一次 configure 再画；尺寸不是本组件会画的（0 或超过 `MAX_FRAME_BYTES`）就不提交，`Input::resize` 与 `Interface::resize` 都拒绝这种尺寸。重绘只在有变化时发生（状态变了、configure、指针交互、glass 参数需要下一次 commit 生效、`wl_buffer.release`）——时钟是 `calloop::timer` 喂同一个 `app::Message::Tick`，不得自建第二个时钟，也不得每帧重画。
- **指针输入在 Rust 里命中，不经 QML**：`wl_pointer` 的 enter/leave/motion/button 翻译成 `Pointer`（纯数据），经 `layer::Interface::pointer` 交给 `layer::Input`，按**最近提交的那一帧的场景**（`Input::scene`，由 `render_argb` 写入）里的 `entries` 命中，产生的 `Message` 由 `app::update` 应用；坐标按表面 scale factor 化成逻辑坐标（`logical`），`SCALE_FACTOR` 是唯一的缩放因子。点击任务图块必须产生与投影测试相同的 `Message::TaskActivated` / `TaskToggled`。
- **玻璃请求由 `src/frost.rs` 决策、由 `layer.rs` 发送**：`glass.blur == 0` → 完全不创建 glass 对象（`asks_for_glass`）；`capabilities` 的 `max_radius == 0` → 纯半透明 + **一条** `info!`；`max_radius > 0` → `radius = min(blur, max_radius)`，region 是窗格自身区域，configure 尺寸变化时重发。glass 对象必须在 layer role 存在**之后**创建（ADR 0031），拆卸时先 `destroy()` 它。任务是**不透明条带**，不是窗格：任务栏不创建 glass 对象。
- **启动必须自检**：没有可用的 Wayland 显示时，`main` 必须以可读错误与非零退出码结束（`DisplayEnv`，ADR 0018）；`zwlr_layer_shell_v1` 的存在性查询由 `layer::run` 在绑定 globals 后完成，缺失即 `LegError::MissingGlobal` + 非零退出。
- **窗口服务与 SNI 传输不在本 crate 内**：两者的协议仍不实现。配置重载由 `src/reload.rs` 通过会话 D-Bus（`org.inox.Panel`）与 `SIGHUP` 接入（ADR 0026），把 `Request` 放进通道并用 `calloop::ping::Ping` 唤醒 `layer.rs` 的循环。**SIGINT / SIGTERM / SIGHUP 的信号源由 `layer.rs` 持有，且必须在任何线程（含离屏 Qt 宿主）创建前装好掩码**：`signalfd` 只收到「在所有线程里都被阻塞」的信号。测试子进程同样依赖这一点：`headless_tests` 在 spawn 之前先在父线程阻塞这些信号，让子进程经 `exec` 继承掩码（`signalfd` 才能收到），**不要**把「子进程跑在 main 线程」当作前提。`update` 不做 I/O，只把意图放进 `State` 的 `OutgoingCommand` 队列；`layer.rs` 把队列取出并丢弃（一次 `info!` 说明没有传输）。新增 `Message` 必须同时给出测试。
- **任务区是弹性区域**：任务按钮按顺序排布并各自裁剪，时钟与托盘项从右边缘向内排布，因此长标题只会被裁掉，不会把时钟挤出表面。显示数量受 `panel.max_task_buttons` 限制，不得用滚动或溢出指示替代（尚未实现）。
- **普通展示不夺焦点**：`keyboard_interactivity = None`；时钟与托盘项是纯展示，不进入 `entries`。显式导航焦点只能落在可点击（enabled）的条目标上，且至多一个（D01）。
- 新增依赖前先确认依赖方向合法（根级 §2）：本 crate 依赖 L1（`inox-common`、`inox-config`）以及一条**向下的 L3 → L2 边**（`inox-protocols`，ADR 0006 的协议单一出口；ADR 0002 允许向下依赖，ADR 0013 只禁止同层与合成器依赖）。为协议腿新增：`wayland-client`、`rustix`；为界面新增：`cxx` / `cxx-qt` / `cxx-qt-lib` / `cxx-qt-build`、`serde_json`。**不得**引入同层依赖、`inox-compositor` 或任何 Qt Wayland 模块；协议绑定也不得绕过 `inox-protocols` 直接依赖 `wayland-protocols*`。改依赖后跑 `cargo test -p inox-integration --test layering`。

## 主要依赖

- 上游 Inox crate：`inox-common`（i18n 目录加载）、`inox-config`（shell 配置、设计令牌）、`inox-protocols`（`wlr-layer-shell`、核心 `wl_*` 与 `inox-glass-v1` 的绑定；L3 → L2 的合法向下边）。
- 界面：`cxx` / `cxx-qt` / `cxx-qt-lib`（QML ↔ Rust 桥，ADR 0035）与构建期 `cxx-qt-build`（QML 模块 + C++ 宿主）。
- 第三方：`serde_json`（投影的序列化）、`calloop`（事件循环、信号源与时钟）、`wayland-client`（连接、globals 与指针）、`rustix`（`memfd_create` 共享内存帧）、`chrono`（本地时间）、`zbus`（`org.inox.Panel` 重载端点）、`thiserror`、`tracing`、`tracing-subscriber`、`anyhow`（仅 `main`）。
- 测试：`wayland-server`（协议测试扮演合成器一侧）、`tempfile`、`proptest`。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。`CARGO_HOME` 指向仓库内缓存，`--offline` 避免网络。

| 目的 | 命令 |
|------|------|
| 自检 | `CARGO_HOME=$PWD/.cargo-home cargo check -p inox-panel --all-targets --all-features --offline` |
| 静态检查 | `CARGO_HOME=$PWD/.cargo-home cargo clippy -p inox-panel --all-targets --all-features --offline -- -D warnings` |
| 本 crate 测试 | `CARGO_HOME=$PWD/.cargo-home QT_QPA_PLATFORM=offscreen cargo test -p inox-panel --all-features --offline` |
| 协议腿（E2） | `QT_QPA_PLATFORM=offscreen cargo test -p inox-panel --all-features headless_tests::protocol` |
| 真实一帧（离屏 QML） | `QT_QPA_PLATFORM=offscreen cargo test -p inox-panel --all-features headless_tests::qml` |
| 格式化 | `cargo fmt -p inox-panel` |
| 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-panel --no-deps --all-features` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`），测试代码不受「禁止 unwrap/expect/panic」约束。
- **属性测试**（`proptest!`，`ProptestConfig::with_cases(1000)`）覆盖任务模型：
  - 分组是划分——没有窗口丢失或重复，分组内窗口的 `app_id` 与分组一致；
  - 至多一个窗口处于 focused（因此每组至多一个激活项）；
  - 每组的最后一个窗口离开后分组消失；
  - 顺序稳定：无关窗口更新不改变任何其它窗口的相对次序；
  - 每一步之后都断言 `order` 与窗口集合一致（无重复、无幽灵）。
- 托盘：`register` 幂等、未注册条目的事件被拒绝（`TrayError::UnknownItem`）、菜单项可跨子菜单查找、**同一快照应用两次不产生任何变化**、快照删除已消失条目并加入新条目。
- 时钟：`CLOCK_DIRECTIVES` 的**每个**指令都被投影，`%Q` 之类报错（含位置），结尾孤立 `%` 报错，zh-CN 的星期/月份与 `%p` 与 en-US 不同，`%Z` 为 UTC 偏移。
- **投影测试**（`headless_tests::projection`）取代了旧的像素渲染测试：断言每个表面的场景值——背景等于 `surface` 令牌、启用与禁用条目不同（禁用的没有填充也没有描边）、超长标题后时钟仍在右边缘内、点击任务按钮产生 `Message::TaskActivated` 且只产生一次、玻璃表面用的确实是半透明的 `glass.tint` 且不是不透明 `surface`、顶边有 highlight 发丝、Dock 浮在底部 margin 之上、任务栏的投影**不因 `glass` 令牌改变一个值**、场景里每个颜色都是 `#AARRGGBB`（即令牌转换结果）。
- **QML 加载/渲染测试**（`headless_tests::qml`）：在 `QT_QPA_PLATFORM=offscreen` 子进程里，经与二进制相同的 `layer::Interface` 真实渲染任务栏与 Dock 各一帧，断言帧长等于 configure、首像素等于 `surface` 令牌的预乘 `B,G,R,A`、Dock 的 margin 条带透明且玻璃确实半透明，并断言「画出来的场景」就是「指针命中用的场景」。
- `headless_tests::catalog`：`compare_catalogs(en-US, zh-CN)` 必须干净（Q03），id 前缀为 `panel-` 且不含点号，未翻译的 id 必须回退到默认语言。
- `headless_tests::binary`：把测试二进制自身作为子进程跑本面板入口，覆盖 `--surface` / `--help` / 未知参数 / 没有显示时的非零退出。
- `headless_tests::protocol` 用真实 `wayland-server` 在 socket pair 上扮演合成器并驱动本二进制，必须断言：按 `[panel] surface` 创建的 layer surface 的**锚点 / layer / exclusive zone / margin / 键盘交互 / namespace / 尺寸**与 `placement.rs` 的数据一致；`get_glass` **晚于** layer role，且 `set_radius` 已按 `capabilities` 钳制、`set_region` 是窗格尺寸，configure 尺寸变化时重发；提交的帧声明的尺寸等于 configure、字节是预乘 `Argb8888`；`max_radius = 0` 时仍然提交帧且不发 radius/region；`glass.blur = 0` 时**不创建** glass 对象；合成器不广告 `zwlr_layer_shell_v1` 时二进制以非零码退出并在日志里点名该协议；`SIGHUP` 重载会按新尺寸重新 configure 并提交新帧。
- 覆盖率目标：核心纯逻辑（`tasks`、`tray`、`clock`、`placement`、`view_model`、`contract`、`frost`、`view`、`theme`）≥70%（E1）。

## 目录结构

```
src/main.rs            crate 文档、模块声明、日志、自检、--surface/--help，然后 layer::run
src/tasks.rs           窗口/任务分组模型（纯）
src/tray.rs            StatusNotifierItem 与 DBusMenu 模型（纯）
src/clock.rs           时钟格式展开（纯 + Fluent）
src/contract.rs        服务状态、请求关联与去重（纯）
src/placement.rs       layer-shell 摆放策略（数据，由 layer.rs 应用）
src/view_model.rs      状态 → 视图投影（纯）
src/menubar.rs         菜单栏几何与投影（纯）
src/dock.rs            Dock 几何与投影（纯）
src/app.rs             State / Message / update（Elm）
src/view.rs            Qt-free 的 JSON 场景、几何与 entries
src/view/menubar.rs    菜单栏的场景
src/view/dock.rs       Dock 的场景
src/theme.rs           令牌 → 场景颜色与尺寸（含 glass 分档）
src/frost.rs           磨砂玻璃的客户端决策（纯，含降级表）
src/layer.rs           协议腿：wlr-layer-shell 表面、ARGB 共享内存帧、指针输入
src/reload.rs          SIGHUP 与 org.inox.Panel 的重载请求
src/cxxqt_object.rs    cxx-qt 桥（唯一 QML↔Rust 边界）
src/offscreen.rs       离屏宿主的 Rust 面
src/offscreen.cpp/h    C++ 宿主：QGuiApplication + QQmlApplicationEngine + grabWindow
src/i18n.rs            内嵌 Fluent 目录
qml/Main.qml           QML 渲染器
i18n/en-US/panel.ftl, i18n/zh-CN/panel.ftl
src/headless_tests/    binary / catalog / projection / protocol / qml / support
build.rs               CxxQtBuilder：QML 模块 + 桥 + C++ 宿主
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `panel`。
