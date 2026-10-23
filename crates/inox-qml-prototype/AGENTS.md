# AGENTS.md — inox-qml-prototype

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

`inox-qml-prototype` 是自 Iced 迁移到 QML/Qt Quick（ADR 0033）过程中，证明 **cxx-qt 0.10 绑定链路（Rust → C++ → QML → 返回）** 可用性的最小验证 crate：一个 Rust-backed `QObject`（`MyObject`）暴露 `Q_PROPERTY` 与一个 `qinvokable`，由 QML 模块与 `main.qml` 消费，另以 headless 单元测试验证桥接编译/链接/加载全链路。本 crate **bin-only**（无库目标）：cxx-qt 0.10 会把 bridge 生成的 `cxx_qt_init_*` 定义嵌入「声明该 bridge 的目标」；若在同一个包里把 bridge 放库、由二进制引用，二进制对象图未引用这些定义，`--gc-sections` 会将其丢弃，导致 `undefined reference to 'cxx_qt_init_*'`（上游 [KDAB/cxx-qt #770](https://github.com/KDAB/cxx-qt/issues/770)，0.10 未修复）。故 bridge 与 headless 测试都放在 bin 内，与上游 `cargo_without_cmake` 例子的形态一致。

所属层级：**L3**（迁移原型的 UI 载体，与 `inox-shell` 家族同层；不依赖任何其他 Inox crate，保持自包含）。

## crate 特有约束

- **`unsafe_code` 例外（ADR 0035）**：根级 §3.2 默认 `#![forbid(unsafe_code)]`。但 `#[cxx_qt::bridge]` 宏生成的 FFI 胶水（`unsafe extern "C++"`、`unsafe fn`）**不自带 `#[allow]`**，根工作区的 `unsafe_code = "forbid"` 会使其编译失败。因此本 crate 在 `Cargo.toml` 单独声明

  ```toml
  [lints.rust]
  unsafe_code = "allow"
  missing_docs = "warn"
  ```

  仅放行宏**生成**的 `unsafe`；**Inox 手写的代码仍保持不含 `unsafe`**（本仓库任何手写文件不出现 `unsafe` 关键字，`src/main.rs` / `src/cxxqt_object.rs` / `src/headless_tests.rs` 也不写 `#![...unsafe_code]`）。根级其余硬性规则一律不变。
- bridge 宏内部的条目（`unsafe extern "C++"` / `extern "RustQt"` 里的类型、函数）**不能写 `///` 文档注释**：cxx 解析器会报 `error[cxx]: unsupported attribute`。文档注释集中在 `MyObjectRust` 结构体、`impl qobject::MyObject` 块与 `pub mod qobject` 之上。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()`；headless 测试用 `assert!` / `assert_eq!`。测试中设置的 `QT_QPA_PLATFORM` 属测试专用 `unsafe` 环境写，属允许范围。
- 日志统一 `tracing`（`debug!`），禁止 `println!`（测试除外）。

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
| headless 桥接测试 | `QT_QPA_PLATFORM=offscreen cargo test -p inox-qml-prototype --bin inox-qml-prototype` |
| 真实合成器上的表面断言（D02，opt-in） | 先 `cargo build --workspace --features inox-compositor/test-input`，再 `INOX_WAYDRIVER_E2=1 cargo test -p inox-integration --all-features --test waydriver_host` |
| 运行原型 GUI | `QT_QPA_PLATFORM=offscreen cargo run -p inox-qml-prototype`（无显示环境） |
| 本 crate 构建 | `cargo build -p inox-qml-prototype --all-features` |
| 本 crate 文档 | `cargo doc -p inox-qml-prototype --no-deps --all-features` |
| 全工作区测试 | `cargo test --workspace --all-features` |
| 静态检查 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt --all` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- **headless 单元测试**（`src/headless_tests.rs`，`#[cfg(test)]` 于 bin 内）覆盖：
  - `#[qproperty]` 的 Rust 后备结构体（`MyObjectRust`）默认构造可用、属性默认值正确（证明桥接声明 → 生成属性胶水的 Rust 侧成立）。
  - `QQmlApplicationEngine` 在 offscreen 平台可构造、`load` 不中止（证明 bridge 生成代码 + QML 类型注册编译并链接）。
  - 运行需 `QT_QPA_PLATFORM=offscreen`（测试内已设置），无需 Wayland 合成器或真实显示。
  - 因与本 crate 的 lib+bin 链接问题（见「职责」）同理，headless 测试放在 bin 的 `#[cfg(test)]` 模块内，而非独立 integration crate。
  - **它不证明 QML 树真的加载成功**：`QQmlApplicationEngine::load` 是异步的，cxx-qt-lib 0.10 未暴露 Rust 侧 `rootObjects()`，因此该测试观察不到加载失败。真正的「加载成功」由 D02 waydriver 宿主测试断言（要求合成的 Qt 表面出现在合成器帧里）。
- **QML 资源 URL**：cxx-qt 把 `QmlModule::qml_file` 挂在 `qrc:/qt/qml/<URI 转目录>/` 前缀下，**并保留文件自身的相对路径**作为 alias。模块 URI 为 `com.inox.prototype.qml`、文件为 `qml/main.qml`，故正确 URL 是 `qrc:/qt/qml/com/inox/prototype/qml/qml/main.qml`（比 URI 前缀多一段 `qml`）。常量 `MAIN_QML_URL` 定义在 `src/main.rs`，`main` 与 headless 测试共用，避免两处漂移。写错 URL 时客户端仍能连上合成器、但**永不创建表面**（合成器只记录 client connected，dump-frame 全是背景色）。
- **`qml/main.qml` 的 `import` 必须与模块 URI 一致**：`import com.inox.prototype.qml`（不是 `com.inox.prototype`）。
- **为何不从纯 Rust 直接驱动 C++ QObject**：cxx-qt 0.10 对裸 `#[qobject]` **不生成公开的 Rust 侧工厂**（仅生成 `createRs` 的 Rust 结构体构造与 QML 注册；`make_unique`/`Threading` 需显式声明或开启特性）。因此 headless 测试不 `make_unique` 驱动 `MyObject`；真正的 `QML → Rust`（实例化 `MyObject`、改属性、调 `incrementNumber`）在真实 Qt 平台验证。
- **真实平台验证**：`crates/inox-integration/tests/waydriver_host.rs`（D02，opt-in `INOX_WAYDRIVER_E2=1`）在真实 Inox headless 合成器上启动本 crate 的二进制，断言其像素出现在合成器帧里，并以注入 `Escape` 让它经自身事件循环以退出码 0 结束。`qml/main.qml` 的 `Escape` 处理（focus sink 上的 `Keys.onPressed` → `Qt.quit()`）就是为此提供的干净退出路径：`Window` 不是 `Item`、不能挂 `Keys`，故由填满窗口的 focus sink 承担。
- 覆盖率阈值：本 crate 为迁移原型，bridge 生成代码不计覆盖；Rust 手写逻辑简单，不作硬性阈值。

## 已知例外（原型期）

- **`qml/main.qml` 硬编码颜色与英文字面量**：根级 §3.10（禁止硬编码颜色/尺寸）与 §7.3（UI 文案必须 Fluent）不适用于**验证原型**；本 crate 的目的仅是演示绑定链路可编译/链接/加载，尚未接入设计令牌与 Fluent。迁移到真实组件（`inox-shell` 等）时必须改用主题令牌与 Fluent，本例外随即失效。

## 目录结构

```
crates/inox-qml-prototype/
  Cargo.toml        # *.workspace = true；[lints.rust] unsafe_code = "allow"（ADR 0035）
  build.rs          # CxxQtBuilder::new_qml_module(QmlModule::new("com.inox.prototype.qml").qml_file("qml/main.qml")).files(["src/cxxqt_object.rs"]).build()
  src/main.rs       # bin-only 入口：mod cxxqt_object + mod headless_tests；QGuiApplication + QQmlApplicationEngine
  src/cxxqt_object.rs  # #[cxx_qt::bridge] 定义 MyObject（Q_PROPERTY + qinvokable）
  src/headless_tests.rs # headless 桥接单元测试（#[cfg(test)]）
  qml/main.qml      # com.inox.prototype.qml QML 模块；消费 MyObject；设置 Accessible 角色；Escape 退出
  README.md         # 本 crate 说明
  AGENTS.md         # 本文件
```
