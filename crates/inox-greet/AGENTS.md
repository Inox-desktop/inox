# AGENTS.md — inox-greet

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

显示管理器。所属层级：**L4**（见根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

它拥有的是**登录生命周期与登录界面**：六个阶段、代次关联与期限、滚动尝试限额、界面的投影与焦点模型、账户列表与默认账户、口令字段的掩码与凭据移交，以及 `auth-v1` 客户端（`src/boundary.rs`）。它**不**拥有认证本身（那是 `inox-auth`，ADR 0029），也**不**拥有 greeter 宿主：宿主是 [ADR 0037](../../docs/adr/0037-greeter-host-vt-handoff-and-the-boot-login-chain.md) §决策 1/3 定义的 L4 `inox-host`（起合成器与登录界面、有界预算重启、SIGTERM 逆序清理），本 crate 只按路径被它拉起、只通过边界套接字说话，不关窗也不重启自己。

**两条启动路径**：不带参数是**真实路径**，必须能找到边界套接字，否则在开窗之前以非零码退出并说明；`--preview` 打开的是「本构建没有边界」那一幅（不可用、不可输入）。`--self-check` 只诊断 `--preview` 那一幅，**不是**第三条登录路径。**不得**添加任何让运行的二进制在调用它时假装有认证边界的开关，也**不得**让真实构建显示「不收集凭据」之类的预览文案。

## 表面模式

`QGuiApplication` + `QQmlApplicationEngine`，QML 根是 `Window`（`qml/Main.qml`），与迁移前一样是**普通窗口**。它不是 `ext_session_lock_v1` 表面、也不是 layer 表面：登录界面在裸 VT 上的设备/VT 归属由宿主仲裁（ADR 0037 §决策 1–2），离屏渲染那一套（[ADR 0038](../../docs/adr/0038-offscreen-qml-for-layer-and-lock-surfaces.md)）属于 lock/panel，与 `inox-greet` 无关。

## QML + cxx-qt 结构

```text
crates/inox-greet/
  build.rs              CxxQtBuilder::new_qml_module("com.inox.greet.qml").qml_file("qml/Main.qml")
  qml/Main.qml          声明式界面：一个 Window，绑定 modelJson 的路径
  src/main.rs           入口：参数、配置、账户库、Qt 宿主（QGuiApplication + QQmlApplicationEngine）、--self-check
  src/cxxqt_object.rs   唯一的 QML↔Rust 边界：Greeter { modelJson } + dispatch/key/text/pump/tick/reportReady
  src/view.rs           纯投影：状态 + 令牌 → 一份 JSON 文档
  src/theme.rs          令牌 → QML 颜色/尺寸的纯函数
  src/runtime.rs        自己的 Host/Transport：命令线程 + auth-v1 客户端、线程本地安装
  src/app.rs            Elm：State / Message / update / key_binding（纯）
  src/state.rs          Greeter：六阶段、代次、限额、期限（纯）
  src/view_model.rs     状态 → 画面模型（纯）
  src/accounts.rs       系统账户库、默认选中、last-user 记录
  src/boundary.rs       auth-v1 客户端
  src/i18n.rs           Fluent 目录装配
  src/headless_tests.rs 二进制内的 boundary / catalog / cli / qml 测试
```

- **Rust → QML 只有一个属性**：`model_json`，`src/view.rs` 生成，每次状态变化整体重发；QML 里 `JSON.parse(greeter.modelJson)`。**不得**为字段新增 `#[qproperty]`：那会在边界上再写一份 UI 定义（契约 §3）。
- **QML → Rust 只有窄通道**：`dispatch(action)` 收固定动作词表（`activate` / `submit` / `cancel` / `switch-user` / `focus-next` / `focus-previous` / `select-account`，后者只带**下标**不带名字）、`key(name)` 收固定键名（tab / shift-tab / enter / escape / backspace）、`text(one character)` 收一个字符、`pump()` 排空边界线程送来的答复、`tick()` 推进模型时钟、`reportReady(title, a11yItems)` 回报自检。解析失败只 `warn!`，不 panic。
- QML 侧没有颜色/像素字面量与用户可见英文串（契约 §4）：颜色是 `theme::qml_color` 的 `#AARRGGBB`，尺寸由 spacing / radii / typography 派生，文案是中英两套 Fluent 里已经拼好的句子。结构性哨兵（`"transparent"`、`radius: width / 2`、JSON 动作串）不是设计值。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 的 `Cargo.toml` 把它覆盖为 `allow`，唯一原因是 `#[cxx_qt::bridge]` 宏自己生成 `unsafe extern "C++"` 块。**手写 Rust 里不得出现 `unsafe`。**
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- 与工作区内其他 Inox crate 的依赖使用 **path 依赖**（根级 §8）：在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明一次，本 crate 用 `{ workspace = true }` 引用。**新增依赖前先确认依赖方向合法**（根级 §2），否则 `cargo test -p inox-integration --test layering` 会失败。
- **登录界面可以重复锁屏的样式，但不能重复它的机制。** `view_model`/`view`/`theme` 按 [ADR 0018](../../docs/adr/0018-l3-component-architecture.md) 的模式各有一份（同形不同源，这是该 ADR 明确接受的成本）；但**不得**依赖 `inox-lock`（同层横向依赖）或把它的状态机搬过来。两者的边界不同：锁屏的锁状态归合成器，greeter 的登录状态归这里。
- **身份是选出来的，不是打出来的（[ADR 0032](../../docs/adr/0032-greeter-account-selection-and-login-interface.md)）。** 界面**不得**提供手工输入账户名的路径：账户来自 `src/accounts.rs` 读到的系统账户库（`getent passwd`，回落 `/etc/passwd`；`uid ∈ [1000, 65534)` 且 shell 不以 `nologin`/`false` 结尾，`root` 额外提供并置末），由 `Event::SelectAccount` 选中，`Message::AccountChosen` 只接受列表里已有的名字。QML 的行只上报**下标**，名字由 Rust 从投影自己的账户表里取。**不得**在 `accounts` 里静默丢弃一条名字不合 `auth-v1` 语法的账户：列出它、拒绝提交它、把原因写在界面上。切换账户必须清空已键入的口令。
- **「最近使用」的账户来自本组件自己的记录。** 会话**真正启动**后（`Phase::SessionStarted`）把账户名写入 `$XDG_STATE_HOME/inox/greet/last-user`，失败只 `warn` 不失败登录；**不得**为了这个功能去解析 `wtmp`/`lastlog`，也**不得**把它说成系统的登录历史。记录里的名字只在账户列表中匹配得到时才生效。它是**每个会话写一次**而不是每个进程写一次。
- **会话启动后登录界面驻留，3 秒后回到登录画面（[ADR 0037](../../docs/adr/0037-greeter-host-vt-handoff-and-the-boot-login-chain.md) §决策 4）。** `Phase::SessionStarted` **不再**关闭窗口：启动态停在屏幕上，`state::SESSION_STARTED_DWELL` 到期后回到 `Ready`——保留选中的账户、清空口令、焦点由投影落回首个可操作控件（口令字段）。驻留靠 QML 每秒调一次的 `tick()` 推进（与答复期限同一个时钟），**不得**为它新增线程、定时器或 sleep；驻留期间除时钟外的所有事件都被模型拒绝。`started:false` 仍是 `Failed` + 原因，**没有**计时器。
- **账户库读不到不是致命错误。** 界面必须仍然打开并区分「读不到列表」与「列表里没有可登录的账户」两个原因；不得因为读不到账户而拒绝启动，也不得在两者之间用同一句话。
- **凭据边界（ADR 0029）**：口令只以 `inox_common::secret::Secret`（`zeroize`、`Debug` 只输出 `<redacted>`）存在，只经 `auth-v1` 帧离开本进程。具体到 QML：
  - 口令**永不**进入 `#[qproperty]`、`model_json`、任何 QML 属性/变量，也**永不**进入 `TextField`——本 crate 的 QML 里**没有** `TextInput` / `TextField`：口令字段是一个 `Rectangle` + `Text`，画的是投影给的掩码（`view_model::MASK_GLYPH` 按长度重复）。
  - 按键以单个字符经 `cxxqt_object::text()` 到达，由 `app::key_binding` 转成 `Message::Credential(char)`；模型只在焦点落在口令字段时接受它。**QML 从不回读字段内容**。
  - `Event::Submit` 把 `Secret` **移动**进 `Command`，模型不再留副本（被拒绝的登录是重新输入而不是重放）；投影、日志、`Debug` 只拿得到字符数。
  - 因此 `view::projection` 里不可能出现口令：`GreeterViewModel` 里根本没有它的字段，`view.rs` 的测试断言「键入过的口令不出现在投影里」。
- **焦点模型归本 crate，不由 QML 决定。** `view_model` 只把可操作的控件放进 `focus_order`（登录画面：口令字段 → 登录 → 切换用户 → 清空；账户列表画面：每个账户一行 → 返回），`app` 的焦点在其中移动，QML 把焦点环画在投影说的那个控件上。QML 只上报键名，不解释焦点。**不得**让可见的焦点与键盘作用的控件变成两件事。
- **不能按 Space 触发操作。** 它是口令里可能出现的字符；`app::key_binding` 只认 Tab/Shift+Tab/Enter/Escape/Backspace，新增绑定前先想清楚它会不会吃掉一个字符。
- **没有边界就必须拒绝，而且要说清楚缺的是什么。** 不带 `--preview` 时若 `INOX_AUTH_SOCKET`（默认 `/run/inox/auth.sock`）上没有边界，就在开窗之前以非零码退出，文案里同时出现「authentication boundary」与「inox-auth」——`headless_tests::cli` 依赖这两个事实。**不得**让界面在没有边界时表现出可以登录的样子，也**不得**让有边界的构建显示预览横幅：屏幕上写的必须是这个构建真实会做的事。
- **用户可见文案只来自 Fluent**，`en-US` 与 `zh-CN` 同步；id 统一 `greet-` 前缀，`headless_tests::catalog` 双向检查「代码提到的 id 都在目录里」与「目录里的 id 都被用到」。
- **颜色、尺寸、间距、圆角只来自 `inox_config::Theme`**，在 `theme.rs` 里算成 QML 值：字号四角色由 `spacing.lg × typography.scale` 派生（1.5 / 1.25 / 1.0 / 0.875），hairline 为 `spacing.xs / 2`，焦点环为 `spacing.xs`，头像圆章与行内圆章直径是 `base` 的具名倍数（3.0 / 2.0）。不可用的字体缩放（NaN / 无穷 / ≤ 0）回落到 1.0，而不是把字号算成 0 或负数——登录界面看不见文字是安全问题，不是样式问题。
- **界面形态由 ADR 0032 定：整屏居中的单卡片。** 卡片头部是账户圆章与账户名（`title` 角色），口令字段、主按钮与次级控件占卡片宽度的 **2/3**（QML 里按 `layout` 的比例算，比例是 Rust 给的），账户列表是同一张卡片的第二幅画面。**不得**把控件拉满卡片，也不得再加回第二张卡片。
- **语义树是算出来的，不是声称的**：`qml/Main.qml` 里每个带 `Accessible.role` 的可见节点都由 `view::a11y_items` 计入，QML 在 `reportReady` 里回报自己走出来的数量，Rust 比对投影后再接受自检。新增/删除节点必须同时改这两处。

## 主要依赖

`anyhow`, `tracing`, `tracing-subscriber`；`cxx`、`cxx-qt`、`cxx-qt-lib`（build-dependency `cxx-qt-build`）；`serde_json`（投影）；`inox-common`（i18n、`secret::Secret`）、`inox-config`（设计令牌）、**`inox-auth`（`default-features = false`：只有 `auth-v1` 的帧与类型，不链接 `libpam`，也不含启动会话的代码）**。

测试：`tempfile`。**向下有一条 L4 → L2 边**：`inox-greet → inox-auth`（`default-features = false`，只有帧与类型，见职责与 ADR 0029）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

本 crate 是工作区成员，命令应尽量用 `-p` 限定范围（CI 的测试矩阵跑的就是同一批命令）。在 `crates/<name>/` 目录内或仓库根执行均可。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `QT_QPA_PLATFORM=offscreen cargo test -p inox-greet --all-features` |
| 本 crate 检查 | `cargo check -p inox-greet --all-targets --all-features` |
| 静态检查 | `cargo clippy -p inox-greet --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt -p inox-greet` |
| 本 crate 构建 | `cargo build -p inox-greet --release --all-features` |
| 本 crate 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-greet --no-deps --all-features` |
| 预览（无边界） | `cargo run -p inox-greet -- --preview` |
| 离屏自检 | `QT_QPA_PLATFORM=offscreen cargo run -p inox-greet -- --preview --self-check` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

`QT_QPA_PLATFORM=offscreen` 加在 `cargo test` **外层**：测试进程自己不改环境（edition 2024 下 `std::env::set_var` 是 `unsafe`），QML 自检会 spawn 自己作为子进程并把环境交给它。

## 测试要求

本 crate 是 **bin-only**：没有 `src/lib.rs`，也没有 `tests/`。cxx-qt 0.10 把 bridge 生成的 `cxx_qt_init_*` 定义嵌在声明 bridge 的目标里，同 package 的第二个 target 链接时会 `undefined reference`（KDAB/cxx-qt#770）。因此所有测试都在二进制内部的 `#[cfg(test)]` 模块里，`src/headless_tests.rs` 按 `boundary` / `catalog` / `cli` / `qml` 四组组织。

- 纯逻辑必须有单元测试（`#[cfg(test)]`），**含拒绝与错误路径**：过期、乱序、重复与来自从未发出的尝试的答复都不能启动会话；取消绕不过滚动限额；超长或不合契约的账户名被拒绝并说明；口令过长为拒绝而非截断；**会话启动后驻留到期才复位**（账户保留、口令为空、可再次提交），`started:false` 停在 `Failed` 且不被时钟改写，而从未启动过会话的画面上时钟不改变任何东西。
- `accounts` 的纯逻辑必须有单元测试：uid 区间的两个端点、`nologin`/`false`、`root` 置末、同名去重、坏行跳过、GECOS 全名与首字母、默认选中的四种情形，以及记录文件的读回。
- 界面必须有 `view_model` 与 `view` 层的单测（每个 `Phase` 的画面、禁用必有原因、焦点只落在可操作控件上、账户列表只在有得选时打开、**口令不出现在投影里**、投影与 scaffolding 形状一致），不能只靠 QML 加载测试。
- `headless_tests::catalog` 做双向检查（代码提到的 id 都在目录里、目录里的 id 都被用到），因此新增/删除文案必须两边一起改。
- `headless_tests::cli` 覆盖真实路径在没有边界时的拒绝（非零退出、文案含 `authentication boundary` 与 `inox-auth`）、`--help`、未知参数，以及「`--self-check` 不能当成第三条登录路径」。
- `headless_tests::boundary` 用手写的假 `auth-v1` 服务端（真实套接字）覆盖客户端的请求内容与答复映射；`src/boundary.rs` 的每次行为改动都要在这里有对应用例。
- **QML 加载测试**（`headless_tests::qml`）：`QT_QPA_PLATFORM=offscreen` + `QT_QUICK_BACKEND=software` 下加载本 crate 的 QML 模块，断言根对象非空、`reportReady` 被接受（标题与语义项数与投影一致）、且输出里没有 QML 运行时错误。
- 迁移前那套 1280×720 的真实控件树像素断言（双主题 × 双语言 × 200% 字号）已随控件树一起删除；它现在的位置是投影断言 + QML 加载测试。**像素截图证据**属于尚未完成的 waydriver 工作，不得被读作已完成（验收 Q02）。
- 测试代码不受「禁止 unwrap/expect/panic」约束。
- 真实认证、锁屏与挂起接线必须先满足 ADR 0015 的人类安全审查前提；登录链的该前提由 [ADR 0037](../../docs/adr/0037-greeter-host-vt-handoff-and-the-boot-login-chain.md) 的批准记录给出，但**真机登录（E4）仍未被验证**，本 crate 不声称它已通过。ADR 0021 记录的 waydriver 与 AT-SPI 未验证项与本 crate 无关地仍未完成；宿主（`inox-host`）的实现与真机证据同样不在本 crate 的验收范围内。

## 目录结构

```
crates/inox-greet/
  Cargo.toml    工作区继承：字段一律 *.workspace = true
  build.rs      QML 模块 com.inox.greet.qml + qml/Main.qml
  qml/          Main.qml（唯一的界面文件）
  src/          源码（模块划分见上）
  i18n/         en-US / zh-CN 的 Fluent 目录
  README.md     本 crate 说明
  AGENTS.md     本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `greet`。
