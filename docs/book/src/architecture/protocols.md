# Wayland 协议

协议支持矩阵以 PRD §2.3 为准。本页补充两点：矩阵中每一项的当前实现状态，以及「新协议必须先进 `inox-protocols`」这条规则的由来。

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
| `xdg-decoration` | 必须 | 服务端/客户端装饰协商；M3 不广告，X11 窗口标题栏由 satellite 自绘 | 尚未实现 |
| `wlr-layer-shell-unstable-v1` v4 | 必须 | 面板等桌面层；不用于安全锁屏 | 合成器已接入（真实客户端测试纳入 CI）；`inox-panel` 是真实 layer 客户端，界面由离屏 QML 渲染进 `wl_shm` 帧（ADR 0038）；启动器、通知与壁纸按 ADR 0038 是普通窗口。真实宿主截图证据（waydriver）仍未接线 |
| `viewporter`（`wp_viewporter`） | 必须 | 缩放与裁剪；`xwayland-satellite` 的宿主硬前提（缺失即启动失败） | 已支持（M3；Smithay `wayland::viewporter`） |
| `linux-dmabuf` | 必须 | GPU 缓冲零拷贝 | 尚未实现 |
| `presentation-time` | 必须 | 帧同步反馈 | 尚未实现 |
| `idle-inhibit` / `ext-idle-notify-v1` | 必须 | 空闲管理 | 尚未实现 |
| `pointer-constraints` / `relative-pointer` | 计划 | 游戏与 3D 应用 | 尚未实现 |
| `fractional-scale-v1` | M4 必须 | 分数缩放 | 已支持（M4；`wl_compositor` v6 的 `preferred_buffer_scale` + `wp_fractional_scale_manager_v1` v1，见 [ADR 0020](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0020-fractional-scaling.md)，状态 `proposed`）。E1+E3 已取，E4 未取 |
| `wlr-output-management` | 计划 | 显示器管理 | 尚未实现 |
| `wlr-screencopy` / `ext-image-copy-capture` | 计划 | 截图与录屏 | 尚未实现 |
| `ext-session-lock-v1` | M5 必须 | 安全锁屏 | 已实现（M5；合成器广告 `ext_session_lock_manager_v1` v1 并以「会话令牌 + `inox_ipc` 认领」默认拒绝授权，`inox-lock --take-lock` 真实取锁并把界面画到每个锁表面，见 [ADR 0022](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0022-m5-session-lock-notification-bus-and-first-privilege-boundary.md) 与 [ADR 0025](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0025-lock-screen-surface-ui-and-login-interface.md)）。**认证仍未交付**，因此界面呈现的是「本构建没有认证边界」；E3 会话用例已取，真机 E4 未取 |
| `zwp_primary_selection_device_manager_v1` | 必须 | 主选区（X11 中键粘贴兼容） | 已支持（M3；与 `wl_data_device_manager` 一并广告） |
| `xwayland-shell` | 计划 | XWayland 集成 | 尚未实现 |
| `text-input-v3` / `input-method-v2` / `virtual-keyboard-v1` | 提前实现 | 输入法与候选窗 | 管理器接口 v1，受信连接与锁屏隔离已接入；新增 CI 用例待运行，见[输入法](../usage/input-methods.md) |
| `inox-glass-v1`（**私有 Inox 协议**，全局 `inox_glass_manager_v1` v1） | 实验性（私有、unstable，非上游） | 桌面玻璃材质（背景模糊）：客户端请求合成器模糊自己表面背后的区域，`capabilities` 报半径上限，模糊像素不回传、区域裁剪到该表面、仅 layer 角色生效、锁定会话报 0 | 绑定与真实 socket 往返（`inox-protocols`，E0+E1）、`[glass]` 令牌、面板的菜单栏/dock 材质，以及合成器在 **headless** 路径的模糊 pass（E1 参数层测试 + E2 离屏像素判别性证据 `tests/glass_blur.rs`）均已实现；**物理（DRM）路径报 0 且场景保持锐利**（未决项）。面板已绑定本协议，按 `frost` 的决策发请求（`glass.blur == 0` 时根本不创建对象），但真实合成器上的磨砂效果未验证。见 [ADR 0031](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0031-compositor-backdrop-blur-and-the-glass-protocol.md)（`proposed`） |

「当前状态」依据本仓库 `crates/inox-compositor/` 的实现：它在 headless 路径上向客户端广告的全局为 `wl_compositor`（v6）/ `wl_subcompositor`（Smithay `CompositorState`）、`wl_shm`、`wl_seat`、`wl_output`（每个输出一个）与 `xdg_wm_base`；M4 起新增 `wp_fractional_scale_manager_v1`（v1），M3 新增 `wp_viewporter`（Smithay `wayland::viewporter`）、`wl_data_device_manager`（v3）与 `zwp_primary_selection_device_manager_v1`（后两者来自 Smithay `wayland::selection`），随本分支合入，决策见 [ADR 0017](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md)（状态仍为 `proposed`）。L2 后续补齐 `zwlr_layer_shell_v1` v4：四层合成与输入命中、锚点/边距/独占区、configure/ack、空缓冲卸载与重映射、销毁和断连清理；键盘焦点覆盖 None、Exclusive、OnDemand，选择权限仍跟随实际焦点。其余标为未实现的行尚无对应实现。

剪贴板与拖放的读取权**跟随键盘焦点**：只有焦点客户端能读取或设置选区，不需要任何特权接口。因此需要全局、无焦点读取路径的剪贴板管理器（`wlr-data-control`）**不在 M3 范围内**——开放它必须先给出按客户端凭据的授权过滤，推迟到 M5；跨 X11/Wayland 边界的拖放则被上游阻塞（`xwayland-satellite` v0.8.3 未实现 XDND），M3 交付的是 Wayland↔Wayland 拖放与 X11 内部拖放。两条都在 [ADR 0017](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md) 中记录。

## 为什么新协议必须先落在 `inox-protocols`

根级 `AGENTS.md` §4 与 [ADR 0002](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0002-layering-and-dependency-direction.md) 规定：**新增 Wayland 协议前，必须先在 `inox-protocols` 定义或引用，并说明安全与生命周期约束。**

理由是协议不只属于合成器。同一条协议通常有两端：

- 合成器侧（L2）要广告全局并处理请求；
- Shell 侧（L3）要作为客户端绑定并使用它。

如果两端各自生成一份绑定，就会出现两份可能漂移的 `xml`/绑定代码，而且漂移不会在任一侧的 CI 中暴露 —— 在单一工作区内两份绑定会同时被编译，差异更容易被发现，但仍没有机制强制它们一致。把协议集中到 `inox-protocols` 后，协议成为**一个 L2 crate 的单一产物**，两端依赖的是同一份代码；破坏性变更因此有唯一入口与唯一的版本号，并由 ADR 0002 规则 4（协议只能从 `inox-protocols` 出口）固定下来。

`inox-protocols` 已实现，分两半：`standard` 把 Inox 目标的标准协议**再导出**（只再导出，不包裹、不重写上游绑定）—— 核心 `wl_*`、`xdg-shell`/`xdg-decoration`/`xdg-output`、`viewporter`、`linux-dmabuf`、`presentation-time`、`idle-inhibit`、`ext-idle-notify-v1`、`ext-session-lock-v1`（staging）与 `wlr-layer-shell`；`ipc` 与 `glass` 用 `wayland-scanner` 在编译期分别从 `protocol/inox-ipc.xml` 与 `protocol/inox-glass-v1.xml` 生成私有协议的客户端与服务端绑定。

`tests/standard_bindings.rs` 断言每个再导出的接口名与版本，并断言客户端/服务端两半都可到达；`tests/ipc_roundtrip.rs` 在真实连接上跑一次 `inox_ipc` 的版本查询、配置重载、锁屏角色认领与锁状态事件的往返；`tests/glass_roundtrip.rs` 在真实 Unix socket pair 上驱动 `inox_glass_manager_v1`：断言 `capabilities` 先于任何效果到达、半径与区域逐字到达服务端、非正尺寸原样过线（由合成器读作清空），且除整数半径外没有像素跨回客户端。注意「有绑定」不等于「有实现」：`wlr-layer-shell` 已由合成器广告，`inox-panel` 也确实在自己的 `layer.rs` 里创建真实 layer 表面（界面是离屏 QML 渲染的帧），但**真实合成器宿主上的截图断言仍缺**；`ext-session-lock-v1` 的客户端是 `inox-lock`，它同样自己建表面（见[组件清单](./components.md)）。

## Inox 私有协议（`inox-ipc` 与 `inox-glass-v1`）

除标准协议外，Inox 定义了自己的私有协议。`inox-ipc` 用于合成器与 Shell 之间传递标准协议不覆盖的信息（配置重载通知、可信锁屏角色的认领与锁状态广播），接口版本为 **2**（v2 新增 `claim_lock_role` 与 `session_lock_state`，见 [ADR 0022](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0022-m5-session-lock-notification-bus-and-first-privilege-boundary.md)）；消费者必须通过 `version` 事件协商，不得假定固定版本。

`inox-glass-v1` 让客户端请求合成器模糊自己表面背后的区域（桌面玻璃材质）：`inox_glass_manager_v1.get_glass(surface)` 创建每表面一个的 `inox_glass_v1`，`set_radius`/`set_region` 描述效果，唯一事件 `capabilities` 报告合成器接受的最大半径。模糊像素**不**回传客户端，区域裁剪到该表面，半径被钳制到上限，普通（非 layer 角色）表面与锁定会话都得到 `max_radius = 0`。合成器已广告全局并在 **headless** 路径实现模糊 pass（`tests/glass_blur.rs` 用无 glass 对象的对照帧做判别性像素断言）；**物理（DRM）路径报 0 且把场景保持锐利**（它还没有诚实的 damage 描述）。决策、安全与生命周期见 [ADR 0031](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0031-compositor-backdrop-blur-and-the-glass-protocol.md)（`proposed`）；面板已经在 `layer.rs` 里绑定该协议、按 `frost.rs` 的决策请求（半径按 `capabilities` 钳制，`blur = 0` 时不创建对象），但合成器只在 headless 路径真的画模糊，因此它仍不能被读作「桌面上的磨砂已可用」。

- 私有协议状态为 **unstable**：破坏性变更必须版本化，并同步更新 PRD 的协议矩阵。
- 新增或改动私有协议同样属于「引入新的协议策略」，需要 ADR（PRD §5.4）。

## 相关页面

- 协议绑定的归属与依赖方向：[总体架构](./overview.md)；
- 协议变更的决策流程：[架构决策记录（ADR）](./decisions.md)。
