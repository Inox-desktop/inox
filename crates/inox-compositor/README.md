# inox-compositor

基于 Smithay 的 Wayland 合成器。属于 Inox 桌面环境的 **L2** 层。

## 状态

M1：headless 合成器已实现——创建 `wl_output`、以 EGL surfaceless（`EGL_MESA_platform_surfaceless`）+ GLES 渲染纯色帧、收到终止信号或渲染完指定帧数后干净退出（退出码 0）。渲染路径与 `unsafe` 策略见 [ADR 0005](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0005-headless-compositor-runtime.md)。

M2（PRD §13）：在 M1 的 headless 路径上扩展——通过 `wl_seat` 处理键盘/指针事件（确定性 keymap、修饰键跟踪、焦点投递），以 `xdg-shell` 客户端 toplevel 作为窗口并经 Smithay `desktop::Space` 管理 **≥2 个窗口**，按 master/stack 平铺布局（`proptest` 验证块不重叠、不越界、覆盖率完整）。键位绑定为纯逻辑（`Alt+Tab`/`Alt+Shift+Tab` 切换焦点、`Super+Q` 干净退出）；输入事件通过进程内注入 API 驱动，集成测试用**真实 Wayland 客户端**断言窗口数与平铺落点。M2 沿用 headless 路径（**不启用** libinput/udev/DRM 后端），决策见 [ADR 0008](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0008-m2-input-windows-tiling.md)。

合成器同时广告私有 `inox_ipc` 全局（来自 [`inox-protocols`](../../crates/inox-protocols)）：`get_version` 返回协议版本，`reload_config` 现阶段以 `unsupported` 应答，配置热重载在后续里程碑接入。

M3（PRD §13，[ADR 0017](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md)，承接 [ADR 0007](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0007-xwayland-satellite-integration.md) 的进程边界）：合成器侧补齐 XWayland 互操作所需的协议面。

- 新增三个全局：`wp_viewporter`（v1，`xwayland-satellite` 缩放 X11 窗口的前提，缺了它合成器根本无法充当宿主）、`wl_data_device_manager`（v3，剪贴板与拖放）与 `zwp_primary_selection_device_manager_v1`（v1，中键粘贴用的主选区）。完整全局清单见 `src/state.rs` 的模块文档。
- **选择可见性跟随键盘焦点**：只有获得焦点的客户端会被投递选择（也只有它能设置选择）。`sync_focus` 是合成器唯一改动键盘焦点的地方，因此也是唯一交接选择焦点的地方——这正是 X11 ↔ Wayland 剪贴板桥能工作的原因：`xwayland-satellite` 只是一个普通 Wayland 客户端，它拿到焦点才拿得到 X11 的选择。
- **每帧渲染后发送 `wl_surface.frame` 回调**：此前从不发送，真实客户端（XWayland 顶层、GTK 应用）会因此永远等不到重绘时机。回调由事件循环在每帧渲染后 flush 出去，焦点变更排入的选择事件同样如此。
- 拖放不需要额外合成器逻辑：客户端调用 `start_drag` 后 Smithay 的 `DnDGrab` 接管指针并转发 enter/motion/drop/action。

真实后端已按获批的 [ADR 0012](../../docs/adr/0012-output-input-and-frame-lifecycle.md)
接入 libseat、DRM/GBM、udev 与 libinput。代码完成不代表 O01/O04 真机验收通过，支持边界见下节。

L2 的 layer-shell 宿主现已接入 Smithay `WlrLayerShellState`，广告
`zwlr_layer_shell_v1` v4。四层按 background → bottom → 普通窗口 → top → overlay
合成并命中测试，支持锚点、边距、独占区、初次 configure/ack、空缓冲卸载、重新映射与销毁清理。
普通窗口平铺在剩余工作区，背景与渲染目标仍使用完整输出尺寸。`None` 不接收键盘焦点，
top/overlay 的 `Exclusive` 优先，`OnDemand` 通过点击进入、点击应用或 Alt+Tab 离开；
选择焦点继续跟随实际键盘焦点。layer-shell 只用于普通桌面，不能替代安全锁屏。

`tests/layer_shell.rs` 用真实客户端与 shm 缓冲断言像素、指针、焦点、frame callback、
独占区恢复和坏客户端隔离；CI 的现有合成器分片会执行它。本次没有更换 Iced 客户端宿主，
L3 的 layer-shell 客户端接线仍待完成，亦不宣称真机 DRM 或完整 M4 已验收。

### 多输出、多 GPU 与缩放（ADR 0019 / ADR 0020）

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
- 真实会话启动时读取 `InoxConfig` 的输入、平铺、主题及输出 mode；不支持的 mode/scale
  明确拒绝。键盘重复 rate 为 0 关闭 repeat；delay/rate 必须可表示为 Wayland 有符号字段。
  headless 保持确定性的测试配置。运行中的配置事务与重载回执仍待后续工作。

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

## 开发约定

- 本项目遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- `--test-input` 只服务于测试；改动它时请同步维护 `src/test_input.rs` 的模块文档与 [`AGENTS.md`](./AGENTS.md) 的测试要求。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，不在本地进行编译验证。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)。

## 许可证

BSD 3-Clause，见仓库根的 [`LICENSE`](../../LICENSE)。
