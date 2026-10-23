# ADR 0035: 决定采用 cxx-qt 作为 QML + Qt Quick 的 Rust 绑定

- 状态：accepted
- 日期：2026-10-23
- 决策者：架构组（项目所有者 rownix101）
- 批准记录：2026-10-23 项目所有者拍板：绑定采用 **cxx-qt**，且执行路径为**先写本选型 ADR + 提交可验证原型证据，再逐组件迁移**（本决策记录由研究评估改写为采纳决策）。
- 前置：[ADR 0033](./0033-qml-qt-quick-shell-evaluation.md)（Shell GUI 由 Iced 迁移 QML + Qt Quick）、[ADR 0034](./0034-lgplv3-relicensing-evaluation.md)（重许可 LGPLv3，使 Qt 主体 LGPL 依赖图合法）。本 ADR 是 ADR 0033「复审条件 1——实施阶段选型依据」的具体落地。

---

## 背景

[ADR 0033](./0033-qml-qt-quick-shell-evaluation.md) 决定 Shell/L3 的 GUI 由 Iced 迁到 QML + Qt Quick，并把「Rust 绑定选型」列为进入实施阶段的复审条件之一。本 ADR 在 Phase 1（重许可 LGPLv3，ADR 0034）已落地的前提下，回答这个选型问题。

目标约束集（来自根级 AGENTS.md、crate 级 AGENTS.md 与 ADR 0033/0034）：

1. **领域逻辑留在 Rust**：QML/Qt Quick 只承载声明式 UI；领域模型、状态机、纯投影逻辑与安全边界仍在 Rust（根级 §4）。
2. **合成器绝不使用 Qt Wayland Compositor（GPL-only）**：只可能涉足 Shell/L3 的 UI 宿主，合成器仍是 Smithay（ADR 0033、0034 §不变量）。
3. **不得引入 GPL-only 的 Qt 模块**（Qt Wayland Compositor / Qt Virtual Keyboard / Qt QML Compiler）（ADR 0033、0034）。项目自身为 LGPL-3.0。
4. **Shell 组件必须保持键盘可导航并暴露语义树（AT-SPI 桥接）**（根级 §7.4）。
5. **禁用 `unsafe`**（根级 §3.2：除 `inox-compositor` / `inox-xwayland` 的最小胶水外 `#![forbid(unsafe_code)]`）。
6. 三种 L3 表面（taskbar/menubar/dock）是一份数据的三种呈现；flage、`inox-lock` 会话锁表面（ADR 0025）与 `inox-greet`（L4）的边界必须保留。
7. MSRV：工作区 `rust-version` 1.89。

候选绑定：**qmetaobject-rs**（woboq）、**cxx-qt**（KDAB）。

---

## 客观事实（2026-10-23 从主源核对）

### qmetaobject-rs（woboq/off-gofar）

- **许可证**：MIT。版本 0.2.10 发布于 2023-11-03。
- **维护状态**：作者 Olivier Goffart 在 README 明确「目前处于**被动维护**，重心已转向 Slint」，并引导用户转 Slint；该绑定与 Slint 同作者。effectively 处于生命周期末端。
- **Qt 6 支持**：要求 Qt ≥ 5.8；官方 README 只宣称「**部分**场景图支持」，且其自带场景图示例被显式以 `#[cfg(no_qt)] ... panic!("not supported on Qt 6 and above")` 守卫——即核心 QML/SG 示例在 Qt 6 上**不被支持**。CI 矩阵主要在 Qt 5.15；Qt 6 job 只覆盖 6.2.0/6.5.0 的 qttypes/impl，**无 6.12 覆盖**。
- **layer-shell**：自身无 layer-shell 绑定，只能走「手写 wl-layer-shell」分支；且结合 Qt6 SG 缺口的风险最高，没有真实项目用 qmetaobject 做 layer-shell 面板。
- **AT-SPI/a11y**：无自有 a11y 支持，依赖底层（Qt6-不完整）引擎暴露的 Qt Quick `Accessible`；在 Qt6 路径不完整的情况下，AGENTS.md §7.4 需要的 AT-SPI 桥虚悬。
- **`unsafe` 负担**：基于 `cpp!` 宏 + `cpp_build`，FFI 走大量 `unsafe` 块，与根级 §3.2 的 `#![forbid(unsafe_code)]` 冲突。

### cxx-qt（KDAB）

- **许可证**：MIT OR Apache-2.0。版本 0.10.0 发布于 2026-08-24，0.9.x/0.8.x 均在 2026 内，**活跃维护**，仓库持续推送。
- **Qt 6 支持**：官方宣称在 Linux/Win/macOS/WebAssembly 上测试，版本处理支持「Qt 5 或 Qt 6」，并提供 `qt_minimal`（自动下载匹配 Qt，`qt_version_at_least_6_10`）特性；**主动面向当前 Qt 6**，无「Qt 6 不支持」的守卫；文档化路径始终是完整 `QGuiApplication` + `QQmlApplicationEngine` 宿主。
- **layer-shell**：绑定本身同样没有 LayerShellQt；但在完整 `QGuiApplication`/`QQuickWindow` 窗口模型下，可在 Qt 宿主层对 QML 窗口应用 layer-shell——该路径已被真实 Qt Quick 项目（Nitrux `marina` dock 的 LayerShellQt `Window::get(window)`，BSD-3；KDE LayerShellQt 为 **LGPL**）验证。
- **AT-SPI/a11y**：因为应用是标准的 Qt 应用（QGuiApplication + QQmlApplicationEngine + Qt Quick），Qt 标准 QPA 无障碍桥照常工作：Qt Quick `Accessible` 项映射进 AT-SPI。**这与 AGENTS.md §7.4 所需语义树一致**。
- **`unsafe` 负担**：生成 C++ 薄胶水；`unsafe extern "C++"` 由宏/`cxx` 管理，暴露给 Rust 侧的安全 API 比 `cpp!` 干净得多。
- **MSRV 1.85** ≤ 工作区 MSRV 1.89，兼容 Rust 2024，无摩擦。
- **集成方式**：`#[cxx_qt::bridge]` + `cxx_qt_build::CxxQtBuilder`（`build.rs`），系统依赖为 C/C++ 编译器 + qmake 可发现（或 `qt_minimal`）；链接发行版 Qt Core/Gui/Qml/Quick/WaylandClient。

### 本地工具链验证（Phase 2 原型的一次真实检验）

在本仓库的 Linux 宿主（Qt 6.12.0、rustc 1.99.1，晚于工作区 MSRV 1.89）做了一次完整的 cxx-qt 0.10.0 纯 Cargo 探针（复用官方 `cargo_without_cmake` 的集成方式：`CxxQtBuilder::new_qml_module(QmlModule::new(...).qml_file(...))` + `.files(...)` + `cxx`/`cxx-qt-lib` 依赖）：

- `cxx-qt` / `cxx-qt-build` / `cxx-qt-lib` 0.10.0 **下载、编译、链接全部通过**；`#[cxx_qt::bridge]` 宏展开并生成 C++ 薄胶水。
- 链路：`QGuiApplication::new()` + `QQmlApplicationEngine::new()` + `engine.load(QUrl)` 在 `QT_QPA_PLATFORM=offscreen` 下正常进入事件循环（无崩溃、QML 加载成功），二进制通过 `ldd` 确认只链接 `libQt6Core/Gui/Qml/Network/DBus`（均为 LGPL/允许模块）。
- 结论：**cxx-qt 0.10 兼容当前 stable rustc（包括高于工作区 MSRV 的工具链）**，在本项目目标 Qt 6.12 上可构建、可链接、可离屏运行。早前一次失败的「rustc 1.99 下 bridge 宏报 non-foreign item macro」是探针桥源码缺依赖/写法不完整所致，**并非 cxx-qt 与工具链的兼容性问题**；修正为完整官方写法后在同一 rustc 下通过。

本地验证只作为选型参考信号，不构成验收；真正判定仍在 CI。

---

## 决策

**本 ADR 决定：Shell/L3 的 Qt Quick/QML 迁移采用 cxx-qt（KDAB，MIT OR Apache-2.0）作为 Rust 绑定；以「QML 窗口 + LayerShellQt（LGPL）」作为 layer-shell 宿主机制；AT-SPI 走完整 Qt 应用的 QPA 无障碍桥。** 否决 qmetaobject-rs。具体判断：

1. **cxx-qt 与「领域逻辑留在 Rust」契合。** 其模型正是 Rust 定义 QObject（`#[cxx_qt::bridge]`），暴露给 QML 作为普通 QObject；C++ 只生成薄胶水。这正对 Inox「核心逻辑在 Rust、UI 交给声明式 QML」的架构约束。
2. **cxx-qt 的完整 Qt 应用模型保住 AT-SPI。** QGuiApplication + QQmlApplicationEngine 之下 Qt 标准无障碍桥照常；qmetaobject 的 Qt6-不完整路径下这条桥虚悬。
3. **layer-shell 有真实先例。** KDE `LayerShellQt` 是 LGPL、被 Nitrux `marina` dock 等 Qt Quick 面板在 Qt6 下生产使用；在 Qt 宿主层对 QML 窗口应用 layer-shell，满足 ADR 0033「layer 表面不是窗口、但可用 Qt Wayland client 窗口包装」的路线，且**不需要任何 GPL-only Qt 模块**（Qt WaylandClient 不是被禁的 QtWaylandCompositor）。
4. **许可证安全。** cxx-qt 为 MIT/Apache-2.0，LayerShellQt 为 LGPL，Qt Core/Gui/Qml/Quick/WaylandClient 为 LGPL；全在 LGPL-3.0 项目的 allow 集内（ADR 0034）。不引入 GPL-only Qt 模块。
5. **否决 qmetaobject-rs**：失维护、Qt 6 支持不完整（场景图示例显式不支持 Qt 6）、无 layer-shell 方案、`unsafe` 负担重，对本项目属高风险选项。

### 实施执行约束（Phase 2 强制执行）

- 统一以 `CxxQtBuilder` + `#[cxx_qt::bridge]` 集成；`build.rs` 通过 `.file(...)`（不是 `.bridge(...)`，0.10 用 `.file`）声明桥源码。
- **锁定 rustc 版本**：工作区沿用 `rust-toolchain.toml`（stable）+ MSRV 1.89；cxx-qt 0.10 MSRV 1.85、并已在本项目工具链验证过新 stable，无需特殊锁版。
- **`unsafe` 边界**：cxx-qt/cxx 的 FFI 由 `cxx` 管理；Inox 侧禁用 `unsafe` 的 crate 若必须触碰 Qt 类型，把 FFI 局限在 crate 内受控的最小胶水模块并附 `SAFETY` 注释（遵照根级 §3.2 例外精神，但尽量用 cxx-qt 的安全 API 避免 `unsafe`）。
- **layer-shell**：在 Qt 宿主层用 `LayerShellQt::Window::get(window)` 包装 QML 窗口；`inox-lock` 的会话锁表面（ADR 0025）与 `inox-greet` 的登录表面必须保留各自的安全/生命周期边界，不以「普通窗口」对待。
- **设计令牌 / Fluent / a11y 属性（QML `Accessible`）**：随每个组件迁移一并带入 QML，颜色只能在 Rust 或 C++ 侧由令牌显式转 `QColor`，禁止在 QML 里硬编码。
- **依赖图只允许 LGPL+permissive**：`license-audit` CI job（ADR 0034 落地）继续校验系统 Qt 模块仅链接 LGPL 且无 GPL-only 模块入图。

### 备选与否决依据

| 方案 | 收益 | 成本 / 风险 | 选择或否决 |
|------|------|-------------|------------|
| **cxx-qt（采用）** | 活跃、Qt 6.12 支持、完整 GUI 模型保住 AT-SPI、`unsafe` 较少、LGPL 安全、适配「逻辑留 Rust」 | 绑定/生成代码随 Qt 与 rustc 演进，需跟随上游 0.10.x 维护 | **采用** |
| qmetaobject-rs | 老牌、API 简单、MIT | 失维护、Qt 6 场景图不支持、无 layer-shell、`unsafe`/`cpp!` 负担、AT-SPI 虚悬 | **否决**（Qt6 不完整 + 失维护） |
| 手写 C-ABI + C++ 宿主（open-pet 路线） | 完全自控、无绑定版本 churn、跨平台 ABI 固定 | 需自维护桥、工作量最大、超出本项目单 Qt 宿主需求 | 保留为远景备选，不在此步采用 |

---

## 影响

- 影响 crate：`inox-panel`、`inox-launcher`、`inox-notifications`、`inox-settings`、`inox-lock`、`inox-wallpaper`（L3）、`inox-greet`（L4）。`inox-shell` 无 UI 不受影响。合成器（`inox-compositor`）**不改**，仍是 Smithay。
- 每迁移一个组件都有独立的、可审查的、聚焦的 PR（沿 ADR 0033 的「复审条件 3——不把迁移与其它工作混在一起」），CI 是唯一判定。
- 新增系统依赖：构建期内 Qt6 dev 包（Core/Gui/Qml/Quick/QmlModels/WaylandClient）与 LayerShellQt；由 CI 的 `system-deps`/`license-audit` 覆盖。
- 依赖图：去掉 `iced` / `iced_core` / `iced_runtime` / `iced_tiny_skia` 等 Iced 依赖（迁移完成后的收尾 PR），换成 `cxx-qt` / `cxx-qt-build` 及其 Qt 宿主。
- 分层方向：L3→L2 向下边继续合法；QML 绑定不改变 crate 间 Rust 依赖方向，`layering.rs` 仍需通过。

---

## 后续

1. 在 CI `system-deps` action 上铺好 Qt6 dev（Core/Gui/Qml/Quick + LayerShellQt）依赖面。
2. 提交一个 headless 可验证的 QML + LayerShellQt + AT-SPI 原型，作为「绑定可用」的证据（ADR 0033 复审条件 1 的缺口补上）。
3. 按组件逐个迁移（panel/launcher/notifications/settings/lock/wallpaper/greet），每步一个聚焦 PR；保留 inox-lock/greet 的安全边界。
4. 全量迁移后移除 Iced 依赖与 Iced 测试 harness；更新 PRD §3.1 / §9、crate 级 AGENTS.md、docs/book。