# 构建与运行

> **这还不是一份可用于日常桌面的安装指南。** `inox-session` 已能通过真实 Wayland 往返等待合成器并监督 L3 子进程；七个 L3 组件的界面已从 Iced 迁到 **QML + Qt Quick**（经 cxx-qt 绑定），面板与锁屏仍持有真实的 layer-shell / 会话锁表面（QML 离屏渲染），其余五个是普通 Qt 窗口；完整服务 ready 与真实宿主截图证据（waydriver）尚未接线。锁屏的协议腿与界面已经落地。**认证边界与开机登录链**已接线（`inox-host` + `packaging/`），但**真机登录尚未取得证据**；挂起仍未接线。本页说明可运行路径及其限制，登录链的安装、试运行与回退另见[登录与开机链](login.md)。

## 今天能跑到什么程度

| 能力 | 现状 |
|------|------|
| 构建 `inox-compositor` 并启动 headless 合成器 | **可以**：创建 `wl_output`、接受客户端、平铺窗口、渲染帧 |
| 在物理显示器上看到画面 | **不行**：没有 DRM/KMS 后端，输出是逻辑输出，渲染到离屏缓冲 |
| 用真实键盘/鼠标操作 | **不行**：没有 libinput/udev 后端；输入事件只能由测试注入（M3 起经测试专用套接字，需 `test-input` feature 构建） |
| 构建并启动 L3 组件（面板、启动器、设置、通知、锁屏、壁纸、Shell 框架） | **可以，但看不到桌面层**：七个组件的界面是 QML + Qt Quick。面板与锁屏**不是普通窗口**——它们自己创建 `zwlr_layer_shell_v1` / `ext_session_lock_v1` 表面，QML 只做离屏渲染，所以需要支持对应协议的合成器，缺协议时以可读错误与非零码退出；启动器、设置、通知与壁纸是普通 Qt 窗口，放置策略只被建模与测试、未被应用；`inox-lock` 默认以可读错误非零退出（认证边界未评审），`--take-lock` 则真实取锁并把界面画到每个输出上 |
| 启动与监督会话 | **部分可以**：`inox-session` 能启动合成器及桌面组件，提供有限重启、故障隔离和注销；尚无完整服务 ready，也未接入 XWayland 的 DISPLAY 发布 |
| 真实登录、自动锁定与挂起 | **未验证**：登录链（`inox-host` 起合成器与登录界面，`inox-auth` 认证并建立用户会话）与部署面 `packaging/` 已接线，但**没有任何真机登录证据**；`inox-greet` 不带 `--preview` 时要求认证边界，`inox-idle` 有协议客户端与策略，而 Inox 宿主尚无 idle notifier，可信锁/挂起通道未接线。步骤见[登录与开机链](login.md) |
| 锁定会话、收发通知 | **部分可以**：`inox-lock --take-lock` 会话内真的会锁住屏幕并画出界面，`inox-notifications` 提供 `org.freedesktop.Notifications`；但锁屏**不能认证任何人**，因此它会如实显示「本构建没有认证边界」，释放会话只能靠注销 |
| 运行 X11 应用 | **可以，但看不到画面**：合成器已广告 satellite 所需的宿主协议（含 `wp_viewporter`），`inox-xwayland` 可在会话中启动并监督它；运行时需自备 `xwayland-satellite`（≥ v0.8.3）与 `Xwayland` |
| 剪贴板与拖放 | **可以（仅限 headless 内）**：剪贴板读取权跟随键盘焦点；Wayland↔Wayland 拖放与 X11 内部拖放可用；跨 X11/Wayland 边界的拖放**不可用**（上游 satellite 未实现 XDND） |
| 构建本站点 | **可以**：`mdbook build` |

## L4 会话入口

使用同一次 CI 产出的完整二进制目录，在当前用户私有的 `XDG_RUNTIME_DIR` 下运行：

```bash
dbus-run-session -- /path/to/bin/inox-session --headless
```

`--headless` 显式选择软件渲染；省略它会要求真实设备后端。`--bin-dir PATH` 可以选择另一套完整二进制，默认使用 session 自身所在目录。SIGINT/SIGTERM 请求注销；`--exit-after-seconds N` 从启动时开始计时，到期请求正常注销。合成器故障会结束会话，不恢复已断开的应用连接。`inox-idle` 是可选服务，宿主缺少空闲协议时会有限重试后报告退化。

### 锁住会话（并且只能靠注销释放）

会话在 `INOX_SESSION_TAKE_LOCK=1` 时会启动可信锁屏客户端：

```bash
INOX_SESSION_TAKE_LOCK=1 dbus-run-session -- /path/to/bin/inox-session --headless
```

也可以单独运行 `inox-lock --take-lock`，但它需要 `inox-session` 投递的 `INOX_LOCK_TOKEN`，手工会话拿不到。

**这条模式没有解锁路径，误用会把会话留在锁定状态。** 它经 `inox_ipc` 用一次性令牌认领可信角色、请求锁、给每个输出一块锁表面，并把锁屏界面画上去；它**不**读键盘、**不**校验任何凭据，收到 `SIGTERM`/`SIGINT` 时退出而**不**发 `unlock_and_destroy`，所以合成器保持锁定，恢复只能靠注销。

屏幕上是**不可用**画面而不是密码提示，这是刻意的：本构建没有经过评审的认证边界，状态里的事实是「没有边界」，界面因此说它没法校验任何东西。这是会话锁边界要求的人类安全评审项，不是本页的安装步骤能绕过的。

在不带 `--take-lock` 的默认路径下，`inox-lock` 会打印原因并以非零状态码退出——会话脚本据此就能看出「这一次并没有锁上」。

### 登录界面

在可用 Wayland 宿主中，`inox-greet --preview` 打开登录界面。要看清它显示的是什么：**界面为六个生命周期阶段各有一幅画面，而这台构建只会到达其中一幅**。

界面在设计上覆盖不可用、就绪、验证中、会话启动中、失败与已启动六个阶段，键盘（Tab/Shift+Tab/Enter/Escape）与每个禁用控件的原因都已建模，并由测试逐阶段覆盖。但 `--preview` 把认证传输的可用性固定为「没有」——这正是本构建的事实——所以它显示的是**不可用**那一幅：口令字段不可输入，登录按钮禁用，键盘只剩 Escape（清空）。它**不采集密码、不认证、不启动用户会话**，界面上也写着这一点。

身份不靠键入：界面读系统账户库（`getent passwd`，回落 `/etc/passwd`）列出可登录的账户，默认选中本 greeter 上次登录成功过的那个（记录在 `$XDG_STATE_HOME/inox/greet/last-user`），账户多于一个时可以「切换用户」。账户库读不到时界面仍然打开，并说明是读不到列表还是列表里没有可登录的账户。

换句话说：预览展示的是诚实的状态，不是一次演示。其余阶段只出现在**真实路径**上——由 `inox-host` 在裸 VT 上拉起、边界套接字可连时（见[登录与开机链](login.md)）——以及测试与截图工件里；没有运行的二进制会因为一个开关就假装自己有认证边界。

安全接线与证据边界见 L4 会话服务的决策。

### 开机登录链（安装面）

登录链不再靠手工起进程：`packaging/` 提供三个 PAM 栈与三个 systemd 单元
（`inox-auth.service`、`inox-greeter@<VT>.service`、登录界面起不来时启动 greetd 的
`inox-login-fallback.service`），以及安装、试运行、预检、回退、卸载等脚本。
默认安装**不改开机链**，试运行用 VT4 并保留原显示管理器；`install.sh --enable` 才把
`display-manager.service` 交给 `inox-greeter@1.service`。完整步骤、验证项与排障见[登录与开机链](login.md)。
**真机证据尚未取得**，因此本页与那页都不构成「已能登录」的声明。

## 前置条件

构建整个工作区、运行合成器与 QML 组件需要：

| 依赖 | 用途 |
|------|------|
| Rust stable 工具链 | 根目录 `rust-toolchain.toml` 固定 stable、edition 2024；工作区统一 MSRV 为 1.89 |
| `libxkbcommon-dev` | 构建期：Smithay 的 `wayland_frontend` 在链接期需要 `-lxkbcommon` |
| `xkb-data` | 运行期：合成器用它提供的 rules 文件编译确定性 keymap（`evdev` / `pc105` / `us`） |
| EGL 与 Mesa DRI 驱动（如 `libegl1`、`libgl1-mesa-dri`） | 运行期：Smithay 的 `GlesRenderer` 通过 `dlopen` 加载 `libEGL.so.1`；无 GPU 时用 llvmpipe 软件渲染 |
| 一个已存在的绝对路径目录作为 `$XDG_RUNTIME_DIR` | Wayland 套接字在该目录下创建 |
| `xwayland-satellite`（≥ v0.8.3）与 `Xwayland` | 运行期：只有要跑 X11 应用时才需要，二者都不参与构建；`inox-xwayland` 用 `--satellite <PATH>` 拉起前者，由它自行 spawn 后者 |
| `qt6-base-dev`、`qt6-declarative-dev` | 构建期：七个 QML 组件与 `inox-qml-prototype` 通过 cxx-qt 链接 Qt6Core/Gui/Qml/Quick，构建脚本用 `qmake`/`pkg-config` 解析它们；缺 Qt 时 `cxx-qt` 的构建脚本直接 panic，连 `cargo check` 都过不去 |
| `qml6-module-qtquick`、`-qtquick-window`、`-qtquick-templates`、`-qtquick-controls`、`-qtquick-layouts`、`-qtquick-dialogs`、`-qtqml-workerscript` | 运行期：QML 组件 `import` 的模块，缺一个模块就加载失败（包名以 Debian/Ubuntu 为准） |
| `qt6-wayland-dev` | 构建/运行期：普通窗口组件经 Qt 的 Wayland 客户端建连 |
| `fonts-dejavu-core`、`fonts-noto-cjk`（或任意已安装字体） | 运行期：Qt 在出帧时自己排版文本，没有字体时标签是空的 |
| `lld` | 链接期：`qt-build-utils` 为 Qt 相关的链接步骤选用 lld；用 gold 时 cxx-qt 的 `cxx_qt_init_*` 符号会变成 `undefined reference` |

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
| `--test-input <PATH>` | **仅测试**：在 UNIX 套接字 `<PATH>` 上接受注入的键盘/指针事件；需以非默认 feature `test-input` 构建，否则该选项被明确拒绝 | 未启用 |
| `-h`, `--help` | 打印帮助并退出 | — |
| `-V`, `--version` | 打印版本并退出 | — |

尺寸上限为 16384 像素，刷新率上限为 1000 Hz，越界的取值会被拒绝。

`test-input` 不是默认 feature：默认构建中不包含该代码，`--all-features` 构建会把它编译进来但默认不启用。该套接字**不是 Wayland 全局**，除「能写入该套接字」外不授予任何权限，只为测试存在。

终止信号 `SIGINT` / `SIGTERM` 会被转换为事件循环退出，**退出码 0**；日志走 `tracing`，默认级别 `info`，可用 `RUST_LOG` 调整。

## 运行 L3 组件

七个 L3 组件都是独立的二进制，配置从 `~/.config/inox/` 读取（缺失即用默认值，见[配置目录布局](../configuration/layout.md)）：

```bash
cargo run -p inox-shell          # 解析入口注册表并报告，然后退出
cargo run -p inox-panel          # 面板：真实 layer 表面（需要支持 wlr-layer-shell 的合成器）
cargo run -p inox-launcher       # 启动器：普通 Qt 窗口
cargo run -p inox-settings       # 设置：普通 Qt 窗口（唯一会写配置的组件）
cargo run -p inox-wallpaper      # 壁纸预览：普通 Qt 窗口
cargo run -p inox-notifications  # 通知的弹窗层：普通 Qt 窗口
cargo run -p inox-lock           # 锁屏：打印原因并以非零状态退出（认证边界未评审；--take-lock 见下文）
```

两种形态要分开看：

- **面板与锁屏不是普通窗口。** 它们在自己的 `calloop` 循环里创建 `zwlr_layer_shell_v1` / `ext_session_lock_v1` 表面，QML 只负责把界面渲染成帧写进 `wl_shm`。缺 `wl_compositor`、`wl_shm`、`zwlr_layer_shell_v1` 中的任何一个，面板都会以可读错误与非零码退出——它不会退回成一个浮窗来假装自己摆了位。
- **启动器、设置、通知与壁纸是普通 Qt 窗口**，由 Qt 事件循环驱动，QML 同时负责渲染与交互。`placement.rs` 里的锚点、独占区与边距仍是**已经测试但尚未应用**的数据。

界面本身是 QML + Qt Quick（Rust 投影 JSON，QML 只绑定），所以它们的测试不需要合成器也不需要 GPU：在 `QT_QPA_PLATFORM=offscreen` 下断言投影、加载 QML，面板与锁屏还会读回自己提交的离屏帧。命令与 CI 逐字一致：

```bash
QT_QPA_PLATFORM=offscreen cargo test -p inox-panel --all-features
```

迁移后这些测试**不写 PNG 截图**：Iced 时代的 `tests/render.rs` 与 `INOX_SCREENSHOT_DIR` 装置随 Iced 一起删除了，本页不声称存在截图工件（原因见[测试策略](../development/testing.md)）。

语言由 `config.toml` 的 `[locale]` 或环境变量（`LC_ALL` / `LC_MESSAGES` / `LANG`）决定，`en-US` 与 `zh-CN` 都有完整目录。

从源码检出直接运行时，内置壁纸还没有装到 `$XDG_DATA_DIRS/inox/wallpapers/`，壁纸组件会退到主题背景并记录一条
`warn!`。用 `INOX_WALLPAPER_DIR` 指向仓库里的那份即可看到真正的默认壁纸：

```bash
INOX_WALLPAPER_DIR="$PWD/data/wallpapers" cargo run -p inox-wallpaper
```

## 用真实客户端验证

合成器可以接受任意 Wayland 客户端。把 `WAYLAND_DISPLAY` 指向合成器创建的套接字即可：

```bash
# 终端 1：让合成器占用一个固定的套接字名
XDG_RUNTIME_DIR=/tmp/inox-runtime LIBGL_ALWAYS_SOFTWARE=1 \
  cargo run -- --headless --socket wayland-1

# 终端 2：把任意 Wayland 客户端连上去（这里用 Weston 的示例客户端）
XDG_RUNTIME_DIR=/tmp/inox-runtime WAYLAND_DISPLAY=wayland-1 weston-simple-shm
```

窗口会按 master/stack 平铺排列（`master_ratio` 60%、`gap` 8 像素），但同样只在离屏缓冲中渲染，看不到画面。`Alt+Tab` / `Alt+Shift+Tab` 切换焦点、`Super+Q` 触发干净退出 —— 注意由于没有真实输入后端，这些按键绑定只有在输入事件经测试注入路径（M3 起为 `test-input` 套接字）送达时才生效，用真实键盘按下不会触发。

## 运行 X11 应用（XWayland）

X11 支持由**外部 `xwayland-satellite` 进程**提供：`inox-xwayland` 只负责在会话中拉起、监督与终止它，不链接它的 crate（决策已记录）。satellite 与它自行 spawn 的 `Xwayland` 都是**运行期系统依赖**，请按你的发行版或上游源码安装。

```bash
# 终端 1：合成器（它广告的协议面包含 satellite 的宿主前提 xdg-shell 与 wp_viewporter）
XDG_RUNTIME_DIR=/tmp/inox-runtime LIBGL_ALWAYS_SOFTWARE=1 \
  cargo run -p inox-compositor -- --headless --socket wayland-1

# 终端 2：把这个显示接入会话（用 --display 指定显示号，默认 :1）
XDG_RUNTIME_DIR=/tmp/inox-runtime WAYLAND_DISPLAY=wayland-1 \
  cargo run -p inox-xwayland -- --display :1 --ready-file /tmp/inox-xwayland.ready

# 终端 3：启动一个真实的 X11 应用
DISPLAY=:1 xeyes
```

就绪判定是**一次真实的 X11 连接建立往返**，不是「套接字文件存在」：就绪后日志出现 `XWayland ready on :1`，`--ready-file` 也会写入 `:1`。X11 侧的屏幕几何来自合成器的 `wl_output` 事件（satellite 以 `-force-xrandr-emulation` 启动 Xwayland），因此合成器必须持续发送带 `Current` 标志的输出 mode，否则 X11 会得到 0×0 屏幕。

X11 窗口会像 Wayland 窗口一样进入 master/stack 平铺，但同样只在离屏缓冲中渲染 —— 屏幕上依旧不会出现任何东西。

**剪贴板与拖放**：

- 剪贴板的读取权**跟随键盘焦点**：只有焦点客户端能读取或设置选区。X11↔Wayland 的粘贴因此要走正常的焦点交接 —— 先把焦点切到目标窗口（`Alt+Tab`；没有真实输入后端时按键只能经测试注入路径送达，见上一节），再粘贴；焦点不在对端时它不会收到任何选区事件。
- 拖放：Wayland↔Wayland 与 X11 内部（X 客户端之间）可用；**跨 X11/Wayland 边界的拖放不可用**，因为 `xwayland-satellite` v0.8.3 未实现 XDND。
- 锁屏时读剪贴板（验收项 X02 的一部分）与全局、无焦点限制的 `wlr-data-control` 都**不在 M3 范围内**，推迟到 M5。

## 配置

合成器**目前只读取命令行参数**，不读取任何 TOML 文件。`~/.config/inox/` 下的配置文件布局与字段仍处于提案状态，尚未在任何组件中实现 —— 详见[配置目录布局](../configuration/layout.md)与[合成器配置](../configuration/compositor.md)。

## 构建文档站点

本站点使用 mdBook：

```bash
# 构建到 book/
mdbook build

# 本地预览（会启动一个 HTTP 服务）
mdbook serve
```

本站点不引入 mdBook 插件或额外工具链；CI 通过 `mdbook build docs/book` 验证站点。
