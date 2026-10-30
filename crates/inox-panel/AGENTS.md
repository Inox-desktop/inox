# AGENTS.md — inox-panel

本 crate 遵循根仓库的**全部硬性规则**（[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)），本文件只补充本 crate 特有内容，不放宽任何硬性规则。

## 职责

面板：任务栏、菜单栏与 Dock，以及系统托盘与时钟（PRD §3.1）。所属层级：**L3**。它不拥有窗口状态、也不拥有托盘状态：窗口状态来自合成器，托盘项来自各自的应用程序，面板只订阅并呈现。

## crate 特有约束

- **`unsafe` 的唯一例外是 cxx-qt 生成的胶水。** `#[cxx_qt::bridge]`（`src/cxxqt_object.rs`）会展开出 `unsafe extern "C++"`，无法在 `forbid(unsafe_code)` 下编译，因此本 crate 的 `[lints.rust]` 把它放宽为 `allow`。**Inox 手写的 Rust 仍然不写 `unsafe`**；本 crate 不使用 Qt 私有头文件或私有 Qt Quick API。**唯一例外是 ADR 0046 授权的表面迁移（P3）**：把 `wl_surface` 变成 `inox_shell_surface_v1` 必须经 QtWaylandClient 的私有 ABI，那份代码只允许存在于 `src/native/qt_shell_surface.cpp` 一个翻译单元里，且必须同时带编译期 `QT_VERSION` 闸门、运行期版本检查与「失败即非零退出」；私有类型不得出现在任何 `.h` 或 Rust 边界上。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`；默认级别 `info`，不得每帧打印。
- **crate 只能有 bin target。** cxx-qt 0.10 把桥的初始化符号放进「声明桥的那个 target」，被 `--gc-sections` 丢掉的静态库会链接失败（KDAB/cxx-qt#770）。因此没有 `src/lib.rs`：模块全挂在 `src/main.rs`，原本的 `tests/*.rs` 迁到 `src/headless_tests/` 的 `#[cfg(test)]` 模块（迁移契约 §2.1、§2.2）。测试要驱动进程时，就把**测试二进制自己**作为子进程再跑一遍（`headless_tests::binary`）。
- **窗口身份只来自服务端**：`WindowKey` = 服务端分配的 `WindowId` + `SessionGeneration`。禁止用 PID、标题或列表下标识别窗口，也禁止用它们做分组。分组只按服务端给出的 `app_id`。
- **代次不匹配的状态一律丢弃**：跨代次的窗口状态返回 `Applied::Stale`，代次变化必须先 `TaskList::resync`，不得把两个会话拼在一起。
- **颜色/尺寸/圆角/动效只来自令牌**（`inox_config::Theme`）：`src/theme.rs` 是唯一把令牌变成场景值的地方（QML 颜色用 `#AARRGGBB`，尺寸/圆角/字号是数值），`src/view.rs` 只调用它，`qml/Main.qml` 只绑定。文字尺寸由面板高度与 `typography.scale` 推导（令牌集没有字号），不得写死像素。
- **QML 是纯渲染器。** `qml/Main.qml` 只读 `bridge.modelJson`：没有颜色字面量、没有像素字面量、没有用户可见英文字符串（迁移契约 §4）；它不接输入、不持状态、不做决策。`wm` 之外的边界只有 `load` / `setModel` / `render` 三个调用。**ADR 0046 的 P3 会放宽「不接输入」这一条**（原生表面之后指针事件只在 Qt 里），但放宽的只有**坐标**：文案、颜色、尺寸、圆角、材质、焦点顺序与动作语义仍全部来自 Rust 投影，QML 只把命中的矩形映射回一个 `id` 并 `dispatch`（ADR 0046 §5）。在 P3 落地前，本条文按现状执行。
- **投影是 Qt-free 的 JSON。** `src/view.rs` 把「状态 + 令牌 + Fluent 文案」投影成 `{ font_family, background, items: [ … ] }` 加上可点击的 `entries`，每次状态变化整体重发（迁移契约 §3）。不得为每个字段加 `#[qproperty]`，也不得让 QML 自己拼句子或挑颜色。
- **离屏宿主现在是过渡态，P3 之后整个删除。** 今天 `src/offscreen.cpp` 只创建 `QGuiApplication`（**offscreen** 平台、**software** 场景图）+ `QQmlApplicationEngine` + `QQuickWindow`，再用 `grabWindow()` 取帧；它不打开 Qt Wayland 连接、不创建 `xdg_toplevel`、不引入 `LayerShellQt`、不引入 Qt Wayland Compositor。宿主起不来是可读错误 + 非零退出，**没有**退回普通窗口的路径。**ADR 0046 的 P3 起这条反过来**：面板由 crate 内的 C++ 集成（`src/native/qt_shell_surface.cpp`）经 Qt 的 wayland 连接拿到**唯一**的表面，并向合成器申请 `inox_shell_surface_v1` 角色（ADR 0046 §10 授权）。两条路径**不得**在同一个二进制里靠运行时开关共存——那是两条都缺测试的写法；离屏路径在 P3 的同一个 PR 里删除，而不是留着「以防万一」。
- **帧必须带真实 alpha，取不到就重建。** Qt 6.8 之前 offscreen 插件的 backing store 硬编码为 `QImage::Format_RGB32`（`qoffscreencommon.cpp`），任何 `QSurfaceFormat` / `setDefaultAlphaBuffer` / 窗口标志都改不了，单次 `grabWindow()` 必然不透明。因此宿主要先按正常路径取一帧，**首帧**发现没有 alpha 通道就缓存该判定，此后每帧用黑/白两次清屏抓帧重建：`a = 255 - max(white - black)`，预乘颜色取黑底那一帧（推导与舍入见 `src/offscreen.cpp` 的 `reconstruct_alpha`）。两条路径输出同一帧，`src/layer.rs` 不得分支；重建是宿主的事，不是协议腿的事。`INOX_PANEL_OFFSCREEN_NO_ALPHA_GRAB` 是测试钩子，强制走重建路径，生产环境不设。
- **三种表面是同一份数据的三种呈现**：`[panel] surface` 决定进程画哪一种（`taskbar` / `menubar` / `dock`），三者共用 `view_model` 的投影与 `tasks` / `tray` / `clock` 模型；菜单栏与 Dock 只各自新增几何与排布（`src/view/menubar.rs`、`src/view/dock.rs`），不得新增窗口/托盘/时钟数据。运行中不得改变 `surface`：那是另一个窗口，重载**不得采用候选文档里的值**，必须保留运行中的表面并记一条 `warn!`（`Message::ConfigReloaded` 同理），而不是让整次重载失败。
- **菜单栏没有应用菜单可画**：Inox 没有应用菜单协议，菜单栏只显示焦点应用名（`show_app_name`）、面板自身的状态与重试、托盘项与时钟；**禁止**编造 "File"/"Edit"/"View" 之类的菜单标题。这条必须同时留在 `src/menubar.rs` 的模块文档里。
- **玻璃材质只来自 `glass` 令牌**（`tint` / `tint_strong` / `highlight` / `shade` / `blur` / `refraction` / `refraction_amount` / `depth` / `dispersion`）：`src/theme.rs` 是唯一把它们变成绘制值与请求值的地方（`pane_style(tokens, strong, radius, shaded) -> PaneStyle` 给出 `tint` / `highlight` / `shade` / `hairline` / `radius` / `highlighted` / `shaded`，另有 `glass_tint` / `glass_highlight` / `glass_shade`、`glass_material` 与折射访问器），组件不得自选颜色、透明度或折射量。**窗格由共享的 `GlassPane` 组件绘制**（`inox-glass`，ADR 0040 §3）：投影里它是一种独立的图元 `kind: "pane"`，`qml/Main.qml` 用它实例化 `GlassPane`，一道 highlight 发丝画在顶边、一道 shade 发丝画在底边，都是组件自己的一部分，**不得**用占满宽度的子控件去画。面板因此不再自己把窗格铺成色带（`glass_stops` / `Stop` 已删除），也仍然没有做出任何绘制决策：五个值全部来自 `pane_style`。背景模糊**与边缘折射**都由合成器绘制，`glass_blur` / 折射令牌只是请求值：面板不得声称自己画了模糊或折射。
- **Dock 的图块是徽标，不是图标**：面板没有图标主题解码器，也不得为此新增依赖；图块用应用名首字符，没有名字用 `panel-dock-icon-placeholder`（与启动器 `row_widget` 同类的降级）。图块是条目（`button` 语义），消息与任务栏一致（`TaskActivated` / `TaskToggled`），数量受既有 `panel.max_task_buttons` 限制，溢出不滚动、不加溢出指示。图块的点击区域覆盖徽标与其下方活动标记槽位，不得只盖住其中一半。
- **玻璃表面的背景是结构性透明**：三个表面的根背景都是 `theme::TRANSPARENT`（`#00000000`，结构性哨兵，不是设计颜色）；菜单栏与 Dock 的玻璃窗格画在它上面，任务栏则自己画不透明的 `surface` 填充，其投影不得因 `glass` 令牌改变一个值（有测试断言）。
- **Dock 的表面宽度必须覆盖它会画的一切**：`DockContent { tiles, tray_width, notice_width }` 是宽度的唯一输入（`src/dock.rs` → `DockGeometry::surface_width` → `PanelPlacement::dock(config, tokens, content)`），托盘宽度与句子宽度都是**字符估算**（`AVERAGE_ADVANCE_RATIO`，与启动器 `label_budget` 同一类做法；**不得**为此引入字体度量依赖），句子另有 `minimum_width` 保底槽位，因此没有窗口服务时空 dock 也必须是一个可读的居中胶囊。窗格宽度必须等于表面宽度：合成器磨砂的是整个 configure 区域（`headless_tests::protocol` 断言 `set_region == (0,0,width,height)`）。字符估算是估算而非度量：CJK 托盘标签可能被低估、更宽的字体可能裁掉最后的字形，这两条是已知的、被记录的残余误差。
- **文案只来自 Fluent**：只能出现消息 id，不得在代码里写用户可见字符串（产品名与组件名除外）。新增 id 必须同时进 `en-US` 与 `zh-CN`，占位符保持一致（`headless_tests::catalog` 断言）。
- **摆放策略是数据，但 `src/layer.rs` 会把它应用到协议上**：`src/placement.rs` 计算锚点/layer/exclusive zone/margin/尺寸/键盘交互，`src/layer.rs` 是唯一把这些值变成 `zwlr_layer_surface_v1` 请求的地方（`anchor_bits` / `layer_of` / `keyboard_of` / `surface_size`）。**禁止**在别处再算一遍锚点或尺寸。
- **没有 layer 宿主就是失败，不是降级**：`wl_compositor`、`wl_shm`、`zwlr_layer_shell_v1` 三者缺一，`layer::run` 返回可读的 `LegError::MissingGlobal`，`main` 记录并以非零码退出。`inox_glass_manager_v1` 可选：拿不到就一次 `info!` 说明，画纯半透明；没有 `wl_seat` 或该 seat 没有 pointer capability 同理（一次 `info!`，继续画、不接受点击，**绝不 panic**）。
- **协议腿自己建连、自己画帧、自己收指针**：`src/layer.rs` 用 `Connection::connect_to_env()` + `calloop` 事件循环（与 `inox-lock/src/session.rs` 同一套模式：`wl_registry` 收集 globals、`memfd_create` + `wl_shm` 帧、`wl_buffer.release` 记账、`MAX_FRAMES_IN_FLIGHT` 上限、`Generic` 连接源）。帧是离屏 QML 在 configure 尺寸下渲染、再经 `write_argb` 换过通道的像素，格式**必须是 `Argb8888`**（窗格半透明；`Xrgb8888` 会画成不透明方块），字节序为**预乘** `B, G, R, A`（Qt 的 `Format_RGBA8888_Premultiplied` 已经是预乘；写成非预乘会在圆角与字形周围出现光晕）。根背景必须是结构性透明，否则圆角外会是一整块颜色。
- **只提交与 configure 同尺寸的帧**：ack 必须在第一次 attach 之前；`set_size` 之后要等下一次 configure 再画；尺寸不是本组件会画的（0 或超过 `MAX_FRAME_BYTES`）就不提交，`Input::resize` 与 `Interface::resize` 都拒绝这种尺寸。重绘只在有变化时发生（状态变了、configure、指针交互、glass 参数需要下一次 commit 生效、`wl_buffer.release`）——时钟是 `calloop::timer` 喂同一个 `app::Message::Tick`，不得自建第二个时钟，也不得每帧重画。
- **指针输入在 Rust 里命中，不经 QML**（P3 之后改由 QML 上报，见下一条）。`wl_pointer` 的 enter/leave/motion/button 翻译成 `Pointer`（纯数据），经 `layer::Interface::pointer` 交给 `layer::Input`，按**最近提交的那一帧的场景**（`Input::scene`，由 `render_argb` 写入）里的 `entries` 命中，产生的 `Message` 由 `app::update` 应用；坐标按表面 scale factor 化成逻辑坐标（`logical`），`SCALE_FACTOR` 是唯一的缩放因子。点击任务图块必须产生与投影测试相同的 `Message::TaskActivated` / `TaskToggled`。
  **ADR 0046 的 P3 之后**：命中矩形仍由 `src/view.rs` 投影给出（`entries`），点击由 QML 的 `TapHandler` 产生 `dispatch`，`layer::Input` 与 `Pointer` 一起删除；左键激活/已聚焦最小化、中键关闭、右键循环模式仍由同一条 `Message` 路径决定，不得在 QML 里复制这套语义。唯一保留的 Rust 侧「所见即所点」断言，是投影里的条目矩形与 `app::update` 的动作一致（今天由 `headless_tests::projection` 覆盖）。
- **玻璃请求由 `src/frost.rs` 决策、由 `layer.rs` 发送，clamp 与 v1/v2 协商在 `inox-glass`**：面板只在 `frost` 里保留自己的策略——菜单栏与 Dock 是窗格、任务栏不是（`is_pane`，任务栏返回 `None` 而不是一种降级理由）；只要材质请求了**模糊或折射之一**（`Material::has_effect`）就创建 glass 对象，`blur = 0` + `refraction > 0` 依然是对象（无模糊的透镜），两者皆 0 才是「纯半透明、不创建对象」。`capabilities`（v1）与 `liquid_capabilities`（v2，优先）都归一到 `inox_glass::Capabilities`；`inox_glass::decide` 给出 `Effect`（半径、区域、四角半径、被钳制的折射带，由 `Request::apply` 发到线上）或带理由的 `Plain`，后者只记**一条** `info!` 并清掉对象的请求（`frost::off()`，不销毁对象）。合成器既不能模糊也不能折射（两个上限都为 0）时必须接受纯半透明，绝不自造效果。glass 对象必须在 layer role 存在**之后**创建，拆卸时先 `destroy()` 它；`since=2` 的请求只能发给 v2 对象（库的适配器与 `apply` 双重把关）。configure 尺寸、令牌或重载变化时重发。
- **启动必须自检**：没有可用的 Wayland 显示时，`main` 必须以可读错误与非零退出码结束；`zwlr_layer_shell_v1` 的存在性查询由 `layer::run` 在绑定 globals 后完成，缺失即 `LegError::MissingGlobal` + 非零退出。
- **窗口服务的传输是 `src/window_service.rs`，SNI（托盘）传输仍不在本 crate 内**：窗口侧说的是合成器的私有 `inox-window-manager-v1`（ADR 0045 §4），模块把协议事件翻译成**既有**的 `app::Message`、把既有的 `OutgoingCommand` 队列翻译成协议请求——它**不新增窗口模型**，`src/tasks.rs` 就是那份簿记。事件映射（ADR 0045 §4 的表，也是模块文档里的那张表）：`session_generation` → `Message::ServiceReady`、`workspace_state` → `Message::WorkspacesChanged`、`window` → `Message::WindowUpdated`（快照期间同一条路径）、`window_removed` → `Message::WindowRemoved`、`snapshot_done` → `Message::Snapshot`、`request_failed` → `Message::CommandSettled { outcome: Rejected { reason } }`；三个 wire `request_error` 一一对应 `RejectionReason::{UnknownWindow, NotPermitted}`，「`unsupported`」落进 `Protocol { detail }`，**不得**新增第四个错误变体。反向：`Snapshot` 创建一个新的 `inox_window_list_v1`（列表**就是**快照，重连等于换观察者）、`Activate`/`Toggle`/`Close`/`SetMode`/`MoveToWorkspace`/`SwitchWorkspace` → 同名请求。
  **两条规则不可放宽**：**一条请求由状态事件确认，不由应答确认**——`CommandState::Applied` 不由这条腿产生，`window`/`window_removed`/`workspace_state` 才是确认，`Rejected` 只来自 `request_failed`；**句柄只在它的 `session_generation` 内有意义**——每个 `WindowKey` 都把 wire 句柄与列表宣告的代次配成一对。`request_failed` 的 `window` 是保留的「无窗口」句柄（`u32::MAX`，与合成器 `NO_WINDOW_HANDLE` 同值）时，它是**工作区切换**的否定结果，落成 `Message::WorkspaceSwitchFailed`，settle 的是那唯一一个工作区请求（`PendingRequests::issue` / `pending_workspace` / `settle_workspace` 的独立槽位），不与任何窗格请求混用。
  `update` 仍不做 I/O：它只把意图放进 `OutgoingCommand` 队列，`layer.rs::pump` 把它交给这条腿发到线上——**不再是「取出并丢弃」**。点击语义：左键沿用既有定义（未聚焦=激活、已聚焦=最小化），中键=关闭（`TaskClosed` → `OutgoingCommand::Close`，是发给客户端的**请求**，面板在收到 `window_removed` 之前不移除按钮），右键=模式循环 `Tiled → Floating → Maximized → Fullscreen`（`TaskModeCycled`，由 `WindowMode::cycled` 一处定义），任务区左侧的工作区指示器点击=切换工作区（`WorkspaceSwitched`；`WorkspaceView::is_shown` 只在 `count > 1` 时画它，单工作区的控件没有可改变的东西）。
  **服务不可用是设计状态，不是崩溃**：合成器不广告该全局时，面板以一次 `info!` 说明（`WindowService::report_unavailable` 用字段而不是调用次数保证「一次」），停在既有 `ServiceState::Unavailable`，画出既有「窗口服务不可用」句子与重试按钮（D05），**不**非零退出——面板没有窗口服务仍是一个可用的时钟与托盘。`Message::Retry` 重新排一次 `Snapshot`。配置重载仍由 `src/reload.rs` 通过会话 D-Bus（`org.inox.Panel`）与 `SIGHUP` 接入，把 `Request` 放进通道并用 `calloop::ping::Ping` 唤醒 `layer.rs` 的循环。**SIGINT / SIGTERM / SIGHUP 的信号源由 `layer.rs` 持有，且必须在任何线程（含离屏 Qt 宿主）创建前装好掩码**：`signalfd` 只收到「在所有线程里都被阻塞」的信号。测试子进程同样依赖这一点：`headless_tests` 在 spawn 之前先在父线程阻塞这些信号，让子进程经 `exec` 继承掩码（`signalfd` 才能收到），**不要**把「子进程跑在 main 线程」当作前提。新增 `Message` 必须同时给出测试。
  **可测试性与证据边界**：事件→消息的整张表由 `src/window_service.rs` 里的纯函数 `ListState` / `rejection` / `Failure` 单元测试覆盖（不需要合成器）；`WindowService` 只是持有 proxy 的那层薄壳。**不得**把这条腿的单元测试读作 D03 通过：D03 还要求真实客户端与 D-Bus 断言，E3 证据仍缺。
- **任务区是弹性区域**：任务按钮按顺序排布并各自裁剪，时钟与托盘项从右边缘向内排布，因此长标题只会被裁掉，不会把时钟挤出表面。显示数量受 `panel.max_task_buttons` 限制，不得用滚动或溢出指示替代（尚未实现）。
- **普通展示不夺焦点**：`keyboard_interactivity = None`；时钟与托盘项是纯展示，不进入 `entries`。显式导航焦点只能落在可点击（enabled）的条目标上，且至多一个（D01）。
- 新增依赖前先确认依赖方向合法（根级 §2）：本 crate 依赖 L1（`inox-common`、`inox-config`）、一条**向下的 L3 → L2 边**（`inox-protocols`），以及一条**同层例外** `inox-glass`（L3 → L3，ADR 0040 授权并登记在根 `Cargo.toml` 的 `same_layer_exceptions`；删除这条边时登记必须一起删。通知栈的同类边由 ADR 0044 授权，不影响本 crate）。为协议腿新增：`wayland-client`、`rustix`；为界面新增：`cxx` / `cxx-qt` / `cxx-qt-lib` / `cxx-qt-build`、`serde_json`；为把共享玻璃 QML 编进本 crate 的 qrc 新增构建期 `qt-build-utils`（与 `cxx-qt-build` 0.10 同一版本行）与 `inox-glass` 的 build-dependency。**不得**引入其它同层依赖、`inox-compositor` 或任何 Qt Wayland 模块；协议绑定也不得绕过 `inox-protocols` 直接依赖 `wayland-protocols*`。**ADR 0046 的 P3 会新增**：`inox-protocols` 的 `shell` 绑定（同一条已有的 L3 → L2 边，不新增依赖方向）、链接 `Qt6::WaylandClient`（与 `Qt6::Quick` 同一次 C++ 构建，不引入新的 Cargo 依赖）、以及构建期对 `qt6-wayland-private-dev` 头文件的存在性检查（缺失即构建失败，不静默降级）。仍然**禁止** `LayerShellQt` 与 Qt Wayland Compositor（GPL-only）。改依赖后跑 `cargo test -p inox-integration --test layering`。

## 主要依赖

- 上游 Inox crate：`inox-common`（i18n 目录加载）、`inox-config`（shell 配置、设计令牌）、`inox-protocols`（`wlr-layer-shell`、核心 `wl_*`、`inox-glass-v1` 与 `inox-window-manager-v1` 的绑定；L3 → L2 的合法向下边）、`inox-glass`（玻璃材质决策与共享玻璃 QML 组件；ADR 0040 的同层例外）。
- 界面：`cxx` / `cxx-qt` / `cxx-qt-lib`与构建期 `cxx-qt-build`（QML 模块 + C++ 宿主）与 `qt-build-utils`（把 `inox-glass` 的 QML 以显式别名编进本 crate 的 qrc）。
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
- **QML 加载/渲染测试**（`headless_tests::qml`）：在 `QT_QPA_PLATFORM=offscreen` 子进程里，经与二进制相同的 `layer::Interface` 真实渲染任务栏与 Dock 各一帧，断言帧长等于 configure、首像素等于 `surface` 令牌的预乘 `B,G,R,A`、Dock 的 margin 条带透明、玻璃确实半透明且窗格像素就是 `glass.tint` 令牌（±1 舍入），并断言「画出来的场景」就是「指针命中用的场景」。**同一组断言要跑两遍**：一遍走平台给的像素路径，一遍用 `INOX_PANEL_OFFSCREEN_NO_ALPHA_GRAB` 强制走 alpha 重建路径；强制那一遍必须断言自己确实走了重建（`Interface::reconstructs_alpha`），平台那一遍不猜平台能力（有 alpha 的 Qt 与无 alpha 的 Qt 都必须通过同一组像素断言）——这是重建「精确而非近似」的唯一本地证据。**此外还渲染 `inox-glass` 的组件目录页**（`Host::with_url(CATALOG_URL)`，模型来自 `inox_glass::catalog_model(&tokens)`）：断言窗格中心是 `glass.tint` 令牌、圆角外完全透明、勾选开关的轨道是 accent 令牌、滑杆填充在 `value` 处结束（改 `value` 后填充跟着走）。这是共享玻璃组件库的第二个像素证据点（第一个是同一文件里的 Dock 窗格帧：`GlassPane` 接管绘制后，那条逐通道断言原样通过，正是「组件画的和旧色带一样」的证据）；坐标一律从投影里读，不得写死。
- `headless_tests::catalog`：`compare_catalogs(en-US, zh-CN)` 必须干净（Q03），id 前缀为 `panel-` 且不含点号，未翻译的 id 必须回退到默认语言。
- `headless_tests::binary`：把测试二进制自身作为子进程跑本面板入口，覆盖 `--surface` / `--help` / 未知参数 / 没有显示时的非零退出。
- `headless_tests::protocol` 用真实 `wayland-server` 在 socket pair 上扮演合成器并驱动本二进制，必须断言：按 `[panel] surface` 创建的 layer surface 的**锚点 / layer / exclusive zone / margin / 键盘交互 / namespace / 尺寸**与 `placement.rs` 的数据一致；`get_glass` **晚于** layer role；`inox-glass-v1` 的 manager 绑定在 **v2**（`min(advertised, 2)`），`capabilities` 先到、`liquid_capabilities` 后到，请求里 `set_radius` 已按上限钳制、`set_region` 是窗格尺寸、`set_corner_radii` 是 `radii.md`、`set_refraction` 是被钳制的折射带与合成器实际广告的 flags；配置成 v1 的合成器只收到 `set_radius` / `set_region`（绝不发 `since=2` 的请求，也不收 `liquid_capabilities`）；提交的帧声明的尺寸等于 configure、字节是预乘 `Argb8888`；两个上限都是 0 时仍然提交帧且不发任何效果请求、日志里出现那**一条**降级说明；`blur = 0` 但 `refraction > 0` 时**创建**对象并只发透镜；两者皆 0 时**不创建** glass 对象；合成器不广告 `zwlr_layer_shell_v1` 时二进制以非零码退出并在日志里点名该协议；`SIGHUP` 重载会按新尺寸重新 configure 并提交新帧。
  **这条 peer 尚未扮演 `inox_window_manager_v1`**：窗口服务一侧的「全局缺失 → 一次 `info!` + `Unavailable` + 重试、进程不退出」与「全局存在 → 绑定 manager、建观察者、`window` 事件画出按钮」今天只有代码路径与纯函数单测，**没有进程级断言**。补这条 peer 是窗口服务腿从 E1 走向 E2 的缺口，不得先把结论写成已有。
- **窗口管理协议的单元测试**（`src/window_service.rs` 的 `#[cfg(test)]`）：事件→消息表逐行断言（`session_generation` → `ServiceReady`、`window` → `WindowUpdated`、`window_removed` → `WindowRemoved`、`snapshot_done` → `Snapshot` 且窗口按到达顺序、`workspace_state` → `WorkspacesChanged`、三个 wire 原因 → 三个 `RejectionReason`、未知原因 → `Protocol`）；快照前到达的 `window` 不产出消息；本构建不认识的模式码读作 `Tiled` 而不是错的模式；`request_failed` 对未知窗口或无在途请求不改任何状态；保留句柄 `u32::MAX` 不与真实句柄冲突。这些是 E1，**不是** D03。`cargo test -p inox-panel --all-features` 必须全绿才算 E1 取得——这条腿落地时 `contract::tests::a_workspace_switch_settles_only_its_own_identifier` 曾因期望值写错而失败（模式 344 通过 / 1 失败），修好之前不得宣称已取得。
- 覆盖率目标：核心纯逻辑（`tasks`、`tray`、`clock`、`placement`、`view_model`、`contract`、`frost`、`view`、`theme`、`window_service` 的纯半）≥70%（E1）。

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
src/theme.rs           令牌 → 场景颜色与尺寸（含 `pane_style` 的玻璃材质与折射访问器）
src/frost.rs           液态玻璃的客户端决策（纯，含降级表；clamp 与 v1/v2 协商在 inox-glass）
src/window_service.rs  窗口服务的协议腿：inox-window-manager-v1 事件 → 既有 app::Message，OutgoingCommand → 协议请求（纯半：ListState / rejection / Failure）
src/layer.rs           协议腿：wlr-layer-shell 表面、ARGB 共享内存帧、指针输入、inox-glass-v1 v2、window_service 的绑定与泵送
src/reload.rs          SIGHUP 与 org.inox.Panel 的重载请求
src/cxxqt_object.rs    cxx-qt 桥（唯一 QML↔Rust 边界）
src/offscreen.rs       离屏宿主的 Rust 面（含测试用的 `Host::with_url`）
src/offscreen.cpp/h    C++ 宿主：QGuiApplication + QQmlApplicationEngine + grabWindow
src/native_surface.rs  原生宿主的 Rust 面（`cfg(native_surface)`）：Surface、Event、Geometry
src/native_bridge.rs   cxx 桥（只在 `cfg(native_surface)` 下编译）
src/native/host.cpp/h  原生宿主：QGuiApplication + QML 窗口 + shell 角色 + popup
src/native/qt_shell_surface.cpp  唯一允许出现 QtWaylandClient 私有 ABI 的翻译单元
src/native/shell_surface.h       该翻译单元的公共面（只有 Qt 公开类型与整数）
src/i18n.rs            内嵌 Fluent 目录
qml/Main.qml           QML 渲染器（`kind: "pane"` 的图元实例化 `inox-glass` 的 `GlassPane`）
i18n/en-US/panel.ftl, i18n/zh-CN/panel.ftl
src/headless_tests/    binary / catalog / projection / protocol（仅离屏腿）/ qml / support / native（仅原生腿）
build.rs               CxxQtBuilder：QML 模块 + 桥 + C++ 宿主 + inox-glass 的玻璃组件（显式 qrc 别名）；
                       特性开启时另生成 inox-shell-v1 的 C 绑定并编译 src/native/
```

## 两条宿主路径（ADR 0046 P3）

默认构建只有一条：`layer.rs` 的 `wlr-layer-shell` 表面 + `offscreen.cpp` 的离屏 QML 帧，`main` 调用
`layer::run`。

构建宿主能承载私有 ABI（Qt ≥ 6.5 且私有头已安装）时，`build.rs` 打开 `cfg(native_surface)`，
宿主换成 `src/native/`：Qt 的 wayland 平台插件建真实表面，`inox_shell_v1` 给角色，`configure` 决定
尺寸，合成器插帧；`main` 改调 `src/native_run.rs::run`（状态机 + 循环，见下一节）。**不用 cargo feature
表达这件事**：本工作区用 `--all-features` 验收，那会把一个「需要更新 Qt」的特性在每一台 CI 上都打开。
`INOX_NATIVE_SURFACE=0` 可以强制走离屏（排查用）。
三条硬规则：

1. **私有 ABI 只允许出现在 `src/native/qt_shell_surface.cpp` 一个翻译单元里。**
   `rg 'QtWaylandClient/private' crates/inox-panel/src` 必须只命中它；私有类型不得出现在任何头文件或
   cxx 边界上。新增私有调用时同时更新 ADR 0046 §10 的清单，并加一道闸门。
2. **闸门三道，缺一不可**：编译期 `#if QT_VERSION < 6.5 #error`、运行期 `qVersion()` 比较、失败即非零
   退出（不退回 `offscreen`、不退回 `xdg_toplevel`、不退回 layer-shell）。
3. **角色必须在窗口第一次 `show` 之前安装。** 一个 `wl_surface` 只有一个角色，先成 `xdg_toplevel` 就没
   得改。QML 的 `visible` 绑在 `inoxHostNative` 上，由原生宿主负责显示。

**已知缺口，不得在文档里声称已解决**：锁屏（P4）与删离屏腿（P5）未做；原生路径在无 GPU 的宿主上
`QWindow::isExposed()` 为假、`frameSwapped` 从不触发（没有附着 buffer），所以原生路径**没有出图证据**，
只有协议层与状态机的证据（`headless_tests/native.rs` 六个用例）。细节与原因见 ADR 0046 的「落地状态」一节。

### 运行路径（`main` 选哪条腿）

`src/main.rs` 里的 `run()` 有两个 `cfg` 版本，**编译期二选一，没有运行时开关**（ADR 0046 §9 禁止
「拿不到 `inox_shell_v1` 就退回 layer-shell」）：

| `cfg(native_surface)` | `main` 调用 | 表面 | 输入 | 玻璃 |
|---|---|---|---|---|
| 成立 | `src/native_run.rs::run` | `inox_shell_surface_v1`（Qt 的连接） | QML `TapHandler`/`HoverHandler` → `dispatch` → `Message` | `inox_glass_v1`，建在 Qt 的连接上 |
| 不成立 | `src/layer.rs::run` | `zwlr_layer_surface_v1`（Rust 自己的连接） | `wl_pointer` + `Scene::entry_at` | `inox_glass_v1`，建在 Rust 的连接上 |

两条腿共享**除表面以外的一切**：`app` 的状态与 `Message`、`placement` 的放置数据、`view` 的投影、
`frost`/`inox-glass` 的玻璃决策、`window_service` 的协议对象、`reload` 的配置重载。改语义时先问
「这是哪一层的问题」：坐标与表面归表面腿，其余都不该分叉。

`native_run` 的循环里有一条不显然的规则：**`event_loop.dispatch(Some(16ms))` 是唯一的等待**。Qt 有自己的
事件线程，只有 `pump()` 才会把表面的答案搬进这个线程；16 ms 是帧间隔（原生表面本来就画在
`wl_surface.frame` 上），不是轮询补丁。改成 `dispatch(None)` 会让点击在 Qt 的队列里一直等到下一个
`calloop` 事件——通常是下一分钟的时钟 tick。

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `panel`。
