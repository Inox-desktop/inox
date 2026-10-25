# inox-qml-prototype

自 Iced 迁移到 QML/Qt Quick（)过程中，**cxx-qt 0.10 绑定链路**的验证原型 crate。

本 crate 以最小可运行的方式证明：**一个用 Rust 编写的 `QObject`（`MyObject`），暴露 `Q_PROPERTY` 与 `qinvokable`，能被 QML 实例化并双向驱动** —— 这是 Inox 迁移策略是否可行的关键证据。

## 它证明什么

cxx-qt 0.10 的完整链路 `Rust → C++ → QML → 返回`：

1. `src/cxxqt_object.rs` 用 `#[cxx_qt::bridge]` 声明一个 Rust-backed `QObject`（`MyObject`），含两个 `Q_PROPERTY`（`number`、`string`）和一个 `qinvokable`（`incrementNumber`）。
2. `build.rs` 用 `CxxQtBuilder::new_qml_module(...)` 声明 QML 模块 `com.inox.prototype.qml`，并把 `qml/main.qml` 打进资源（qrc 根路径 `qrc:/qt/qml/com/inox/prototype/qml/main.qml`）。
3. `qml/main.qml` `import com.inox.prototype`，实例化 `MyObject` 并展示属性 / 触发 `incrementNumber`。
4. `tests/headless.rs` **无需 Wayland 合成器或真实显示**，在 `QT_QPA_PLATFORM=offscreen` 下构造 Rust 后备结构体（验证 `#[qproperty]` 的 Rust 侧胶水），并让 `QQmlApplicationEngine` 加载 `com.inox.prototype` 模块的 `main.qml`（验证 bridge 生成代码 + QML 类型注册编译、链接、运行全链路）。

任何一个环节（bridge 宏展开、C++ 编译、Qt 链接、QML 类型注册）出错，headless 测试都会编译失败或加载/断言失败。

> **关于从纯 Rust 直接驱动 C++ QObject**：cxx-qt 0.10 对裸 `#[qobject]` **不生成公开的 Rust 侧工厂**（仅生成 `createRs` 的 Rust 结构体构造与 QML 注册；`make_unique` / `Threading` 需显式声明或开启特性）。因此 headless 测试不 `make_unique` 驱动 `MyObject`；真正的 `QML → Rust`（实例化 `MyObject`、改属性、调 `incrementNumber`）在真实 Qt 平台上验证。

## 目录结构

```
crates/inox-qml-prototype/
  Cargo.toml        # 清单（crate 专属 lints，见 AGENTS.md）
  build.rs          # cxx-qt 构建脚本：声明 QML 模块并编译 bridge
  src/lib.rs        # 库入口：#pub mod cxxqt_object
  src/main.rs       # 可执行入口：QGuiApplication + QQmlApplicationEngine
  src/cxxqt_object.rs  # #[cxx_qt::bridge] 定义 MyObject
  qml/main.qml      # com.inox.prototype 模块的界面（带 AT-SPI Accessible）
  tests/headless.rs # headless 桥接集成测试（核心证据）
  AGENTS.md / README.md
```

## 前置条件（CI 会处理）

- Qt 6（含 QGui / Qml / Quick），且 `qmake` 可在环境变量 `QMAKE` 或 `PATH` 中找到。cxx-qt 依赖 `qmake` 定位 Qt 的头文件与库。
- 静态链接 Qt 6 时需启用 `cxx-qt-build` 的 `link_qt_object_files` feature（本原型按动态链接配置，未启用）。
- Rust 1.89（工作区 MSRV）。

## 运行与测试

> 这些命令仅供参考，不作为本地验收依据（根级 AGENTS.md §3.1）；真正的判定在 CI。若本地没有 Qt/工具链，直接交给 CI。

Headless 桥接测试（**无需显示设备 / Wayland 合成器**）：

```sh
QT_QPA_PLATFORM=offscreen cargo test -p inox-qml-prototype --test headless
```

运行原型窗口：

```sh
QMAKE=/path/to/qmake cargo run -p inox-qml-prototype
# 无显示环境：
QT_QPA_PLATFORM=offscreen cargo run -p inox-qml-prototype
```

构建本 crate & 文档：

```sh
cargo build -p inox-qml-prototype --all-features
cargo doc -p inox-qml-prototype --no-deps --all-features
```

## 设计说明与取舍

- **`unsafe`**：cxx-qt bridge 宏生成的 FFI 胶水不含 `#[allow]`，在根级 `unsafe_code = "forbid"` 下编译失败。本 crate 因此按 [cxx-qt 绑定] 在 `Cargo.toml` 单独放行 `unsafe_code = "allow"`；**Inox 手写代码不含任何 `unsafe`**。详见 `AGENTS.md`。
- **bridge 内文档注释**：cxx 解析器不接受 bridge 块（`unsafe extern "C++"` / `extern "RustQt"`）内的 `///` 注释，文档注释放在宏外的 Rust 类型 / `impl` 块上。
- **QML 模块 uri**：`com.inox.prototype.qml`。main.qml 同最 import 该 uri，qrc 路径把 `.` 换成目录段。
- **Accessible / AT-SPI**：`qml/main.qml` 为窗口、数字文本与按钮设置了 `Accessible.role` / `Accessible.name`，验证 Qt Quick 无障碍树可暴露给 AT-SPI（根级 §7.4）。
- **原型期例外**：`qml/main.qml` 为演示绑定链路而硬编码了颜色与英文字面量（根级 §3.10/§7.3 不适用于验证原型）。迁移到真实组件时必须改用设计令牌与 Fluent，此例外随即失效（见 `AGENTS.md` 的“已知例外”）。

## License

LGPL-3.0。
