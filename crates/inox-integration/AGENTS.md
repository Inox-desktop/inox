# AGENTS.md — inox-integration

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

端到端集成测试与**分层方向检查**（验收项 B04）。所属层级：**测试**，按 ADR 0002 规则 7 不受依赖方向限制（但反过来：任何 crate 都不得依赖本 crate，该规则也由本 crate 的检查断言）。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- 与工作区内其他 Inox crate 的依赖使用 **path 依赖**（根级 §8）：在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明一次，本 crate 用 `{ workspace = true }` 引用。**新增依赖前先确认依赖方向合法**（根级 §2），否则 `cargo test -p inox-integration --test layering` 会失败。

## 主要依赖

`anyhow`, `tracing`, `tracing-subscriber`

当前**不依赖任何 Inox crate**：`layering.rs` 只读 `cargo metadata` 的输出，因此它无法因为某个 crate 编译失败而失去检查能力；M3 的 E3 用例以**进程方式**驱动工作区二进制（见「测试要求」），同样不引入任何 Inox crate 的 path 依赖，层级表与依赖方向检查不受影响。对各组件的 path 依赖仍计划在 **M6**（端到端兼容性验证）才引入。dev-dependencies：`serde_json`、`tempfile`、`rustix`（`event`）、`wayland-client`、`wayland-protocols`（`client`+`unstable`）、`x11rb`。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

本 crate 是工作区成员，命令应尽量用 `-p` 限定范围（CI 的测试矩阵跑的就是同一批命令）。在 `crates/<name>/` 目录内或仓库根执行均可。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-integration --all-features` |
| 本 crate 构建 | `cargo build -p inox-integration --release --all-features` |
| 本 crate 文档 | `cargo doc -p inox-integration --no-deps --all-features` |
| 全工作区测试 | `cargo test --workspace --all-features` |
| 静态检查 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt --all` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 测试代码不受“禁止 unwrap/expect/panic”约束。
- **`tests/layering.rs`（当前的主体）**：读取 `cargo metadata --no-deps` 与根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`，逐条断言 ADR 0002 的规则，并断言层级表恰好覆盖全部工作区成员。
  - 规则逻辑在 `rules::check` / `rules::check_coverage` 中，接受普通值参数，因此**可以对合成图做单元测试**：`tests` 模块用 15 个反例与正例固定每条规则的行为（反向边、同层边、合成器/Shell 双向互依、未登记的 L3→合成器边、依赖本 crate、叶子被依赖、幽灵成员、重复层级）。**新增规则必须同时新增正例与反例**，否则该规则按「未验证」处理。
  - 失败信息必须给出具体的边与被违反的规则条目，不能只报「检查失败」。
  - 检查不得依赖任何 Inox crate，否则某个 crate 编译失败就会连带让检查失去意义。
- **M3 端到端（E3）用例**（验收 X01–X03，ADR 0017 第 8 条）位于 `tests/m3_*.rs`。它们**不以 Cargo 依赖**方式链接任何 Inox crate：从测试可执行文件自身路径定位同一 `target/<profile>/` 下的 `inox-compositor`（须以 `--features inox-compositor/test-input` 构建）与 `inox-xwayland`，作为子进程驱动，进程之间只用 Wayland / X11 协议通信。因此本 crate 的依赖面与层级表不变，`layering.rs` 仍然有效。
  - 开关：`INOX_M3_E2E=1`；未设置时每个用例打印一行 `skipping` 并直接返回，常规分片保持绿色。缺少必需系统工具（`INOX_M3_E2E_REAL_APPS=1` 时的 `xeyes`/`xwininfo`）或缺少二进制时**失败**，不静默跳过。
  - 命令：先 `cargo build --workspace --features inox-compositor/test-input`，再 `INOX_M3_E2E=1 cargo test -p inox-integration --all-features`。
  - 共享装置在 `tests/support/`：`mod.rs`（私有 runtime 目录、空闲显示号、子进程与进程组回收、注入套接字客户端、帧转储解码、布局期望值）、`wayland_client.rs`（真实 `wayland-client` 客户端）、`x11_client.rs`（基于 `x11rb` 的真实 X11 客户端）。
  - 全部等待有超时；就绪以注入套接字的 `ok`、就绪文件、套接字或客户端自身观察到的事件为准，不用固定 sleep 判定。
  - 同一二进制内的用例各自开一个会话（私有 runtime/显示号/socket），并以进程内互斥串行执行，避免争抢同一个空闲显示号。
- M6 会在同一装置上扩展完整会话的兼容性用例（PRD §13）。

## 目录结构

```
crates/inox-integration/
  Cargo.toml    工作区继承：字段一律 *.workspace = true
  src/main.rs   可执行入口（当前为骨架：日志初始化）
  tests/layering.rs  分层方向检查（B04）：规则 + 合成图单元测试 + 工作区断言
  tests/m3_x11_window.rs     X01：真实 X11 客户端在 Inox 会话中的映射、平铺、resize、焦点与输入
  tests/m3_clipboard.rs      X02：X11↔Wayland 剪贴板逐字节一致、不支持类型、焦点授权、source 退出
  tests/m3_drag_and_drop.rs  X03：Wayland↔Wayland 拖放（跨 X11 边界那条腿见该文件模块文档的说明）
  tests/support/             上述用例的共享装置（见「测试要求」）
  README.md     本 crate 说明
  AGENTS.md     本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `integration`。
