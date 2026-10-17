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

尚未实现（后续里程碑）：XWayland 服务端本身（在 [`inox-xwayland`](../../crates/inox-xwayland)）、真实输入设备链路（libinput/udev/seat）、DRM/KMS 真实输出。

### 运行

```console
# 无显示设备的环境（CI 用的软件渲染）
XDG_RUNTIME_DIR=/tmp/inox-runtime LIBGL_ALWAYS_SOFTWARE=1 \
  cargo run -- --headless --exit-after-frames 120

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
