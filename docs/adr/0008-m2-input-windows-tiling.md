# ADR 0008: M2 的输入事件、多窗口与平铺布局（在 headless 路径上扩展）

- 状态：proposed
- 日期：2026-10-16
- 决策者：架构组

## 背景

PRD §13 把 M2 的验收标准（DoD）定义为：「处理键盘/指针事件；管理 ≥2 窗口；平铺布局通过 proptest」，涉及仓库为 `inox-compositor`。ADR 0005 已确定 M1 的 headless 渲染路径：自建 surfaceless EGL + `GlesRenderer` + 离屏渲染目标，并把 Smithay 特性集压到最小（`wayland_frontend` + `renderer_gl`），**不启用** `backend_libinput` / `backend_udev` / `backend_session_libseat` 等输入后端。M2 必须在这条既有路径上扩展，同时不违反 ADR 0005 与 ADR 0003 的约束。

约束与依据：

- 根级 `AGENTS.md` §4：「必须使用 Smithay 抽象实现合成器」「必须使用 calloop 作为主事件循环」「Shell 组件基于 Iced，遵循 Elm 架构」。M2 的输入与窗口管理必须落在 Smithay 的 `input::Seat` 与 `desktop` 抽象上。
- 根级 `AGENTS.md` §3.2：M2 **不新增** `unsafe`；ADR 0005 已把 `unsafe` 收敛到 `src/render.rs` 与 `src/runtime.rs` 两处，新增第三处必须另行提交 ADR。
- 根级 `AGENTS.md` §6：纯逻辑必须有单元测试，核心算法用 `proptest`；合成器须提供 headless 集成测试（软件渲染）；新增或修改行为必须同时新增/更新测试。
- PRD §6.1：CI 必须在**无显示设备、无输入设备**的环境中可复现。
- PRD §9：UI 文案走 Fluent、组件只用主题令牌——M2 不产生面向用户的 UI，故不涉及。
- ADR 0004 §3：Smithay 在 M1 引入；M2 不引入新的系统库依赖以外的排期变化。

考虑过但未被采纳的方案：

- **引入 `backend_libinput` + `backend_udev`（或 `backend_session_libseat`）驱动真实输入设备**：需要 udev/seat 权限与真实设备，CI 无法提供，且会拉入 libinput/libudev 系统库，扩大 ADR 0005 刻意压小的 CI 失败面 —— 不成立。
- **改用 `backend_winit` / `backend_x11` 后端在真实窗口系统里跑输入**：偏离 M1 的 headless 路径，依赖 X11/Wayland 显示，CI 不可复现 —— 不成立。
- **自建窗口管理数据结构、完全不用 Smithay `desktop` 抽象**：违反 `AGENTS.md` §4「必须使用 Smithay 抽象」，且重复实现命中测试、输出绑定、渲染元素收集等既有能力 —— 不成立。
- **把 M2 的输入/窗口验证推迟到 M4 的 waydriver（weston）测试**：M2 的 DoD 要求在本里程碑内可验证，且 waydriver 面向 Iced Shell 组件，不适用于合成器内部状态断言 —— 不成立。

## 决策

M2 在 ADR 0005 固定的 headless 路径上实现输入事件、窗口管理与平铺布局，具体如下：

1. **输入后端保持 headless**。**不新增** `backend_libinput` 等特性；`Seat` / `KeyboardHandle` / `PointerHandle` 由 Smithay 的 `input` 模块提供（键盘 keymap、修饰键跟踪、焦点投递、pointer enter/motion/button 均在 Smithay 内完成）。输入事件通过 `InoxState::inject_key` / `inject_pointer_motion` / `inject_pointer_button` 在**进程内**注入，既服务集成测试，也为后续真实后端（libinput/wayland 平台）预留统一入口。真实输入设备链路推迟到具备后端能力的里程碑。
2. **键位绑定为纯逻辑**。`src/input.rs` 的 `action_for_key(keysym, modifiers) -> Option<Action>` 只做「按键 → 合成器动作」的判定（`Alt+Tab` → 聚焦下一个、`Alt+Shift+Tab` → 聚焦上一个、`Super+Q` → 干净退出），与事件循环解耦，可单元测试。未绑定的按键经 `FilterResult::Forward` 原样转发给焦点客户端。
3. **确定性 keymap**。seat 键盘的 XKB 配置固定为 `rules=evdev, model=pc105, layout=us`，不读取 `XKB_DEFAULT_*` 环境变量，使键位在不同环境下解析一致；因此 CI 需安装 `xkb-data` 提供 rules 文件。
4. **窗口来自 xdg-shell，管理基于 Smithay `desktop`**。新增 `xdg_wm_base`（`XdgShellState`）与 `wl_shm`（`ShmState`）全局；客户端 toplevel 经 `XdgShellHandler::new_toplevel` → `desktop::Window` → `Space<Window>` 管理。Smithay 特性集在 ADR 0005 基础上**新增 `desktop`**（`Space` / `Window` / 命中测试），仍**不启用**任何后端特性。
5. **窗口簿记与布局算法保持纯逻辑**。`src/workspace.rs` 的 `Workspace`（视图顺序、焦点、增删/循环）与 `src/layout.rs` 的 `LayoutConfig`（master/stack 平铺几何）不依赖 Smithay 运行时状态；布局按 PRD §13 的要求由 `proptest` 验证（每窗口一块、块不越界、块互不重叠、master 列在 stack 左侧）。Smithay 层只负责把 `(WindowId, Rectangle)` 结果映射进 `Space` 并发送 configure。
6. **合成渲染**。`render_frame` 先 `Space::refresh` 丢弃已关闭窗口，再用 `Space::render_elements_for_region` 生成整个工作区的渲染元素，交由离屏 `GlesRenderer` 合成（背景色 + 客户端表面）。
7. **不新增 `unsafe`**。ADR 0005 的两处 `unsafe` 保持唯一；M2 的代码全部使用安全 API。
8. **集成测试使用真实 Wayland 客户端（仅测试依赖）**。`tests/window_management.rs` 在同一进程内用 `UnixStream::pair()` 连接一个真实 `wayland-client`，创建 **≥2 个 xdg toplevel**，断言合成器管理的窗口数、平铺位置（默认 1280×720 + 60% master + 8px gap 下的 master/stack 落点）以及键盘/指针注入行为。`wayland-client` / `wayland-protocols` 仅作为 **dev-dependency** 引入（MIT 许可），不进入生产依赖；这符合 `AGENTS.md` §6「不得 mock 掉被测核心逻辑」——被验证的是合成器真实协议处理路径，而非桩。
9. **依赖与 MSRV 不变**。新增 `proptest`（测试，MIT/Apache-2.0）与 `wayland-client` / `wayland-protocols`（测试，MIT）；`rust-version` 维持 `1.86`（ADR 0005）。

## 后果

正面：

- 在无输入设备、无显示设备的 CI 中达成 PRD §13 M2 的 DoD：真实 xdg-shell 客户端窗口被管理并平铺，键盘/指针事件被处理。
- 输入、窗口簿记与布局三块核心逻辑均为**纯逻辑 + 单元/proptest 覆盖**，符合 `AGENTS.md` §6，且测试不依赖系统后端。
- 窗口管理复用 Smithay `desktop` 抽象（命中测试、输出绑定、渲染元素收集），避免自建重复实现。
- 集成测试驱动**真实 Wayland 客户端与真实协议路径**，断言来自可观测的合成器状态（窗口数、映射位置、焦点）而非日志。
- 未新增 `unsafe`，未扩大系统库依赖面（仅新增 `xkb-data` 运行时数据包）。

负面：

- **真实输入设备链路（libinput/udev/seat）仍未覆盖**，M2 的「处理键盘/指针事件」以进程内注入验证；真机输入风险推迟到后续里程碑。
- 合成器仍**无 DRM/KMS 真实输出**，多窗口的平铺在离屏渲染上验证，真实显示链路风险延续 ADR 0005 的遗留项。
- 新增 `wayland-client` / `wayland-protocols` 作为 dev-dependency 会拉长 `cargo test` 的编译时间（仅测试，不影响发布产物）。
- 仅实现 master/stack 一种平铺策略与固定键位集，属于 M2 的最小可用集，后续需要时可扩展而不改变本 ADR 的接口边界。

影响面：作用于 PRD §13 的 M2 行（DoD 文本不变），`inox-compositor` 的 `Cargo.toml`（Smithay `desktop` 特性、dev-dependencies）、`src/{state,layout,workspace,input}.rs`、`tests/window_management.rs`、`.github/workflows/ci.yml`（新增 `xkb-data`）、`AGENTS.md` 与 `README.md`。**不影响** PRD 协定的协议矩阵（M2 未引入 Inox 私有协议）。

取代条件：当后续里程碑正式引入真实输入后端（libinput/udev/seat 或 wayland 平台）或 DRM/KMS 真实输出时，需撰写取代本 ADR 的新 ADR，并同步更新 `inox-compositor` 的 `Cargo.toml` / `AGENTS.md` 与 CI 工作流。
