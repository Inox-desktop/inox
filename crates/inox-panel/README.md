# inox-panel

Inox 桌面的面板：任务栏、系统托盘与时钟。属于 Inox 桌面环境的 **L3** 层（PRD §3.1；层级成员的唯一事实来源是根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

面板不拥有任何状态：窗口属于合成器（ADR 0013），托盘项属于各自的应用程序。面板订阅它们，并把自己看到的东西画出来。

任务区最多显示 `panel.max_task_buttons` 个按钮（默认 12）；超长标题在任务区内被截断，不会把时钟或托盘挤出屏幕（D05）。

## 状态

当前证据等级 **E0/E1**（ADR 0018 的口径）：编译、lint、文档与纯逻辑测试通过；`tests/render.rs` 用与二进制相同的 tiny-skia 渲染器离屏绘制真实控件树并断言像素。

**摆放、帧与玻璃请求是 E2**：`tests/protocol.rs` 用真实 `wayland-server` 在 socket pair 上扮演合成器并驱动编译出的二进制，断言它真的创建了 `wlr-layer-shell` 表面、真的按 `placement.rs` 的锚点/exclusive zone/layer/margin/尺寸提交了预乘 `Argb8888` 共享内存帧、真的在 layer role 之后用钳制过的半径请求了磨砂，以及合成器不支持 layer-shell 时以可读错误非零退出。

已交付：

- `src/tasks.rs` —— 窗口与任务分组模型。窗口身份是**服务端分配的 `WindowId` + 会话代次**，绝不使用 PID、标题或列表下标（ADR 0013 决策 1）；分组按服务端给出的 `app_id`；激活态由「哪个窗口被报告为 focused」推导，至多一个；顺序稳定，无关窗口更新不会让按钮重排；应用退出后其窗口与分组一并消失。
- `src/tray.rs` —— StatusNotifierItem / DBusMenu 模型：注册（总线名 + 对象路径）、标题/图标/状态/关注/菜单更新、注销与退出、以及**按快照重连**（重建集合，不产生重复条目）。
- `src/clock.rs` —— 展开 `inox_config::CLOCK_DIRECTIVES` 的每个指令；`%a/%A/%b/%B` 与 `%p` 取自本 crate 的 Fluent 目录，而不是 chrono 的语言数据或硬编码英文。
- `src/contract.rs` —— ADR 0013 的服务契约：快照/代次、`Connecting / Ready / Unavailable`、请求的 `Sent / Accepted / Applied / Rejected / TimedOut`，以及「同一窗口同时至多一个请求」的去重。
- `src/placement.rs` —— layer-shell 摆放策略（锚点、exclusive zone、margin、高度、键盘交互、逐输出分配），**纯数据**；`src/layer.rs` 是唯一把它应用到协议上的地方。
- `src/layer.rs` —— **协议腿**：`Connection::connect_to_env()` + `calloop` 事件循环，绑定 `wl_compositor` / `wl_shm` / `wl_seat` / `zwlr_layer_shell_v1`（必需，缺失即非零退出）与 `inox_glass_manager_v1`（可选，缺失只降级）；一帧 = `app::view` 的真实控件树在 configure 尺寸下的 `Argb8888` 渲染（预乘 `B,G,R,A`，透明背景截图）；`wl_pointer` 的 enter/leave/motion/button 经 `layer::Interface` 走与渲染测试同一套 `UserInterface::update`；`SIGINT`/`SIGTERM` 干净拆卸，`SIGHUP`/D-Bus 重载由 `src/reload.rs` 通过 ping 喂进来。
- `src/frost.rs` —— 磨砂玻璃的**纯决策**：`glass.blur == 0` 不创建对象；`capabilities` 的 `max_radius == 0` 时画纯半透明并只记一条 `info!`；否则 `radius = min(blur, max_radius)`、region 是窗格自身区域，尺寸变化时重发。
- `src/reload.rs` —— 会话 D-Bus（`org.inox.Panel`）与 `SIGHUP` 的重载主机（库模块）；候选文档在另一个线程读取解析，经通道 + `calloop::ping` 交给 `layer.rs` 应用。`[panel] surface` 不采用候选值（那是另一个进程；`--surface` 已覆盖文件），保留运行中的表面并记一条 `warn!`，其余摆放字段热改并在下一个 commit 生效。
- `src/view_model.rs`、`src/view.rs`、`src/theme.rs`、`src/app.rs` —— 投影、Iced 界面（Elm）、令牌到 iced 样式。
- `i18n/en-US/panel.ftl`、`i18n/zh-CN/panel.ftl` —— 全部用户可见文案；`tests/catalog.rs` 断言两者 id 与占位符一致。
- `src/main.rs` —— 日志、配置与目录装配、`--surface <taskbar|menubar|dock>` 与 `--help`（经 `tracing` 输出），以及启动自检：没有可用的 Wayland 显示时以可读错误与非零退出结束（`DisplayEnv`，ADR 0018），然后把控制权交给 `layer::run`。

未交付，且不声称：

| 项 | 原因 |
|----|------|
| **窗口服务与 SNI 传输** | `inox-ipc` / D-Bus 的协议腿不在本次范围内。面板启动后停在 `Starting`，把意图放进 `app::OutgoingCommand` 队列；`app::Message` 就是未来传输层要喂进来的接口，`layer.rs` 把队列取出并丢弃（一次 `info!` 说明没有传输可送）。 |
| **每个输出一个面板** | 协议腿只创建一个表面，并把输出交给合成器选择（`wlr-layer-shell` 的 null output）。一个输出一个面板需要枚举 `wl_output` 并按 `placement::assign` 建表面；当前由会话按输出各起一个进程代替。 |
| **E3 GUI 证据** | 需要真实硬件与支持 layer-shell 的合成器；CI 有 socket pair，因此 E2 可跑，E3 不能。 |
| `%Z` 的时区缩写 | `clock` feature 下拿不到 `tzname`，因此 `%Z` 渲染为 `UTC` / `UTC+08:00` 这类 UTC 偏移（对没有缩写的时区也正确）。 |
| 时钟按格式降频 | 目前固定每秒一次唤醒（`%S` 必须准）。等 P03 的空闲预算说明这次唤醒有代价时再做。 |
| 任务区溢出时的点击 | 任务按钮的内容宽度之和超过任务区时，多出来的部分被裁剪（视觉上不会盖住时钟与托盘），但 iced 的命中测试仍按按钮自身矩形判定：极端情况下（把 `panel.max_task_buttons` 调到上限、且十几个应用都用超长标题）被裁掉的按钮仍可能接收点击。任务区滚动/溢出指示尚未实现。 |
| 图标渲染、托盘菜单弹出 | `tray` 已保留 `icon` 与菜单树，但面板目前只画文字；托盘菜单是后续的弹出层工作。 |
| AT-SPI 语义导出 | Q01 需要的桥接尚未原型验证（ADR 0013/0018 记为阻塞项）。 |

## 结构

| 文件 | 内容 |
|------|------|
| `src/tasks.rs` | 窗口 / 分组模型（纯逻辑，属性测试） |
| `src/tray.rs` | StatusNotifierItem 与 DBusMenu 模型（纯逻辑） |
| `src/clock.rs` | 时钟格式展开 |
| `src/contract.rs` | 服务状态、请求关联与去重 |
| `src/placement.rs` | layer-shell 摆放策略（数据，由 `layer.rs` 应用） |
| `src/frost.rs` | 磨砂玻璃的客户端决策（纯，含降级表） |
| `src/layer.rs` | 协议腿：layer 表面、ARGB 共享内存帧、指针输入 |
| `src/reload.rs` | `SIGHUP` 与 `org.inox.Panel` 的重载请求 |
| `src/view_model.rs` | 状态 → 视图的投影 |
| `src/app.rs` | `State` / `Message` / `update`（Elm）/ `view` / `subscription` |
| `src/view.rs` | Iced 控件 |
| `src/main.rs` | 日志、自检、`--surface`/`--help`，然后 `layer::run` |
| `src/theme.rs` | 设计令牌 → iced 主题与样式 |
| `src/i18n.rs` | 内嵌的 Fluent 目录 |
| `i18n/<locale>/panel.ftl` | 文案 |
| `tests/catalog.rs` | 目录一致性（Q03） |
| `tests/render.rs` | 离屏渲染与点击（Q02/D01） |
| `tests/protocol.rs` | 真实 `wayland-server` 扮演合成器的协议腿（E2） |

`crates/<name>/AGENTS.md`（`CLAUDE.md` 是其符号链接）是本 crate 的约束文档。

## 开发约定

- 本项目遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，不在本地进行编译验证。
- 颜色、尺寸、间距、圆角与动效时长只来自 `inox_config::Theme` 令牌；用户可见文案只来自 Fluent 目录。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 使用 `panel`。

| 目的 | 命令 |
|------|------|
| 测试 | `cargo test -p inox-panel --all-targets --all-features` |
| 协议腿 | `cargo test -p inox-panel --test protocol` |
| 静态检查 | `cargo clippy -p inox-panel --all-targets --all-features -- -D warnings` |
| 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-panel --no-deps --all-features` |
| 截图（可选） | `INOX_SCREENSHOT_DIR=./shots cargo test -p inox-panel --test render` |

## 许可证

LGPL-3.0，见仓库根的 [`LICENSE`](../../LICENSE)。
