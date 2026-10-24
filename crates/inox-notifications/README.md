# inox-notifications

Inox 桌面的通知守护进程：`org.freedesktop.Notifications` 背后的进程。属于 Inox 桌面环境的 **L3** 层（PRD §3.1；层级成员的唯一事实来源是根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

它**拥有**通知栈本身：标识符、寿命、队列上限、勿扰状态，以及锁屏时能显示什么。它不拥有窗口、不拥有会话的锁定状态（合成器拥有，ADR 0015）；锁状态是合成器经 `inox-ipc` 告诉它的。

## 状态

当前证据等级 **E1 + 一条 E2 腿**（ADR 0022 中 D06 那一行的口径）：编译、lint、文档与纯逻辑测试通过；`tests/render.rs` 用与二进制相同的 tiny-skia 渲染器离屏绘制真实控件树并断言像素；**`tests/bus.rs` 用真实 D-Bus 连接驱动真实状态机**（点对点连接，因为 CI 没有会话总线），覆盖 D06 要求的 Notify / 替换 ID / 关闭与关闭原因 / 超时 / 动作至多一次 / 勿扰 / 每秒 100 条持续 10 s 的有界性 / 锁屏脱敏，以及本守护进程自己的三种拒绝错误。

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
- `src/bus.rs` —— `org.freedesktop.Notifications` 服务本体（ADR 0022 决策 5）：在**自己的线程**上持有 `zbus::Connection`（zbus 6 的异步操作由 `zbus::block_on` 驱动；[ADR 0027](../../docs/adr/0027-zbus-6-migration.md)），把方法调用翻译成 `contract::Request` 交给 iced 应用，再把 `Outgoing` 变成回复与 `NotificationClosed` / `ActionInvoked`；信号由第二条线程发出，否则一次到期没有客户端调用就带不出总线。**一次只服务一个调用**并带**上限**等待答复（超时返回 D-Bus 错误，迟到的答复被去重记忆吸收）；调用身份取自消息 serial；hint 表有上限、无法用 `HintValue` 表示的取值会被**计数后丢弃**；另外服务 `Introspectable` / `Properties.GetAll` / `Peer.Ping`，因为真实客户端的代理会先问这些。
- `src/lock_state.rs` —— 锁状态的**观察者**（ADR 0022 决策 4）：在自己的线程上用自己的 `wayland-client` 连接绑定 `inox_ipc`，把 `session_lock_state` 事件作为 `Message::SessionLockChanged` 交给应用。它不是 capability：没有合成器、没有 v2 全局或连接断开时，它**如实报告 Disabled 并说明原因**，不猜「未锁定」，也不让守护进程退出。
- `src/main.rs` —— 日志、配置、目录、**传输的启动与自检**：先声明总线名（已被别的守护进程占用时以可读错误与非零码退出，ADR 0022 决策 5），再接上锁状态，最后交给 iced；`Runtime::update` 是唯一把 `take_outgoing` 交给传输的地方。没有可用的 Wayland/X11 显示时同样以可读错误与非零退出结束（ADR 0018）。

未交付，且不声称：

| 项 | 原因 |
|----|------|
| **会话总线上的名字所有权** | 服务会声明 `org.freedesktop.Notifications`，别的守护进程已持有时拒绝启动；这条拒绝的映射有单元测试，而「两条守护进程争一个名字」的真实场景需要一个会话总线，crate 的 CI 分片没有。`tests/bus.rs` 因此走点对点连接：线上协议、回复与信号都是真的，但 p2p 不携带 well-known name。 |
| **能真正被问锁状态的合成器** | `src/lock_state.rs` 已接线且它的判断有测试，但本 crate 没有任何测试绑定真实的 `inox_ipc` 全局：工作区里没有可以充当假合成器的服务端，另一端由 `inox-compositor` 的测试负责。 |
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
| `src/bus.rs` | `org.freedesktop.Notifications` 的 D-Bus 传输（两条线程） |
| `src/lock_state.rs` | `inox-ipc` 锁状态观察者（自己的连接与线程） |
| `src/placement.rs` | layer-shell 摆放策略（数据，未应用） |
| `src/view_model.rs` | 状态 → 视图的投影 |
| `src/app.rs` | `State` / `Message` / `update`（Elm）/ `view` / `subscription` |
| `src/view.rs` | Iced 控件 |
| `src/theme.rs` | 设计令牌 → iced 主题与样式 |
| `src/i18n.rs` | 内嵌的 Fluent 目录 |
| `i18n/<locale>/notifications.ftl` | 文案 |
| `tests/catalog.rs` | 目录一致性（Q03） |
| `tests/render.rs` | 离屏渲染、角落摆放与点击（Q02/D05/D06） |
| `tests/bus.rs` | 真实 D-Bus 连接 + 真实状态机（D06） |

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

LGPL-3.0，见仓库根的 [`LICENSE`](../../LICENSE)。
