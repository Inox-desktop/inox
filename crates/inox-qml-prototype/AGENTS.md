# AGENTS.md — inox-qml-prototype

本 crate 遵循根仓库的**全部硬性规则**（[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)），本文件只补充本 crate 特有内容，不放宽任何硬性规则。

## 职责

`inox-qml-prototype` 是自 Iced 迁移到 QML/Qt Quick过程中，证明 **cxx-qt 0.10 绑定链路（Rust → C++ → QML → 返回）** 可用性的最小验证 crate：一个 Rust-backed `QObject`（`MyObject`）暴露 `Q_PROPERTY` 与一个 `qinvokable`，由 QML 模块与 `main.qml` 消费，另以 headless 集成测试验证桥接编译/链接/加载全链路。

所属层级：**L3**（迁移原型的 UI 载体，与 `inox-shell` 家族同层；不依赖任何其他 Inox crate，保持自包含）。

## crate 特有约束

- **`unsafe_code` 例外**：根级 §3.2 默认 `#![forbid(unsafe_code)]`。但 `#[cxx_qt::bridge]` 宏生成的 FFI 胶水（`unsafe extern "C++"`、`unsafe fn`）**不自带 `#[allow]`**，根工作区的 `unsafe_code = "forbid"` 会使其编译失败。因此本 crate 在 `Cargo.toml` 单独声明

  ```toml
  [lints.rust]
  unsafe_code = "allow"
  missing_docs = "warn"
  ```

  仅放行宏**生成**的 `unsafe`；**Inox 手写的代码仍保持不含 `unsafe`**（本仓库任何手写文件不出现 `unsafe` 关键字，`src/lib.rs` 也不写 `#![...unsafe_code]`）。根级其余硬性规则一律不变。

- bridge 宏内部的条目（`unsafe extern "C++"` / `extern "RustQt"` 里的类型、函数）**不能写 `///` 文档注释**：cxx 解析器会报 `error[cxx]: unsupported attribute`。文档注释集中在 `MyObjectRust` 结构体、`impl qobject::MyObject` 块与 `pub mod qobject` 之上。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()`；headless 测试用 `assert!` / `assert_eq!`。测试中设置的 `QT_QPA_PLATFORM` 属测试专用 `unsafe` 环境写，属允许范围。
- 日志统一 `tracing`（`debug!`），禁止 `println!`（测试除外）。
- **令牌规则的适用范围**：根级 §4「只消费设计令牌、禁止硬编码颜色/尺寸」约束的是交付组件；本 crate 是绑定链路的验证样本，`qml/main.qml` 里的 `#2b2b2b`/`#cccccc` 两个字面量是有意为之的最小示例，不构成产品界面。迁移完成后不要再往这里加界面代码——真正的组件在 `inox-<component>` 里。
- **私有 ABI**：本 crate 只使用 Qt 公开 API，没有使用迁移收尾授权的私有 ABI。

## 主要依赖

- 无工作区内 Inox crate 依赖。
- 关键第三方：
  - `cxx`（Rust↔C++ FFI）
  - `cxx-qt` 0.10.0（bridge 宏）
  - `cxx-qt-lib` 0.10.0 `features = ["full"]`（Qt 类型绑定）
  - `cxx-qt-build` 0.10.0（build-dep）
  - `anyhow`、`tracing`（工作区依赖）
  - 系统库：Qt 6（QGui、Qml、Quick），经 `qmake` 定位（见 README）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

| 目的 | 命令 |
|------|------|
| headless 桥接测试 | `QT_QPA_PLATFORM=offscreen cargo test -p inox-qml-prototype --test headless` |
| 运行原型 GUI | `QT_QPA_PLATFORM=offscreen cargo run -p inox-qml-prototype`（无显示环境） |
| 本 crate 构建 | `cargo build -p inox-qml-prototype --all-features` |
| 本 crate 文档 | `cargo doc -p inox-qml-prototype --no-deps --all-features` |
| 全工作区测试 | `cargo test --workspace --all-features` |
| 静态检查 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt --all` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- **headless 集成测试**（`tests/headless.rs`）是核心证据，覆盖：
  - `#[qproperty]` 的 Rust 后备结构体（`MyObjectRust`）默认构造可用、属性默认值正确（证明桥接声明 → 生成属性胶水的 Rust 侧成立）。
  - `QQmlApplicationEngine` 在 offscreen 平台可构造并能加载 `com.inox.prototype` 模块的 `main.qml`（证明 bridge 生成代码 + QML 类型注册编译、链接、运行全链路无误）。
  - 运行需 `QT_QPA_PLATFORM=offscreen`（测试内已设置），无需 Wayland 合成器或真实显示。
- **为何不从纯 Rust 直接驱动 C++ QObject**：cxx-qt 0.10 对裸 `#[qobject]` **不生成公开的 Rust 侧工厂**（仅生成 `createRs` 的 Rust 结构体构造与 QML 注册；`make_unique`/`Threading` 需显式声明或开启特性）。因此 headless 测试不 `make_unique` 驱动 `MyObject`；真正的 `QML → Rust`（实例化 `MyObject`、改属性、调 `incrementNumber`）在真实 Qt 平台（waydriver 截图测试）验证。
- 覆盖率阈值：本 crate 为迁移原型，bridge 生成代码不计覆盖；Rust 手写逻辑简单，不作硬性阈值。

## 已知例外（原型期）

- **`qml/main.qml` 硬编码颜色与英文字面量**：根级 §3.10（禁止硬编码颜色/尺寸）与 §7.3（UI 文案必须 Fluent）不适用于**验证原型**；本 crate 的目的仅是演示绑定链路可编译/链接/加载，尚未接入设计令牌与 Fluent。迁移到真实组件（`inox-shell` 等）时必须改用主题令牌与 Fluent，本例外随即失效。

## 目录结构

```
crates/inox-qml-prototype/
  Cargo.toml        # *.workspace = true；[lints.rust] unsafe_code = "allow"
  build.rs          # CxxQtBuilder::new_qml_module(QmlModule::new("com.inox.prototype.qml").qml_file("qml/main.qml")).files(["src/cxxqt_object.rs"]).build()
  src/lib.rs        # 库入口：pub mod cxxqt_object
  src/main.rs       # QGuiApplication + QQmlApplicationEngine，加载 qrc:/qt/qml/com/inox/prototype/qml/main.qml
  src/cxxqt_object.rs  # #[cxx_qt::bridge] 定义 MyObject（Q_PROPERTY + qinvokable）
  qml/main.qml      # com.inox.prototype QML 模块；消费 MyObject；设置 Accessible 角色
  tests/headless.rs # headless 桥接集成测试
  README.md         # 本 crate 说明
  AGENTS.md         # 本文件
```
