# inox-greet

L4 可选显示管理器。属于 Inox 桌面环境的 **L4** 层（[ADR 0021](../../docs/adr/0021-l4-session-services.md)）。认证本身不在本 crate：它经 `auth-v1` 私有契约（[ADR 0029](../../docs/adr/0029-inox-authentication-boundary.md)）交给 L2 的 [`inox-auth`](../inox-auth/README.md)。

**不带参数是真实登录路径。** 它要求认证边界套接字存在（`INOX_AUTH_SOCKET`，默认 `/run/inox/auth.sock`），否则在开窗之前以非零状态码退出并说明缺的是什么；找到边界后界面才可用，口令字段是掩码（`view_model::MASK_GLYPH` 按长度重复，按键以消息到达，**永不进 `text_input`**），凭据以 `inox_common::secret::Secret` 的形式在提交时移交给传输，认证成功后由边界以该账号身份启动会话，界面随即关闭自己的窗口。

**身份是选出来的，不是打出来的**（[ADR 0032](../../docs/adr/0032-greeter-account-selection-and-login-interface.md)）。界面不提供手工输入账户名的路径：`src/accounts.rs` 读系统账户库（`getent passwd`，失败或答空时回落 `/etc/passwd`），提供 `uid ∈ [1000, 65534)` 且 shell 不以 `nologin`/`false` 结尾的账户并额外把 `root` 排在最后；默认选中本组件记录的上次使用账户（`$XDG_STATE_HOME/inox/greet/last-user`，在会话真正启动后写入），没有记录时选第一个普通账户。界面是一张整屏居中的卡片：账户头像圆章 + 账户名 + 口令字段 + 控件 + 会话行；账户多于一个时有「切换用户」，它把同一张卡片换成账户列表（每个账户一行，选中的一行是填 `accent` 的那一行）。

**`--preview` 打开的是「本构建没有边界」那一幅。** 界面为 `state::Greeter` 的六个生命周期阶段各有一幅画面（不可用、就绪、验证中、会话启动中、失败、已启动）；预览显示的只有*不可用*那一幅：口令字段不可输入、登录按钮禁用、键盘只剩 Escape，横幅明确写着「仅供预览，不收集凭据」。其余阶段由测试与截图工件覆盖（`tests/render.rs`），没有任何开关能让运行的二进制假装自己有认证边界，真实构建也不会显示预览横幅。

## 状态

**已实现**（证据等级 E0/E1 + 离屏渲染；纯逻辑有单元测试）：

| 模块 | 内容 | 验收 |
|------|------|------|
| `accounts` | 系统账户库的读取与过滤（uid 区间、`nologin`/`false`、`root` 置末、同名去重、坏行跳过、GECOS 全名与首字母）、默认选中的四种情形、`last-user` 记录的读写；纯解析部分与系统读取分开，前者可单测 | ADR 0032 |
| `state` | 登录生命周期：六个阶段、代次关联（旧代次、重复、从未发出的答复一律拒绝）、30 秒答复期限、滚动 60 秒最多 3 次尝试、取消清理（口令清空而账户保留）、启动回执；账户选择（`Event::SelectAccount`，切换即清空口令）、账户名边界（`MAX_IDENTITY_BYTES`、与契约同一份账户名规则）；口令以 `Secret` 存在，提交时**移出**模型（不留副本），超长口令拒绝而非截断 | D05、Q01 |
| `view_model` | 状态到画面的纯投影：六个 `Phase` 各一个 `Face`，账户视图（名字、全名、首字母、是否选中）、账户列表只在有得选时打开、禁用原因按用户遇到的顺序给出（账户库读不到 / 没有可登录账户 / 没有选中 / 边界不可用 / 正在验证 / 已启动），`focus_order` 只含可操作的控件，`notice` 保证同一句原因只出现一次 | D05、Q01 |
| `boundary` | `auth-v1` 客户端：一条连接一条请求，答复映射为 `Report`（`Accepted`/`Rejected`/`Unavailable`/`Unreachable`/`SessionStarted`）；不链接 `libpam` | D05 |
| `view` | Iced 组件树：账户圆章、账户名、横幅、口令掩码字段、状态行、会话行、提示、控件与账户列表；每帧只消费投影与令牌，没有决策；控件占卡片 2/3，卡片在窗口里水平垂直居中 | Q02 |
| `theme` | 令牌 → `iced::Theme` 与样式闭包；焦点环用「与该控件填充色可辨」的那一个令牌画，因此焦点在一个填充按钮（登录按钮、选中的账户行）上也看得见；头像圆章直径是 `base` 的具名倍数 | Q01、Q02 |
| `app` | Elm 循环：`State`/`Message`/`update`，`Greeter::apply` 的全部事件、时钟由 `Ticked` 推进、焦点模型、账户列表的开关与选择、`key_binding` 纯函数、`take_outgoing` 是唯一的对外出口（`update` 每轮把它排空成传输任务）、会话启动后记录账户并关闭窗口 | D05、Q01 |
| `i18n` | `en-US` / `zh-CN` 目录，一组 `greet-` 前缀的 id，双向一致性由 `tests/catalog.rs` 断言 | Q03 |

**未实现 / 被阻塞**（不声称上表之外的任何等级）：

- **认证在别处。** 本 crate 没有 PAM、没有特权进程、不能启动会话、也无法凭空产生一个成功答复：一切都经 `auth-v1` 交给 `inox-auth`。
- **不是安全登录表面。** 界面（含真实路径）仍是一个普通 xdg-shell 窗口，可以截图、可以被别的窗口遮挡，也不提供任何输入隔离；把登录界面放到裸 VT 上属于 greeter 宿主，见下。
- **没有 greeter 宿主。** 真实显示管理器宿主（在会话启动前拥有显示器、在登录后交接给会话）仍未实现；ADR 0021 记录的同一条边界，ADR 0029 §6.6 重申。
- **真机登录未验收。** 边界能在 CI 里被完整驱动（真实套接字、脚本化认证器），真实 PAM 与真实账号登录属于 E4，必须由人完成（ADR 0029 §验收）。
- **没有语义树。** iced 0.14 没有 AT-SPI 桥，Q01 要求的 role/name/state/action 导出被该原型阻塞（与其余组件相同）。

## 目录结构

```
src/lib.rs            crate 文档、模块声明与再导出
src/accounts.rs       系统账户库的读取、过滤与默认选中；last-user 记录
src/state.rs          登录生命周期与代次关联（纯）
src/view_model.rs     状态 → 画面（纯投影）
src/app.rs            Elm：State / Message / update / subscription / key_binding
src/view.rs           Iced 组件树
src/boundary.rs       auth-v1 客户端（一条连接一条请求）
src/theme.rs          令牌 → iced::Theme 与样式闭包
src/i18n.rs           Fluent 目录装配
src/main.rs           日志、配置、目录、账户列表；默认路径要求认证边界，--preview 打开界面
i18n/{en-US,zh-CN}/messages.ftl
tests/{boundary.rs,catalog.rs,cli.rs,render.rs,support/mod.rs}
```

`last-user` 记录的位置是 `$XDG_STATE_HOME/inox/greet/last-user`（`$XDG_STATE_HOME` 缺席时回落 `$HOME/.local/state/...`）。写入失败只记 `warn`：下一次登录退回第一个账户，而不是让登录失败。**它记录的是本 greeter 发起过的登录，不是系统的登录历史**（不解析 `wtmp`/`lastlog`）。

## 测试

```sh
cargo test -p inox-greet --all-features
```

- 单元测试：每条规则各一条，**含拒绝路径**——过期、乱序、重复、来自从未发出的尝试的成功一律不能启动会话；取消绕不过滚动限额；超长或不合 `auth-v1` 语法的账户名被拒绝并说明；超长口令**拒绝**而不截断；口令只在焦点落在口令字段时进入模型，提交即移交且不留副本；限流文案里不出现任何计时；账户过滤的边界（uid 999/1000/65534、`nologin`/`false`/`root`/同名/坏行）、默认选中的四种情形与记录文件的读回各有断言。
- `tests/boundary.rs`：手写的假 `auth-v1` 服务端（真实套接字）驱动真实客户端，断言请求内容与四种答复的映射；边界不存在、版本不匹配、连接中途关闭都必须是 `Unreachable`。
- `tests/catalog.rs`：`en-US` 与 `zh-CN` 的 id 与占位符一致；代码里出现的每个 `greet-…` id 都在目录里，目录里每个 id 都被代码用到。
- `tests/cli.rs`：真实子进程没有边界时拒绝登录（非零退出、文案含 `authentication boundary` 与 `inox-auth`）；`--help` 可用；未知参数失败。
- `tests/render.rs`：1280×720 的双主题 × 双语言 × 200% 字号矩阵（含「最后一行文字与卡片描边都落在窗口内」的断言），以及不可用、空口令、账户名不可提交、口令过长、失败、限流、验证中、已启动、无账户、账户列表各场景的真实控件树离屏像素；口径圆章用「最长游程」（`find_disc`）而不是颜色包围盒来断言它是圆的；口令字段画出的是**计数而不是值**；交互测试用像素定位登录按钮与账户行，断言一次点击只产生一次提交、点哪一行就选哪个账户。

设置 `INOX_SCREENSHOT_DIR` 时，每个场景会额外落盘为 `inox-greet-<场景>.png`，供 CI 收集为工件（验收 Q02）。

## 开发约定

- 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径。**
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 用 `greet`。

## 许可证

LGPL-3.0，见仓库根的 [`LICENSE`](../../LICENSE)。
