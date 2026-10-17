# AGENTS.md — inox-greet

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

显示管理器。所属层级：**L4**（见根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

它拥有的是**登录生命周期与登录界面**：六个阶段、代次关联与期限、滚动尝试限额、界面的投影与控件树、焦点模型，以及 `--preview` 这条非认证路径。它**不**拥有认证，也**不**拥有 greeter 宿主。

**`--preview` 只显示本构建真实所处的那个阶段**（没有认证边界，因此是不可用、不可输入的一幅）。其余五个阶段只有测试覆盖；不得添加任何让运行的二进制假装有认证边界的开关。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- 与工作区内其他 Inox crate 的依赖使用 **path 依赖**（根级 §8）：在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明一次，本 crate 用 `{ workspace = true }` 引用。**新增依赖前先确认依赖方向合法**（根级 §2），否则 `cargo test -p inox-integration --test layering` 会失败。
- **登录界面可以重复锁屏的样式，但不能重复它的机制。** `view_model`/`view`/`theme` 按 ADR 0018 的模式各有一份（同形不同源，这是 ADR 0018 明确接受的成本）；但**不得**依赖 `inox-lock`（同层横向依赖）或把它的状态机搬过来。两者的边界不同：锁屏的锁状态归合成器，greeter 的登录状态归这里。
- **凭据永远不进本 crate。** 身份字段是 `iced::widget::text_input`，因为它装的是**用户名**——公开数据，想要光标、选区、输入法与粘贴。口令不存在：`state::Greeter` 没有存它的字段，界面上也没有放它的控件。任何时候要给这个界面加口令输入，先读 ADR 0015 的人类评审要求。
- **焦点模型归本 crate，不由 iced 决定。** iced 0.14 的 `button` 没有 `id`、不可聚焦，`operation` 也没有 `unfocus`；因此 `view_model` 只把可操作的控件放进 `focus_order`，`app` 的焦点在其中移动，`view` 只在焦点落在身份字段上时才把它画成可编辑框。**不得**为了方便把文本框永久留在树里：它会在焦点环说「键盘在按钮上」时继续吃掉按键，让「可见的焦点」与「键盘作用的控件」变成两件事。
- **不能按 Space 触发操作。** 它是用户名里可能出现的字符；`app::key_binding` 只认 Tab/Shift+Tab/Enter/Escape，新增绑定前先想清楚它会不会吃掉一个字符。
- **默认路径必须继续拒绝。** 不带 `--preview` 时以非零码退出，且文案里保留 ADR 0015 的说明——`tests/cli.rs` 依赖这两个事实。不得让界面在没有认证边界时表现出可以登录的样子：屏幕上写着这是预览、不收集凭据、不启动会话。
- **用户可见文案只来自 Fluent**，`en-US` 与 `zh-CN` 同步；id 统一 `greet-` 前缀，`tests/catalog.rs` 双向检查「代码提到的 id 都在目录里」与「目录里的 id 都被用到」。
- **颜色、尺寸、间距、圆角只来自 `inox_config::Theme`。** `view.rs` 与 `theme.rs` 里没有字面量颜色或像素尺寸；字号由 `typography.scale` 乘 iced 自己的默认字号推出。

## 主要依赖

`anyhow`, `tracing`, `tracing-subscriber`；`iced` 0.14（`tiny-skia` 渲染器）、`inox-common`（i18n）、`inox-config`（设计令牌）。

测试：`iced_core` / `iced_runtime` / `iced_tiny_skia`（离屏渲染真实控件树）、`image`（仅在设置 `INOX_SCREENSHOT_DIR` 时写 PNG 工件）、`tempfile`。本 crate 没有协议腿，因此没有向下到 L2 的边。

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

- 纯逻辑必须有单元测试（`#[cfg(test)]`），**含拒绝与错误路径**：过期、乱序、重复与来自从未发出的尝试的答复都不能启动会话；取消绕不过滚动限额；超长身份被截断且被界面说明。
- 界面必须有 `view_model` 层的单测（每个 `Phase` 的画面、禁用必有原因、焦点只落在可操作控件上），不能只靠像素测试。
- `tests/render.rs` 必须覆盖双主题 × 双语言 × 200% 字号与全部关键状态；渲染尺寸是验收 Q02 要求的 **1280×720**。
- `tests/catalog.rs` 做双向检查（代码提到的 id 都在目录里、目录里的 id 都被用到），因此新增/删除文案必须两边一起改。
- `tests/cli.rs` 覆盖默认拒绝路径与 `--help`。
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
