# AGENTS.md — inox-greet

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

显示管理器（登录表面）。所属层级：**L4**。ADR 0036 M03 的首个完整 QML 迁移样本。

## crate 特有约束

### `unsafe_code` 例外（ADR 0035）

根级 §3.2 默认 `unsafe_code = "forbid"`。但 `#[cxx_qt::bridge]` 宏生成的 FFI 胶水（`unsafe extern "C++"`、`unsafe fn`）**不自带 `#[allow]`**，在 `forbid` 下无法编译。因此本 crate 在 `Cargo.toml` 单独声明：

```toml
[lints.rust]
unsafe_code = "allow"
missing_docs = "warn"
```

**边界**：只放行宏**生成**的 `unsafe`。Inox 手写的代码不出现 `unsafe` 关键字——`src/**` 中唯一的 `unsafe` 是 `src/cxxqt_object.rs` 里 `#[cxx_qt::bridge]` 块内的 `unsafe extern "C++"` 声明（cxx-qt 惯例，由宏处理），其余文件零 `unsafe`。FFI 局限在这一个 bridge 模块；认证/会话决策仍在安全 Rust 侧（`src/state.rs`、`src/app.rs`）。

### 安全边界（ADR 0015 / ADR 0036）

- 默认 `inox-greet` **必须**可读报错并非零退出，`--preview` 才进入预览。
- `Greeter` 状态机与 `Command` 契约**语义不得改动**；QML 只做渲染与事件上报，不暴露凭据或敏感状态。bridge 的 invokable 只能上报「用户名被编辑 / 清空 / 激活 / 树已就绪」，没有任何可认证入口。
- `--self-check` 是诊断开关（供 headless 测试），**不得**改变默认拒绝语义，也不得成为第二种登录模式。

### 其他

- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`（测试除外）。
- **QML 内禁止硬编码颜色 / 尺寸 / 用户可见文案**：全部经 `src/view.rs` 从 `inox_config::Theme` 与 Fluent 目录投影。
- **bin-only**：不得新增 `src/lib.rs` 或 `tests/` 集成测试。cxx-qt 0.10 把 bridge 生成的 C++ 归档链接进包内每个 target，库/集成测试 target 会因 `undefined reference to 'cxxbridge1$…'` 链接失败（KDAB/cxx-qt #770）。测试一律放 `src/**` 的 `#[cfg(test)]`，需要进程级证据时以子进程重跑自身（`std::process::Command` 设置 `QT_QPA_PLATFORM`，不用 `std::env::set_var`——edition 2024 下它是 `unsafe`）。
- 与工作区内其他 Inox crate 的依赖使用 **path 依赖**（根级 §8）：在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明一次，本 crate 用 `{ workspace = true }` 引用。**新增依赖前先确认依赖方向合法**（根级 §2），否则 `cargo test -p inox-integration --test layering` 会失败。

## 主要依赖

- 工作区：`inox-common`（i18n / logging）、`inox-config`（设计令牌与配置）。
- 第三方：`cxx`、`cxx-qt`、`cxx-qt-lib`（Rust→QML 绑定，ADR 0035）、`cxx-qt-build`（build-dependency）、`anyhow`、`tracing`。
- 系统库：Qt 6（Core/Gui/Qml/Quick/QuickControls2），经 `qmake`/`pkg-config` 定位；不得引入 GPL-only Qt 模块（ADR 0033）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

| 目的 | 命令 |
|------|------|
| 本 crate 测试（含 offscreen QML 加载） | `cargo test -p inox-greet --all-features` |
| 无头预览自检 | `QT_QPA_PLATFORM=offscreen cargo run -p inox-greet -- --preview --self-check` |
| 本 crate 构建 | `cargo build -p inox-greet --release --all-features` |
| 本 crate 文档 | `cargo doc -p inox-greet --no-deps --all-features` |
| 全工作区测试 | `cargo test --workspace --all-features` |
| 静态检查 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt --all` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`）：`src/state.rs`（状态机）、`src/app.rs`（动作→事件）、`src/view.rs`（令牌/文案投影）、`src/headless_tests.rs`（bridge 投影、进程级拒绝、offscreen QML 加载）。
- 测试代码不受「禁止 unwrap/expect/panic」约束。
- **QML 加载证据必须是真证据**：`QQmlApplicationEngine::load` 是异步的，只 `load` 不泵事件循环会假通过。现有测试通过 `objectCreated` 信号 + `process_events()` + QML 树回报 `reportReady` 三重确认，并已用「故意改坏 URL / 移除 reportReady」验证会失败。
- 无像素断言：预览是普通窗口，不是安全登录表面，offscreen 平台无合成器可渲染；waydriver 截图属后续 D02/D04 基座。
- 真实认证、锁屏与挂起接线必须先满足 ADR 0015 的人类安全审查前提。

## 目录结构

```
crates/inox-greet/
  Cargo.toml        # *.workspace = true；[lints.rust] unsafe_code = "allow"（ADR 0035 例外）
  build.rs          # CxxQtBuilder + QmlModule("com.inox.greet.qml") + .files(["src/cxxqt_object.rs"])
  qml/main.qml      # 只做渲染，import com.inox.greet.qml；带 Accessible 角色
  src/main.rs       # bin 入口（无 lib 目标）：参数、配置、Qt 启动、--self-check
  src/view.rs       # 令牌 + Fluent → QML 投影（Qt-free，可单测）
  src/app.rs        # QML 动作 → Greeter 事件
  src/cxxqt_object.rs # #[cxx_qt::bridge] GreetPreview（属性 + invokable）
  src/state.rs      # 登录状态机（语义未改）
  src/i18n.rs       # 双语言目录（en-US / zh-CN）
  src/headless_tests.rs # bin 内测试（含子进程 offscreen 证据）
  i18n/             # Fluent 目录
  README.md / AGENTS.md
```

## 提交信息格式

Conventional Commits，scope 使用 `greet`。
