# ADR 0038: panel 与 lock 用离屏 QML 渲染保留 layer-shell 与会话锁表面

- 状态：proposed
- 日期：2026-10-24
- 决策者：架构组（项目所有者 rownix101）
- 批准记录：待补。本 ADR 按项目所有者 2026-10-24 的当次指令「把全部 Iced 迁移到 QML + Qt Quick」执行（根级 `AGENTS.md` §1.3）；人类评审后转 `accepted`。
- 关系：补充 [ADR 0033](./0033-qml-qt-quick-shell-evaluation.md)（Shell GUI 迁移 QML + Qt Quick）、[ADR 0035](./0035-cxx-qt-binding.md)（采用 cxx-qt）与 [ADR 0036](./0036-component-migration-plan.md)（Phase 2d 逐组件迁移）。ADR 0033/0035/0036 的核心决策（QML/Qt Quick、cxx-qt、领域逻辑留 Rust）不变；本 ADR 只裁决「两个真实协议表面如何承载 QML」。

---

## 背景

ADR 0035 选择「QML 窗口 + `LayerShellQt::Window::get(window)`」作为 layer-shell 宿主，ADR 0036 的 waydriver 基座 PR 已实测记录：GitHub Actions 的 `ubuntu-latest` 是 Ubuntu 24.04（noble），归档里只有 Qt 5 的 `layer-shell-qt`（`liblayershellqtinterface-dev` 5.27.11），没有 `liblayershellqtinterface6-dev`；因此 CI 上的 Qt 表面是普通 `xdg_toplevel`，layer 腿被明确列为 blocker，不得伪装成已生效的放置。

在本仓库 2026-10-24 的状态下，七个 GUI crate 里只有两个持有真实协议表面：

- `inox-panel`：`src/layer.rs` 自己建 `zwlr_layer_surface_v1`（锚点、层、独占区、键盘交互、玻璃请求），并用 `wl_shm` 提交帧；节点、放置与输入都是本仓库的实现。
- `inox-lock`：`src/session.rs` 自己建 `ext_session_lock_v1` 表面（ADR 0025：锁表面不是窗口），同样经 `wl_shm` 提交帧；`src/render.rs` 负责把控件树画成缓冲区。

其余五个（`inox-wallpaper`、`inox-launcher`、`inox-notifications`、`inox-settings`、`inox-greet`）在 Iced 下本来就是普通窗口：它们的 `placement` 只建模、未应用（ADR 0018 记录）。把它们换成 Qt 窗口是等价替换，不需要 LayerShellQt。

问题只出在 panel 与 lock：

1. **LayerShellQt 在 CI 上不可得**（上）。即使换成更新的 Ubuntu，也还要把 runner 换成容器镜像或自建 LayerShellQt，才谈得上验证。
2. **Qt 没有 `ext-session-lock-v1` 客户端**。LayerShellQt 只包装 `zwlr_layer_shell_v1`，锁表面无法用「Qt 窗口 + LayerShellQt」得到；把锁屏做成普通 Qt 窗口会违反 ADR 0025 与 ADR 0033 的安全边界。
3. 把这两个组件降级为普通窗口，会让「面板占住一条边、留出独占区」和「锁表面接管输出」这两个桌面级契约失效，且现有协议测试（`inox-panel/tests/protocol.rs`、`inox-lock/tests/protocol.rs`）会变成断言错误行为的测试。

## 决策

**panel 与 lock 保留现有 Wayland 协议腿，QML 只做离屏渲染：**

- 两个组件仍然用 `calloop` 作为主事件循环，仍然由现有代码创建/管理 layer 表面与会话锁表面、输入与 `wl_shm` 帧。
- 控件树从 Iced 换成 QML。QML 由 Qt Quick 的**软件场景图**在离屏平台（`QT_QPA_PLATFORM=offscreen`）渲染到一个 `QQuickWindow`，取回 RGBA8 预乘像素后写入现有的 `wl_shm` 缓冲。渲染入口是一个 crate 内的 C++ 薄宿主（`QGuiApplication` + `QQmlApplicationEngine` + `QQuickWindow::grabWindow`），通过 cxx-qt 暴露给 Rust；不引入 Qt Wayland 客户端，也不引入 LayerShellQt。
- QML 在这两个组件里是**纯渲染层**：输入、焦点、状态机、放置与安全边界继续在 Rust（`layer.rs` / `session.rs` / `app.rs`）。这是有意的：锁屏/面板的输入语义已经由协议腿与状态机拥有，不需要再经 QML 事件树走一遍。
- 其余五个组件用标准 Qt 窗口（`QGuiApplication` + `QQmlApplicationEngine`，QML `Window` 根），QML 负责渲染与交互；这是 ADR 0036 的原路径。

### 证据（本地探针，2026-10-24）

本 ADR 的可行性不是推测：在仓库宿主（Qt 6.12.0、cxx-qt 0.10.0、rustc 1.98.1）上跑通了一个最小探针，链路与决定一致：

```text
Rust -> cxx-qt bridge -> C++ Offscreen(QGuiApplication offscreen + QQmlApplicationEngine)
     -> QQuickWindow::grabWindow() -> QImage RGBA8888_Premultiplied -> QByteArray -> Rust
```

- `QT_QPA_PLATFORM=offscreen` + `QT_QUICK_BACKEND=software` 下 QML 正常渲染，取回的 200×100 帧里背景、矩形与文字像素都正确（中心像素为文字抗锯齿色）。
- `cxx-qt-build` 的 `.cpp_file("src/offscreen.cpp")` + `.include_dir("src")` + 显式 `.qt_module("Quick")` 可以编译自写 C++ 宿主；cxx bridge 用不透明 `type Offscreen;` 声明即可，不需要把 QObject 暴露给 QML。
- 该探针只作为选型/实现依据，不是验收；最终判定仍在 CI。

### 为何不选另外两条路

| 方案 | 否决理由 |
|------|----------|
| Qt 窗口 + LayerShellQt（ADR 0035 原路径） | CI 无 Qt 6 LayerShellQt（noble）；换 runner 是另一项基础设施工作。更重要的是 Qt 客户端无法创建 `ext-session-lock-v1` 表面，lock 必然不再是「锁表面」。 |
| 把 panel/lock 降级为普通窗口 | 违反 ADR 0025 与 ADR 0033 的安全/几何契约；现有协议测试要改写为断言错误行为。 |
| 自建 QtWaylandClient 私有 API 的 layer/lock 集成 | 等同于重写 LayerShellQt，还绑定 Qt 私有 ABI；维护面比离屏宿主大一个量级。 |

## 后果

**正面**

- 面板的 layer 放置与锁屏的会话锁语义在迁移后保持原样；`inox-panel` / `inox-lock` 的协议测试仍断言真实协议行为。
- 不需要 LayerShellQt，也不需要 Qt Wayland 客户端；依赖图里不新增 GPL-only 或版本受阻的 Qt 模块，许可面更小。
- QML 与 Rust 的边界与其余五个组件一致（cxx-qt + Rust 投影），只是呈现目标从窗口换成 `wl_shm`。

**负面**

- panel/lock 的 Qt 应用跑在 `offscreen` 平台，**没有 QPA 无障碍桥**：这两处的 AT-SPI 语义导出仍缺证据（Iced 时代同样缺，不是回归，但不能宣称 Q01 已解决）。五个窗口组件不受影响。
- 每帧多一次 `grabWindow()` 的整帧拷贝；按当前面板/锁屏的刷新率这是可接受的，若以后成为瓶颈再评估 `QQuickRenderControl`。
- 两个 crate 各自持有一份 C++ 宿主（约 60–100 行）与 cxx 声明；按 ADR 0018 的既有取舍，重复优于新增同层依赖。

## 验收与证据计划

| 验收 ID | 等级 | 证据 |
|---------|------|------|
| M04a panel 离屏 QML | E1/E2 | `inox-panel` 单测 + 现有 `tests/protocol.rs`（真实 layer 表面仍被创建）；离屏帧断言由渲染投影测试覆盖 |
| M05a lock 离屏 QML | E1/E2 | `inox-lock` 单测 + `tests/protocol.rs`（`ext_session_lock_v1` 仍被创建）；ADR 0029 的凭据边界不变量测试 |
| M03b 窗口组件 | E1 | 各 crate 的 headless QML 加载测试 + 投影测试 |

未实现的证据不预填 pass。

## 影响面

- `inox-panel`、`inox-lock` 的 `Cargo.toml` / `build.rs` / `src/` / `AGENTS.md` / `README.md`。
- 根 `AGENTS.md` §4（calloop 主循环与 Qt 的关系需要一句话说明：有 Qt 窗口的五个组件由 Qt 事件循环驱动，panel/lock 仍由 calloop 驱动）。
- `docs/book/src/development/*`、PRD §4/§11、依赖说明（移除 iced、新增 cxx-qt）。
- 不改变分层、不新增 crate、不改协议。

## 复审触发

1. CI runner 能提供 Qt 6 LayerShellQt 且 Qt 出现 `ext-session-lock-v1` 客户端路径 → 重新评估 panel/lock 的宿主方式。
2. 离屏整帧拷贝在 P03 空闲预算或锁屏首帧延迟上成为瓶颈 → 改用 `QQuickRenderControl` 或只重绘脏区。
3. panel/lock 的 AT-SPI 成为 Q01 的阻塞项 → 单独裁决无障碍桥，不把 offscreen 当成已解决。

*本 ADR 记录实现层面的表面承载方式；ADR 0033/0035/0036 的迁移方向与绑定选型不变。*
