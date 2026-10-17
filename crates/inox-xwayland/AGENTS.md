# AGENTS.md — inox-xwayland

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

XWayland 集成服务：**监管外部 `xwayland-satellite` 进程**，把 rootless XWayland 接入 Inox 会话。所属层级：**L2**（不依赖其他 L2/L3 crate，只依赖 L1 的 `inox-common`）。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`），本 crate **无任何例外**（源码中无 `#[allow(unsafe_code)]`，故不进入根级 §3.2 的例外名单）。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`（默认级别 `info`）；`--help` / `--version` 与用法错误走显式 stdout / stderr 写入，不使用 `println!` / `eprintln!`。
- **禁止链接 `xwayland-satellite`**：它是运行期系统依赖（MPL-2.0、应用形态、不在 crates.io 上），只能通过 `std::process::Command` 拉起；**不得**写入 `[dependencies]`、不得在构建期获取。命令行约定：显示号 `:N` 在**第一位**，其余均为 `--satellite-arg` 透传。
- 事件循环必须使用 `calloop`（根级 §4），信号与定时器都注册为事件源；禁止自建线程轮询。
- 就绪判定为「连接 `<x11-socket-dir>/X<N>` 成功」，探测失败一律视为未就绪，不得 panic。
- 退出策略：`SIGINT` / `SIGTERM` 归零退出；子进程自行退出则沿用其退出码（被信号杀死视为失败）。
- 面向用户的字符串目前只有 CLI 帮助与错误信息；新增用户可见文案必须走 Fluent（根级 §7.3），并在本文件登记。
- MSRV：**1.86**（edition 2024）。
- 工作区内依赖：`inox-common` 是**path 依赖**（在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明，本 crate 用 `{ workspace = true }` 引用），不再是 git 依赖。本 crate 属 L2，只能依赖 L1。

## 主要依赖

- 运行期：`anyhow`、`calloop`（`signals`）、`rustix`（`process`，用于向子进程发送 `SIGTERM`）、`thiserror`、`tracing`
- 工作区内：`inox-common`（path 依赖，用于统一的日志初始化）
- 开发：`tempfile`
- 运行期外部依赖（不参与构建）：`xwayland-satellite`（≥ v0.8）、`Xwayland`

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

本 crate 是工作区成员，命令应尽量用 `-p` 限定范围（CI 的测试矩阵跑的就是同一批命令）。在 `crates/<name>/` 目录内或仓库根执行均可。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-xwayland --all-features` |
| 本 crate 构建 | `cargo build -p inox-xwayland --release --all-features` |
| 本 crate 文档 | `cargo doc -p inox-xwayland --no-deps --all-features` |
| 全工作区测试 | `cargo test --workspace --all-features` |
| 静态检查 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt --all` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |
| 端到端（需真实 XWayland） | `INOX_XWAYLAND_E2E=1 cargo test --all-features --test headless_x11_mapping -- --nocapture` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`）；测试代码不受「禁止 unwrap/expect/panic」约束。
- 服务状态机必须通过**注入的假实现**（`ChildSupervisor` / `ReadinessProbe`）测试，不得为测试而 mock 掉被测逻辑本身。
- 进程级测试（`tests/lifecycle.rs`）用 shell 替身驱动真实二进制，必须自备私有临时目录（含 `--x11-socket-dir`），并为每个等待设置超时，避免 CI 挂死；测试结束必须清理替身进程。
- 真实 headless 端到端测试（`tests/headless_x11_mapping.rs`）以 `INOX_XWAYLAND_E2E` 为开关：未设置时打印 `skipping` 并直接返回（保证常规 CI 全绿）；设置后若缺少必需工具则**失败**并给出清晰提示。
- 新增或修改行为必须同时新增/更新对应测试。

## 目录结构

```
src/
  lib.rs        库入口、crate 文档与再导出
  main.rs       可执行入口（CLI 输出、日志初始化、退出码策略）
  config.rs     命令行解析与配置数据（无进程/X11 副作用）
  x11.rs        显示号与其 UNIX 套接字、就绪探测
  process.rs    xwayland-satellite 的生成与终止（ChildSupervisor）
  service.rs    监管状态机（Phase/Progress/ServiceError）
  runtime.rs    calloop 事件循环：信号源 + 轮询定时器
tests/
  lifecycle.rs            进程级生命周期测试（shell 替身卫星）
  headless_x11_mapping.rs 真实 headless 端到端测试（INOX_XWAYLAND_E2E 开关）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `xwayland`。
