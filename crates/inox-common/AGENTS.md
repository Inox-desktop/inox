# AGENTS.md — inox-common

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

共享类型、错误处理与日志工具。所属层级：**L1**。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- 本 crate **被工作区内的其他 crate 消费**：`inox-compositor`、`inox-config`、`inox-xwayland` 都依赖它。依赖是工作区内的 path 依赖（根级 §8），因此破坏性变更会在**同一次 CI** 中暴露给全部下游，不存在「下游仍锁在旧版本」的漂移窗口——这也意味着改动本 crate 的公开 API 时必须同时修好全部消费者。
- 新增模块只在**出现真实消费者**时引入（依赖排期见 [ADR 0004](../../docs/adr/0004-m0-skeleton-and-dependency-policy.md)）：不接受为「以后可能要用」而提前声明抽象或依赖。
- **`secret::Secret` 是全工作区唯一的凭据缓冲实现**（ADR 0029 §1.4）。它的三条不变量（清零、不可打印、唯一出口）是安全属性：**不得**给它加 `Clone` / `PartialEq` / `Deref` / `AsRef`，**不得**在 `Debug`/`Display` 里输出值或长度。`inox-greet`（登录）与 `inox-auth`（边界）已经消费它；`inox-lock` 目前仍有自己的一份（解锁路径未接），两份必须保持同一组规则，**解锁路径落地时把锁屏那份换成这里这一份**。

## 主要依赖

`tracing`, `tracing-subscriber`（`env-filter`）, `thiserror`, `zeroize`（`secret` 模块：口令缓冲在清零与 drop 时归零）

`i18n` 模块另外依赖 `fluent-bundle`、`fluent-syntax` 与 `unic-langid`（Apache-2.0 OR MIT / MIT OR
Apache-2.0）：`fluent-syntax` 直接使用是为了让目录一致性检查（id 与占位符）走解析后的 AST，
而不是靠正则扫描源文件。三者都是纯数据依赖，不引入 UI 工具包、事件循环或设备 API —— 这是本模块
能留在 L1 的前提（见 ADR 0018 与 ADR 0014 对 `inox-config` 的同类约束）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

本 crate 是工作区成员，命令应尽量用 `-p` 限定范围（CI 的测试矩阵跑的就是同一批命令）。在 `crates/<name>/` 目录内或仓库根执行均可。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-common --all-features` |
| 本 crate 构建 | `cargo build -p inox-common --release --all-features` |
| 本 crate 文档 | `cargo doc -p inox-common --no-deps --all-features` |
| 全工作区测试 | `cargo test --workspace --all-features` |
| 静态检查 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt --all` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`）。
- 测试代码不受“禁止 unwrap/expect/panic”约束。
- `logging` 模块的单元测试覆盖：显式指令优先、缺失/空白回退默认、首尾空白 trim、非法指令报错并携带原指令。
- `paths` 模块的单元测试覆盖：绝对路径直达、空值与相对路径被忽略并回退到默认、各基础目录的规范默认值、`$HOME` 缺失/空/相对时报错、`$XDG_RUNTIME_DIR` 无回退、`<base>/inox` 嵌套、非 XDG 变量被忽略且后出现的取值生效；数据目录回退链为 `$XDG_DATA_HOME` → `$XDG_DATA_DIRS` 的每一项（空项按规范跳过）→ 规范默认，壁纸目录是这些基目录下的 `inox/wallpapers`，并接受 `$INOX_WALLPAPER_DIR`（绝对路径）作为最先搜索的目录。
- **`XdgEnv::from_process` 不做单元测试**：edition 2024 下 `std::env::set_var` 是 `unsafe`，而本 crate `forbid(unsafe_code)`，测试中改写进程环境不可行；因此解析逻辑全部经 `XdgEnv::from_pairs` 注入快照来验证，`from_process` 只负责取值这一层薄封装。
- `cargo test --all-features` 必须保持全绿。

## 目录结构

```
crates/<name>/
  Cargo.toml    工作区继承：字段一律 *.workspace = true
  src/          源码
  tests/        集成测试（按需创建）
  README.md     本 crate 说明
  AGENTS.md     本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `common`。
