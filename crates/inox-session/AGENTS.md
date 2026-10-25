# AGENTS.md — inox-session

本 crate 遵循根仓库的**全部硬性规则**（[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)），本文件只补充本 crate 特有内容，不放宽任何硬性规则。

## 职责

会话管理器。所属层级：**L4**。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- 与工作区内其他 Inox crate 的依赖使用 **path 依赖**（根级 §8）：在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明一次，本 crate 用 `{ workspace = true }` 引用。**新增依赖前先确认依赖方向合法**（根级 §2），否则 `cargo test -p inox-integration --test layering` 会失败。

- **锁屏令牌是能力，不是身份。** 会话为每个会话生成一次性令牌，传给合成器（`--lock-token`）并**只**放进可信锁屏客户端的环境（`INOX_LOCK_TOKEN`）。不得把它加进会话级环境：那会把取锁权限发给 shell 与它启动的一切。令牌不得进日志、不得落盘。`ServiceSpec::environment` 就是为此存在的逐组件环境。
- **锁屏客户端只在 `INOX_SESSION_TAKE_LOCK=1` 时启动。** `inox-lock --take-lock` 没有解锁路径，无条件启动会锁住一个没人要求锁住的会话。手动锁定与空闲触发的锁定仍未接线，这条开关是在那之前唯一的启动途径。
- **桌面首页是两个面板实例，不是一个。** 会话按 `--surface menubar` 与 `--surface dock` 各启动一个 `inox-panel`：一个 `wlr-layer-shell` 表面属于一条客户端连接，菜单栏（顶边）与 Dock（底边）因此必须分别是进程。两者都是必需服务，任一耗尽重启预算都会结束会话（与原先单个面板实例的严格程度一致）。`[panel] surface` 只是单独运行 `inox-panel` 时的默认值，**不**决定会话拉起哪几个表面；改这条前先确认 `component=panel-menubar` / `component=panel-dock` 的日志断言（`tests/headless.rs`）仍是准确的证据。

## 主要依赖

`anyhow`, `tracing`, `tracing-subscriber`；`calloop`、`rustix`、`wayland-client`、`inox-common`、`inox-config`、`inox-protocols`、`thiserror`。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

本 crate 是工作区成员，命令应尽量用 `-p` 限定范围（CI 的测试矩阵跑的就是同一批命令）。在 `crates/<name>/` 目录内或仓库根执行均可。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-session --all-features` |
| 本 crate 构建 | `cargo build -p inox-session --release --all-features` |
| 本 crate 文档 | `cargo doc -p inox-session --no-deps --all-features` |
| 全工作区测试 | `cargo test --workspace --all-features` |
| 静态检查 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt --all` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`）。
- 测试代码不受“禁止 unwrap/expect/panic”约束。
- 监督/策略/登录状态与边界失败必须有测试；证据以 CI 为准，限制见 L4 会话服务。
- 真实认证、锁屏与挂起接线必须先满足会话锁边界的人类安全审查前提。

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

Conventional Commits，scope 使用 `session`。
