# inox-greet

L4 可选显示管理器。属于 Inox 桌面环境的 **L4** 层（[ADR 0021](../../docs/adr/0021-l4-session-services.md)）。

`inox-greet --preview` 打开登录界面。界面为 `state::Greeter` 的六个生命周期阶段各有一幅画面（不可用、就绪、验证中、会话启动中、失败、已启动），身份字段是真正的文本框，Tab/Shift+Tab/Enter/Escape 各有用处，每个禁用控件都带可读原因。界面全部走 Fluent 双语、设计令牌与字体缩放，**不收集凭据、不启动会话**。

**但 `--preview` 只会显示其中一幅。** 认证传输的可用性在构造时固定为「没有」（这是本构建的事实），所以运行的二进制显示的是*不可用*那一幅：身份字段不可输入、登录按钮禁用、键盘只剩 Escape。其余五个阶段由测试与截图工件覆盖（`tests/render.rs`），没有任何开关能让运行的二进制假装自己有认证边界。

**默认拒绝是这本 crate 的第一条规则。** 不带 `--preview` 时 `inox-greet` 以非零状态码退出并说明原因：ADR 0015 要求的认证服务、helper 权限/许可证、可信通道及凭据清理尚待安全审查，不能用模型测试替代系统认证。预览是一个**普通窗口**，不是安全登录表面；界面上写着这一点。

## 状态

**已实现**（证据等级 E0/E1 + 离屏渲染；纯逻辑有单元测试）：

| 模块 | 内容 | 验收 |
|------|------|------|
| `state` | 登录生命周期：六个阶段、代次关联（旧代次、重复、从未发出的答复一律拒绝）、30 秒答复期限、滚动 60 秒最多 3 次尝试、取消清理、启动回执；用户名边界（`MAX_IDENTITY_BYTES`、保守的可移植账户名规则）；**不存储任何令牌或口令** | D05、Q01 |
| `view_model` | 状态到画面的纯投影：六个 `Phase` 各一个 `Face`，禁用原因按用户遇到的顺序给出，`focus_order` 只含可操作的控件，`notice` 保证同一句原因只出现一次 | D05、Q01 |
| `view` | Iced 组件树：标题、横幅、状态行、身份字段、会话行、提示、两个控件；每帧只消费投影与令牌，没有决策 | Q02 |
| `theme` | 令牌 → `iced::Theme` 与样式闭包；焦点环用「与该控件填充色可辨」的那一个令牌画，因此焦点在一个填充按钮上也看得见 | Q01、Q02 |
| `app` | Elm 循环：`State`/`Message`/`update`，`Greeter::apply` 的全部六个事件、时钟由 `Ticked` 推进、焦点模型、`key_binding` 纯函数、`take_outgoing` 是唯一的对外出口 | D05、Q01 |
| `i18n` | `en-US` / `zh-CN` 目录，一组 `greet-` 前缀的 id，双向一致性由 `tests/catalog.rs` 断言 | Q03 |

**未实现 / 被阻塞**（不声称上表之外的任何等级）：

- **没有认证。** 没有 PAM、没有 greetd、没有 helper、没有特权进程、没有任何口令校验。`state::AuthReply` 与 `state::Command` 是答复与请求将到达的形状；谁有权产生它们，是 ADR 0015 点名的那项人类评审。`app::Message` 里的两个答复消息因此只由测试产生。
- **不是安全登录表面。** 预览是一个普通窗口，可以截图、可以被别的窗口遮挡，也不提供任何输入隔离。
- **没有 greeter 宿主。** 真实显示管理器宿主（在会话启动前拥有显示器、在登录后交接给会话）仍未实现；ADR 0021 记录的同一条边界。
- **没有语义树。** iced 0.14 没有 AT-SPI 桥，Q01 要求的 role/name/state/action 导出被该原型阻塞（与其余组件相同）。

## 目录结构

```
src/lib.rs            crate 文档、模块声明与再导出
src/state.rs          登录生命周期与代次关联（纯）
src/view_model.rs     状态 → 画面（纯投影）
src/app.rs            Elm：State / Message / update / subscription / key_binding
src/view.rs           Iced 组件树
src/theme.rs          令牌 → iced::Theme 与样式闭包
src/i18n.rs           Fluent 目录装配
src/main.rs           日志、配置、目录；默认拒绝登录，--preview 打开界面
i18n/{en-US,zh-CN}/messages.ftl
tests/{catalog.rs,cli.rs,render.rs,support/mod.rs}
```

## 测试

```sh
cargo test -p inox-greet --all-features
```

- 单元测试：每条规则各一条，**含拒绝路径**——过期、乱序、重复、来自从未发出的尝试的成功一律不能启动会话；取消绕不过滚动限额；超长身份被截断且被界面说明；限流文案里不出现任何计时。
- `tests/catalog.rs`：`en-US` 与 `zh-CN` 的 id 与占位符一致；代码里出现的每个 `greet-…` id 都在目录里，目录里每个 id 都被代码用到。
- `tests/cli.rs`：真实子进程不带参数时拒绝登录（非零退出、文案含 ADR 0015）；`--help` 可用；未知参数失败。
- `tests/render.rs`：1280×720 的双主题 × 双语言 × 200% 字号矩阵，以及不可用、空字段、非法用户名、失败、限流、验证中、已启动各场景的真实控件树离屏像素；一个交互测试用像素定位登录按钮并断言一次点击只产生一次提交。

设置 `INOX_SCREENSHOT_DIR` 时，每个场景会额外落盘为 `inox-greet-<场景>.png`，供 CI 收集为工件（验收 Q02）。

## 开发约定

- 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径。**
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 用 `greet`。

## 许可证

BSD 3-Clause，见仓库根的 [`LICENSE`](../../LICENSE)。
