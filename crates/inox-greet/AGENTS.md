# AGENTS.md — inox-greet

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

显示管理器。所属层级：**L4**（见根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

它拥有的是**登录生命周期与登录界面**：六个阶段、代次关联与期限、滚动尝试限额、界面的投影与控件树、焦点模型、账户列表与默认账户、口令字段的掩码与凭据移交，以及 `auth-v1` 客户端（`src/boundary.rs`）。它**不**拥有认证本身（那是 `inox-auth`，ADR 0029），也**不**拥有 greeter 宿主（仍不存在，ADR 0021 未决项）。

**两条启动路径**：不带参数是**真实路径**，必须能找到边界套接字，否则在开窗之前以非零码退出并说明；`--preview` 打开的是「本构建没有边界」那一幅（不可用、不可输入）。**不得**添加任何让运行的二进制在调用它时假装有认证边界的开关，也**不得**让真实构建显示「不收集凭据」之类的预览文案。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- 与工作区内其他 Inox crate 的依赖使用 **path 依赖**（根级 §8）：在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明一次，本 crate 用 `{ workspace = true }` 引用。**新增依赖前先确认依赖方向合法**（根级 §2），否则 `cargo test -p inox-integration --test layering` 会失败。
- **登录界面可以重复锁屏的样式，但不能重复它的机制。** `view_model`/`view`/`theme` 按 ADR 0018 的模式各有一份（同形不同源，这是 ADR 0018 明确接受的成本）；但**不得**依赖 `inox-lock`（同层横向依赖）或把它的状态机搬过来。两者的边界不同：锁屏的锁状态归合成器，greeter 的登录状态归这里。
- **身份是选出来的，不是打出来的（ADR 0032）。** 界面**不得**提供手工输入账户名的路径：账户来自 `src/accounts.rs` 读到的系统账户库（`getent passwd`，回落 `/etc/passwd`；`uid ∈ [1000, 65534)` 且 shell 不以 `nologin`/`false` 结尾，`root` 额外提供并置末），由 `Event::SelectAccount` 选中，`Message::AccountChosen` 只接受列表里已有的名字。**不得**在 `accounts` 里静默丢弃一条名字不合 `auth-v1` 语法的账户：列出它、拒绝提交它、把原因写在界面上，是这一条的可检验形式。切换账户必须清空已键入的口令（为一个账户打的口令不是另一个账户的口令）。
- **「最近使用」的账户来自本组件自己的记录。** 会话**真正启动**后（`Phase::SessionStarted`）把账户名写入 `$XDG_STATE_HOME/inox/greet/last-user`，失败只 `warn` 不失败登录；**不得**为了这个功能去解析 `wtmp`/`lastlog`，也**不得**把它说成系统的登录历史。记录里的名字只在账户列表中匹配得到时才生效。
- **账户库读不到不是致命错误。** 界面必须仍然打开并区分「读不到列表」与「列表里没有可登录的账户」两个原因；不得因为读不到账户而拒绝启动，也不得在两者之间用同一句话。
- **口令只以 `inox_common::secret::Secret` 的形式存在，且只在被输入到被送走之间存在。** ADR 0029 批准后本 crate 有口令字段，规则随之收紧而非放宽：字段是**掩码**（`view_model::MASK_GLYPH` 按长度重复），按键经 `app::key_binding` 以消息到达，**永不进 `iced::widget::text_input`**（iced 的编辑器持有本 crate 无法清零的 `String`，还提供复制粘贴）；`Event::Submit` 把 `Secret` **移动**进命令，模型不再留副本（因此被拒绝的登录是重新输入而不是重放）；`Model`/投影/日志只拿得到字符数，拿不到值。**本 crate 因此没有任何 `text_input`**：账户名是系统数据，口令是掩码。
- **焦点模型归本 crate，不由 iced 决定。** iced 0.14 的 `button` 没有 `id`、不可聚焦，`operation` 也没有 `unfocus`；因此 `view_model` 只把可操作的控件放进 `focus_order`（登录画面：口令字段 → 登录 → 切换用户 → 清空；账户列表画面：每个账户一行 → 返回），`app` 的焦点在其中移动，`view` 把焦点环画在投影说的那个控件上。**不得**让可见的焦点与键盘作用的控件变成两件事。
- **不能按 Space 触发操作。** 它是口令里可能出现的字符；`app::key_binding` 只认 Tab/Shift+Tab/Enter/Escape，新增绑定前先想清楚它会不会吃掉一个字符。
- **没有边界就必须拒绝，而且要说清楚缺的是什么。** 不带 `--preview` 时若 `INOX_AUTH_SOCKET`（默认 `/run/inox/auth.sock`）上没有边界，就在开窗之前以非零码退出，文案里同时出现「authentication boundary」与「inox-auth」——`tests/cli.rs` 依赖这两个事实。**不得**让界面在没有边界时表现出可以登录的样子，也**不得**让有边界的构建显示预览横幅：屏幕上写的必须是这个构建真实会做的事。
- **用户可见文案只来自 Fluent**，`en-US` 与 `zh-CN` 同步；id 统一 `greet-` 前缀，`tests/catalog.rs` 双向检查「代码提到的 id 都在目录里」与「目录里的 id 都被用到」。
- **颜色、尺寸、间距、圆角只来自 `inox_config::Theme`。** `view.rs` 与 `theme.rs` 里没有**设计色与设计尺寸**的字面量（结构性哨兵如 `Color::TRANSPARENT`、边框宽 `0.0` 除外）；字号四角色由 `spacing.lg × typography.scale` 派生，倍数为 1.5 / 1.25 / 1.0 / 0.875，hairline 为 `spacing.xs / 2`，焦点环为 `spacing.xs`，头像圆章直径与行内圆章直径是 `base` 的两个具名倍数（3.0 / 2.0，见 `theme::medallion_size` / `row_medallion_size`）（`docs/quality/ui-guidelines.md` §1–§2）。
- **界面形态由 ADR 0032 定：整屏居中的单卡片。** 卡片头部是账户圆章与账户名（`title` 角色），口令字段、主按钮与次级控件占卡片宽度的 **2/3**（`view::control_row`：两侧各一份 gutter、控件四份——`FillPortion` 只在同一行里有兄弟才成立，放进居中的列会解析成整宽），账户列表是同一张卡片的第二幅画面。**不得**把控件拉满卡片，也不得再加回第二张卡片。

## 主要依赖

`anyhow`, `tracing`, `tracing-subscriber`；`iced` 0.14（`tiny-skia` 渲染器）、`inox-common`（i18n、`secret::Secret`）、`inox-config`（设计令牌）、**`inox-auth`（`default-features = false`：只有 `auth-v1` 的帧与类型，不链接 `libpam`，也不含启动会话的代码）**。

测试：`iced_core` / `iced_runtime` / `iced_tiny_skia`（离屏渲染真实控件树）、`image`（仅在设置 `INOX_SCREENSHOT_DIR` 时写 PNG 工件）、`tempfile`。**向下有一条 L4 → L2 边**：`inox-greet → inox-auth`（`default-features = false`，只有帧与类型，见职责与 ADR 0029）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

本 crate 是工作区成员，命令应尽量用 `-p` 限定范围（CI 的测试矩阵跑的就是同一批命令）。在 `crates/<name>/` 目录内或仓库根执行均可。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-greet --all-features` |
| 本 crate 构建 | `cargo build -p inox-greet --release --all-features` |
| 本 crate 文档 | `cargo doc -p inox-greet --no-deps --all-features` |
| 全工作区测试 | `cargo test --workspace --all-features` |
| 静态检查 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt --all` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`），**含拒绝与错误路径**：过期、乱序、重复与来自从未发出的尝试的答复都不能启动会话；取消绕不过滚动限额；超长或不合契约的账户名被拒绝并说明；口令过长为拒绝而非截断。
- `accounts` 的纯逻辑必须有单元测试：uid 区间的两个端点、`nologin`/`false`、`root` 置末、同名去重、坏行跳过、GECOS 全名与首字母、默认选中的四种情形，以及记录文件的读回。
- 界面必须有 `view_model` 层的单测（每个 `Phase` 的画面、禁用必有原因、焦点只落在可操作控件上、账户列表只在有得选时打开），不能只靠像素测试。
- `tests/render.rs` 必须覆盖双主题 × 双语言 × 200% 字号与全部关键状态；渲染尺寸是验收 Q02 要求的 **1280×720**。
- `tests/catalog.rs` 做双向检查（代码提到的 id 都在目录里、目录里的 id 都被用到），因此新增/删除文案必须两边一起改。
- `tests/cli.rs` 覆盖真实路径在没有边界时的拒绝、`--help` 与未知参数。
- `tests/boundary.rs` 用手写的假 `auth-v1` 服务端（真实套接字）覆盖客户端的请求内容与四种答复的映射；`src/boundary.rs` 的每次行为改动都要在这里有对应用例。
- 测试代码不受「禁止 unwrap/expect/panic」约束。
- 真实认证、greeter 宿主、锁屏与挂起接线必须先满足 ADR 0015 的人类安全审查前提；ADR 0021 记录的未验证项（greeter 宿主、waydriver、AT-SPI）不得被读作已完成。

## 目录结构

```
crates/inox-greet/
  Cargo.toml    工作区继承：字段一律 *.workspace = true
  src/          源码（模块划分见 README）
  i18n/         en-US / zh-CN 的 Fluent 目录
  tests/        catalog.rs、cli.rs、render.rs、support/mod.rs
  README.md     本 crate 说明
  AGENTS.md     本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `greet`。
