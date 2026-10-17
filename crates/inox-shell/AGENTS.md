# AGENTS.md — inox-shell

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

Shell 主框架：入口之间的导航规则（ADR 0013 的「桌面交互协调、跨入口的导航规则」）与各入口共同遵循的跨进程服务状态契约。所属层级：**L3**。本 crate **不绘制任何内容**，因此没有 Iced 依赖、没有 view、也没有截图测试。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`；`--help` 的用法文本也走 `tracing::info!`，不打印到标准输出。
- **不得引入 Iced、事件循环或任何绘制依赖**：本 crate 的职责是规则与契约，不是界面。界面属于各入口组件。
- **不得新增到其他 L3 crate 或 `inox-compositor` 的 Cargo 依赖**（ADR 0002 规则 1/3、ADR 0013）：入口之间只通过协议协作，本条由 `cargo test -p inox-integration --test layering`（B04）机械断言。
- **不实现第三方插件加载**：ADR 0013 要求通用插件平台另写 ADR。`src/registry.rs` 只负责内部入口注册表，新增动态加载、路径搜索或第三方清单前必须先有 ADR。
- **用户可见文案一律走 Fluent**：`src/` 里不得出现硬编码句子，只能出现 `crate::i18n::id` 中的消息 ID 与占位符（`src/diagnostic.rs`）。新增消息必须同时登记到 `id::ALL` 与两个目录；`tests/catalog.rs` 会在任一侧缺失时失败。
- **返回目标只能是「服务端窗口 ID + 会话代次」**：禁止改用 PID、标题或列表索引来表示窗口（ADR 0013 决策 1）。

## 主要依赖

- 上游 Inox crate：`inox-common`（Fluent 目录装配与语言协商）、`inox-config`（`InoxConfig`、`ConfigPaths`、`ComponentFiles`）。
- 第三方：`anyhow`（二进制入口）、`thiserror`（库错误类型）、`tracing`、`tracing-subscriber`。
- 测试：`proptest`（`[dev-dependencies]`）。
- **不依赖 `iced` / `iced_core` / `iced_runtime` / `iced_tiny_skia`**：本 crate 没有视图，故没有 `tests/render.rs` 与 `tests/support/mod.rs`，`image` 也不需要。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-shell --all-features` |
| 本 crate 构建 | `cargo build -p inox-shell --release --all-features` |
| 本 crate 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-shell --no-deps --all-features` |
| 静态检查 | `cargo clippy -p inox-shell --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt -p inox-shell` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`），**拒绝与失败路径与成功路径同等重要**：未知入口名、重复登记、逃出配置目录的路径、空栈上按 `Esc`、目标已销毁、代次已变化，都要有用例。
- 属性测试（`proptest!` + `ProptestConfig::with_cases(1000)`）覆盖：导航状态机（用一份独立写的模型实现同批操作并逐条比对，含「每层唯一」「深度不越界」「已销毁目标绝不作为返回目标」）、重试有界性、请求代次、快照单调性。
- `tests/catalog.rs`：`compare_catalogs` 断言 `en-US` 与 `zh-CN` 的 ID 与占位符一致；断言目录恰好覆盖 `i18n::id::ALL`（既不缺也不多）；断言每个 ID 在两种语言下都渲染成非 ID 文本，且本 crate 能产生的每个 `Diagnostic` 不残留占位符。
- `tests/navigation.rs`：把 D01 的场景按验收条文写成用例（`Esc` 回到原窗口、普通展示不夺焦点、窗口销毁与会话重建后的回退）。
- `tests/registry.rs`：注册表的解析结果与全部拒绝用例（C01）。
- `tests/cli.rs`：以子进程运行协调器二进制，断言 `--help` 与参数拒绝的文案走 **stderr**、stdout 保持为空（本文件第 13 行与根 `AGENTS.md` §3.5 的「不打印到标准输出」）。
- **没有渲染测试**：本 crate 无视图，故不创建 `tests/render.rs` / `tests/support/mod.rs`，也不写 PNG。需要视觉证据的验收（Q02）属于各入口组件。

## 目录结构

```
src/lib.rs          crate 入口、模块声明与公开 API 再导出
src/navigation.rs   入口栈与返回目标（纯状态机，含单元与属性测试）
src/contract.rs     快照/命令结果/请求代次/服务状态/重试（纯逻辑，含单元与属性测试）
src/registry.rs     内部入口注册表与构造期校验
src/diagnostic.rs   只携带 Fluent ID 与占位符的可读原因
src/i18n.rs         目录装载与 `id` 模块（全部消息 ID 登记在 `id::ALL`）
src/main.rs         协调器二进制：日志、配置、文案、注册表、`--help`
i18n/en-US/shell.ftl, i18n/zh-CN/shell.ftl
tests/catalog.rs   tests/navigation.rs   tests/registry.rs
tests/cli.rs       （以子进程方式运行二进制，断言两条流）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `shell`。
