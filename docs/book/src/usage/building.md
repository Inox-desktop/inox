# 构建与运行

> **这不是一份「安装桌面环境」的指南。** Inox 目前只有 headless 合成器可以运行，还没有会话管理器、Shell、锁屏或 XWayland。本页说明今天可以构建并运行什么，以及哪些能力尚不存在。

## 今天能跑到什么程度

| 能力 | 现状 |
|------|------|
| 构建 `inox-compositor` 并启动 headless 合成器 | **可以**：创建 `wl_output`、接受客户端、平铺窗口、渲染帧 |
| 在物理显示器上看到画面 | **不行**：没有 DRM/KMS 后端，输出是逻辑输出，渲染到离屏缓冲 |
| 用真实键盘/鼠标操作 | **不行**：没有 libinput/udev 后端；输入事件只能由测试在进程内注入 |
| 运行 Shell（面板、启动器） | **不行**：`inox-shell` 等 L3 crate 仍是骨架（计划 M4） |
| 登录会话、锁屏、通知 | **不行**：`inox-session`、`inox-lock`、`inox-notifications` 仍是骨架（计划 M5） |
| 运行 X11 应用 | **不行**：XWayland 集成尚未合入 `main`（计划 M3） |
| 构建本站点 | **可以**：`mdbook build` |

## 前置条件

`inox-compositor` 的构建与运行需要：

| 依赖 | 用途 |
|------|------|
| Rust stable 工具链 | 根目录 `rust-toolchain.toml` 固定 stable、edition 2024；工作区统一 MSRV 为 1.86 |
| `libxkbcommon-dev` | 构建期：Smithay 的 `wayland_frontend` 在链接期需要 `-lxkbcommon` |
| `xkb-data` | 运行期：合成器用它提供的 rules 文件编译确定性 keymap（`evdev` / `pc105` / `us`） |
| EGL 与 Mesa DRI 驱动（如 `libegl1`、`libgl1-mesa-dri`） | 运行期：Smithay 的 `GlesRenderer` 通过 `dlopen` 加载 `libEGL.so.1`；无 GPU 时用 llvmpipe 软件渲染 |
| 一个已存在的绝对路径目录作为 `$XDG_RUNTIME_DIR` | Wayland 套接字在该目录下创建 |

系统包名以你的发行版为准；上表列出的是 Debian/Ubuntu 上的名字，与 CI 使用的包一致。

## 构建

```bash
# 调试构建
cargo build

# 发布构建
cargo build --release --all-features
```

产物是 `target/release/inox-compositor`。PRD §3.1 把它记作 `inox-comp`，那只是简写。

> 按根级 `AGENTS.md` §3.1，**本地构建结果不是验收依据**；验收一律以 CI 为准。这里给出构建命令是为了让你能实际运行，而不是为了替代 CI。

## 运行 headless 合成器

```bash
mkdir -p /tmp/inox-runtime

XDG_RUNTIME_DIR=/tmp/inox-runtime LIBGL_ALWAYS_SOFTWARE=1 \
  cargo run -- --headless --exit-after-frames 120
```

- `XDG_RUNTIME_DIR` 必须指向**已存在**的绝对路径目录，合成器不会替你创建它。
- `LIBGL_ALWAYS_SOFTWARE=1` 让 Mesa 走 llvmpipe 软件光栅化，在无 GPU 或驱动不可用的环境里也能渲染。
- `--exit-after-frames 120` 渲染 120 帧后以退出码 0 结束，便于脚本化验证；省略它则一直运行到收到终止信号。

进程会创建逻辑输出并渲染帧，但**屏幕上不会出现任何东西** —— 没有 DRM/KMS 后端，渲染目标是离屏缓冲。这是 headless 路径的定义，不是配置问题。

## 命令行选项

| 选项 | 说明 | 默认值 |
|------|------|--------|
| `--headless` | 使用 headless 后端（目前唯一可用的后端） | — |
| `--output-size <WxH>` | headless 输出尺寸（像素） | `1280x720` |
| `--refresh-rate <HZ>` | 输出刷新率（Hz） | `60` |
| `--socket <NAME>` | Wayland 套接字名 | 自动选择下一个空闲名字 |
| `--exit-after-frames <N>` | 渲染 N 帧后以退出码 0 退出 | 不限制 |
| `-h`, `--help` | 打印帮助并退出 | — |
| `-V`, `--version` | 打印版本并退出 | — |

尺寸上限为 16384 像素，刷新率上限为 1000 Hz，越界的取值会被拒绝。

终止信号 `SIGINT` / `SIGTERM` 会被转换为事件循环退出，**退出码 0**；日志走 `tracing`，默认级别 `info`，可用 `RUST_LOG` 调整。

## 用真实客户端验证

合成器可以接受任意 Wayland 客户端。把 `WAYLAND_DISPLAY` 指向合成器创建的套接字即可：

```bash
# 终端 1：让合成器占用一个固定的套接字名
XDG_RUNTIME_DIR=/tmp/inox-runtime LIBGL_ALWAYS_SOFTWARE=1 \
  cargo run -- --headless --socket wayland-1

# 终端 2：把任意 Wayland 客户端连上去（这里用 Weston 的示例客户端）
XDG_RUNTIME_DIR=/tmp/inox-runtime WAYLAND_DISPLAY=wayland-1 weston-simple-shm
```

窗口会按 master/stack 平铺排列（`master_ratio` 60%、`gap` 8 像素），但同样只在离屏缓冲中渲染，看不到画面。`Alt+Tab` / `Alt+Shift+Tab` 切换焦点、`Super+Q` 触发干净退出 —— 注意由于没有真实输入后端，这些按键绑定只有在输入事件经进程内注入路径送达时才生效，用真实键盘按下不会触发。

## 配置

合成器**目前只读取命令行参数**，不读取任何 TOML 文件。`~/.config/inox/` 下的配置文件布局与字段是 [ADR 0009](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0009-configuration-model-and-reload.md) 的提案，尚未在任何组件中实现 —— 详见[配置目录布局](../configuration/layout.md)与[合成器配置](../configuration/compositor.md)。

## 构建文档站点

本站点使用 mdBook：

```bash
# 构建到 book/
mdbook build

# 本地预览（会启动一个 HTTP 服务）
mdbook serve
```

本站点不引入 mdBook 插件或额外工具链；CI 通过 `mdbook build docs/book` 验证站点。
