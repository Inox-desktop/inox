# Wayland 协议

## 支持矩阵

「级别」列的含义：**必须** 属于 1.0 之前必须支持的协议；**计划** 已列入计划但尚未排期；**仅规划** 明确推迟到 1.0 之后。

| 协议 | 级别 | 说明 | 当前状态 |
|------|------|------|----------|
| `wl_compositor` / `wl_subcompositor` | 必须 | 基础表面与子表面 | 已支持（Smithay `CompositorState`） |
| `wl_shm` | 必须 | 共享内存缓冲（软件渲染回退） | 已支持 |
| `wl_seat` / `wl_pointer` / `wl_keyboard` | 必须 | 输入设备 | 已支持（`wl_seat`，事件经进程内注入验证） |
| `wl_output` | 必须 | 输出设备 | 已支持（headless 逻辑输出） |
| `wl_data_device`（`wl_data_device_manager` v3） | 必须 | 剪贴板与拖放；读取权跟随键盘焦点 | 已支持（M3；Smithay `wayland::selection`） |
| `xdg-shell` | 必须 | 标准窗口 | 已支持（toplevel 作为窗口） |
| `xdg-decoration` | 必须 | 服务端/客户端装饰协商；合成器不画服务端装饰，所以恒答 `client_side`——答案本身就是 toolkit 拿不到的信息，X11 窗口标题栏仍由 satellite 自绘 | 已支持（v1；`crates/inox-compositor/src/decorations.rs`，真实 toplevel 的协商断言在 `tests/presentation.rs`） |
| `wlr-layer-shell-unstable-v1` v4 | 必须 | 面板等桌面层；不用于安全锁屏 | 合成器已接入（v4）：四层合成、锚点/边距/独占区、configure/ack、空缓冲卸载与重映射、销毁与断连清理；客户端是全进程内的真实 `wayland-client`，像素、工作区与焦点断言在 `tests/layer_shell.rs`，`INOX_SCREENSHOT_DIR` 下另写 `inox-layer-*.png`（CI 作为工件收集）。`inox-panel` 是真实 layer 客户端，界面由离屏 QML 渲染进 `wl_shm` 帧；启动器、通知与壁纸是普通窗口。**仍缺**：`inox-panel` 本身没有在合成器宿主上被驱动过，跨 L3 的 E3（面板真的出现在合成的画面里、点击真的落到面板上）仍未取得——画面证据目前只有测试里的替身客户端 |
| `viewporter`（`wp_viewporter`） | 必须 | 缩放与裁剪；`xwayland-satellite` 的宿主硬前提（缺失即启动失败） | 已支持（M3；Smithay `wayland::viewporter`） |
| `linux-dmabuf` | 必须 | GPU 缓冲零拷贝 | 尚未实现 |
| `presentation-time` | 必须 | 帧同步反馈：`presented`/`discarded`、时钟与刷新周期 | 已支持（`wp_presentation` v2，时钟为 `CLOCK_MONOTONIC`；feedback 与 `wl_surface.frame` 在**同一次遍历**里发出，因此两者对「这个输出画了哪些表面」不会各说一套；headless 与 DRM 的确认路径都走 `frame_presented_on`。`tests/presentation.rs` 用真实 toplevel 断言时钟、周期与 vsync 位，`tests/protocol_surface.rs` 断言可绑定） |
| `idle-inhibit` / `ext-idle-notify-v1` | 必须 | 空闲管理 | 两侧都通：客户端腿（`inox-idle`：空闲通知、经 `session-control-v1` 请求锁定、订阅 `inox_ipc` 的锁状态并只以合成器的 `Locked` 为确认）与**合成器腿**（`zwp_idle_inhibit_manager_v1` v1 + `ext_idle_notifier_v1` v1）。计时是**逐通知**的绝对截止时刻，任何输入都把它推到「现在 + timeout」；抑制只对「本合成器真的在画」的表面生效（活动工作区上未最小化的窗口、映射的 layer、锁表面），因此一个什么都不显示的表面无法长期按住空闲锁定。证据是 `tests/idle_notify.rs`（真实进程：零超时收到 `idled`、60 s 通知不发、注入按键后收到 `resumed`）与 `tests/protocol_surface.rs`（全局可绑定）。**仍缺**真机 E4 与 logind 挂起传输 |
| `pointer-constraints` / `relative-pointer` | 计划 | 游戏与 3D 应用 | 尚未实现 |
| `fractional-scale-v1` | M4 必须 | 分数缩放 | 已支持（M4；`wl_compositor` v6 的 `preferred_buffer_scale` + `wp_fractional_scale_manager_v1` v1，决策状态 `proposed`）。E1+E3 已取，E4 未取 |
| `wlr-output-management` | 计划 | 显示器管理 | 尚未实现 |
| `wlr-screencopy` / `ext-image-copy-capture` | 计划 | 截图与录屏 | 尚未实现 |
| `ext-session-lock-v1` | M5 必须 | 安全锁屏 | 已实现（M5；合成器广告 `ext_session_lock_manager_v1` v1 并以「会话令牌 + `inox_ipc` 认领」默认拒绝授权，`inox-lock --take-lock` 真实取锁、把界面画到每个锁表面、并在 `auth-v1` 接受用户后发送唯一的 `unlock_and_destroy`）。E3 会话用例已取，真机 E4 未取 |
| `zwp_primary_selection_device_manager_v1` | 必须 | 主选区（X11 中键粘贴兼容） | 已支持（M3；与 `wl_data_device_manager` 一并广告） |
| `xwayland-shell` | 计划 | XWayland 集成 | 尚未实现 |
| `text-input-v3` / `input-method-v2` / `virtual-keyboard-v1` | 提前实现 | 输入法与候选窗 | 管理器接口 v1，受信连接与锁屏隔离已接入；新增 CI 用例待运行，见[输入法](../usage/input-methods.md) |
| `inox-glass-v1`（**私有 Inox 协议**，全局 `inox_glass_manager_v1` v2） | 实验性（私有、unstable，非上游） | 液态玻璃材质：v1 请求背景模糊（`set_radius`/`set_region`），v2 加边缘折射（`set_corner_radii`/`set_refraction`，含 `depth`/`dispersion` 位）与 `liquid_capabilities` 事件。像素**不**回传、区域裁剪到该表面、仅 layer 角色生效、锁定会话报 0 | 绑定与两个版本的 socket 往返（`inox-protocols`，E0+E1）、`[glass]` 折射令牌、L3 的 `inox-glass` 材质决策与 QML 组件、合成器在 **headless** 路径的模糊 + 折射 pass（E1 参数测试 + E2 离屏像素判别性证据 `tests/glass_blur.rs`：边缘外移、内部不变、圆角遮罩、色散分离；`INOX_GLASS_ARTIFACT` 可导出 PPM）均已实现；**物理（DRM）路径报 0 且场景保持锐利**（未决项）。面板绑定 v2 并按 `inox-glass` 的决策请求（见 ADR 0040，状态 `proposed`） |

「当前状态」依据本仓库 `crates/inox-compositor/` 的实现：它在 headless 路径上向客户端广告的全局为 `wl_compositor`（v6）/ `wl_subcompositor`（Smithay `CompositorState`）、`wl_shm`、`wl_seat`、`wl_output`（每个输出一个）与 `xdg_wm_base`；M4 起新增 `wp_fractional_scale_manager_v1`（v1），M3 新增 `wp_viewporter`（Smithay `wayland::viewporter`）、`wl_data_device_manager`（v3）与 `zwp_primary_selection_device_manager_v1`（后两者来自 Smithay `wayland::selection`），随本分支合入，决策已记录（状态仍为 `proposed`）。L2 后续补齐 `zwlr_layer_shell_v1` v4：四层合成与输入命中、锚点/边距/独占区、configure/ack、空缓冲卸载与重映射、销毁和断连清理；键盘焦点覆盖 None、Exclusive、OnDemand，选择权限仍跟随实际焦点。M5 补上三条**必须**级协议：`wp_presentation`（v2）、`zxdg_decoration_manager_v1`（v1）、`zwp_idle_inhibit_manager_v1` + `ext_idle_notifier_v1`（各 v1）。`tests/protocol_surface.rs` 逐条断言这些全局都广告且可绑定，下面各节说明各自的取舍。其余标为未实现的行（`linux-dmabuf`、`wlr-output-management`、`wlr-screencopy`、`xwayland-shell`、`pointer-constraints`/`relative-pointer`）尚无对应实现。

## 帧反馈、装饰协商与空闲：三条补上的标准协议

### `presentation-time`：同一帧、同一批表面

`wl_surface.frame` 回答「你可以画下一帧了」，`wp_presentation` 回答「你这一帧什么时候真的出现在屏幕上」。两者是同一个集合上的两个问题，所以合成器在**一次遍历**里一起发（`InoxState::send_frame_feedback`）：窗口的整棵表面树（toplevel、subsurface、popup）与每个映射的 layer 表面，逐个 `present`。分开写两遍会立刻带来一个真实的 bug——两份「这个输出画了哪些表面」的清单迟早不一致，而客户端无法分辨自己收到的是哪一种。

时钟是 `CLOCK_MONOTONIC`，也就是 `std::time::Instant` 在 Linux 上读的那一个：`presented` 的时间戳与 frame callback 的时间因此可以直接比较，换用 `CLOCK_REALTIME` 会在系统时钟被调整时把两者分开。刷新周期取自输出自己的 mode，没有 mode 时发 `Refresh::Unknown` 而不是猜一个。

### `xdg-decoration`：广告它，是为了能回答「你自己画」

合成器**不画服务端装饰**：窗口矩形完全属于客户端，屏幕上唯一的非客户端标题栏是 X11 的，由 `xwayland-satellite` 自绘。因此每一个 `zxdg_toplevel_decoration_v1` 都只答 `client_side`。

那为什么还要广告这条协议？因为「没有全局」不等于「答案是 CSD」——它等于「客户端必须猜」。GTK 3/4 会猜 CSD 并画出自己的 header bar；SDL、GLFW 和部分 Qt 路径会猜「不要装饰」，于是用户得到一个既没有标题栏也无法拖动的窗口。答 `client_side` 是一条每个 toolkit 都懂、且与合成器实际画法一致的答案。版本停在 1：版本 2 放宽了「toplevel 已经提交缓冲后还能创建装饰对象」，并加了 `orphaned` 错误，这两条都没有实现，广告 2 就是一句不兑现的承诺。

### `idle-inhibit` / `ext-idle-notify-v1`：一条现在两端都通的链路

`inox-idle`（L4）一直有客户端腿：它向合成器要一条空闲通知，`idled` 到达时经 `session-control-v1` 请求锁定。缺的是另一端，所以空闲自动锁定从来没有真正发生过。现在合成器广告 `ext_idle_notifier_v1`（**v1**：v2 的 `get_input_idle_notification` 要求忽略抑制，而抑制语义正是这里的策略所在，广告 2 会带来一个不兑现的请求）与 `zwp_idle_inhibit_manager_v1`。

三条规则值得写下来，因为它们都是安全相关的：

- **计时是逐通知的绝对截止时刻**，从「创建 + timeout」开始，任何输入推成「现在 + timeout」。一个全局的「桌面空闲了」标志会让一个要 30 分钟的通知在一条 1 秒的通知触发时一起触发。
- **抑制只对「本合成器真的在画」的表面生效**：映射的 layer 表面、处于活动工作区且未最小化的窗口、锁定期间的锁表面。协议说的是「可见表面」，这里按字面执行——一个什么都不显示的表面不能按住空闲锁定，否则任何客户端都能靠一个从不映射的 `wl_surface`（或一个藏在别的工作区上的窗口）让会话永不自动锁定。
- **抑制期间不累积，且已经 `idled` 的通知会被 `resumed`**。把「锁定请求被按住」这件事告诉客户端，比事后才让它发生要诚实：`inox-idle` 因此可以撤回它排队的锁定请求。

一个 100 ms 的轮询来源（`IDLE_TICK`）负责比较截止时刻并发送事件。它不是每个通知一个 calloop 定时器：合成器状态并不拥有事件循环，它拿到的 `LoopHandle` 类型是运行时自己的数据，因此定时器不能借用这份状态。一次唤醒 10 次/秒的代价换掉了整条所有权管线。

剪贴板与拖放的读取权**跟随键盘焦点**：只有焦点客户端能读取或设置选区，不需要任何特权接口。因此需要全局、无焦点读取路径的剪贴板管理器（`wlr-data-control`）**不在 M3 范围内**——开放它必须先给出按客户端凭据的授权过滤，推迟到 M5；跨 X11/Wayland 边界的拖放则被上游阻塞（`xwayland-satellite` v0.8.3 未实现 XDND），M3 交付的是 Wayland↔Wayland 拖放与 X11 内部拖放。两条都有记录。

### `wlr-layer-shell`：工作区与层几何是两件事

层表面有两套坐标：**独占区（`exclusive_zone`）决定的是「别的窗口能用多少地方」**，而**表面自己的尺寸与位置来自它被放置的那个矩形**。这两件事在一个「不关心其他客户端独占区」的表面（`exclusive_zone = 0` 或 `-1`）上会分开：它是按整个输出放置的，也就必须按整个输出**取尺寸**。若把它按已扣掉面板后剩下的工作区来居中/拉伸，一个 `0×0` 的壁纸层只会和面板之外的那条区域一样高，屏幕上就留出一条什么都没有的带子。因此 `arrange_layers` 里「用哪个矩形去算几何」与「用哪个矩形去扣预留」是分开的：`DontCare` 的表面用输出矩形算几何（照样可以 `reserve`，`-1` 就是不扣），其余表面用已经扣过的矩形。`layers::dont_care_surfaces_keep_the_whole_output` 钉住这条区别。

层叠顺序与命中顺序是两回事：四层按 `Overlay → Top → Bottom → Background` 自上而下合成，输入命中则先问被每个输出选中的那个层（`Exclusive` 持有键盘、`OnDemand` 在指针进入时抢、`None` 不碰焦点），只有它不接的点击才落到窗口上。**安全锁屏不经这条路径**：锁定期间场景里只有锁表面，层表面连像素都不参与（见 `ext-session-lock-v1` 一节）。

至于证据：`tests/layer_shell.rs` 里的客户端是真的 `wayland-client`，但它是**写着测的替身**，不是 `inox-panel`。所以这个测试证明的是协议腿与合成行为，不能当作「面板在真实桌面上长这样」的证据——跨 L3 的 E3 仍缺（见[测试策略](../development/testing.md)与[路线图](../roadmap.md)）。

## 为什么新协议必须先落在 `inox-protocols`

根级 `AGENTS.md` §4 与分层规则规定：**新增 Wayland 协议前，必须先在 `inox-protocols` 定义或引用，并说明安全与生命周期约束。**

理由是协议不只属于合成器。同一条协议通常有两端：

- 合成器侧（L2）要广告全局并处理请求；
- Shell 侧（L3）要作为客户端绑定并使用它。

如果两端各自生成一份绑定，就会出现两份可能漂移的 `xml`/绑定代码，而且漂移不会在任一侧的 CI 中暴露 —— 在单一工作区内两份绑定会同时被编译，差异更容易被发现，但仍没有机制强制它们一致。把协议集中到 `inox-protocols` 后，协议成为**一个 L2 crate 的单一产物**，两端依赖的是同一份代码；破坏性变更因此有唯一入口与唯一的版本号，并由分层规则 4（协议只能从 `inox-protocols` 出口）固定下来。

`inox-protocols` 已实现，分两半：`standard` 把 Inox 目标的标准协议**再导出**（只再导出，不包裹、不重写上游绑定）—— 核心 `wl_*`、`xdg-shell`/`xdg-decoration`/`xdg-output`、`viewporter`、`linux-dmabuf`、`presentation-time`、`idle-inhibit`、`ext-idle-notify-v1`、`ext-session-lock-v1`（staging）与 `wlr-layer-shell`；`ipc` 与 `glass` 用 `wayland-scanner` 在编译期分别从 `protocol/inox-ipc.xml` 与 `protocol/inox-glass-v1.xml` 生成私有协议的客户端与服务端绑定；`control` 是 `session-control-v1` 这一类**非 Wayland 私有契约**的类型与帧（见下）。

`tests/standard_bindings.rs` 断言每个再导出的接口名与版本，并断言客户端/服务端两半都可到达；`tests/ipc_roundtrip.rs` 在真实连接上跑一次 `inox_ipc` 的版本查询、配置重载、锁屏角色认领与锁状态事件的往返；`tests/glass_roundtrip.rs` 在真实 Unix socket pair 上驱动 `inox_glass_manager_v1`：断言 `capabilities` 先于任何效果到达、半径与区域逐字到达服务端、非正尺寸原样过线（由合成器读作清空），且除整数半径外没有像素跨回客户端。注意「有绑定」不等于「有实现」：`wlr-layer-shell` 已由合成器广告，`inox-panel` 也确实在自己的 `layer.rs` 里创建真实 layer 表面（界面是离屏 QML 渲染的帧），但**真实合成器宿主上的截图断言仍缺**；`ext-session-lock-v1` 的客户端是 `inox-lock`，它同样自己建表面（见[组件清单](./components.md)）。

## Inox 私有协议（`inox-ipc`、`inox-glass-v1`、`inox-window-manager-v1` 与 `session-control-v1`）

除标准协议外，Inox 定义了自己的私有协议。`session-control-v1`（`inox-protocols/src/control.rs`，规范文本 `protocol/session-control-v1.md`）不是 Wayland 协议，而是会话监督进程的私有 Unix 套接字：一个动词 `lock`，回复 `ok`/`busy`/`unknown`，目录 `0700`、套接字 `0600`、对端 uid 必须相同。它只负责「谁有资格请求锁定」，不提供解锁能力（ADR 0041）。`inox-ipc` 用于合成器与 Shell 之间传递标准协议不覆盖的信息（配置重载通知、可信锁屏角色的认领与锁状态广播），接口版本为 **2**（v2 新增 `claim_lock_role` 与 `session_lock_state`）；消费者必须通过 `version` 事件协商，不得假定固定版本。

`inox-window-manager-v1` 是合成器与 Shell 之间的**窗口观察与控制**通道（ADR 0045，`proposed`）：`inox_window_manager_v1.get_window_list` 为每个调用创建一个 `inox_window_list_v1` 观察者，服务端先发快照（`session_generation` → `workspace_state` → 每个窗口一条 `window` → `snapshot_done`），之后发增量（`window` / `window_removed` / `workspace_state`）。`window` 事件带的是**完整状态**而不是增量：句柄、`app_id`、标题、工作区、输出、模式（平铺/浮动/最大化/全屏）、是否最小化、是否在当前工作区可见、是否有焦点——漏掉一条事件的观察者会在下一条上收敛。六个动词是任务栏需要的全部动作：`activate`、`minimize`、`close`、`set_mode`、`move_to_workspace`、`switch_workspace`。两条性质不可放宽：**一条请求由状态事件确认**（`window`/`window_removed` 就是确认，`request_failed` 是唯一的否定结果，合成器不会在改动生效前说「已完成」），以及**句柄只在它的 `session_generation` 内有意义**（代次在任何句柄之前发送，合成器重启会复用同一个整数表示别的窗口）。协议不授予任何像素：事件只有句柄、字符串与枚举，请求只是用户通过快捷键已经能做的动作；会话锁定时改动窗口可见性的请求以 `not_permitted` 拒绝。`tests/window_manager_roundtrip.rs` 在真实 socket pair 上断言快照顺序、`window` 事件的每个字段与六个动词的参数逐字到达。

`inox-glass-v1` 让客户端请求合成器处理自己表面背后的区域（液态玻璃材质）：`inox_glass_manager_v1.get_glass(surface)` 创建每表面一个的 `inox_glass_v1`。v1 的 `set_radius`/`set_region` 描述模糊，唯一事件 `capabilities` 报告合成器接受的最大半径。v2（当前接口版本）是加法式的：`set_corner_radii` 给出折射轮廓的四个圆角，`set_refraction(height, amount, flags)` 给出折射带厚度、轮廓峰值位移与可选位（`depth`、`dispersion`），`liquid_capabilities` 报告两个折射上限与实际支持的位；`capabilities` 仍然先发，v1 对象收不到 v2 事件、也发不出 v2 请求。像素**不**回传客户端，区域与折射轮廓裁剪到该表面，所有数值被钳制到上限，普通（非 layer 角色）表面与锁定会话都得到 0。合成器在 **headless** 路径实现了模糊与折射两个 pass（`tests/glass_blur.rs` 用无 glass 对象的对照帧做判别性像素断言：v1 边被抹开，v2 轮廓外的条纹向外推移、pane 内部与对照帧一致、圆角外不被覆盖、色散把 R/B 分开）；**物理（DRM）路径报 0 且把场景保持锐利**（它还没有诚实的 damage 描述）。决策、安全与生命周期见 ADR 0040（`proposed`）；L3 的 `inox-glass` 是唯一决定「要多少、拿不到怎么降级」的地方，面板经它绑定 v2。由于合成器只在 headless 路径真的画效果，这份实现仍不能被读作「真机上的液态玻璃已可用」。

- 私有协议状态为 **unstable**：破坏性变更必须版本化，并同步更新 PRD 的协议矩阵。
- 新增或改动私有协议同样属于「引入新的协议策略」，需要 ADR（PRD §5.4）。

## 相关页面

- 协议绑定的归属与依赖方向：[总体架构](./overview.md)；
