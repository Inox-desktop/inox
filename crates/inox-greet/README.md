# inox-greet

L4 可选显示管理器。属于 Inox 桌面环境的 **L4** 层（[ADR 0021](../../docs/adr/0021-l4-session-services.md)）。认证本身不在本 crate：它经 `auth-v1` 私有契约（[ADR 0029](../../docs/adr/0029-inox-authentication-boundary.md)）交给 L2 的 [`inox-auth`](../inox-auth/README.md)。

界面是 **QML + Qt Quick**，经 cxx-qt 绑定到 Rust（[ADR 0033](../../docs/adr/0033-qml-qt-quick-shell-evaluation.md)、[ADR 0035](../../docs/adr/0035-cxx-qt-binding.md)）。状态机、焦点、文案与令牌留在 Rust：界面拿到的是一份 JSON 投影，QML 只负责画，并把按键与点击原样报回来。

**不带参数是真实登录路径。** 它要求认证边界套接字存在（`INOX_AUTH_SOCKET`，默认 `/run/inox/auth.sock`），否则在开窗之前以非零状态码退出并说明缺的是什么；找到边界后界面才可用。认证成功后由边界以该账号身份启动会话。**界面不随会话启动而关闭**：它停在启动态，3 秒后回到登录画面——账户保留、口令清空、焦点回到首个可操作控件（[ADR 0037](../../docs/adr/0037-greeter-host-vt-handoff-and-the-boot-login-chain.md) §决策 4）。用户会话在另一个 VT 上（该 ADR §决策 2），VT 被暂停期间这一幅没人看得见，注销回来时屏幕上必须已经是登录画面，而不是一个需要宿主重新拉起的空位。

**`--preview` 打开的是「本构建没有边界」那一幅。** 界面为 `state::Greeter` 的六个生命周期阶段各有一幅画面（不可用、就绪、验证中、会话启动中、失败、已启动）；预览显示的只有*不可用*那一幅：口令字段不可输入、登录按钮禁用、键盘只剩 Escape，横幅明确写着「仅供预览，不收集凭据」。`--self-check` 只是 `--preview` 那一幅的离屏诊断（真的会加载 QML 模块并要求树回报与投影一致的标题与语义项数），**不是**第三条登录路径，也不能在真实路径上使用。没有任何开关能让运行的二进制假装自己有认证边界，真实构建也不会显示预览横幅。

## 口令去哪了（凭据边界）

- 口令只以 `inox_common::secret::Secret` 的形式存在（`zeroize`，`Debug`/`Display` 只给 `<redacted>`），只经 `auth-v1` 帧离开本进程。
- **它从不进入 QML。** 没有 `#[qproperty]`、没有 `model_json` 字段、没有 QML 变量、也**没有 `TextField`**：QML 里没有 `TextInput`，口令字段是一个 `Rectangle` + `Text`，画的是投影给出的**掩码**（`view_model::MASK_GLYPH` 按长度重复）。QML 只知道「有几个字符」，从来拿不到那几个字符。
- 键以单个字符经 `cxxqt_object::text()` 到达，由 `app::key_binding` 变成 `Message::Credential(char)`；模型只在焦点落在口令字段时接受它。提交时 `Secret` 被**移动**进命令，模型不再留副本——所以被拒绝的登录是重新输入，而不是重放。
- 本 crate 以 `inox-auth` 的 `default-features = false` 依赖它：拿到的是 `auth-v1` 的帧与类型，**不链接 `libpam`**，也不含能替别人启动会话的代码。

## 界面与 QML 结构

```text
crates/inox-greet/
  build.rs      QML 模块 com.inox.greet.qml + qml/Main.qml（加载 URL：qrc:/qt/qml/com/inox/greet/qml/qml/Main.qml）
  qml/Main.qml  唯一的界面文件：一个 Window、一整屏居中的卡片、账户列表第二幅画面
  src/main.rs   入口：参数、配置、账户库、Qt 宿主（QGuiApplication + QQmlApplicationEngine）、--self-check
  src/cxxqt_object.rs  唯一的 QML↔Rust 边界
  src/view.rs   状态 + 令牌 → 一份 JSON 文档（纯）
  src/theme.rs  令牌 → QML 颜色/尺寸（纯）
  src/runtime.rs  Host/Transport：命令线程、auth-v1 客户端、线程本地安装
  src/app.rs    Elm：State / Message / update / key_binding（纯）
  src/state.rs  Greeter：六阶段、代次、限额、期限（纯）
  src/accounts.rs  系统账户库、默认选中、last-user 记录
  src/view_model.rs  状态 → 画面模型（纯）
  src/boundary.rs    auth-v1 客户端
  src/i18n.rs        Fluent 目录装配
  src/headless_tests.rs  boundary / catalog / cli / qml 四组测试
  i18n/{en-US,zh-CN}/messages.ftl
```

- **Rust → QML**：只有一个只读属性 `modelJson`（`src/view.rs` 生成），每次状态变化整体重发；QML 里 `JSON.parse(greeter.modelJson)`，所有绑定读这个对象。不为单个字段开 `#[qproperty]`。
- **QML → Rust**：`dispatch(action)`（固定动作词表；账户行只上报**下标**，名字由 Rust 从投影自己的账户表取）、`key(name)`（tab / shift-tab / enter / escape / backspace）、`text(one character)`（口令字段的输入）、`pump()`（排空边界答复）、`tick()`（推进模型时钟：答复期限与启动态驻留）、`reportReady(title, a11yItems)`（自检回报）。
- **焦点归 Rust**：`view_model` 只把可操作的控件放进 `focus_order`，QML 把焦点环画在投影说的那个控件上；QML 只上报键，不解释焦点。
- **颜色/尺寸/文案只在投影里**：QML 里没有颜色字面量、没有像素字面量、没有用户可见英文串；颜色是 `theme::qml_color` 的 `#AARRGGBB`，尺寸由 spacing / radii / typography 派生（不可用的字体缩放回落到 1.0，而不是算成 0 或负数），文案是中英两套 Fluent 里已经拼好的句子。

## 账户

**身份是选出来的，不是打出来的**（[ADR 0032](../../docs/adr/0032-greeter-account-selection-and-login-interface.md)）。界面不提供手工输入账户名的路径：`src/accounts.rs` 读系统账户库（`getent passwd`，失败或答空时回落 `/etc/passwd`），提供 `uid ∈ [1000, 65534)` 且 shell 不以 `nologin`/`false` 结尾的账户并额外把 `root` 排在最后；默认选中本组件记录的上次使用账户，没有记录时选第一个普通账户。

`last-user` 记录的位置是 `$XDG_STATE_HOME/inox/greet/last-user`（`$XDG_STATE_HOME` 缺席时回落 `$HOME/.local/state/...`）。写入失败只记 `warn`：下一次登录退回第一个账户，而不是让登录失败。它**每个会话写一次**——在会话真正启动时写，3 秒后的复位不写。**它记录的是本 greeter 发起过的登录，不是系统的登录历史**（不解析 `wtmp`/`lastlog`）。

## 状态

**已实现**（证据等级 E0/E1；纯逻辑与投影有单元测试，QML 有离屏加载测试）：

| 模块 | 内容 | 验收 |
|------|------|------|
| `accounts` | 系统账户库的读取与过滤（uid 区间、`nologin`/`false`、`root` 置末、同名去重、坏行跳过、GECOS 全名与首字母）、默认选中的四种情形、`last-user` 记录的读写；纯解析部分与系统读取分开，前者可单测 | ADR 0032 |
| `state` | 登录生命周期：六个阶段、代次关联（旧代次、重复、从未发出的答复一律拒绝）、30 秒答复期限、滚动 60 秒最多 3 次尝试、取消清理（口令清空而账户保留）、启动回执；账户选择、账户名边界（`MAX_IDENTITY_BYTES`、与契约同一份账户名规则）；**启动态驻留 `SESSION_STARTED_DWELL`（3 秒）后复位到 `Ready`**；口令以 `Secret` 存在，提交时**移出**模型，超长口令拒绝而非截断 | D05、Q01 |
| `view_model` | 状态到画面的纯投影：六个 `Phase` 各一个 `Face`，账户视图，账户列表只在有得选时打开，禁用原因按用户遇到的顺序给出，`focus_order` 只含可操作的控件 | D05、Q01 |
| `boundary` | `auth-v1` 客户端：一条连接一条请求，答复映射为 `Report`（`Accepted`/`Rejected`/`Unavailable`/`Unreachable`/`SessionStarted`）；不链接 `libpam` | D05 |
| `view` | 纯投影：窗口、令牌（颜色/字号/间距/圆角/焦点环/圆章）、卡片头、横幅、口令掩码、状态行、会话行、提示、控件与账户列表，全部是一个 JSON 文档；语义树项数由 `a11y_items` 算出 | Q02 |
| `theme` | 令牌 → QML 颜色与尺寸的纯函数；焦点环用「与该控件填充色可辨」的那一个令牌，因此焦点在填充按钮上也看得见 | Q01、Q02 |
| `app` | Elm 循环：`State`/`Message`/`update`，时钟由 `Ticked` 推进（启动态驻留复位也走它）、焦点模型、`key_binding` 纯函数、`take_outgoing` 是唯一的对外出口、会话真正启动时为该会话记录一次账户 | D05、Q01 |
| `cxxqt_object` / `runtime` | 桥与宿主：`modelJson` + 窄动作通道；命令在各自线程上走 `auth-v1`，答复经通道回到 `pump()` | D05 |
| `i18n` | `en-US` / `zh-CN` 目录，一组 `greet-` 前缀的 id，双向一致性由 `headless_tests::catalog` 断言 | Q03 |

**未实现 / 被阻塞**（不声称上表之外的任何等级）：

- **认证在别处。** 本 crate 没有 PAM、没有特权进程、不能启动会话、也无法凭空产生一个成功答复：一切都经 `auth-v1` 交给 `inox-auth`。
- **不是安全登录表面（本 crate 自己不是）。** 界面（含真实路径）仍是一个普通 xdg-shell 窗口，可以截图、可以被别的窗口遮挡，也不提供任何输入隔离；把它放到裸 VT 上、由 logind/libseat 仲裁设备与 VT 归属，是宿主侧的事（[ADR 0037](../../docs/adr/0037-greeter-host-vt-handoff-and-the-boot-login-chain.md) §决策 1–§决策 2）。
- **宿主不在本 crate。** 谁在裸 VT 上拉起登录界面、合成器与界面崩溃如何重启、会话启动后 VT 如何交接、注销后如何回到登录画面，都由 ADR 0037 定义的 L4 宿主编排。本 crate 因此不再在会话启动时关闭窗口，也不重启自己；**真机登录（E4）尚未取得证据**，不得读作已完成。
- **无障碍只到「语义树存在」这一步。** QML 的每个可见控件都带 `Accessible.role` / `Accessible.name`，树在 `reportReady` 里回报自己数出来的项数，Rust 与投影比对后才接受；但 CI 里没有 AT-SPI 走查，也没有对真实辅助技术的证据（ADR 0021 的未验证项仍然未验证）。
- **没有像素截图证据。** 迁移前那套 1280×720 的真实控件树像素断言随控件树一起删除，现在由投影断言 + QML 离屏加载覆盖；截图证据要等 waydriver 工作（验收 Q02 的这一半未完成）。

## 测试

```sh
QT_QPA_PLATFORM=offscreen cargo test -p inox-greet --all-features
```

本 crate 是 **bin-only**（cxx-qt 0.10 的链接约束，见 [`AGENTS.md`](./AGENTS.md)），所有测试都在二进制内部：

- 单元测试：每条规则各一条，**含拒绝路径**——过期、乱序、重复、来自从未发出的尝试的成功一律不能启动会话；取消绕不过滚动限额；超长或不合 `auth-v1` 语法的账户名被拒绝并说明；超长口令**拒绝**而不截断；口令只在焦点落在口令字段时进入模型，提交即移交且不留副本，**并且不出现在投影里**；账户过滤的边界（uid 999/1000/65534、`nologin`/`false`/`root`/同名/坏行）、默认选中的四种情形与记录文件的读回各有断言。
- `headless_tests::boundary`：手写的假 `auth-v1` 服务端（真实套接字）驱动真实客户端，断言请求内容与答复映射；边界不存在、版本不匹配、连接中途关闭都必须是 `Unreachable`。
- `headless_tests::catalog`：`en-US` 与 `zh-CN` 的 id 与占位符一致；代码里出现的每个 `greet-…` id 都在目录里，目录里每个 id 都被代码用到。
- `headless_tests::cli`：真实子进程没有边界时拒绝登录（非零退出、文案含 `authentication boundary` 与 `inox-auth`）；`--help` 可用；未知参数失败；`--self-check` 不能当第三条登录路径。
- `headless_tests::qml`：`QT_QPA_PLATFORM=offscreen` 下加载本 crate 的 QML 模块，断言根对象非空、`reportReady` 被接受、输出里没有 QML 运行时错误。测试进程不改自己的环境（edition 2024 下 `std::env::set_var` 是 `unsafe`），而是 spawn 自己作为子进程，把 `QT_QPA_PLATFORM` 放进它的环境里。

## 开发约定

- 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径。**
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 用 `greet`。

## 许可证

LGPL-3.0，见仓库根的 [`LICENSE`](../../LICENSE)。
