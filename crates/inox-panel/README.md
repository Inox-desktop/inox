# inox-panel

Inox 桌面的面板：任务栏、系统托盘与时钟。属于 Inox 桌面环境的 **L3** 层（PRD §3.1；层级成员的唯一事实来源是根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

面板不拥有任何状态：窗口属于合成器（ADR 0013），托盘项属于各自的应用程序。面板订阅它们，并把自己看到的东西画出来。

任务区最多显示 `panel.max_task_buttons` 个按钮（默认 12）；超长标题在任务区内被截断，不会把时钟或托盘挤出屏幕（D05）。

## 状态

当前证据等级 **E0/E1**（ADR 0018 的口径）：编译、lint、文档与纯逻辑测试通过；`tests/render.rs` 用与二进制相同的 tiny-skia 渲染器离屏绘制真实控件树并断言像素。

已交付：

- `src/tasks.rs` —— 窗口与任务分组模型。窗口身份是**服务端分配的 `WindowId` + 会话代次**，绝不使用 PID、标题或列表下标（ADR 0013 决策 1）；分组按服务端给出的 `app_id`；激活态由「哪个窗口被报告为 focused」推导，至多一个；顺序稳定，无关窗口更新不会让按钮重排；应用退出后其窗口与分组一并消失。
- `src/tray.rs` —— StatusNotifierItem / DBusMenu 模型：注册（总线名 + 对象路径）、标题/图标/状态/关注/菜单更新、注销与退出、以及**按快照重连**（重建集合，不产生重复条目）。
- `src/clock.rs` —— 展开 `inox_config::CLOCK_DIRECTIVES` 的每个指令；`%a/%A/%b/%B` 与 `%p` 取自本 crate 的 Fluent 目录，而不是 chrono 的语言数据或硬编码英文。
- `src/contract.rs` —— ADR 0013 的服务契约：快照/代次、`Connecting / Ready / Unavailable`、请求的 `Sent / Accepted / Applied / Rejected / TimedOut`，以及「同一窗口同时至多一个请求」的去重。
- `src/placement.rs` —— layer-shell 摆放策略（锚点、exclusive zone、margin、高度、键盘交互、逐输出分配），**纯数据**。
- `src/view_model.rs`、`src/view.rs`、`src/theme.rs`、`src/app.rs` —— 投影、Iced 界面（Elm）、令牌到 iced 样式。
- `i18n/en-US/panel.ftl`、`i18n/zh-CN/panel.ftl` —— 全部用户可见文案；`tests/catalog.rs` 断言两者 id 与占位符一致。
- `src/main.rs` —— 日志、配置与目录装配，以及启动自检：没有可用的 Wayland/X11 显示时以可读错误与非零退出结束，而不是把控制权交给 `winit` 后收获一个内部 panic（ADR 0018）。

未交付，且不声称：

| 项 | 原因 |
|----|------|
| **layer 表面**：面板目前是普通 `xdg-shell` 窗口 | 合成器尚未广告 `zwlr_layer_shell_v1`，且 `iced_layershell` 0.19 在当前 iced 0.14 的 winit 世代下无法编译。启动时输出**恰好一条** `tracing::warn!` 说明这一点并点名 ADR 0018。`src/placement.rs` 的策略是给未来宿主消费的数据，**没有被应用**。 |
| **窗口服务与 SNI 传输** | `inox-ipc` / D-Bus 的协议腿不在本次范围内。面板启动后停在 `Starting`，把意图放进 `app::OutgoingCommand` 队列；`app::Message` 就是未来传输层要喂进来的接口。 |
| **E2/E3 GUI 证据** | 需要支持 layer-shell 的宿主，CI 目前没有。 |
| **`zwlr_layer_shell_v1` 存在性查询** | ADR 0018 要求的启动自检还包含「连接 Wayland 并查询该全局是否存在」。面板不与合成器建立连接（本 crate 没有 Wayland 客户端依赖，摆放也不被应用），因此只实现了「有没有可绘制的显示」这一层；协议层的自检随 layer-shell 宿主一并落地。 |
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
| `src/placement.rs` | layer-shell 摆放策略（数据，未应用） |
| `src/view_model.rs` | 状态 → 视图的投影 |
| `src/app.rs` | `State` / `Message` / `update`（Elm）/ `view` / `subscription` |
| `src/view.rs` | Iced 控件 |
| `src/theme.rs` | 设计令牌 → iced 主题与样式 |
| `src/i18n.rs` | 内嵌的 Fluent 目录 |
| `i18n/<locale>/panel.ftl` | 文案 |
| `tests/catalog.rs` | 目录一致性（Q03） |
| `tests/render.rs` | 离屏渲染与点击（Q02/D01） |

`crates/<name>/AGENTS.md`（`CLAUDE.md` 是其符号链接）是本 crate 的约束文档。

## 开发约定

- 本项目遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，不在本地进行编译验证。
- 颜色、尺寸、间距、圆角与动效时长只来自 `inox_config::Theme` 令牌；用户可见文案只来自 Fluent 目录。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 使用 `panel`。

| 目的 | 命令 |
|------|------|
| 测试 | `cargo test -p inox-panel --all-features` |
| 静态检查 | `cargo clippy -p inox-panel --all-targets --all-features -- -D warnings` |
| 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-panel --no-deps --all-features` |
| 截图（可选） | `INOX_SCREENSHOT_DIR=./shots cargo test -p inox-panel --test render` |

## 许可证

BSD 3-Clause，见仓库根的 [`LICENSE`](../../LICENSE)。
