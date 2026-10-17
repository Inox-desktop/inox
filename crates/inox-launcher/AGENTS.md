# AGENTS.md — inox-launcher

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

应用启动器：XDG 应用索引、搜索与排序、按条目规范启动并验证结果。所属层级：**L3**（见根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。`State::new` 会读文件系统，但失败都以 `ServiceState::Unavailable` + 可读原因呈现，不 panic。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- **`Exec` 只能展开成 argv。** 本 crate 不得出现返回命令行字符串的 API，也不得把搜索词传入任何进程启动路径：`exec::ExecContext` 没有查询字段，`exec::argv` 返回 `Vec<String>`。这是验收 D02「不把搜索词拼成 shell 命令」的结构性保证，破坏它需要重写而非放宽。
- **`update` 里不做 I/O。** 读索引与等待被启动进程的证据都在 `Task` 里（扫描走 `Task::perform`，探针走独立线程 + `Task::stream`）。`State::new` 的首次扫描发生在窗口存在之前，不占用交互循环。
- **用户可见文案只来自 Fluent**，`en-US` 与 `zh-CN` 同步；`tests/catalog.rs` 双向检查「代码提到的 id 都在目录里」与「目录里的 id 都被用到」，所以新增/删除文案必须两边一起改。
- **颜色、尺寸、间距、圆角只能来自 `inox_config::Theme`。** `view.rs` 与 `theme.rs` 里没有字面量颜色或像素尺寸；`placement.rs` 里的字面量只有无量纲比例与个数（表面尺寸是配置与令牌的乘积），`theme::text_size` 缩放的是 iced 自己的默认字号。
- 与工作区内其他 Inox crate 的依赖使用 **path 依赖**（根级 §8）。本 crate 只依赖 L1：`inox-common`（i18n 与 XDG 路径）、`inox-config`（`shell.toml` 的 `[launcher]` 与主题令牌）。**不得**新增 L2 或 L3 依赖（ADR 0013、ADR 0018）。
- 令牌映射、Fluent 装配与 layer 引导在每个 L3 组件里各有一份，这是 ADR 0018 明确接受的成本；**不要**为此新建共享 crate 或同层依赖，那需要先有 ADR。

## 主要依赖

- 上游 Inox crate：`inox-common`（`i18n::Localizer`/`Locale`、`paths::data_home`）、`inox-config`（`LauncherConfig`、`Theme` 令牌）。
- 第三方：`iced` 0.14（`tiny-skia`，另加**本 crate 自选的** `advanced` feature —— iced 0.14 只有经它才把 `Operation` 变成 `Task`，也就是唯一能把光标放进搜索框的办法）、`thiserror`、`tracing`、`tracing-subscriber`、`anyhow`（仅 `main`）。
- 测试：`proptest`、`tempfile`、`iced_core`、`iced_runtime`、`iced_tiny_skia`（无头渲染），`image`（仅在设置 `INOX_SCREENSHOT_DIR` 时写 PNG 工件）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。以下四条即本 crate 的自检。

| 目的 | 命令 |
|------|------|
| 格式化 | `cargo fmt -p inox-launcher` |
| 本 crate 测试 | `cargo test -p inox-launcher --all-features` |
| 静态检查 | `cargo clippy -p inox-launcher --all-targets --all-features -- -D warnings` |
| 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-launcher --no-deps --all-features` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`），**含拒绝与错误路径**，不只覆盖成功路径。
- 属性测试每项 ≥1000 样本（`ProptestConfig::with_cases(1000)`）：`Exec` 展开、搜索排序与代次、解析器健壮性。
- `tests/catalog.rs`：`compare_catalogs` 的 id 与占位符一致性，加上代码 ↔ 目录的双向检查。
- `tests/render.rs`：至少浅色、深色与一个空/错误态，在 1280×720 下经真实组件树渲染并断言像素。
- 测试代码不受「禁止 unwrap/expect/panic」约束。

## 目录结构

```
crates/inox-launcher/
  Cargo.toml    工作区继承：字段一律 *.workspace = true
  src/          源码（模块划分见 README）
  i18n/         en-US / zh-CN 的 Fluent 目录
  tests/        catalog.rs、render.rs、support/mod.rs
  README.md     本 crate 说明
  AGENTS.md     本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `launcher`。
