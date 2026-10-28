# inox-compositor

基于 Smithay 的 Wayland 合成器。属于 Inox 桌面环境的 **L2** 层。

## 状态

M1：headless 合成器已实现——创建 `wl_output`、以 EGL surfaceless（`EGL_MESA_platform_surfaceless`）+ GLES 渲染纯色帧、收到终止信号或渲染完指定帧数后干净退出（退出码 0）。渲染路径与 `unsafe` 策略见 [`AGENTS.md`](./AGENTS.md) 的 `unsafe` 例外一节。

M2（PRD §13）：在 M1 的 headless 路径上扩展——通过 `wl_seat` 处理键盘/指针事件（确定性 keymap、修饰键跟踪、焦点投递），以 `xdg-shell` 客户端 toplevel 作为窗口并经 Smithay `desktop::Space` 管理 **≥2 个窗口**，按 master/stack 平铺布局（`proptest` 验证块不重叠、不越界、覆盖率完整）。键位绑定为纯逻辑（`Alt+Tab`/`Alt+Shift+Tab` 切换焦点、`Super+Q` 干净退出）；输入事件通过进程内注入 API 驱动，集成测试用**真实 Wayland 客户端**断言窗口数与平铺落点。M2 沿用 headless 路径（**不启用** libinput/udev/DRM 后端）。平铺此后成为四种窗口模式之一，见下文「窗口模式、工作区与任务栏协议」。

合成器同时广告私有 `inox_ipc` 全局（来自 [`inox-protocols`](../../crates/inox-protocols)）：`get_version` 返回协议版本，`reload_config` 把请求交给运行中的配置重载路径并以 `config_reloaded` 回答 `Success` 或 `Rejected`；没有注册重载路径时（例如物理后端未就绪）回答 `Unsupported`。

M3（PRD §13，承接 XWayland 外部进程的边界）：合成器侧补齐 XWayland 互操作所需的协议面。

- 新增三个全局：`wp_viewporter`（v1，`xwayland-satellite` 缩放 X11 窗口的前提，缺了它合成器根本无法充当宿主）、`wl_data_device_manager`（v3，剪贴板与拖放）与 `zwp_primary_selection_device_manager_v1`（v1，中键粘贴用的主选区）。完整全局清单见 `src/state.rs` 的模块文档。
- **选择可见性跟随键盘焦点**：只有获得焦点的客户端会被投递选择（也只有它能设置选择）。`sync_focus` 是合成器唯一改动键盘焦点的地方，因此也是唯一交接选择焦点的地方——这正是 X11 ↔ Wayland 剪贴板桥能工作的原因：`xwayland-satellite` 只是一个普通 Wayland 客户端，它拿到焦点才拿得到 X11 的选择。
- **每帧渲染后发送 `wl_surface.frame` 回调**：此前从不发送，真实客户端（XWayland 顶层、GTK 应用）会因此永远等不到重绘时机。回调由事件循环在每帧渲染后 flush 出去，焦点变更排入的选择事件同样如此。
- 拖放不需要额外合成器逻辑：客户端调用 `start_drag` 后 Smithay 的 `DnDGrab` 接管指针并转发 enter/motion/drop/action。

真实后端已按获批的帧生命周期决策
接入 libseat、DRM/GBM、udev 与 libinput。代码完成不代表 O01/O04 真机验收通过，支持边界见下节。

L2 的 layer-shell 宿主现已接入 Smithay `WlrLayerShellState`，广告
`zwlr_layer_shell_v1` v4。四层按 background → bottom → 普通窗口 → top → overlay
合成并命中测试，支持锚点、边距、独占区、初次 configure/ack、空缓冲卸载、重新映射与销毁清理。
普通窗口平铺在剩余工作区，背景与渲染目标仍使用完整输出尺寸。`None` 不接收键盘焦点，
top/overlay 的 `Exclusive` 优先，`OnDemand` 通过点击进入、点击应用或 Alt+Tab 离开；
选择焦点继续跟随实际键盘焦点。layer-shell 只用于普通桌面，不能替代安全锁屏。
安全锁屏走 `ext_session_lock_manager_v1` v1：只有先在 `inox_ipc` 上以会话令牌认领角色的连接才能取锁，锁定期间只合成锁表面（无锁表面时为不透明底色）、键盘与选区焦点清空、指针只命中锁表面、快捷键不再拦截，`locked` 只在每个输出都渲染过一帧锁定帧之后发送；客户端死亡不解除锁定。见 `tests/session_lock.rs`。

`tests/layer_shell.rs` 用真实客户端与 shm 缓冲断言像素、指针、焦点、frame callback、
独占区恢复和坏客户端隔离；CI 的现有合成器分片会执行它。本次没有更换 L3 客户端宿主（Iced 已由 QML + Qt Quick 取代；合成器侧不参与该迁移），
L3 的 layer-shell 客户端接线仍待完成，亦不宣称真机 DRM 或完整 M4 已验收。

### 窗口生命周期与关闭

`xdg_toplevel` 创建时只是一个空表面。合成器给它发初始 configure 后把它放进
`awaiting_buffer`，**第一次携带缓冲的 commit** 才把它变成窗口——占一个 tile、进入
Alt+Tab 顺序、获得键盘焦点。因此创建了却从不绘制的 toplevel（隐藏的辅助窗口、崩溃前
没画出第一帧的客户端）不会挤占别人的布局，也不会抢走焦点。

`xdg_toplevel.activated` 以键盘实际焦点为准，由 `sync_focus` 统一推送：焦点窗口收到
`Activated`，失去焦点的窗口被清除；layer 表面以 `Exclusive` 拿走键盘时窗口确实处于非激活
态。`Super+Shift+Q` 向焦点窗口发 `xdg_toplevel.close`（`Action::CloseWindow`）——这是请求，
客户端可以保存、确认或拒绝，合成器不会替它拆窗口；`Super+Q` 仍然退出合成器。

`xdg_popup` 的 grab 交给 Smithay 的 `PopupGrab` / `PopupKeyboardGrab` /
`PopupPointerGrab`：菜单打开期间指针与键盘属于它，点击外部由 grab 自己发 `popup_done`
并收起。被拒绝的 grab 只记 `debug!`，客户端看到的仍是一个普通（无 grab）弹窗。

`tests/window_management.rs` 用真实客户端断言这四条：空提交后窗口数为 0、绘制后平铺落点、
`activated` 随 Alt+Tab 在两个客户端之间翻转、关闭请求只到达焦点窗口且不解散窗口。

### 窗口模式、工作区与任务栏协议

平铺不再是唯一形态。每个窗口在任一时刻只处于一种模式——`tiled`、`floating`、`maximized` 或 `fullscreen`
（`src/workspace.rs` 的 `WindowMode`，互斥枚举而不是可叠加的标志位）；最小化与模式**正交**，保留模式与几何。
窗口分布在至少一个、至多九个工作区上（`workspace_count`，编号从 0 开始，`Super+1`..=`Super+9` 逐个切换）：

- `Tiled` 走既有的 master/stack 布局；`Floating` 用自己的几何、不参与平铺（首次浮动时取工作区的 2/3 居中矩形）；
- `Maximized` 填满该输出的**工作区**（`work_area`），因此独占区面板不会被盖住，与 O02 的面板保留区规则一致；
- `Fullscreen` 填满输出的**整个** area（面板被盖住，这是全屏的定义）；
- `arrange_windows` 仍是唯一摆放窗口的地方，`sync_focus` 仍是 `xdg_toplevel.activated` 的唯一来源。

`xdg-shell` 的窗口管理请求不再是空实现：`maximize`/`unmaximize`/`fullscreen`/`unfullscreen`/`minimize` 改窗口模式并重排，
`move`/`resize` 把窗口转为浮动，`title_changed`/`app_id_changed` 不再丢失（标题与 app id 是任务栏画的字）。
**客户端请求与任务栏请求走同一条路径**：两处都只调用 `InoxState` 上的同一个方法，因此「点最大化按钮」与「面板切换模式」
不会各自演化出第二套语义。一次 configure 里同时推送 `activated` 与该模式对应的状态位——`maximized`、`fullscreen`，
或 `Tiled` 的四个边缘位（`xdg-shell` 没有 floating 位，浮动窗口的位全部清空）。

键盘绑定（`src/input.rs` 的纯逻辑，有单元测试）：`Super+F` 浮动/平铺切换、`Super+M` 最大化/还原切换、
`Super+Shift+F` 全屏/还原切换、`Super+H` 最小化、`Super+1..=9` 切工作区、`Super+Shift+1..=9` 把焦点窗口移到该工作区。
它们是同一个状态机的键盘半边，不是第二套实现。

`workspace_count` 在启动时来自配置：物理后端读 `compositor.toml` 的 `[layout] workspace_count`，
headless 用 `--workspaces <N>`（范围 `1..=9`，默认 `1`；越界是解析错误而不是夹取）。headless 启动时**不读取 TOML**，
`--workspaces` 就是它这一项的等价参数；但**显式重载**（D-Bus `Reload` 或 `SIGHUP`）在两条路径上都会读取配置文档，headless 也能应用其中的 `[layout]`。
数量在会话期间固定，运行中不动态增删；重载一个更小的数量只会把
落在消失工作区上的窗口移到最后一个仍然存在的工作区。**浮动窗口的几何不持久化**：会话重启后回到默认位置，不写任何文件
（持久化属于 `inox-settings` 的后续工作）。**指针交互式移动/缩放尚未实现**：`move`/`resize` 只把窗口转为浮动，
合成器没有接管拖动与缩放。

#### 私有 `inox-window-manager-v1`（v1）

合成器广告私有 `inox_window_manager_v1` **v1**（`src/window_manager.rs` 是实现，协议规范在
`inox-protocols/protocol/inox-window-manager-v1.xml`）：面板由此观察并驱动窗口。`get_window_list` 为每个调用创建一个
`inox_window_list_v1` 观察者，先发快照（`session_generation` → `workspace_state` → 每个窗口一条 `window` → `snapshot_done`），
之后发增量。三条性质不可放宽：

- **一条请求由状态事件确认，不由应答确认**：`window`/`window_removed` *就是*确认，`request_failed` 是唯一的否定结果
  （`unknown_window` / `not_permitted` / `unsupported`）。在改动生效前发「已完成」是在撒谎。
- **事件是完整状态而不是增量**：`window` 带句柄、`app_id`、标题、工作区、输出、模式、是否最小化、是否可见、是否有焦点；
  漏掉一条事件的观察者会在下一条上收敛。这也是 `diff` 可以只算一次、发给所有观察者的原因。
- **事件只有两个出口**：`get_window_list` 之后的快照，与 `InoxState::announce_windows()`（窗口/模式/工作区/焦点变化之后调用一次）。
  别处零散发窗口事件就是观察者失去同步的写法。

`window` 只描述**已经 map 的窗口**（第一次带缓冲的 commit 之后），与 `promote_awaiting_toplevel` 的门槛是同一条规则：
一个创建了却从不绘制的 toplevel 不应在任务栏留下一个可能永远不出现的按钮。句柄只在它的 `session_generation` 内有意义，
代次在任何句柄之前发送。协议不授予任何像素，事件只有句柄、字符串与枚举。授权与 `inox_ipc` 相同——任何客户端可绑定，
因为泄露的正是任务栏本来就画在屏幕上的标题与 app id，请求只是用户通过快捷键已经能做的动作；未来要加特权动词，
必须同时加授权，不得拓宽这个接口。会话锁定时改动窗口可见性的请求以 `not_permitted` 拒绝（锁定期间不得把普通窗口带到前台），
`move_to_workspace` / `switch_workspace` 越界同样以 `not_permitted` 拒绝而不是创建工作区。
**中键关闭是请求**：协议只向客户端发 `xdg_toplevel.close`，客户端可以保存、弹窗或拒绝，面板只在收到 `window_removed` 后才移除按钮。

证据等级：本 crate 的新增证据是 **E1**（`workspace`/`layout`/`input`/`window_manager` 的单元与属性测试，`window_manager` 的
`diff` 与模式映射另有单元测试）加 **E2**（真实 Wayland 客户端驱动的进程内测试与真实 socket pair 上的协议往返）。
**E3/E4 仍未取得**：没有截图证据、没有真机运行，waydriver 宿主 job 尚未接线，因此 W05 与 D03 **不得**被读作通过。

### 玻璃材质（私有 `inox-glass-v1` v2）

合成器广告私有 `inox_glass_manager_v1` **v2**：拥有 layer 角色的表面可以创建
`inox_glass_v1`，指定一个表面局部坐标的区域、一个模糊半径，以及（v2 起）四角圆角半径与一条
折射带。合成器把该区域**背后已经合成好的像素**模糊、沿圆角轮廓折射后覆盖回去，客户端再以
自己的半透明像素叠上去。玻璃像素不会以任何形式回传给客户端（协议里没有 image/`fd`/颜色
事件），请求也被裁剪到该表面与所在输出之内。

- **v1（模糊）**：半径上限 64（`src/glass.rs::MAX_BLUR_RADIUS`，冻结），`capabilities` 事件
  如实报告：只有「表面此刻有 layer 角色」∧「会话未锁定」∧「该表面的合成路径真的会施加效果」
  三者同时成立才报 64，否则报 0。普通 `xdg_toplevel`（无论哪种窗口模式）由窗口路径合成，不实现玻璃效果，因此报 0；
  同一表面第二个 `get_glass` 是致命协议错误 `already_created`。
- **v2（折射）**：新增请求 `set_corner_radii(top_left, top_right, bottom_right, bottom_left)`
  与 `set_refraction(height, amount, flags)`，新增事件 `liquid_capabilities`（上限
  `MAX_CORNER_RADIUS = 16384`、`MAX_REFRACTION_HEIGHT = 128`、
  `MAX_REFRACTION_AMOUNT = 64`；`flags` 报告合成器真的实现的 `depth` / `dispersion` 位）。
  `capabilities` 仍然先发一次，v1 对象永远收不到新事件，因此 v1 客户端完全不受影响；两边的
  上限都遵守同一套三条件诚实规则。
- 实现是 `src/render.rs`：v1 走两个 region 大小的可分离 pass（横向、纵向各一次 9-tap 高斯，
  `step = radius / 4`）；带透镜的 pane 先把外扩 `ceil(blur) + ceil(amount)` 的源矩形模糊
  （`radius == 0` 时同一个 shader 以 `step = 0` 做恒等拷贝），再由一趟 pane 大小的折射 pass
  采样它——圆角矩形 SDF 遮罩、带内沿 SDF 梯度位移、可选径向 depth 分量、可选 7-tap 色散，
  轮廓外 alpha 为 0。两个 shader 都是 GLSL ES 1.00 且带 `//_DEFINES_` 标记，只用一个
  sampler，所有采样 clamp 到半 texel 内；程序与 scratch / lens 纹理在 `HeadlessRenderer` 上
  缓存。为了能采样背景，headless 的渲染目标从 renderbuffer 换成了 `GlesTexture`；无 pane 时
  仍走原来的「clear + 逐元素绘制」路径，**没有透镜的 pane 与 v1 逐字节一致**。
- 参数以表面为键，表面 / glass 对象 / 连接任一消失都会清除；锁定期间 `scene_elements` 在考虑
  任何 layer 表面之前就返回，所以锁定场景里不可能出现 pane。
- **已知限制**：物理（DRM）路径还没有玻璃效果。它的呈现经 `OutputDamageTracker`，而模糊/折射
  passthrough 在该路径还没有诚实的 damage 描述，因此 `backend::drm` 在场景里出现 pane 时
  只 `warn!` 一次并保持画面清晰，同时两个 capabilities 事件都报 0——不广告不会兑现的效果。
  真机玻璃属 E4，尚未执行。

`tests/glass_blur.rs` 是判别性证据，两半都测。v1 部分：pane 外的锐利竖直边保持锐利（严格居中
的列 ≤ 2），pane 内的过渡至少 `radius/2` 列，未创建 glass 对象的同场景对照渲染为 0 列。v2 部分：
背景换成竖条纹，pane 只开折射（`radius = 0`），断言条纹边缘向外推移若干列、pane 内部与对照逐
字节一致、圆角外的像素仍等于对照（SDF 遮罩真的在遮）、开启 `dispersion` 后同一处红蓝通道分离
而未开启时为 0、`depth` 只改变带内像素。删掉折射 pass、SDF 遮罩或色散分支，对应断言必然失败。
设置 `INOX_GLASS_ARTIFACT=<路径>` 时测试还把最终合成帧写成 P6 PPM 供人眼复核；不设置时不写
任何文件，CI 行为不变。真机（DRM）路径的行为未由此推断。

### 多输出、多 GPU 与缩放

`--backend auto`（默认）枚举当前 libseat seat 上的**全部** DRM 设备，主 GPU 优先，
每块能驱动至少一个 connector 的设备都会打开**自己的** libseat 租约、`DrmDevice`、
`GbmDevice` 与 EGL/GLES 渲染器，并驱动挂在自己名下的 connector。这一版没有跨 GPU 的
缓冲**复制**路径；它对本合成器实际发出的缓冲是完整的（只广告 `wl_shm`，共享内存任何
GPU 都能导入），但**广告 `linux-dmabuf` 之前必须先补上复制路径**。

每个输出在同一个全局逻辑坐标空间里有自己的位置（可为负）、缩放因子、工作区与帧生命周期：
一个输出的拔插、暂停或失败不会牵动其它输出，无输出时停止重绘并等待设备事件。
`InoxState::apply_outputs` 是拓扑变更的唯一入口，按 connector 名与 `Output` 实例复用未变的
输出（保留其 `wl_output` 全局、layer 表面与窗口），消失输出的窗口迁移到主存活输出。

缩放支持非整数取值（`0.25..=8.0`，解析期校验）。只理解整数缩放的客户端从 `wl_compositor` v6
的 `preferred_buffer_scale` 拿到上限（向上取整），理解 `wp_fractional_scale_v1` 的客户端拿到
精确值；渲染、光标与 layer 放置按该因子折算成物理像素，命中测试与工作区留在逻辑坐标。

`tests/multi_output.rs` 覆盖双输出平铺、迁移与 20 轮拔插往返（O02 的虚拟拓扑证据），
`tests/fractional_scale.rs` 用读回的物理像素断言 1.5x 下的真实落点（O03 的几何/像素证据）。
两者都是 E1+E3；真机热插拔与真实客户端的模糊度仍属 E4，未执行。

`--backend drm` 同样要求真实设备。`auto` 与 `drm` 在获取设备、配置输出或初次呈现失败时返回错误，
**不会回落到 headless 并报告 ready**。旋转、PRIME 复制路径与 HDR/VRR 仍未交付。

- 使用 Smithay 的 `DrmDevice`、`GbmAllocator`、`GbmBufferedSurface`、EGL/GLES 与
  `OutputDamageTracker`，交集选择可扫描输出和可渲染的格式；不手写 DRM/libinput FFI。
- 首帧、客户端提交和指针变化请求重绘；提交期间最多一个 pending frame。
  page-flip 完成后释放 swapchain 槽位并发送 frame callback。没有 damage 的帧不提交，
  callback 请求仍得到反馈；显式 `--exit-after-frames` 是有限帧测试模式，允许强制重绘。
  启动 ready 必须等到第一帧的 page-flip 完成，完成事件按代次和提交时间过滤。
- 暂时提交失败最多尝试三次；page-flip 完成有 2 秒超时。启动失败非零退出，
  已启动会话退化时保留客户端，停止提交并等待设备恢复事件，不无限轮询重试。
- libseat pause 停止设备提交和输入，释放 DRM master；resume 重建设备状态与
  swapchain 状态、清除陈旧按键并调度恢复帧。`Ctrl+Alt+F1`–`F12` 通过 libseat 切换 VT。
- udev 断连时撤下逻辑输出，关闭其 layer 表面并保留普通应用；恢复后重新安排窗口，
  layer 客户端按协议为新输出重新创建表面。全无输出时没有重绘定时器。
- libinput 通过 libseat 打开设备，按键与按钮按物理设备引用计数；设备移除和暂停
  释放遗留状态，seat 能力随设备变化。Smithay 的键码已是 XKB 单位，不再追加 `+8`。
- 始终提供软件光标路径：支持客户端 cursor surface 与热点，缺省箭头从主题的
  `spacing.lg/xl`、`colors.background/text` 生成；不依赖硬件 cursor plane。
- 真实会话启动时读取 `InoxConfig` 的输入、平铺（含工作区数量）、主题及输出 mode；不支持的 mode/scale
  明确拒绝。键盘重复 rate 为 0 关闭 repeat；delay/rate 必须可表示为 Wayland 有符号字段。
  headless 保持确定性的测试配置：它启动时**不读取 `compositor.toml`**，工作区数量来自 `--workspaces <N>`
  （`1..=9`，默认 `1`，越界是解析错误），平铺参数为内置默认值；显式重载仍会读取配置文档并应用其中的 `[layout]`。
- 运行中的配置通过会话 D-Bus 应用：`org.inox.Configuration1.Reload` 立即应用并拒绝输出
  改动（SIGHUP 与 `inoxctl` 没有确认通道）；`org.inox.Configuration2` 的
  `Trial`/`Confirm`/`Rollback` 让输出模式、缩放与位置**试用**：应用成功后 30 秒内没有
  `Confirm`，合成器自己恢复试用前的偏好。成功回执只在设备状态被逐 connector 读回核对
  之后才发出，因此「不支持的 mode」不会被报成成功。试用只存在于物理后端：headless 的
  输出来自 `--output`，该路径继续拒绝输出改动。

本地只执行 fmt/check/clippy。CI 负责纯状态机、属性测试、真实 Wayland 客户端、软件光标像素
及进程生命周期回归。DRM 翻页、VT、真实输入与热插拔须按
[硬件验收步骤](../../docs/quality/l2-hardware-validation.md)取得 E4，当前结果标为未执行。

### 运行

```console
# 无显示设备的环境（CI 用的软件渲染）
XDG_RUNTIME_DIR=/tmp/inox-runtime LIBGL_ALWAYS_SOFTWARE=1 \
  cargo run -- --headless --exit-after-frames 120

# 两块不同分辨率/缩放的虚拟屏并排（O02/O03 的虚拟拓扑）
XDG_RUNTIME_DIR=/tmp/inox-runtime LIBGL_ALWAYS_SOFTWARE=1 \
  cargo run -- --headless \
    --output name=DP-1,width=1920,height=1080,x=0,y=0 \
    --output name=DP-2,width=2560,height=1440,scale=1.5,x=1920,y=0

# 三块工作区（headless 没有 TOML 可读，这一项只能走命令行）
XDG_RUNTIME_DIR=/tmp/inox-runtime LIBGL_ALWAYS_SOFTWARE=1 \
  cargo run -- --headless --workspaces 3

# 查看选项
cargo run -- --help
```

`XDG_RUNTIME_DIR` 必须指向已存在的绝对路径目录，Wayland 套接字在该目录下创建。

### 测试驱动通道 `--test-input`（仅测试，默认关闭）

端到端测试必须驱动一个**正在运行的合成器进程**（切换焦点、向 X11 窗口注入按键、注入指针以触发拖放、读回画面以证明窗口真的被渲染），进程内注入做不到这些。因此提供一条刻意限定用途的通道：

```console
# 需要以 test-input feature 构建：cargo build --features test-input
XDG_RUNTIME_DIR=/tmp/inox-runtime cargo run --features test-input -- \
  --headless --test-input /tmp/inox-runtime/test-input.sock
```

在该 unix socket 上按行接受命令（ASCII，`\n` 结尾，容忍 CRLF）：

| 命令 | 含义 |
|------|------|
| `key <evdev-keycode> press\|release` | 注入按键（evdev 码，内部换算成 XKB keycode） |
| `motion <x> <y>` | 把指针移到绝对坐标 |
| `button <code> press\|release` | 注入指针按键 |
| `dump-frame <path>` | 渲染一帧并以二进制 PPM（`P6`）写入 `path` |

每条命令在**应用完成并 flush 客户端事件之后**回 `ok\n`，否则回 `err <message>\n`；因此 `ok` 意味着「客户端已经收到事件」，测试不必靠 sleep 猜时机。格式错误的行只回 `err`，不会终止合成器，也不会让监听器失效。

该通道：

- 由 cargo feature `test-input` 控制，**默认关闭**，且**默认构建的二进制里根本不存在**——`--test-input` 会被命令行解析拒绝；
- **不是 Wayland 全局**，不向任何客户端暴露，也不会被客户端发现；
- 唯一的能力是向调用者指定的 socket 写命令，权限不超过合成器自身的文件访问；套接字路径由调用者给出（测试放在私有的 `XDG_RUNTIME_DIR` 下），不存在固定公共路径。

## 输入法

使用 CI 产物启动 `inox-compositor --input-method /usr/bin/fcitx5`，或在
`inox-session` 上传递相同选项；额外参数逐项用 `--input-method-arg`。
输入法经独立的受信 Wayland 连接接入，普通应用只看到文本输入全局。
支持 v3 文本事务、v2 输入法、虚拟键盘透传与候选窗，锁屏期间暂停输入法权限。
输入法退出有界重启，会话退出回收子进程；不默认启动或安装引擎。
详见[使用说明](../../docs/book/src/usage/input-methods.md)。新增用例仍待 CI 验收。

## 开发约定

- 本项目遵循[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- `--test-input` 只服务于测试；改动它时请同步维护 `src/test_input.rs` 的模块文档与 [`AGENTS.md`](./AGENTS.md) 的测试要求。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，不在本地进行编译验证。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)。

## 许可证

LGPL-3.0，见仓库根的 [`LICENSE`](../../LICENSE)。
