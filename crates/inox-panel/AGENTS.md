# AGENTS.md — inox-panel

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

面板：任务栏、菜单栏与 Dock，以及系统托盘与时钟（PRD §3.1）。所属层级：**L3**。它不拥有窗口状态、也不拥有托盘状态：窗口状态来自合成器，托盘项来自各自的应用程序，面板只订阅并呈现（ADR 0013）。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`；默认级别 `info`，不得每帧打印。
- **窗口身份只来自服务端**：`WindowKey` = 服务端分配的 `WindowId` + `SessionGeneration`。禁止用 PID、标题或列表下标识别窗口，也禁止用它们做分组（ADR 0013 决策 1）。分组只按服务端给出的 `app_id`。
- **代次不匹配的状态一律丢弃**：跨代次的窗口状态返回 `Applied::Stale`，代次变化必须先 `TaskList::resync`，不得把两个会话拼在一起。
- **颜色/尺寸/圆角/动效只来自令牌**（`inox_config::Theme`）：`src/theme.rs` 是唯一把令牌变成颜色与样式的地方，`src/view.rs` 只调用它。文字尺寸由面板高度与 `typography.scale` 推导（令牌集没有字号），不得写死像素。
- **三种表面是同一份数据的三种呈现**：`[panel] surface` 决定进程画哪一种（`taskbar` / `menubar` / `dock`），三者共用 `view_model` 的投影与 `tasks` / `tray` / `clock` 模型；菜单栏与 Dock 只各自新增几何与排布（`src/menubar.rs`、`src/dock.rs`），不得新增窗口/托盘/时钟数据。运行中不得改变 `surface`：那是另一个窗口，重载**不得采用候选文档里的值**，必须保留运行中的表面并记一条 `warn!`（`Message::ConfigReloaded` 同理），而不是让整次重载失败。
- **菜单栏没有应用菜单可画**：Inox 没有应用菜单协议，菜单栏只显示焦点应用名（`show_app_name`）、面板自身的状态与重试、托盘项与时钟；**禁止**编造 "File"/"Edit"/"View" 之类的菜单标题。这条必须同时留在 `src/menubar.rs` 的模块文档里。
- **玻璃材质只来自 `glass` 令牌**（`tint` / `tint_strong` / `highlight` / `shade` / `blur`）：`src/theme.rs` 是唯一把它们变成 iced 样式的地方（`glass_fill` / `glass_bar` / `glass_dock` / `glass_surface`），组件不得自选颜色或透明度。两道发丝边（顶 `highlight`、底 `shade`）是窗格自身竖直渐变的一部分，**不得**用占满宽度的子控件去画：那会把窗格撑满整个表面。背景模糊由合成器绘制，`glass_blur` 只是请求值：面板不得声称自己画了模糊。
- **Dock 的图块是徽标，不是图标**：面板没有图标主题解码器，也不得为此新增依赖；图块用应用名首字符，没有名字用 `panel-dock-icon-placeholder`（与启动器 `row_widget` 同类的降级）。图块是 `button`，消息与任务栏一致（`TaskActivated` / `TaskToggled`），数量受既有 `panel.max_task_buttons` 限制，溢出不滚动、不加溢出指示。
- **玻璃表面的背景是结构性透明**：菜单栏与 Dock 的根容器用 `Color::TRANSPARENT` 哨兵（`theme::glass_surface`），不是设计颜色；任务栏保持不透明的 `surface` 填充，其渲染不得因 `glass` 令牌改变一个像素（有测试断言）。
- **Dock 的表面宽度必须覆盖它会画的一切**：`DockContent { tiles, tray_width, notice_width }` 是宽度的唯一输入（`src/dock.rs` → `DockGeometry::surface_width` → `PanelPlacement::dock(config, tokens, content)`），托盘宽度与句子宽度都是**字符估算**（`AVERAGE_ADVANCE_RATIO`，与启动器 `label_budget` 同一类做法；**不得**为此引入字体度量依赖），句子另有 `minimum_width` 保底槽位，因此没有窗口服务时空 dock 也必须是一个可读的居中胶囊（它曾经只是一根 16 px 细条）。窗格宽度必须等于表面宽度：合成器磨砂的是整个 configure 区域，表面比窗格宽会在胶囊周围露出一圈模糊的壁纸（`tests/protocol.rs` 断言 `set_region == (0,0,width,height)`）。字符估算是估算而非度量：CJK 托盘标签可能被低估、更宽的字体可能裁掉最后的字形，这两条是已知的、被记录的残余误差。
- **文案只来自 Fluent**：只能出现消息 id，不得在代码里写用户可见字符串（产品名与组件名除外）。新增 id 必须同时进 `en-US` 与 `zh-CN`，占位符保持一致（`tests/catalog.rs` 断言）。
- **摆放策略是数据，但 `src/layer.rs` 会把它应用到协议上**：`src/placement.rs` 计算锚点/layer/exclusive zone/margin/尺寸/键盘交互，`src/layer.rs` 是唯一把这些值变成 `zwlr_layer_surface_v1` 请求的地方（`anchor_bits` / `layer_of` / `keyboard_of` / `surface_size`）。**禁止**在别处再算一遍锚点或尺寸。`[panel] surface` 变更是**另一个进程**（另一种表面、另一种尺寸与摆放）：`reload::apply` 不采用候选里的值、保留运行中的表面并记一条 `warn!`（命令行 `--surface` 已经覆盖了文件，若因为该字段让整次重载失败，菜单栏+Dock 这类用 flag 启动的会话就再也重载不了）；其余摆放字段（高度、margin、exclusive、菜单栏/Dock 自己的段）可以热改，由 `layer.rs` 重新提交协议状态（`tests/protocol.rs` 用 `SIGHUP` 端到端断言）。`src/main.rs` 不再有「普通窗口 + 一条 warn!」的路径。
- **没有 layer 宿主就是失败，不是降级**：`wl_compositor`、`wl_shm`、`zwlr_layer_shell_v1` 三者缺一，`layer::run` 返回可读的 `LegError::MissingGlobal`，`main` 记录并以非零码退出（ADR 0018：声称摆了位而实际是浮窗就是在撒谎）。`inox_glass_manager_v1` 可选：拿不到就一次 `info!` 说明，画纯半透明；没有 `wl_seat` 或该 seat 没有 pointer capability 同理（一次 `info!`，继续画、不接受点击，**绝不 panic**）。`iced_layershell` 依旧禁止。
- **协议腿自己建连、自己画帧、自己收指针**：`src/layer.rs` 用 `Connection::connect_to_env()` + `calloop` 事件循环（与 `inox-lock/src/session.rs` 同一套模式：`wl_registry` 收集 globals、`memfd_create` + `wl_shm` 帧、`wl_buffer.release` 记账、`MAX_FRAMES_IN_FLIGHT` 上限、`Generic` 连接源）。帧是 `app::view` 的真实控件树在 configure 尺寸下的离屏渲染，格式**必须是 `Argb8888`**（窗格半透明；`Xrgb8888` 会画成不透明方块），字节序为**预乘** `B, G, R, A`（`iced_tiny_skia` 的 `Headless::screenshot` 返回的就是预乘 RGBA；写成非预乘会在圆角与字形周围出现光晕），截图背景必须是 `Color::TRANSPARENT`。
- **只提交与 configure 同尺寸的帧**：ack 必须在第一次 attach 之前；`set_size` 之后要等下一次 configure 再画；尺寸不是本组件会画的（0 或超过 `MAX_FRAME_BYTES`）就不提交。重绘只在有变化时发生（状态变了、configure、指针交互、glass 参数需要下一次 commit 生效、`wl_buffer.release`）——时钟是 `calloop::timer` 喂同一个 `app::Message::Tick`，不得自建第二个时钟，也不得每帧重画。
- **指针输入走 iced 的既有输入路径**：`wl_pointer` 的 enter/leave/motion/button 翻译成 `Pointer`（纯数据），经 `layer::Interface::pointer` 送进与渲染测试同一套 `UserInterface::update`，产生的 `Message` 由 `app::update` 应用；坐标按表面 scale factor 化成逻辑坐标（`logical`），`SCALE_FACTOR` 是唯一的缩放因子。点击任务图块必须产生与渲染测试相同的 `Message::TaskActivated` / `TaskToggled`（`tests/protocol.rs` 断言）。
- **玻璃请求由 `src/frost.rs` 决策、由 `layer.rs` 发送**：`glass.blur == 0` → 完全不创建 glass 对象（`asks_for_glass`）；`capabilities` 的 `max_radius == 0` → 纯半透明 + **一条** `info!`（不得声称模糊，也不得反复重建对象）；`max_radius > 0` → `radius = min(blur, max_radius)`，region 是窗格自身区域，configure 尺寸变化时重发。glass 对象必须在 layer role 存在**之后**创建（协议对无 layer role 的表面回答 0，ADR 0031），拆卸时先 `destroy()` 它。任务是**不透明条带**，不是窗格：任务栏不创建 glass 对象。
- **启动必须自检**：没有可用的 Wayland 显示时，`main` 必须以可读错误与非零退出码结束（`DisplayEnv`，ADR 0018）；`zwlr_layer_shell_v1` 的存在性查询由 `layer::run` 在绑定 globals 后完成，缺失即 `LegError::MissingGlobal` + 非零退出（`tests/protocol.rs` 断言）。
- **窗口服务与 SNI 传输不在本 crate 内**：两者的协议仍不实现。配置重载由 `src/reload.rs`（库模块）通过会话 D-Bus（`org.inox.Panel`）与 `SIGHUP` 接入（ADR 0026），把 `Request` 放进通道并用 `calloop::ping::Ping` 唤醒 `layer.rs` 的循环。**SIGINT / SIGTERM / SIGHUP 的信号源由 `layer.rs` 持有，且必须在任何线程创建前装好掩码**：`signalfd` 只收到「在所有线程里都被阻塞」的信号，别的线程若没继承掩码就会按默认动作杀掉进程（`SIGHUP` 也是一次 `client.reload.prepare()`，文件读取在独立线程上完成，不阻塞循环）。`update` 不做 I/O，只把意图放进 `State` 的 `OutgoingCommand` 队列；`layer.rs` 把队列取出并丢弃（一次 `info!` 说明没有传输），不接管窗口或托盘状态。新增 `Message` 必须同时给出测试。
- **任务区是弹性区域**：任务按钮放在 `Length::Fill` + `clip` 的容器里，时钟与托盘项是 `Shrink` 子项（iced 先排非弹性子项），因此长标题只会被截断，不会把时钟挤出面板。显示数量受 `panel.max_task_buttons` 限制，不得用滚动或溢出指示替代（尚未实现）。
- **普通展示不夺焦点**：`keyboard_interactivity = None`；时钟与托盘项是纯展示容器，不套 `button`。显式导航焦点只能落在可点击（enabled）的条目上，且至多一个（D01）。
- 新增依赖前先确认依赖方向合法（根级 §2）：本 crate 依赖 L1（`inox-common`、`inox-config`）以及一条**向下的 L3 → L2 边**（`inox-protocols`，ADR 0006 的协议单一出口；ADR 0002 允许向下依赖，ADR 0013 只禁止同层与合成器依赖），`wayland-client` / `rustix` / `iced_core` / `iced_runtime` / `iced_tiny_skia` 是为这条协议腿新增的；**不得**引入同层依赖或 `inox-compositor`，协议绑定也不得绕过 `inox-protocols` 直接依赖 `wayland-protocols*`。改依赖后跑 `cargo test -p inox-integration --test layering`。

## 主要依赖

- 上游 Inox crate：`inox-common`（i18n 目录加载）、`inox-config`（shell 配置、设计令牌）、`inox-protocols`（`wlr-layer-shell`、核心 `wl_*` 与 `inox-glass-v1` 的绑定；L3 → L2 的合法向下边）。
- 第三方：`iced` 0.14 与 `iced_core` / `iced_runtime` / `iced_tiny_skia` 0.14（界面、离屏布局与像素；`iced` 不重新导出后三者，因此直接命名——它们是**正式依赖**，二进制用它们提交真实帧）、`calloop`（事件循环、信号源与时钟）、`wayland-client`（连接、globals 与指针）、`rustix`（`memfd_create` 共享内存帧）、`chrono`（本地时间）、`zbus`（`org.inox.Panel` 重载端点）、`thiserror`、`tracing`、`tracing-subscriber`、`anyhow`（仅 `main`）。
- 测试：`wayland-server`（`tests/protocol.rs` 扮演合成器一侧）、`image`（截图产物）、`proptest`、`tempfile`。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-panel --all-features` |
| 本 crate 构建 | `cargo build -p inox-panel --release --all-features` |
| 本 crate 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-panel --no-deps --all-features` |
| 静态检查 | `cargo clippy -p inox-panel --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt -p inox-panel` |
| 渲染截图 | `INOX_SCREENSHOT_DIR=<dir> cargo test -p inox-panel --test render` |
| 协议腿（E2） | `cargo test -p inox-panel --test protocol` |
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
- 时钟：`CLOCK_DIRECTIVES` 的**每个**指令都被渲染，`%Q` 之类报错（含位置），结尾孤立 `%` 报错，zh-CN 的星期/月份与 `%p` 与 en-US 不同，`%Z` 为 UTC 偏移。
- 投影：D05 的 starting / empty / unavailable（消息 + 重试）各有断言；禁用条目必须给出**可读原因**；超长标题被 `MAX_LABEL_CHARS` 截断；导航顺序只含 enabled 条目。
- `tests/catalog.rs`：`compare_catalogs(en-US, zh-CN)` 必须干净（Q03），id 前缀为 `panel-` 且不含点号，未翻译的 id 必须回退到默认语言。
- `tests/render.rs`：1280×32 下至少浅色、深色、空态与不可用态四个场景（Q02），断言缓冲区长度、背景像素等于 `surface` 令牌、启用与禁用条目渲染不同、超长标题后时钟仍在右侧，以及点击任务按钮产生 `Message::TaskActivated` 并且只产生一次激活请求。`INOX_SCREENSHOT_DIR` 有值时写出 `inox-panel-<场景>.png`。
- `tests/render.rs` 的后半段渲染菜单栏与 Dock（`[panel] surface`）：菜单栏 1280×`[menubar] height`、Dock 1280×其窗口高度（另有 1280×120 一档），浅色与深色、空态与不可用态、Dock 的按下态各一组快照。像素断言必须证明玻璃**真的半透明**：`support::render_over(backdrop)` 在选定背景上合成，窗格内像素是背景与 `glass.tint` 的严格中间值（各通道都不同），且既不等于背景也不等于 `glass.tint`、也不是不透明的 `surface`；顶边存在 highlight hairline、Dock 底边存在 shade；Dock 是浮动的（远离窗格处像素等于背景）；**任务栏渲染不因 `glass` 令牌改变一个像素**（同一状态两次渲染字节相同，且换掉全部 `glass` 令牌后仍相同）。
- 菜单栏 / Dock 的纯逻辑同样要有单元测试：几何（高度、highlight 厚度、组间距大于组内间距、圆角、Dock 窗格高度/宽度随图块与 `icon_size` 增长、活动标记尺寸、分隔线、tooltip 方向）与投影（空态、焦点应用名有无、`show_app_name` / `show_clock` / `show_tray` 开关、活动标记、禁用图块的 tooltip 是原因、`panel.max_task_buttons` 上限）。
- `tests/protocol.rs` 用真实 `wayland-server` 在 socket pair 上扮演合成器并驱动编译出的二进制，必须断言：按 `[panel] surface` 创建的 layer surface 的**锚点 / layer / exclusive zone / margin / 键盘交互 / namespace / 尺寸**与 `placement.rs` 的数据一致；`get_glass` **晚于** layer role，且 `set_radius` 已按 `capabilities` 钳制、`set_region` 是窗格尺寸，configure 尺寸变化时重发；提交的帧声明的尺寸等于 configure、字节是预乘 `Argb8888`（首个像素等于 `surface` 令牌的 `B,G,R,A`）；`max_radius = 0` 时仍然提交帧且不发 radius/region；`glass.blur = 0` 时**不创建** glass 对象；合成器不广告 `zwlr_layer_shell_v1` 时二进制以非零码退出并在日志里点名该协议；没有窗口服务时 click 产生的 `Message::TaskActivated` 由 `layer::Interface` 在进程内断言（另有点击空面板不产生任何请求的对照）。
- 覆盖率目标：核心纯逻辑（`tasks`、`tray`、`clock`、`placement`、`view_model`、`contract`、`frost`）≥70%（E1）。

## 目录结构

```
src/lib.rs          crate 文档、模块声明与再导出
src/tasks.rs        窗口/任务分组模型（纯）
src/tray.rs         StatusNotifierItem 与 DBusMenu 模型（纯）
src/clock.rs        时钟格式展开（纯 + Fluent）
src/contract.rs     服务状态、请求关联与去重（纯）
src/placement.rs    layer-shell 摆放策略（数据，由 layer.rs 应用）
src/view_model.rs   状态 → 视图投影（纯）
src/menubar.rs      菜单栏几何与投影（纯）：焦点应用名、托盘、时钟
src/dock.rs         Dock 几何与投影（纯）：图块、活动标记、托盘组、数量上限
src/app.rs          State / Message / update（Elm）/ view / subscription
src/view.rs         Iced 控件、PanelGeometry 与三种表面的选择器
src/view/menubar.rs 菜单栏的 Iced 控件
src/view/dock.rs    Dock 的 Iced 控件
src/theme.rs        令牌 → iced 主题与样式（含 glass 材质）
src/frost.rs        磨砂玻璃的客户端决策（纯，含降级表）
src/layer.rs        协议腿：wlr-layer-shell 表面、ARGB 共享内存帧、指针输入
src/reload.rs       SIGHUP 与 org.inox.Panel 的重载请求（库模块，由 layer.rs 消费）
src/i18n.rs         内嵌 Fluent 目录
src/main.rs         日志、DisplayEnv 自检、--surface/--help，然后 layer::run
i18n/en-US/panel.ftl, i18n/zh-CN/panel.ftl
tests/support/mod.rs  离屏渲染、合成背景与点击驱动
tests/render.rs       渲染与点击测试（任务栏 + 菜单栏 + Dock）
tests/protocol.rs     真实 wayland-server 扮演合成器的协议腿测试（E2）
tests/catalog.rs      目录一致性
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `panel`。
