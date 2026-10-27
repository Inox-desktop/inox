# inox-protocols

标准协议绑定与自定义协议扩展。属于 Inox 桌面环境的 **L2** 层。

## 状态

**M1**：本 crate 已从骨架进入可用实现，当前提供两部分内容：

1. **标准协议绑定再导出**（`src/standard.rs`）：按 PRD 支持级别分组，统一再导出核心 `wl_*`、`wayland-protocols` 扩展与 `wlr` 扩展的 client / server 绑定，使上游版本在单一位置锁定。
2. **Inox 私有控制协议 `inox-ipc` 绑定**（`src/ipc.rs`）：在编译期由 `wayland-scanner` 从协议 XML 生成 client 与 server 两侧的 Rust 绑定。
3. **私有会话控制契约 `session-control-v1`**（`src/control.rs`）：不是 Wayland 协议，而是会话监督进程与它自己的桌面之间的一行文本（`lock`），规范文本在 [`protocol/session-control-v1.md`](./protocol/session-control-v1.md)。类型放在本 crate 是因为客户端是 L3（`inox-lock`）、服务端是 L4（`inox-session`），共享类型只能落在两者都合法的 L2 边界上。

协议定义以 [`protocol/inox-ipc.xml`](./protocol/inox-ipc.xml) 为**唯一事实来源（single source of truth）**：需要修改协议时先改 XML，再重新生成绑定，**不要直接改生成的 Rust 代码**。

会话 D-Bus 上的配置应用接口没有 XML：它们以 `protocol/configuration-v1.md`
（`org.inox.Configuration1.Reload`）与 `protocol/configuration-v2.md`
（`org.inox.Configuration2` 的 `Trial`/`Confirm`/`Rollback`）为规范，
由各消费方二进制内用 `zbus` 声明。

输入法绑定通过 `standard::wp::{text_input, input_method, virtual_keyboard}` 提供。
应用端 v3 与输入法端 v2 的管理器在合成器中均广告接口 v1；输入法与虚拟键盘
只对专用受信连接可见，生命周期与权限规则见[输入法使用说明](../../docs/book/src/usage/input-methods.md)。

### 版本策略

`inox-ipc` 是 Inox 私有协议，且在 **1.0 之前为 unstable**：接口允许破坏性变更。消费者必须通过 `version` 事件协商能力，不得假定固定版本。破坏性变更需先写 ADR（根级 AGENTS.md §4 / §7.2），并在 PRD 协议矩阵中同步更新。

### MSRV

最低支持 Rust **1.86**（`wayland-protocols` 0.32 要求 rustc ≥ 1.86）。版本在 `Cargo.toml` 的 `rust-version` 与 `rust-toolchain.toml` 中锁定。

## 运行 / 测试

> 本地命令仅供参考，**CI 是唯一验收依据**。

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release --all-features
cargo doc --no-deps --all-features
cargo deny check
```

## 开发约定

- 本项目遵循[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，不在本地进行编译验证。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)。

## 许可证

LGPL-3.0，见仓库根的 [`LICENSE`](../../LICENSE)。
