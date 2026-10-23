# AGENTS.md — inox-protocols

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

标准协议绑定与自定义协议扩展。所属层级：**L2**。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- **MSRV 继承工作区的 Rust 1.89**（ADR 0027）：根 `Cargo.toml` 是下限的唯一声明，成员使用 `rust-version.workspace = true`；`rust-toolchain.toml` 的开发工具链继续跟随 stable。
- **协议以 XML 为唯一事实来源**：`protocol/` 下的协议 XML 是唯一事实来源；修改协议必须编辑 XML 并让绑定重新生成，**禁止直接修改生成的 Rust 代码**。`src/ipc.rs` 中的 `inox-ipc` 绑定由 `wayland-scanner` 在编译期从 `protocol/inox-ipc.xml` 生成。接口版本为 **2**：v2 新增请求 `claim_lock_role(string token)` 与事件 `session_lock_state(uint state)`（ADR 0022 决策 4），前者是可信锁屏角色的认领入口，后者是合成器对锁状态的权威广播；`ext-session-lock-v1`（staging）由 `standard::ext::session_lock` 再导出，本项目不复制上游 XML。
- **`protocol/auth-v1.md` 是认证私有契约的规范文本**（ADR 0029）：认证边界必须先于任何合成器存在，因此它不是 Wayland 协议，而是 Unix 套接字上的私有契约；按 ADR 0026 对配置契约的同一做法，**只放文档，不放代码**（类型与实现在 `inox-auth::wire`），因此本节不产生新的依赖边。
- **`inox-ipc` 为 unstable 私有协议**：在 Inox 1.0 之前接口允许破坏性变更。消费者必须通过 `version` 事件协商能力，不得假定固定版本。破坏性变更必须先写 ADR（根级 AGENTS.md §4 / §7.2），并在 PRD 协议矩阵同步更新。
- 本 crate **不依赖任何其他 Inox crate**：按 ADR 0002 规则 4 它是协议的唯一出口，只依赖 `wayland-rs`。若将来确需依赖，使用工作区 path 依赖（根级 §8）并先确认方向合法。
- 反向有依赖：`inox-compositor`（同为 L2）依赖本 crate 消费协议绑定。这是 **L2 → L2 的同层边**，与 ADR 0002 规则 1 冲突，已登记在根 `Cargo.toml` 的 `[workspace.metadata.inox.rules] same_layer_exceptions`；规则 1 的裁决尚未完成。

## 主要依赖

- 核心：`wayland-client`、`wayland-server`、`wayland-backend`。
- 标准协议：`wayland-protocols`（client / server / staging / unstable）、`wayland-protocols-wlr`（client / server）。
- 构建期：`wayland-scanner`（由 `protocol/inox-ipc.xml` 生成绑定）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

本 crate 是工作区成员，命令应尽量用 `-p` 限定范围（CI 的测试矩阵跑的就是同一批命令）。在 `crates/<name>/` 目录内或仓库根执行均可。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-protocols --all-features` |
| 本 crate 构建 | `cargo build -p inox-protocols --release --all-features` |
| 本 crate 文档 | `cargo doc -p inox-protocols --no-deps --all-features` |
| 全工作区测试 | `cargo test --workspace --all-features` |
| 静态检查 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt --all` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 单元测试（`#[cfg(test)]`）：`src/ipc.rs` 内的接口元数据测试，断言 `inox_ipc` 的接口名 / 版本、请求集、唯一析构请求、事件集与 `reload_result` 枚举取值稳定。
- 集成测试：`tests/ipc_roundtrip.rs` 通过真实 Unix socket pair 完成 client ↔ server 往返，断言 `version` 与 `config_reloaded` 事件；两者都必须保持通过。
- 新增或修改行为必须同时新增 / 更新对应测试。
- 测试代码不受“禁止 unwrap / expect / panic”约束。

## 目录结构

```
src/            源码（lib.rs 库入口；standard.rs 标准协议绑定再导出；ipc.rs 由 XML 生成的 inox-ipc 绑定）
protocol/       协议定义（源码级唯一事实来源：inox-ipc.xml 为 XML；auth-v1.md 为非 Wayland 私有契约的规范文本）
tests/          集成测试（如 ipc_roundtrip.rs）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `protocols`。
