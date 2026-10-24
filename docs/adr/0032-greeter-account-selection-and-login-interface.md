# ADR 0032: 登录界面按系统账户选择身份

- 状态：proposed
- 日期：2026-10-18
- 决策者：架构组
- 批准记录：待补（人类评审 PR / 评论与日期）。本仓库维护者已在 2026-10-18 的开发会话中明确要求按此方案实施（「整屏居中 + 头像 + 单卡片口令区，用 `getent passwd` 取可登录账户，额外加 root，默认选中最近使用的账户，不再提供手工输入用户名」），本文记录的是这次实施所依据的决策；ADR 本身仍待评审。
- 关系：细化 [ADR 0021](0021-l4-session-services.md) 的 `inox-greet` 界面条目，与 [ADR 0025](0025-lock-screen-surface-ui-and-login-interface.md) 的登录界面首版**部分取代**（界面形态与身份来源改变，锁屏一侧的决策不变）；复用 ADR 0029 的认证边界与 `auth-v1` 账户名规则，不改动它。**不改变** ADR 0002/0016 的分层与依赖方向（不新增任何依赖：账户经 `std::process::Command` 读取，`thiserror` 未引入）。

## 背景

ADR 0025 把 `inox-greet` 的登录界面从骨架补成六个阶段各有画面、键盘可达、禁用原因可读的界面，但身份仍是**一个字框**：`Identity(String)` 事件把用户键入的名字放进模型，界面按标签—输入框的形式画出来。维护者要求改成「macOS 锁屏式」的样子：整屏居中、头像、单卡片口令区；身份从系统账户列表里选，默认选中最近使用过的账户——因为界面上不再提供手工输入用户名的方案。

这件事有三个需要写下来的后果，而不是一次纯样式改动：

1. **身份的来源变了。** 从「用户键入的字符串」变成「系统账户数据库里的一条记录」。这决定了界面上允许出现什么、以及 `auth-v1` 请求里的账户名从哪来。
2. **本 crate 开始读系统账户库。** `getent passwd` 经 NSS 解析（LDAP/SSSD 账户同样可见），回落 `/etc/passwd`；这是一次只读的读取，但它引入了本 crate 此前没有的进程与文件系统访问。
3. **界面开始记录一个事实。** 「最近使用过的账户」需要一个来源。本 crate 选择记录**自己**发起过的那次登录，而不是去解析系统二进制登录库。

现状（按锁定版本读源码）：

- `crates/inox-greet/src/state.rs`：`Greeter` 持有 `username: String`，`Event::Username` 由 `Message::Identity` 驱动，`MAX_IDENTITY_BYTES = 128` 与 `inox_auth::wire::valid_username` 一起界定「可提交的名字」。
- `crates/inox-greet/src/theme.rs` / `view.rs`：令牌 → 样式，两张卡片（登录卡 + 会话卡），字段框由容器绘制。
- `crates/inox-greet/tests/render.rs`：1280×720 的双主题 × 双语言 × 200% 字号的离屏像素断言，含一个用像素定位按钮的点击用例。
- `crates/inox-auth/src/wire.rs`：`valid_username`（首字符为字母或 `_`，其余为 ASCII 字母数字或 `_ - .`，长度 ≤ 128）与 `MAX_CREDENTIAL_BYTES`；账户名的唯一权威在这一份实现里。

未验证、本文也不声称的：真实显示器上的外观（E4）；greeter 宿主（ADR 0021 未决项）；AT-SPI 语义导出（iced 0.14 无桥）。

## 决策驱动因素与备选

| 方案 | 收益 | 成本 / 风险 | 选择或否决依据 |
|------|------|-------------|----------------|
| 保持现状（键入用户名 + 两张卡片） | 零改动 | 维护者明确要求改；身份仍要人手打，拼错的名字会被送进认证边界 | 否决：与本次指令冲突 |
| **系统账户列表 + 最近使用记录 + 整屏居中单卡片界面** | 身份来自系统事实；不再有拼错名字的路径；界面与锁屏共享同一套令牌与角色；不新增依赖 | 引入账户库读取（只读）与一个状态文件；`State` 的公开构造签名变化 | **采用** |
| 账户列表 + 用户名输入框并存（自动补全） | 兼顾 NSS 之外的账户 | 维护者明确「不再提供手工输入用户名的方案」；两条身份路径要各自处理非法名、超长名与清空语义 | 否决 |
| 最近使用账户来自 `/var/log/wtmp`（`last`/`lastlog`） | 「最近使用」是系统事实，不依赖本组件的记录 | 需要解析 `utmpx` 的二进制布局与字节序，且记录的是**全系统**最后一次登录（可能是 tty、可能是别的 DM）；`wtmp` 缺失或轮转时无解 | 否决（本轮）：解析成本高，而它回答的不是「这个 greeter 上次给谁登录」 |
| 由 greeter 宿主注入账户列表 | 宿主决定策略，本 crate 只管画 | 宿主尚不存在（ADR 0021），会把一个当下无法验证的接口写进契约 | 否决（本轮）：并入宿主未决项 |

## 决策

### 1. 身份是系统账户数据库里的一条记录，不是键入的字符串

`inox-greet` 新增 `src/accounts.rs`：

- **读法**：`getent passwd`（NSS 视角，与系统其余部分一致）；无法运行或答空时读 `/etc/passwd`。两者都只读，解析同一份 `passwd(5)` 行。
- **提供的范围**：`uid ∈ [1000, 65534)` 且登录 shell 不以 `nologin` / `false` 结尾的账户；`root`（uid 0）**额外提供**并排在最后。`nobody`（65534）与其余系统账户不提供。
- **顺序与默认选中**：普通账户按 uid 升序、同名只出现一次；默认选中记录在案的上次使用账户，没有记录（或系统上已不存在）时选第一个普通账户，只有 root 时选 root，列表为空就没有默认选中。
- **不做名字过滤**：账户名不合法（`auth-v1` 语法不接受）时**不隐藏**该账户，而是由模型拒绝并让界面给出可读原因——静默丢弃一条系统上存在的账户会让「为什么没有我的账户」无话可答。

### 2. 界面上没有手工输入用户名的路径

身份由账户列表选择（`Message::AccountChosen` / `Event::SelectAccount`），界面为登录与账户列表两幅画面（`view_model::FocusTarget::{Account, SwitchUser, Cancel}`）。账户多于一个时提供「切换用户」，只有一个账户时不提供（没有可切换的对象）。选择另一个账户会**清空已键入的口令**：为一个账户打的口令不是另一个账户的口令。

`auth-v1` 的账户名规则仍是唯一权威：`Greeter::can_submit` 继续调用 `inox_auth::wire::valid_username`，界面把拒绝理由画出来。

### 3. 「最近使用」是 greeter 自己的记录，不是系统登录库

会话**真正启动**之后（`Phase::SessionStarted`，即边界确认会话已开始），`State` 把该账户名写入 `$XDG_STATE_HOME/inox/greet/last-user`（回落 `$HOME/.local/state/...`，经 `inox_common::paths::XdgEnv`）。写入失败只记 `warn`：下一次登录会退回第一个账户，而不是让登录失败。**不**解析 `wtmp`，也**不**声称这是系统的登录历史。

### 4. 界面形态：整屏居中的单卡片，头像 + 账户名 + 口令

- `view.rs` 画**一张**卡片（`surface` + hairline + `md` 圆角 + `lg` 内边距），在窗口中水平、垂直居中（内容在 `scrollable` 里拿到视口高度，因此内容更少时居中、更多时可滚动，不裁切）。
- 卡片头部是账户的**头像圆章**（`surface_variant` 圆 + hairline，字为账户名首字母，直径 = `base × 3`，行内头像 `base × 2`）与账户名（`title` 角色）。
- 口令字段、主按钮与次级控件占卡片宽度 **2/3**（`control_row`：两侧各 1 份 gutter、控件 4 份），次级控件是「切换用户」与「清空」。
- 账户列表是同一张卡片的第二幅画面：`heading` 标题 + 每账户一行（行内头像 + 名字，选中的一行填 `accent`，焦点环在被填的按钮上用 `on_accent`）+ 返回控件。
- 会话行、提示、状态与禁用原因仍然照旧只在投影里产生一次，界面不重复同一句话。

### 5. 读不到账户库不是致命错误

账户库读不到时界面**仍然打开**，并说明是「读不到列表」还是「列表里没有可登录的账户」（两个不同的 `Face`/原因 id）。一个能登录的用户不应该因为 greeter 列不出别人而被拒之门外。`--preview` 同样读账户列表：预览要展示本构建真实的样子。

### 6. 安全边界一个字都没变

账户列表**不**参与认证：名字仍经 `auth-v1` 交给 `inox-auth`，口令仍是掩码 + `Secret`（永不进 `iced::widget::text_input`），本 crate 仍不链接 `libpam`、不能启动会话、不能凭空产生成功答复。

## 失败、恢复与安全

| 情形 | 行为 |
|------|------|
| `getent` 不存在 / 失败 / 答空 | 回落 `/etc/passwd`；两者都失败 → 界面说「无法读取账户列表」，登录控件禁用且带原因 |
| 账户列表为空 | 界面说「本系统没有可登录的账户」；禁用的字段与按钮各有可读原因；键盘焦点落在「清空」上，不悬空 |
| 账户名不合契约（例如含空格） | 该账户仍列出；登录按钮禁用并给出理由（`greet-disabled-invalid`），不静默丢弃 |
| 账户名超长 | 模型按 `MAX_IDENTITY_BYTES` 截断并标记，界面说明 |
| 切换账户 | 清空口令；正在验证/启动会话时不允许切换（列表不可达） |
| 记录文件不可写 | `warn` 并继续；下次登录退回第一个账户 |
| 记录文件被篡改/为空 | 视为「没有记录」；任意的名字只在账户列表中匹配得到时才生效 |

权限与敏感内容：读取账户库与写状态文件都不需要特权；记录的内容只有账户名，不是凭据；日志只记数量与错误，不记口令（这一点由既有的 `Secret` 规则保证，本次未放宽）。

## 兼容、迁移与回退

- **没有协议与依赖变更**：`auth-v1`、`Cargo.toml`、分层方向都不变（`std::process::Command` 与 `std::fs` 是标准库；未引入 `thiserror`）。
- **crate 公开 API 变化**：`State::new` / `State::connected` / `State::with_boundary` 增加一个 `accounts::Accounts` 参数；`Message::Identity` 删除，新增 `Message::AccountChosen` / `Message::SwitchUser`；`Event::Username` → `Event::SelectAccount`；`view_model::Identity` 由 `AccountView` 取代。消费方只有本 crate 的二进制与测试（`tests/`、`src/main.rs`），已在同一次改动内更新。
- **新增目录约定**：`$XDG_STATE_HOME/inox/greet/last-user`。greeter 宿主（尚未实现）需要让 greeter 用户对 `$XDG_STATE_HOME/inox/greet` 可写；不可写时功能降级为「不记忆」。
- **文案变化**：删除 `greet-headline` / `greet-field-label` / `greet-placeholder` / `greet-invalid-user`，新增 `greet-switch-user` / `greet-back` / `greet-choose-account` / `greet-disabled-{no-accounts,accounts-unreadable,account-unselected}`；`en-US` 与 `zh-CN` 同步。
- **回退**：把账户列表换回输入框即可回到 ADR 0025 的形态，不需要数据迁移；记录文件留着而不被读取。回退会重新引入「键入的名字可能拼错」这条路径。

## 验收与证据

| 验收 ID / 场景 | 责任仓库 | E0–E4 等级 | 测试与失败注入 | 通过条件 / 证据位置 |
|----------------|----------|------------|----------------|----------------------|
| 账户过滤（uid 区间、shell、root 末尾、去重、坏行、GECOS 全名、首字母） | `inox-greet` | E1 | `src/accounts.rs` 单元测试 | 命中与不命中各有断言 |
| 默认选中（记录命中 / 记录失效 / 只有 root / 空表） | `inox-greet` | E1 | `src/accounts.rs`、`src/state.rs` 单元测试 | 四种情形各有断言 |
| 选择账户改变被认证的身份并清空口令 | `inox-greet` | E1 | `src/state.rs`、`src/view_model.rs`、`src/app.rs` 单元测试 | `Event::SelectAccount` 的行为与 `auth-v1` 请求里的 `username` |
| 键盘可达（Tab 走完两幅画面、Enter 选择、Escape 退出列表） | `inox-greet` | E1+E2 | `src/app.rs` 与 `tests/render.rs` | 焦点序列断言 + 真实控件树的像素定位点击 |
| 头像圆章与账户名 | `inox-greet` | E1 | `tests/render.rs`（`find_disc`） | 圆章宽高相等且等于令牌推导出的直径 |
| 无账户 / 账户库不可读 | `inox-greet` | E1 | `tests/render.rs` 场景 + 快照 | 两个不同的原因句；无实心圆章；无实心主按钮 |
| 200% 字号与双主题双语言 | `inox-greet` | E1+E2（Q02） | `tests/render.rs` 矩阵 + `tests/render.rs::the_longest_state_fits_the_screen_at_two_hundred_percent_text` | 最后一行文字与卡片描边都落在 1280×720 内；工件 `inox-greet-*.png` |
| 成功启动会话后记录账户 | `inox-greet` | E1 | `src/app.rs`（`tempfile`） | 只有 `Phase::SessionStarted` 之后文件才存在，内容为账户名 |
| 真机登录、greeter 宿主、真实账户列表 | 人类 + `inox-auth` | E4 | ADR 0029 §验收 | **未取得**，本 ADR 不声称 |

## 实施顺序与未决项

1. `accounts`（纯解析 + 系统读取 + 记录）→ `state`/`view_model` → `theme`/`view` → `app`/`main` → 目录与测试，按此顺序落地；每一步都在同一 PR 内。
2. 未决项：**greeter 宿主**（谁在裸 VT 上拉起 greeter、以哪个用户、`$XDG_STATE_HOME` 指向哪里）——ADR 0021 未决项，本 ADR 只提出对状态目录可写的要求；**真机登录验收**（E4）；**AT-SPI 语义导出**（iced 0.14 无桥，ADR 0018 已记录）。

## 后果

正面：

- 身份来自系统事实，界面不再有「名字打错」这条路径，`auth-v1` 收到的名字一定来自账户库。
- 界面与设计规范（`docs/quality/ui-guidelines.md`）逐条对齐：整屏单一 `title`、卡片分组用 `lg`、控件宽度按语义（2/3）、五种状态可辨、焦点环在填充控件上用 `on_accent`。
- 「最近使用」有了明确、可测试且不依赖系统二进制格式的来源。

负面：

- 本 crate 第一次读系统账户库与写状态文件：多了一处进程/文件系统访问面，也因此多了一条「读不到时怎么办」的分支（已按「不致命、说清楚」处理）。
- 不再能登录账户列表之外的账户（例如尚未被 NSS 解析出来的账号）；这是维护者明确接受的取舍。
- 界面上多了一幅画面（账户列表）与它的焦点序列，测试与文档都要跟着维护。

影响面：`inox-greet` 的 `accounts`/`state`/`view_model`/`view`/`theme`/`app`/`main`/`i18n`、crate 级 `AGENTS.md` 与 `README.md`、用户文档 `docs/book/src/usage/building.md` 与 `architecture/components.md`、验收规范 Q01 的注记。复审触发条件：引入 greeter 宿主或账户列表需要来自宿主/配置（而不是 NSS）时，重开本 ADR；若要把「最近使用」改为系统登录库，另写 ADR 说明解析与失败语义。
