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
| `wl_data_device` | 必须 | 剪贴板与拖放 | 尚未实现 |
| `xdg-shell` | 必须 | 标准窗口 | 已支持（toplevel 作为窗口） |
| `xdg-decoration` | 必须 | 服务端/客户端装饰协商 | 尚未实现 |
| `wlr-layer-shell-unstable-v1` | 必须 | 面板/壁纸/锁屏层 | 尚未实现 |
| `viewporter` | 必须 | 缩放与裁剪 | 尚未实现 |
| `linux-dmabuf` | 必须 | GPU 缓冲零拷贝 | 尚未实现 |
| `presentation-time` | 必须 | 帧同步反馈 | 尚未实现 |
| `idle-inhibit` / `ext-idle-notify-v1` | 必须 | 空闲管理 | 尚未实现 |
| `pointer-constraints` / `relative-pointer` | 计划 | 游戏与 3D 应用 | 尚未实现 |
| `xfractional-scale-v1` | 计划 | 分数缩放 | 尚未实现 |
| `wlr-output-management` | 计划 | 显示器管理 | 尚未实现 |
| `wlr-screencopy` / `ext-image-copy-capture` | 计划 | 截图与录屏 | 尚未实现 |
| `ext-session-lock-v1` | 计划 | 锁屏 | 尚未实现 |
| `gtk-primary-selection` | 计划 | X11 主选区兼容 | 尚未实现 |
| `xwayland-shell` | 计划 | XWayland 集成 | 尚未实现 |
| `wlr-virtual-keyboard` / `input-method-v2` | 仅规划 | 输入法（1.0 之后） | 尚未实现 |

「当前状态」依据 `inox-compositor` 的 `main` 分支：它在 headless 路径上向客户端广告的全局为 `wl_compositor` 与 `wl_subcompositor`（由 Smithay `CompositorState` 创建）、`wl_shm`、`wl_seat`、`wl_output` 与 `xdg_wm_base`。其余行在代码中没有任何对应实现。

## 为什么新协议必须先落在 `inox-protocols`

根级 `AGENTS.md` §4 与 [ADR 0002](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0002-layering-and-dependency-direction.md) 规定：**新增 Wayland 协议前，必须先在 `inox-protocols` 定义或引用，并说明安全与生命周期约束。**

理由是协议不只属于合成器。同一条协议通常有两端：

- 合成器侧（L2）要广告全局并处理请求；
- Shell 侧（L3）要作为客户端绑定并使用它。

如果两端各自生成一份绑定，就会出现两份可能漂移的 `xml`/绑定代码，而且漂移不会在任一侧的 CI 中暴露 —— 在单一工作区内两份绑定会同时被编译，差异更容易被发现，但仍没有机制强制它们一致。把协议集中到 `inox-protocols` 后，协议成为**一个 L2 crate 的单一产物**，两端依赖的是同一份代码；破坏性变更因此有唯一入口与唯一的版本号，并由 ADR 0002 规则 4（协议只能从 `inox-protocols` 出口）固定下来。

`inox-protocols` 当前仍是骨架，尚未定义任何协议绑定或 Inox 私有协议。

## Inox 私有协议（`inox-ipc`）

除标准协议外，Inox 定义了私有通信协议 `inox-ipc`，用于合成器与 Shell 之间传递标准协议不覆盖的信息（如配置重载通知）。

- 私有协议状态为 **unstable**：破坏性变更必须版本化，并同步更新 PRD 的协议矩阵。
- 新增或改动私有协议同样属于「引入新的协议策略」，需要 ADR（PRD §5.4）。

## 相关页面

- 协议绑定的归属与依赖方向：[总体架构](./overview.md)；
- 协议变更的决策流程：[架构决策记录（ADR）](./decisions.md)。
