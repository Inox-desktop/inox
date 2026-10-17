# AGENTS.md — inox-config

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

配置模型与 TOML 解析。所属层级：**L1**。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- 工作区内依赖（`inox-common`）是**path 依赖**：在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明，本 crate 用 `{ workspace = true }` 引用（根级 §8）。path 依赖必须带显式 `version`，否则会被 `deny.toml` 的 `[bans] wildcards = "deny"` 判为通配符依赖。注意 L1 内部存在 `inox-config → inox-common` 的同层顺序，该边登记在 `[workspace.metadata.inox.rules] same_layer_exceptions` 中。
- 所有配置结构体必须带 `#[serde(deny_unknown_fields)]`：拼写错误不得被静默忽略（根级 §13）。
- `config.toml` 与组件配置文件缺失一律回退默认值；文件存在但读取或解析失败必须报错，不得吞掉。
  主题文件（`themes/<name>/theme.toml`）缺失是**错误**（`ThemeNotFound`）：显式选择却读不到，
  不得静默换成内置主题。
- 只实现**当下有消费者**的 schema：`shell.toml` / `notifications.toml` 目前只登记路径，字段模型随各自里程碑引入，不得臆造。
- 越界值必须在**解析期报错**，不得静默 clamp（ADR 0009 决策 4）：平铺参数由 `LayoutConfig`
  校验，输出参数（尺寸 1..=16384、刷新率 1..=1000、`scale` 为正有限数）由 `OutputConfig`
  校验，范围与 `inox-compositor` 的 `MAX_DIMENSION` / `MAX_REFRESH_HZ` 一致；
  `LayoutConfig::sanitised()` 只作消费方对非配置来源的防御兜底。
- `[input]` 目前只校验类型，不校验范围：真实输入后端尚未落地，没有范围依据；范围校验随输入后端引入。
- `ConfigError::InvalidComponentPath` 的校验必须在 `InoxConfig::load` 内对 `[components]`
  的**三个**路径都执行：尚未实现 schema 的文件也要在启动期失败，而不是等组件启动才失败；
  空值、绝对路径与逃逸路径一律拒绝（ADR 0009 决策 11）。
- 主题文件的 `appearance` 只接受 `light` / `dark`（`ThemeAppearance`）；`system` 只属于
  `config.toml` 的选择层，`Appearance::System` 仅在选择层出现。

## 主要依赖

- 上游 Inox crate：`inox-common`（`v0.2.0`，使用其 `paths` 模块解析 XDG 目录）。
- 第三方：`serde`(derive)、`toml`、`serde_path_to_error`（提供解析错误的字段路径）、`thiserror`、`tracing`。
- 测试：`tempfile`（`[dev-dependencies]`，加载与热重载需要真实文件）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

本 crate 是工作区成员，命令应尽量用 `-p` 限定范围（CI 的测试矩阵跑的就是同一批命令）。在 `crates/<name>/` 目录内或仓库根执行均可。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-config --all-features` |
| 本 crate 构建 | `cargo build -p inox-config --release --all-features` |
| 本 crate 文档 | `cargo doc -p inox-config --no-deps --all-features` |
| 全工作区测试 | `cargo test --workspace --all-features` |
| 静态检查 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt --all` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`）；测试代码不受“禁止 unwrap/expect/panic”约束。
- 覆盖范围至少包含：
  - `root` / `compositor` 的**每一项默认值**，以及 `[environment]`、`[components]`、`[theme]`、`[[outputs]]` 的解析与部分字段回退；
  - 平铺参数三个字段各自的越界用例（报 `Parse`、字段路径正确、有行号，且值未落到配置里），以及边界内取值（10/90、1、0）被接受；
  - 输出参数越界用例（尺寸、刷新率、`scale = 0.0` / 负数）报 `Parse` 且字段路径形如 `outputs[0].width`，边界取值（1/16384、1/1000）被接受；
  - `[components]` 三个路径各自的逃逸用例（含 `shell` / `notifications`）在 `load` 时即报 `InvalidComponentPath`；
  - 主题文件 `appearance = "system"` 被拒绝，且错误信息点明只接受 `light` / `dark`；
  - 未知键报 `Parse` 并给出字段路径（含 `components.x`、`input.x` 这类点分路径）；
  - 十六进制颜色的两种合法形式与各类非法字面量（长度、缺 `#`、非十六进制、空、非 ASCII）；
  - 部分主题文件继承深色默认令牌、主题文件自带 `name`/`appearance` 时的覆盖；
  - 主题解析的每个分支：内置名优先、按 `appearance` 选内置、`system` 跟随会话、会话也报 `system` 时回退 `Dark`；
  - `ConfigPaths::file` 拒绝绝对路径与逃逸路径，`theme_dir`/`theme_file` 拒绝目录穿越；
  - `InoxConfig::load`：配置目录不存在（全默认）、组件文件缺失（默认）、组件文件损坏（`Parse` 带文件+行+字段）、磁盘上的自定义主题、主题缺失（`ThemeNotFound`）；
  - `ConfigStore`：首次加载、重载读到改动、重载失败（组件文件损坏、**主题文件损坏**）后保留原 `Arc` 内容并返回 `Err`、失败后仍能成功重载；
  - 覆盖率补齐：TOML 语法错误（无字段路径但有位置）、`config.toml` 自身损坏、`[components]` 只写一个键时其余取默认名、`[[outputs]]` 只写 `scale` 时其余取默认值；
  - 错误位置的列按**字符**计：在出错点之前放多字节字符，断言列号不被字节数带偏。
- 测试中断言的行号/列号必须由所写文件内容推导，不得硬编码猜测。
- **`ConfigPaths::discover`（以及 `XdgEnv::from_process`）不做单元测试**：edition 2024 下 `std::env::set_var` 是 `unsafe`，本 crate `forbid(unsafe_code)`，测试无法改写进程环境；解析逻辑全部经 `ConfigPaths::from_xdg` 注入快照验证。
- `cargo test --all-features` 必须保持全绿。

## 目录结构

```
src/lib.rs          crate 入口与公开 API 再导出
src/error.rs        ConfigError、读文件/解析辅助函数
src/paths.rs        ConfigPaths：PRD §9.1 的目录布局
src/root.rs         config.toml 模型
src/theme.rs        设计令牌、内置主题与主题解析
src/compositor.rs   compositor.toml 模型
src/store.rs        InoxConfig::load 与 ConfigStore 热重载
tests/              集成测试（按需创建）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `config`。
