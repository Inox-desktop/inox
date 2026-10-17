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
| `wlr-layer-shell-unstable-v1` v4 | 必须 | 面板/壁纸等普通桌面层；不用于安全锁屏 | 合成器已接入，真实客户端测试纳入 CI；L3 的 Iced 客户端接线未完成 |
| `viewporter`（`wp_viewporter`） | 必须 | 缩放与裁剪；`xwayland-satellite` 的宿主硬前提（缺失即启动失败） | 已支持（M3；Smithay `wayland::viewporter`） |
| `linux-dmabuf` | 必须 | GPU 缓冲零拷贝 | 尚未实现 |
| `presentation-time` | 必须 | 帧同步反馈 | 尚未实现 |
| `idle-inhibit` / `ext-idle-notify-v1` | 必须 | 空闲管理 | 尚未实现 |
| `pointer-constraints` / `relative-pointer` | 计划 | 游戏与 3D 应用 | 尚未实现 |
| `xfractional-scale-v1` | 计划 | 分数缩放 | 尚未实现 |
| `wlr-output-management` | 计划 | 显示器管理 | 尚未实现 |
| `wlr-screencopy` / `ext-image-copy-capture` | 计划 | 截图与录屏 | 尚未实现 |
| `ext-session-lock-v1` | 计划 | 锁屏 | 尚未实现 |
| `zwp_primary_selection_device_manager_v1` | 必须 | 主选区（X11 中键粘贴兼容） | 已支持（M3；与 `wl_data_device_manager` 一并广告） |
| `xwayland-shell` | 计划 | XWayland 集成 | 尚未实现 |
| `wlr-virtual-keyboard` / `input-method-v2` | 仅规划 | 输入法（1.0 之后） | 尚未实现 |

「当前状态」依据本仓库 `crates/inox-compositor/` 的实现：它在 headless 路径上向客户端广告的全局为 `wl_compositor` / `wl_subcompositor`（Smithay `CompositorState`）、`wl_shm`、`wl_seat`、`wl_output` 与 `xdg_wm_base`；M3 新增 `wp_viewporter`（Smithay `wayland::viewporter`）、`wl_data_device_manager`（v3）与 `zwp_primary_selection_device_manager_v1`（后两者来自 Smithay `wayland::selection`），随本分支合入，决策见 [ADR 0017](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md)（状态仍为 `proposed`）。L2 后续补齐 `zwlr_layer_shell_v1` v4：四层合成与输入命中、锚点/边距/独占区、configure/ack、空缓冲卸载与重映射、销毁和断连清理；键盘焦点覆盖 None、Exclusive、OnDemand，选择权限仍跟随实际焦点。其余标为未实现的行尚无对应实现。

剪贴板与拖放的读取权**跟随键盘焦点**：只有焦点客户端能读取或设置选区，不需要任何特权接口。因此需要全局、无焦点读取路径的剪贴板管理器（`wlr-data-control`）**不在 M3 范围内**——开放它必须先给出按客户端凭据的授权过滤，推迟到 M5；跨 X11/Wayland 边界的拖放则被上游阻塞（`xwayland-satellite` v0.8.3 未实现 XDND），M3 交付的是 Wayland↔Wayland 拖放与 X11 内部拖放。两条都在 [ADR 0017](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md) 中记录。

## 为什么新协议必须先落在 `inox-protocols`

根级 `AGENTS.md` §4 与 [ADR 0002](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0002-layering-and-dependency-direction.md) 规定：**新增 Wayland 协议前，必须先在 `inox-protocols` 定义或引用，并说明安全与生命周期约束。**

理由是协议不只属于合成器。同一条协议通常有两端：

- 合成器侧（L2）要广告全局并处理请求；
- Shell 侧（L3）要作为客户端绑定并使用它。

如果两端各自生成一份绑定，就会出现两份可能漂移的 `xml`/绑定代码，而且漂移不会在任一侧的 CI 中暴露 —— 在单一工作区内两份绑定会同时被编译，差异更容易被发现，但仍没有机制强制它们一致。把协议集中到 `inox-protocols` 后，协议成为**一个 L2 crate 的单一产物**，两端依赖的是同一份代码；破坏性变更因此有唯一入口与唯一的版本号，并由 ADR 0002 规则 4（协议只能从 `inox-protocols` 出口）固定下来。

`inox-protocols` 已实现，分两半：`standard` 把 Inox 目标的标准协议**再导出**（只再导出，不包裹、不重写上游绑定）—— 核心 `wl_*`、`xdg-shell`/`xdg-decoration`/`xdg-output`、`viewporter`、`linux-dmabuf`、`presentation-time`、`idle-inhibit`、`ext-idle-notify-v1` 与 `wlr-layer-shell`；`ipc` 用 `wayland-scanner` 在编译期从 `protocol/inox-ipc.xml` 生成私有协议的客户端与服务端绑定。

`tests/standard_bindings.rs` 断言每个再导出的接口名与版本，并断言客户端/服务端两半都可到达；`tests/ipc_roundtrip.rs` 在真实连接上跑一次 `inox_ipc` 的版本查询与配置重载往返。注意「有绑定」不等于「有实现」：`wlr-layer-shell` 已由合成器广告，但 L3 的 Iced 客户端尚未接入真实 layer 表面（见[组件清单](./components.md)）。

## Inox 私有协议（`inox-ipc`）

除标准协议外，Inox 定义了私有通信协议 `inox-ipc`，用于合成器与 Shell 之间传递标准协议不覆盖的信息（如配置重载通知）。

- 私有协议状态为 **unstable**：破坏性变更必须版本化，并同步更新 PRD 的协议矩阵。
- 新增或改动私有协议同样属于「引入新的协议策略」，需要 ADR（PRD §5.4）。

## 相关页面

- 协议绑定的归属与依赖方向：[总体架构](./overview.md)；
- 协议变更的决策流程：[架构决策记录（ADR）](./decisions.md)。
