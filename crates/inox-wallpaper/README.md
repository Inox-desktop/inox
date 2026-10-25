# inox-wallpaper

壁纸加载与显示。属于 Inox 桌面环境的 **L3** 层，是 `wlr-layer-shell-unstable-v1` 的客户端
（PRD §3.1、[ADR 0013](../../docs/adr/0013-shell-service-and-interaction-contracts.md)）。

界面是 **QML + Qt Quick**，经 cxx-qt 绑定到 Rust（[ADR 0033](../../docs/adr/0033-qml-qt-quick-shell-evaluation.md)、[ADR 0035](../../docs/adr/0035-cxx-qt-binding.md)）。领域模型、令牌与文案留在 Rust：状态机是纯的，界面拿到的是一份 JSON 投影，QML 只负责画。

## 表面：普通窗口（现在是，暂时也只能是）

`QGuiApplication` + `QQmlApplicationEngine` 打开一个 `Window`（`qml/Main.qml`），`--self-check` 时改用离屏平台做一次自检。**这不是 layer 表面**：`inox-compositor` 尚未广告 `wlr-layer-shell-unstable-v1`，因此 layer 表面根本创建不出来（[ADR 0018](../../docs/adr/0018-l3-component-architecture.md)），二进制启动时用一条 `warn!` 说明，窗口里放一条横幅。

`src/placement.rs` 的四锚点 / `exclusive_zone = 0` / 无键盘交互策略**只被计算与测试，不被应用**。本 crate 不声称壁纸已经作为背景层生效。

## 状态

**E0/E1 已交付**（编译、lint、文档、单元/属性/投影/QML 加载测试全绿，见下），**E2/E3 被阻塞**。

交付内容：

| 模块 | 职责 |
|------|------|
| `src/model.rs` | `[wallpaper]` → 每个输出的选择：具名输出 → `*` → 共享 `image` → 内置默认壁纸（取值语义见下，[ADR 0030](../../docs/adr/0030-bundled-wallpapers-and-the-default-background.md)） |
| `src/source.rs` | 选择 → 文件系统：存在、是文件、可打开；失败即降级并携带原因 |
| `src/decode.rs` | PNG/JPEG → RGBA8，带像素上限（`MAX_PIXELS`）与逐类错误；`FrameHandle` 另带一份 `data:` URL 供 QML 的 `Image` 使用 |
| `src/scaling.rs` | `fill` / `fit` / `stretch` / `center` / `tile` 的目标矩形，含平铺上限 |
| `src/apply.rs` | 原子应用：先解码、定位，再整体替换；逐输出独立成败 |
| `src/placement.rs` | 背景层 surface 纯数据策略（四锚点、`exclusive_zone = 0`、无键盘交互），**未被应用** |
| `src/contract.rs` | ADR 0013 的状态契约：代次、逐输出结果、服务状态、快照 |
| `src/app.rs` | Elm 状态与消息；`update` 不做 I/O，解码经 `Task::perform` 离开交互循环 |
| `src/view_model.rs` | 纯投影：文案（Fluent）、启用/禁用与可读原因、焦点顺序、语义树计数 |
| `src/view.rs` | 状态 + 令牌 → **一份 JSON 文档**：每行的文字、颜色（`#AARRGGBB`）、字号、字重、阶段名，以及壁纸矩形 |
| `src/theme.rs` | 令牌 → QML 颜色与尺寸的纯函数（`qml_color` / `units` / `px` / 字号四角色） |
| `src/runtime.rs` | 自己的 `Task<T>`、消息队列、线程本地 `install` / `take`、`pump` |
| `src/reload.rs` | `SIGHUP` 与会话总线（`org.inox.Wallpaper`）→ 准备好的 reload 请求 |
| `src/cxxqt_object.rs` | 唯一的 QML↔Rust 边界：`modelJson` 属性 + `dispatch` / `pump` / `reportReady` |
| `qml/Main.qml` | 声明式界面：一个窗口、壁纸 `Image`、状态卡片、通知卡片、底部说明条 |
| `src/headless_tests.rs` | 二进制内的投影 / 目录 / 命令行 / QML 离屏加载测试 |

**未交付（被上游阻塞）**：

- **真实背景层 surface**：合成器不广告 `wlr-layer-shell-unstable-v1`（ADR 0018），因此二进制以普通窗口运行——那是供人查看的**预览**。`src/placement.rs` 的策略不被应用。
- E2 级别的 GUI 证据（waydriver / 真实宿主截图）与 AT-SPI 实际桥接验证：需要支持 layer-shell 的宿主。

## QML↔Rust 边界

- Rust → QML：**一个**只读属性 `modelJson`，内容由 `src/view.rs` 生成，每次状态变化**整体重发**。QML 里 `JSON.parse(app.modelJson)`，所有绑定读这个对象。不为单个字段开 `#[qproperty]`（否则边界上就有第二份 UI 定义）。
- QML → Rust：`dispatch(action)` 收一行 JSON（目前只有 `{"type":"reload"}`；未知动作记 `warn!` 后丢弃），`pump()` 让宿主排空后台线程送来的消息，`reportReady(title, accessibleItems, imageReady)` 回报自检。
- QML 里没有颜色字面量、没有像素字面量、没有用户可见英文串：颜色是 `theme::qml_color` 的 `#AARRGGBB`，尺寸由 spacing / radii / typography 派生，文案是中英两套 Fluent 里已经拼好的句子。
- 每帧图像以 PNG `data:` URL 过桥（QML `Image` 不能直接吃内存位图），编码在解码线程上做一次；代价与已知缺口见 crate `AGENTS.md`。

## 配置

`shell.toml` 的 `[wallpaper]`（模型见 `inox-config`）：

```toml
[wallpaper]
mode = "fill"                           # fill | fit | stretch | center | tile
image = "nebula"                        # 内置壁纸名称、none（主题背景），或图片路径
per_output = { "HDMI-A-1" = "lake", "*" = "backgrounds/wall.png" }
```

`image`（以及 `per_output` 的每个值）的取值：

| 取值 | 含义 |
|------|------|
| 键**省略** | 内置默认壁纸（`inox_config::wallpapers::DEFAULT_ID`） |
| `"none"` | 主题背景：明确的「无图像」 |
| 内置壁纸的名称 | 该内置壁纸 |
| 其它字符串 | 图片路径；相对路径相对配置文件所在目录，绝对路径原样使用 |

内置壁纸的标识、文件名、作者与许可是 `inox_config::wallpapers::BUNDLED`（L1 的清单，不含路径）；
文件安装到 `$XDG_DATA_DIRS/inox/wallpapers/`，`INOX_WALLPAPER_DIR`（绝对路径）可以覆盖该目录。
`inox-settings` 直接提供挑选这些壁纸的选择控件，署名记录在仓库根的 `data/wallpapers/CREDITS.md`。

配置热重载已接入：`kill -HUP` 与 `org.inox.Wallpaper`（`inoxctl reload`）都在**信号 / 总线线程**上读配置并解码，成功后才把请求投给窗口线程应用；应用失败保留旧帧。

## 失败即降级，且可诊断

ADR 0014 划的线在这里被逐条实现：**用户显式选择却读不到的资源必须报告**，而**默认值与「没有选择」不是错误**。

- 文件缺失、是目录、读不了、解码失败、像素超限、几何退化、平铺份数超限 → 输出退到主题背景，
  屏幕**永不**变成黑屏，进程**永不**崩溃；原因以「输出名 + 路径 + 原因」呈现，文案来自 Fluent 目录。
- **从未选择**的默认壁纸缺失（例如打包时没有安装 `data/wallpapers/`）只记一条 `warn!` 并退到主题背景，
  不进入错误态；**显式**选择的内置壁纸不可用才按上面的路径报告（ADR 0030）。
- 已经显示中的壁纸不会因为一次重载失败而被清空：新帧**完整**之后才替换旧帧，失败时保留旧帧。
- 一个输出失败不影响其它输出，结果逐输出分别报告（ADR 0013 的快照/应用契约）。

对应的验收场景：**D05**（loading/empty/error/disabled/retry，禁用原因可读）、**Q03**（`en-US` 与 `zh-CN` 目录 id 与占位符一致）；**Q02**（浅色/深色/空态/错误态、1280×720）在迁移后由「投影断言 + QML 离屏加载」覆盖，像素截图证据属于尚未完成的 waydriver 工作。

## 运行与测试

> 仅作参考，不作为本地验收依据；真正判定在 CI。

```sh
cargo run -p inox-wallpaper                  # 打开预览窗口（需要一个 Wayland 或 X11 会话）
QT_QPA_PLATFORM=offscreen cargo test -p inox-wallpaper --all-features
cargo clippy -p inox-wallpaper --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc -p inox-wallpaper --no-deps --all-features
QT_QPA_PLATFORM=offscreen cargo run -p inox-wallpaper -- --self-check
```

`QT_QPA_PLATFORM=offscreen` 必须加在命令行外层，不能由测试进程设置（edition 2024 下 `std::env::set_var` 是 `unsafe`）；需要离屏的测试会 spawn 自己作为子进程并把环境交给它。

测试全部在二进制内部（本 crate 是 bin-only，原因见 `AGENTS.md`）：

- 每个模块的单元测试，含拒绝与错误路径；`scaling` / `decode` 的上限另有 `proptest`。
- 投影测试：颜色等于令牌转换结果、字号/间距等于令牌推导值、文案来自 Fluent（中英各一条）。
- **QML 加载测试**：`QT_QPA_PLATFORM=offscreen` 下加载本 crate 的 QML 模块，断言根对象非空、QML 回报的标题与语义项数与投影一致、没有 QML 运行时错误。
- 命令行测试：`--help` 与未知参数的退出码与消息。

## 开发约定

- 本组件遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的
  [`AGENTS.md`](./AGENTS.md)。
- 用户可见文案一律走 Project Fluent；颜色/尺寸/间距/圆角一律来自 `inox_config::Theme` 令牌。
- 新增依赖前先确认依赖方向合法（根级 §2）。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 用 `wallpaper`。

## 许可证

LGPL-3.0，见仓库根的 [`LICENSE`](../../LICENSE)。
