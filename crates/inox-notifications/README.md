# inox-notifications

Inox 桌面的通知守护进程：`org.freedesktop.Notifications` 背后的进程。属于 Inox 桌面环境的 **L3** 层（PRD §3.1；层级成员的唯一事实来源是根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

它**拥有**通知栈本身：标识符、寿命、队列上限、勿扰状态，以及锁屏时能显示什么。它不拥有窗口、不拥有会话的锁定状态（合成器拥有，ADR 0015），也不打开任何总线。

## 状态

当前证据等级 **E0/E1**（ADR 0018 的口径）：编译、lint、文档与纯逻辑测试通过；`tests/render.rs` 用与二进制相同的 tiny-skia 渲染器离屏绘制真实控件树并断言像素。**D-Bus 服务本身按 PRD §13 属于 M5（验收 D06）**，本里程碑交付的是协议契约、队列、策略与界面模型。

已交付：

- `src/protocol.rs` —— freedesktop 通知契约**作为数据**：总线名、对象路径、接口、四个方法各自的完整参数表与签名（`Notify` = `susssasa{sv}iu`）、两个信号、Inox 声明的 capability 列表、关闭原因（1 expired / 2 dismissed / 3 CloseNotification / 4 undefined）、紧急程度（0/1/2）、标准 hint 键（含已废弃的 `image_path` 拼写）、`expire_timeout` 的三种含义。每个方法的签名由参数表拼出来自检，拼错就失败。
- `src/markup.rs` —— 正文清理器：只识别 `<b> <i> <u> <a href> <img src alt>`，其余标签与裸 `<`、`&`、引号**原样保留为文本**（输出是结构化 span 列表，不是 HTML 字符串，所以没有第二个解释器可以出错）；`<a href>` 只接受 http/https/mailto，`<img src>` 只接受 http(s) URL 或路径，`javascript:` / `data:` / `file:` 一律按文本处理。
- `src/actions.rs` —— 动作身份与**至多一次**：动作键唯一、数量有上限、`default` 动作只作为「点击通知本身」的语义；一次动作被触发后再次触发会被**拒绝并报告**，而不是静默重复。
- `src/queue.rs` —— 守护进程拥有的模型：单调分配、替换（`replaces_id`）在原位更新并重开计时、关闭原因、寿命（`TimeoutConfig` 的 `0` = 永不过期，**关键通知永不自过期**）、有界队列（`QueueConfig::max_pending` + `DropOldest` / `CoalesceByApp`）与**可观察计数器**（admitted/created/replaced/dropped/coalesced/expired/dismissed/closed_by_call）。
- `src/policy.rs` —— 勿扰（只抑制呈现，不抑制投递；守护进程持有运行期状态，`notifications.toml` 只是启动初值）与**锁屏脱敏**：`PopupContent::Withheld` **没有**正文、应用名与图标字段，因此不存在「忘记检查」的绘制路径（ADR 0015）。
- `src/contract.rs` —— ADR 0013 的调用契约：`Request`（四个方法）/ `Reply` / `Signal` / `Outgoing`，以及按调用标识去重的有界记忆（重复调用按原答案回答，不产生第二条通知）。
- `src/placement.rs` —— layer-shell 摆放策略（overlay 层、角落锚点、margin、尺寸、**键盘交互永远为 None**、逐输出分配），**纯数据**。
- `src/view_model.rs`、`src/view.rs`、`src/theme.rs`、`src/app.rs` —— 投影（含 D05 的空态、抑制态与「还有 N 条在等待」溢出态，禁用动作带可读原因）、Iced 界面（Elm）、令牌到 iced 样式。
- `i18n/en-US/notifications.ftl`、`i18n/zh-CN/notifications.ftl` —— 守护进程自己的全部文案；`tests/catalog.rs` 断言两个目录 id 与占位符一致，且与 `i18n::MESSAGES` 双向一致。
- `src/main.rs` —— 日志、配置与目录装配，以及启动自检：没有可用的 Wayland/X11 显示时以可读错误与非零退出结束，而不是把控制权交给 `winit` 后收获一个内部 panic（ADR 0018）。

未交付，且不声称：

| 项 | 原因 |
|----|------|
| **D-Bus 服务** | 按 PRD §13 属 M5（D06）。`src/contract.rs` 的 `Request` 就是未来总线要投递的调用，`Outgoing` 是它要取走的回复与信号；`main.rs` 启动时明确说出这一点。没有任何总线被打开。 |
| **layer 表面**：弹窗栈目前是普通 `xdg-shell` 窗口 | 合成器尚未广告 `zwlr_layer_shell_v1`，且 `iced_layershell` 0.19 在当前 iced 0.14 的 winit 世代下无法编译。启动时输出**恰好一条** `tracing::warn!` 说明这一点并点名 ADR 0018；`src/placement.rs` 的策略**没有被应用**。 |
| **图标解码**：`icon-static` 与 `body-images` capability 故意不声明 | 解析主题图标或正文图片需要在交互循环之外做文件查找与解码（`Task::perform`）。带图标的弹窗画出图标将占用的**空位**（按 `display.icon_size`），正文 `<img>` 画 alt 文本。 |
| **声音**：不声明 `sound` capability | `sound-name` / `suppress-sound` 会被记录，但 Inox 目前不播放任何声音。 |
| **持久化**：不声明 `persistence` capability | 该 capability 承诺的是 `resident` hint 的行为——动作被触发后保留通知；本守护进程在动作触发时即关闭通知。`resident` 仍会被读取并记录（`protocol::Hints`）。 |
| `<u>` 的下划线 | 清理器保留 `underline` 强调，但 iced 0.14 的 text span 没有下划线装饰，因此它不影响渲染（模型里有，画面上没有）。 |
| `<b>`/`<i>` 在正文里生效，在 **summary** 里被展平为纯文本 | 标题只画一行文字，标记在那里没有视觉意义；展平还保证摘要里的 `javascript:` 链接不会变成可点的东西。 |
| **配置热重载** | `app::Message::ConfigReloaded` 已实现并有测试，但没有东西在监视 `notifications.toml`。 |
| **E2/E3 GUI 证据** | 需要支持 layer-shell 的宿主与真实会话总线，CI 目前都没有。 |
| AT-SPI 语义导出 | Q01 需要的桥接尚未原型验证（ADR 0013/0018 记为阻塞项）。 |

## 结构

| 文件 | 内容 |
|------|------|
| `src/protocol.rs` | 通知契约（常量、方法表、能力、关闭原因、hint） |
| `src/markup.rs` | 正文清理器（纯逻辑，属性测试） |
| `src/actions.rs` | 动作身份与至多一次（纯逻辑） |
| `src/queue.rs` | 标识符、寿命、上限与计数器（纯逻辑，属性测试） |
| `src/policy.rs` | 勿扰与锁屏脱敏（纯逻辑） |
| `src/contract.rs` | 调用、回复、信号与去重（ADR 0013） |
| `src/placement.rs` | layer-shell 摆放策略（数据，未应用） |
| `src/view_model.rs` | 状态 → 视图的投影 |
| `src/app.rs` | `State` / `Message` / `update`（Elm）/ `view` / `subscription` |
| `src/view.rs` | Iced 控件 |
| `src/theme.rs` | 设计令牌 → iced 主题与样式 |
| `src/i18n.rs` | 内嵌的 Fluent 目录 |
| `i18n/<locale>/notifications.ftl` | 文案 |
| `tests/catalog.rs` | 目录一致性（Q03） |
| `tests/render.rs` | 离屏渲染、角落摆放与点击（Q02/D05/D06） |

`crates/<name>/AGENTS.md`（`CLAUDE.md` 是其符号链接）是本 crate 的约束文档。

## 开发约定

- 本项目遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，不在本地进行编译验证。
- 颜色、尺寸、间距、圆角与动效时长只来自 `inox_config::Theme` 令牌；守护进程自己的文案只来自 Fluent 目录（应用名、摘要、正文与动作标签是随通知到达的**数据**，不翻译）。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 使用 `notifications`。

| 目的 | 命令 |
|------|------|
| 测试 | `cargo test -p inox-notifications --all-features` |
| 静态检查 | `cargo clippy -p inox-notifications --all-targets --all-features -- -D warnings` |
| 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-notifications --no-deps --all-features` |
| 截图（可选） | `INOX_SCREENSHOT_DIR=./shots cargo test -p inox-notifications --test render` |

## 许可证

BSD 3-Clause，见仓库根的 [`LICENSE`](../../LICENSE)。
