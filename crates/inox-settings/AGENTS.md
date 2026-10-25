# AGENTS.md — inox-settings

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-Desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

系统设置应用：Inox 中**唯一写入配置的组件**。所属层级：**L3**。

它编辑 `config.toml`、`compositor.toml`、`shell.toml` 与 `notifications.toml` 中用户可改的字段，按 ADR 0014 区分「已保存」与「已生效」，按 ADR 0013 用快照/代次/命令结果建模与各组件的契约。它不持有 schema、默认值或校验规则的权威——那些都在 L1 的 `inox-config`。

界面是 **QML + Qt Quick**（ADR 0033、0035，经 cxx-qt 绑定）。领域模型、草稿、持久化、焦点顺序、校验与全部 Fluent 文案留在 Rust；QML 只声明式渲染 Rust 投影出来的 JSON。本 crate 是**普通窗口**（`QGuiApplication` + `QQmlApplicationEngine`，QML 根是 `Window`），不使用 Qt Wayland Compositor（GPL-only），没有 webview，也不做离屏渲染（那是 `inox-panel` / `inox-lock` 的形态，ADR 0038）。

## crate 特有约束

### Rust 侧

- `unsafe`：crate 根 `Cargo.toml` 的 `[lints.rust] unsafe_code = "allow"` 是 ADR 0035 的**唯一例外**，只为 `#[cxx_qt::bridge]` 生成的 FFI 胶水服务；Inox 手写的 Rust 仍然一行 `unsafe` 都不写。`missing_docs = "warn"` 同在该处。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- crate 根有 `#![allow(dead_code)]`：本 crate 是 bin-only（见下），测试在同一二进制内，`#[cfg(test)]` 里的读取不算非测试编译的「使用」，状态机访问器与消息变体会被报成死代码。`inox-launcher`、`inox-lock`、`inox-notifications` 迁移后同样如此。新增真正无用的代码仍然要删，不要靠这条豁免。
- **`app::update` 不做 I/O。** 一切副作用都作为 `app::OutgoingCommand` 离开更新循环（写文件、请求组件应用、重读配置），由 `runtime::UiRuntime` 执行并把结果作为 `Message` 送回来。这条是状态机可测的原因，也是它与二进制不会各有一份实现的原因；新增副作用时必须走同一条路。
- **唯一动文件的地方是 `persist::save`。** 它必须保持 ADR 0014 要求的失败原子性（目标目录内临时文件、整份写入、`sync_all`、原子 `rename`、目录同步），并且必须保留「外部编辑检测」：写入携带读取时的 `Guard`，摘要不符时返回 `Conflicted` 而不是覆盖。任何新增写入路径都必须同样经过它。
- **只写用户改过的键。** `document::patch` 只重写被编辑键所在的行，其余行逐字节保留；「取消」不排队任何命令。改动 `draft`/`document` 时不得引入整文件重写，也不得缩小 `patch` 认识键的范围（见下方对外契约）。
- **界面不得声称未确认的成功。** 只有 `apply::ApplyState::Applied`（每个消费方的回读都带当前代次）才能显示「已生效」；无法请求的消费方必须是 `SavedOnly`。
- **显示改动是唯一会追问的改动（ADR 0028）**：只有携带 `Field::Outputs` 的请求以 `Configuration2.Trial` 发出（`live::apply` 的 `trial` 参数），且只有合成器回执之后才开始倒计时（`app::State::arm_display_trial`，由 `Message::DisplayTrialArmed` 触发，不是写盘触发）。四条不可放宽的规则：倒计时是 `DISPLAY_TRIAL_SECONDS`（30）秒、一秒一条 `Message::TrialTick`，归零即请求回退；回答只有「确认」与「恢复原设置」两种，试用期间页面控件与其它动作一律禁用并写明 `settings-display-trial-active`，`Esc` 不回答试用；**回退必须恢复文件**——`outputs` 被改回改动前的值，经 `document::patch` + `validate::document` 只重写这一个键，并**作为独立的 `OutgoingCommand::Restore`** 写入（不是一次新的 `Apply`），因为它回答的是试用而不是发出新请求，且不能因为别的消费方还在应答就被阻塞；合成器不可达时仍要恢复文件并如实报告（`settings-display-trial-notice-unreachable` / `-failed`）。`pending_trial` 是「本次请求是试用」的意图，重试（`Application::retry_apply`）沿用同一意图。headless 或合成器不可达时不弹框、只显示 `SavedOnly`。
- **界面不得提供做不到的动作。** 禁用控件必须写明原因（`disabled_reason`），并且不是键盘可达项；写盘在途时 `Cancel` 属于此类。
- **用户可见文案全部走 Fluent**（en-US 与 zh-CN 同步，`src/headless_tests/catalog.rs` 用 `inox_common::i18n::compare_catalogs` 与 `headless_tests::code_message_ids()` 断言齐备）；颜色、尺寸、圆角只能来自 `inox_config::Theme` 令牌。
- 与工作区内其他 Inox crate 的依赖使用 **path 依赖**（根级 §8）：在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明一次，本 crate 用 `{ workspace = true }` 引用。**新增依赖前先确认依赖方向合法**（根级 §2），否则 `cargo test -p inox-integration --test layering` 会失败。
- **对外契约（不可轻改）**：`Field` 的 `path()` 必须始终是 `inox-config` 解析错误里用的点号路径；`validate` 的规则必须镜像 `inox-config` 而不是自行发明（`src/headless_tests/workflow.rs` 用真实解析器逐条核对边界）；`apply` 的陈旧回执必须被丢弃。
- **壁纸选项来自 L1 清单，不是本 crate 的发明**：`Field::WallpaperImage` 的选择表（`WALLPAPER_CHOICES`）必须与 `inox_config::wallpapers::BUNDLED` 及 `BACKGROUND_ID` 一致，由单元测试强制；「默认」选项在写盘时**删除 `wallpaper.image` 键**（`toml_value` 返回 `None`），而不是写空字符串；配置里手写的自定义路径不在选项表内，投影时按原值追加并逐字写回（ADR 0030）。
- **`document::patch` 必须认出 TOML 允许的每一种键写法**：裸键、引号键、点号键、内联表成员、引号表头（含 `[[表]]` 形式的表头，按表读以免再造一个同名表）。漏认一种，同一个键就会被写第二遍——那份配置 `inox-config` 读不回来，而窗口还会说它保存成功。删掉任何一种识别路径都必须同时删掉对应的 `document.rs` 单元测试与 `src/headless_tests/workflow.rs` 的回读用例。
- **键盘只能落在窗口画为可用的控件上**：`view_model::focus_order` 只收 `enabled` 的控件（禁用控件不画焦点，落上去等于键盘落在空处），`app::State::activate` 对已离开该序列的目标不执行任何事。写盘在途时不存在能撤销它的动作：`Cancel` 不可用（写明原因），`app::State::cancel` 也拒绝在途请求——取消不能阻止一次已经在写的保存。
- 本 crate 是**普通 `xdg-shell` 窗口**，不是 layer-shell 客户端，因此不需要放置策略。配置应用传输由二进制 `live.rs` 通过会话 D-Bus 执行，只有当前代次与语义快照都匹配才确认；窗口服务与 layer-shell 的限制仍按 ADR 0018 报告。

### QML / cxx-qt 边界（契约 §3、§4）

- **唯一边界是 `src/cxxqt_object.rs`。** 它是一个 `#[cxx_qt::bridge]`，向 QML 只暴露 `modelJson`（只读投影）、`ready`、`pump()`、`dispatch(json)`、`reportReady(title, pages, accessibleItems)`、`quit()`。**不要为每个字段加 `#[qproperty]`**：那会把边界变成第二份 UI 定义。
- **一份 JSON 模型，整份重发。** 状态一变就重新投影并整体序列化；QML 里 `property var model: JSON.parse(app.modelJson)`。增量更新是迁移明确推迟的取舍（契约 §3），做之前要有 P03 的数据。
- **QML 不做决定。** 页面、字段、卡片、控件形态、值、哪一项被禁用及原因、焦点在哪、状态行句子、试用弹窗，全部已由 Rust 投影。QML 里新增任何 `if` 式业务判断都是缺陷——把它挪回 `view.rs`。
- **QML 不得出现颜色字面量、像素字面量、用户可见英文字符串。** 颜色是 `theme.rs` 产出的 `#AARRGGBB`，尺寸来自 `theme.geometry`，文案是 Fluent 渲染后的句子。QML 里只允许协议字符串（动作 `type`、`kind`、`width` 这类枚举名）。
- **QML → Rust 只有一个动作通道**：`JSON.stringify({type, ...})`，由 `view::parse_action` 解析成 `view::UiAction` 闭合集合，再由 `runtime` 转成 `app::Message`。解析失败只 `warn!`，绝不 panic。新增交互必须同时加 `UiAction`、`parse_action`、`runtime::message`、以及 `view` 的往返测试。
- **定时器只有 pump 一个**：QML `Timer { interval: 16; repeat: true; onTriggered: app.pump() }` 排空后台线程送来的消息并推进试用倒计时。不要在 QML 里做业务计时。
- **键盘**：QML 只上报 Tab / Enter / Esc / 上下方向键与修饰键，其余按键留给有文本光标的控件。`Keys` 必须挂在 `Item` 上（窗口根 `Window` 不是 `Item`，挂在窗口上会被静默丢弃）：`qml/Main.qml` 用一层铺满窗口的 `Item { focus: true; Keys.onPressed: ... }` 承接。
- **就绪必须自证**：`SettingsSurface` 在 `Component.onCompleted` 里回传标题、导航项数与可访问项数，`cxxqt_object::report_ready` 只在三者与投影一致时置 `QML_READY`。`qml_ready()` 是离屏测试唯一接受的证据（根级 §7.4），不能降级成「URL 被交给 load」。

### bin-only 与「第二个二进制」

- cxx-qt 0.10 把 bridge 的 `cxx_qt_init_*` 定义嵌进**声明 bridge 的那个 target**；静态库里没有引用时会被 `--gc-sections` 丢掉，链接报 `undefined reference`（KDAB/cxx-qt #770，`docs/qml-migration-contract.md` §2.1）。因此：**没有 `src/lib.rs`**，所有模块挂在 `src/main.rs`；**没有 `tests/` 集成测试**，测试是同一二进制里的 `#[cfg(test)] mod headless_tests`。
- 同一个 package 里还有第二个 bin `inoxctl`。它必须**编译 bridge 文件本身**来提供那些符号：`src/bin/inoxctl.rs` 用 `#[path = "../cxxqt_object.rs"] mod cxxqt_object;`（以及 `ui_host.rs`、`i18n.rs`）引入，且从不安装宿主、不创建 `QGuiApplication`。删掉那段 `#[path]` 或把 bridge 挪进库，`cargo build -p inox-settings --bins` 就会链接失败。这条以 `cargo build -p inox-settings --bins` 通过为准，不要用「`cargo check` 过了」代替。

## 主要依赖

- 同工作区：`inox-common`（Fluent 目录加载）、`inox-config`（配置模型与主题令牌）。**没有**同层依赖，也**没有**到 `inox-compositor` 的依赖（ADR 0013、0018）。
- 第三方：`cxx` / `cxx-qt` / `cxx-qt-lib`（QML↔Rust 边界）、`serde` + `serde_json`（投影）、`zbus`（会话 D-Bus 回执）、`toml` / `toml_edit`（只改被编辑键）、`thiserror`、`anyhow`（二进制入口）、`tracing`、`tracing-subscriber`。
- build：`cxx-qt-build`（`build.rs` 里的 `CxxQtBuilder::new_qml_module`）。
- dev：`proptest`（T06 的属性测试）、`tempfile`（真实配置目录）。**没有** `iced` / `iced_*` / `image`，也不再有任何离屏渲染依赖。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。本地自检建议 `CARGO_HOME=<repo>/.cargo-home` + `--offline`。

| 目的 | 命令 |
|------|------|
| 编译检查 | `cargo check -p inox-settings --all-targets --all-features --offline` |
| 静态检查 | `cargo clippy -p inox-settings --all-targets --all-features --offline -- -D warnings` |
| 本 crate 测试 | `QT_QPA_PLATFORM=offscreen cargo test -p inox-settings --all-features --offline` |
| 两个 bin 的链接（cxx-qt 陷阱） | `cargo build -p inox-settings --bins --offline` |
| 格式化 | `cargo fmt -p inox-settings`（检查：`cargo fmt -p inox-settings -- --check`） |
| 本 crate 文档 | `cargo doc -p inox-settings --no-deps --all-features` |
| 本 crate 构建 | `cargo build -p inox-settings --release --all-features` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

`QT_QPA_PLATFORM=offscreen` 只对**子进程**有意义：QML 加载测试自己 spawn 子进程并设置该变量；测试进程内不得 `std::env::set_var`（edition 2024 下是 `unsafe`，本 crate 禁止）。`inoxctl` 的测试不需要显示。

## 测试要求

测试全部在 `src/headless_tests/` 里（bin 内 `#[cfg(test)]`，理由见上），`cargo test -p inox-settings` 跑的就是它们：

- `support.rs`：真实临时目录上的窗口（`Window`），按 runtime 的方式执行 `OutgoingCommand`；只对系统边界打桩（会话 D-Bus、时钟），不 mock 核心逻辑。
- `workflow.rs`：D04 的用户任务（应用 / 取消 / 冲突 / 宕机 / 重启 / 重载）与显示试用三条路径（确认保留、倒计时归零恢复文件、没有可应答的合成器时只保存），全部走真实文件；含 **≥1000 例**属性测试「校验通过的草稿写盘后回读恰好等于窗口显示的值；校验不通过的草稿不写任何文件」。
- `complete_settings.rs`：每个可编辑字段的边界与 TOML 往返。
- `catalog.rs`：en-US 与 zh-CN 的 id 与占位符一致（Q03），代码点名的 id 目录里都有。
- `projection.rs`：**取代原 iced 渲染测试的投影证据**——所有颜色都是令牌、被拒字段带错误且带危险色、写盘在途时每个控件禁用且写明原因、焦点序列的每一项都在投影里有对应控件（Q01）、显示试用弹窗的两个回答与警告色、`SavedOnly` ≠ `Applied`、空配置仍有整页、字号加倍后投影仍完整。断言的是值与句子，不是像素。
- `qml.rs`：**离屏加载证据**。父测试 spawn 本测试二进制（`--ignored --exact headless_tests::qml::qml_child`），子进程用 `QT_QPA_PLATFORM=offscreen` 走真实 `boot()` 加载 `qrc:` 模块，只有 QML 树回报的就绪与投影一致才打印标记；父测试同时断言 stderr 里没有 QML 错误（为此子进程设 `QT_ASSUME_STDERR_HAS_CONSOLE=1`，否则 Qt 在非 tty 下不打印告警）。
- 断言失败时必须能指出**哪个字段/哪个文件/哪一代次**，不允许用「有错误即可」代替定位。
- 纯逻辑单元测试留在各模块的 `#[cfg(test)]`：草稿（只写触碰过的字段、取消不写、恢复默认）、校验（含拒绝路径与 Fluent id）、文档补丁（逐字节保留）、原子保存失败矩阵、应用状态机（含陈旧回执）、投影（禁用原因、焦点顺序、逐字段错误）、桥与宿主的线程契约。

## 已知缺口（迁移后如实记录，不声称已解决）

1. **没有像素证据了。** Qt 画窗口，本 crate 不再渲染；Q02 的视觉验收由投影断言 + 一次离屏加载代替，**真实外观（对齐、间距、换行、对比度）没有截图基线**，也没有 waydriver 会话测试。真机 E2/E3/E4 验收仍需人工。
2. **没有 QML 层交互测试。** 旧测试用两次真实点击验证导航与 Apply；现在点击路径只由 `dispatch`/`parse_action` 的单元测试覆盖，QML 的鼠标事件本身（hover/press）没有自动化证据（要做得引入 Qt Test，未做）。
3. **多列列表的宽度只有值断言**：单元格与行内按钮的宽度由投影里的列权重推导，极端字号下是否真的不溢出没有实际布局测量。
4. **整份投影重发**：每次状态变化重新序列化整个模型（契约 §3 的取舍）。窗口规模下未见问题，但没有性能数据，也没有增量更新。
5. **AT-SPI 端到端未验证**：每个控件都写了 `Accessible.*`，`report_ready` 也要求语义项数量达标，但真实屏幕阅读器链路（Qt QPA → AT-SPI）没有自动测试。
6. **显示试用弹窗的 QML 状态未被离屏加载测试覆盖**：进入试用需要可应答的合成器（D-Bus），离屏测试只加载初始状态；试用投影由 `projection.rs` 断言。
7. **`#![allow(dead_code)]`** 会掩盖 bin target 里真正无人使用的条目；新增代码不得依赖它（见上）。

## 目录结构

```
crates/inox-settings/
  Cargo.toml        cxx / cxx-qt / cxx-qt-lib 依赖；[lints.rust] unsafe_code = "allow"（ADR 0035 例外）
  build.rs          CxxQtBuilder::new_qml_module("com.inox.settings.qml")：11 个 QML 文件逐个登记 + bridge 源文件
  src/
    main.rs         入口：日志、配置、目录 → boot() → QGuiApplication + QQmlApplicationEngine（自检模式自带）
    cxxqt_object.rs 唯一 QML↔Rust 边界：modelJson / ready / pump / dispatch / report_ready / quit + QML_READY
    ui_host.rs      Qt-free 契约：Host trait + 线程局部安装（bridge 只依赖它，inoxctl 也能编译）
    runtime.rs      UiRuntime：State、Task（perform）、pump（排空队列 + 试用 tick）、执行 OutgoingCommand
    app.rs          State / Message / update（Elm，无 I/O）+ OutgoingCommand
    apply.rs        已保存 vs 已生效的状态机、代次、每组件结果
    document.rs     四个配置文件作为文档、只改被编辑键的 TOML 补丁
    draft.rs        字段注册表、值、草稿叠加与改动记录
    persist.rs      原子写入、内容摘要、外部编辑检测
    validate.rs     镜像 inox-config 的校验与 FieldError
    collections.rs  显示器、输入法 argv、终端 argv、分屏壁纸的结构化编辑与转换
    focus.rs        焦点目标标识、真实布局边界与要滚动的容器
    pages/          十个页面：description + 卡片分组
    view_model.rs   状态 → 界面所画内容（含 focus_order）
    view.rs         投影：model + 令牌 → 可序列化 JSON；UiAction 闭合集合与 parse_action
    theme.rs        令牌 → 颜色（#AARRGGBB）与几何，纯函数（ADR 0018 允许的每组件一份）
    i18n.rs         Fluent 目录装配（inoxctl 也编译它）
    live.rs         受影响消费方选择、保存后冲突检测、D-Bus 回读与试用接口
    bin/inoxctl.rs  手工配置重载入口（用 #[path] 编译 bridge 以满足链接，见上）
    headless_tests/ mod.rs / support.rs / workflow.rs / complete_settings.rs / catalog.rs / projection.rs / qml.rs
  qml/              Main.qml（窗口 + pump + 键盘）、SettingsSurface.qml（导航/页面/状态行/动作）、
                    FieldRow.qml、TextFieldControl.qml、NumberControl.qml、ChoiceRow.qml、
                    FlagControl.qml、CollectionControl.qml、ControlButton.qml、StatusStrip.qml、TrialDialog.qml
  i18n/en-US/settings.ftl, i18n/zh-CN/settings.ftl
  README.md         本 crate 说明
  AGENTS.md         本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `settings`。
