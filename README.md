# Inox Desktop

**Inox** 是一个用 **Rust (stable, 2024 edition)** 编写、基于 **Smithay + Iced** 的模块化 Wayland 桌面环境。

名称取自「不锈钢」(Inox)，与 Rust 的「锈」形成反差，寓意坚固、耐腐蚀、长期可维护。

- **组织**：<https://github.com/Inox-desktop>
- **许可证**：BSD 3-Clause（见 [`LICENSE`](./LICENSE)）
- **本仓库**：`inox` 单仓库（Cargo workspace：产品代码、PRD、ADR、文档站点）

产品质量要求见 [PRD 1.3](./PRD.md)、[桌面验收规范](./docs/quality/acceptance.md)与[验收报告模板](./docs/quality/acceptance-report.md)。本轮参考 [Plasma Desktop 固定快照](./docs/quality/plasma-reference.md)，新增 [ADR 0011–0015](./docs/adr/README.md) 均为 proposed，待人类评审；文档中的新门槛不代表 CI 已接线或产品已通过。

> 本仓库是**单一仓库（Cargo workspace）**：全部组件位于 [`crates/`](./crates)，架构决策与文档位于 [`docs/`](./docs)。
> 各组件是工作区成员而非独立仓库，彼此通过 path 依赖连接、一次 PR 一次 CI 完成跨层改动；决策见 [ADR 0016](./docs/adr/0016-single-repository-cargo-workspace.md)（取代 [ADR 0001](./docs/adr/0001-polyrepo-strategy.md) 的多仓库策略）。

---

## 1. 项目简介

Inox 是一个现代 Wayland 桌面环境，目标是构建模块化、可维护、高性能的桌面体验。项目采用 **AI-first 开发模式**：AI 生成绝大部分代码，人类负责产品决策、架构验收与真实硬件测试。**所有编译、测试、lint 均在 GitHub Actions 中完成，不在本地进行编译验证**。

### 目标

- 构建一个模块化、可维护、高性能的 Wayland 桌面环境。
- 最大化利用 Rust 生态中成熟的合成器库与 GUI 框架（Smithay / Iced / calloop）。
- 通过单一职责的 crate 实现清晰的模块边界，既能局部开发与局部测试（`cargo test -p <crate>`），又允许跨 crate 的改动在一个 PR 内原子完成。
- 建立以 GitHub Actions 为核心的持续集成流水线，作为唯一的构建验证途径。
- 采用 BSD 3-Clause 许可证，保持开源友好。

### 非目标

- 不在项目初期支持所有 Wayland 协议或 X11 全部特性。
- 不追求与 GNOME/KDE 功能对等，优先保证核心体验稳定。
- 不进行本地编译验证；开发者与 AI 均依赖 CI 反馈。
- 不商业化；1.0 前不追求全部辅助技术、语言与主题市场。M4 起须验收核心键盘任务、实际语义导出、字体缩放/高对比与 en-US/zh-CN 文案。

### 设计原则

1. **正确性优先于功能广度**：宁可少一个特性，不可留一个崩溃或卡顿路径。
2. **单一职责的 crate**：每个 crate 只解决一类问题，依赖方向严格自底向上。
3. **可测试性内建**：核心逻辑必须可在 headless 环境下被测试，副作用集中于边界层。
4. **配置即数据**：所有用户可见行为由 TOML 驱动，可热重载。
5. **可观测性内建**：使用 `tracing` 结构化日志，默认无遥测上传。
6. **无静默失败**：错误必须显式传播或记录，禁止吞掉错误。
7. **AI 可读性**：命名、目录、文档保持一致性，任何 crate 的约定不得与根级 [`AGENTS.md`](./AGENTS.md) 冲突。

---

## 2. 架构分层

Inox 采用四层架构，**依赖只能向下**（`L4 → L3 → L2 → L1`），禁止反向依赖、同层横向依赖与循环依赖。层级成员登记在根 [`Cargo.toml`](./Cargo.toml) 的 `[workspace.metadata.inox.layers]`，并由 `crates/inox-integration/tests/layering.rs`（验收项 B04）在 CI 中逐条断言。

```text
┌─────────────────────────────────────────────┐
│ Layer 4: 会话与系统集成                      │
│  inox-session, inox-greet, inox-idle         │
├─────────────────────────────────────────────┤
│ Layer 3: Shell 与用户界面                    │
│  inox-shell, inox-panel, inox-launcher,      │
│  inox-notifications, inox-settings,          │
│  inox-lock, inox-wallpaper                   │
├─────────────────────────────────────────────┤
│ Layer 2: 合成器与窗口管理                    │
│  inox-compositor, inox-xwayland,             │
│  inox-protocols                              │
├─────────────────────────────────────────────┤
│ Layer 1: 基础设施与共享库                    │
│  inox-common, inox-config                    │
└─────────────────────────────────────────────┘
```

- **Layer 1**：配置解析、通用类型、日志、工具函数，被所有上层依赖。`inox-common` 是叶子节点，不得依赖任何其他 Inox crate。
- **Layer 2**：核心 Wayland 合成器，基于 Smithay，处理 DRM/KMS、输入、渲染、窗口管理与 XWayland 集成。
- **Layer 3**：桌面 Shell，使用 Iced 构建面板、启动器、通知、设置等用户界面组件。
- **Layer 4**：会话管理、显示管理器、空闲管理等系统级服务。

全部 L1–L4 组件都是本仓库 `crates/` 下的工作区成员。文档站点 `inox-docs` 不属于任何层：它是 `docs/book/` 下的 mdBook，无 `Cargo.toml`，不参与依赖图。

### 运行时数据流

```text
 应用 (Wayland/X11 客户端)
        │  wayland socket
        ▼
 ┌───────────────────┐   inox-ipc (自定义 Wayland 协议)
 │  inox-compositor  │◄───────────────────────────┐
 └───────────────────┘                             │
        ▲  layer-shell / xdg-shell              ┌──┴────────────┐
        │                                       │  inox-shell   │
 ┌──────┴────────┐   D-Bus (notifications,      │  及 L3 组件   │
 │  inox-session │   settings, portals)         └───────────────┘
 └───────────────┘
```

合成器与 Shell **不通过 Rust 依赖**耦合，只通过 Wayland 协议 / D-Bus / IPC 通信，以保证各自可独立运行与测试。

架构决策记录见 [`docs/adr/`](./docs/adr/README.md)。

---

## 3. 组件清单

全部组件都是本仓库 [`crates/`](./crates) 下同一个 Cargo workspace 的成员，依赖关系是工作区内的 **path 依赖**，版本由 `[workspace.package]` 统一给出。**层级成员的唯一事实来源是根 [`Cargo.toml`](./Cargo.toml) 的 `[workspace.metadata.inox.layers]`**，下表必须与它一致（由验收项 B04 断言）。

| crate | 层级 | 路径 | 职责 | 发布物 | 当前状态 |
|-------|------|------|------|--------|----------|
| `inox-common` | L1 | `crates/inox-common` | 共享类型、错误处理、日志、国际化、工具函数 | crate | 已实现 |
| `inox-config` | L1 | `crates/inox-config` | 配置模型、TOML 解析、默认配置 | crate | 已实现（含 `shell.toml`、`notifications.toml` 与会话语言） |
| `inox-protocols` | L2 | `crates/inox-protocols` | 标准协议绑定与自定义 Wayland 协议扩展 | crate | 已实现 |
| `inox-compositor` | L2 | `crates/inox-compositor` | Wayland 合成器，基于 Smithay | 二进制 `inox-compositor` | 部分实现（M1–M3、layer-shell v4；单输出真实后端已实现；配置热重载/多屏与真机验收未完成） |
| `inox-xwayland` | L2 | `crates/inox-xwayland` | XWayland 集成（监督 xwayland-satellite 进程） | 二进制/服务 | 已实现 |
| `inox-shell` | L3 | `crates/inox-shell` | Shell 主框架，插件系统 | 二进制 `inox-shell` | 部分实现 |
| `inox-panel` | L3 | `crates/inox-panel` | 面板（任务栏、系统托盘、时钟） | 二进制 `inox-panel` | 部分实现 |
| `inox-launcher` | L3 | `crates/inox-launcher` | 应用启动器 | 二进制 `inox-launcher` | 部分实现 |
| `inox-notifications` | L3 | `crates/inox-notifications` | 通知守护进程 | 二进制 `inox-notifications` | 部分实现 |
| `inox-settings` | L3 | `crates/inox-settings` | 系统设置应用 | 二进制 `inox-settings` | 部分实现 |
| `inox-lock` | L3 | `crates/inox-lock` | 锁屏（`ext-session-lock-v1`） | 二进制 `inox-lock` | 部分实现：锁协议腿与锁表面界面，认证与输入路径未接线 |
| `inox-wallpaper` | L3 | `crates/inox-wallpaper` | 壁纸设置与渲染 | 二进制 `inox-wallpaper` | 部分实现 |
| `inox-session` | L4 | `crates/inox-session` | 会话管理器，启动合成器与 Shell | 二进制 `inox-session` | 部分实现：真实就绪往返、进程监督与限时注销 |
| `inox-idle` | L4 | `crates/inox-idle` | 空闲管理 | 二进制 `inox-idle` | 部分实现：空闲协议客户端与策略，可信锁/挂起未接线 |
| `inox-greet` | L4 | `crates/inox-greet` | 显示管理器（可选） | 二进制 `inox-greet` | 部分实现：登录模型与阶段驱动的登录界面（`--preview` 显示的是本次构建真实的那个阶段：没有认证边界，因此不可用、不可输入），真实认证与 greeter 宿主未接线 |
| `inox-integration` | 测试 | `crates/inox-integration` | 端到端集成测试与分层方向检查（B04） | 测试报告 | 部分实现 |

工作区之外还有两处非 crate 位置：`docs/book/`（用户与开发者文档站点，mdBook，无 `Cargo.toml`）与组织仓库 `.github`（组织级工作流与 ISSUE/PR 模板）。

**规划中的组件**（1.0 之后或按需提前，新增 crate 须先写 ADR）：`inox-portal`（Portal 后端）、`inox-screenshot`（截图/录屏）、`inox-osd`（屏幕提示）、`inox-cli`（`inoxctl` 命令行工具）。

### 兼容性记录

工作区唯一的 [`Cargo.lock`](./Cargo.lock) 就是组件兼容性记录，也是验收项 L01 的不可变清单来源。此前的 `compat.toml` 兼容矩阵已随 ADR 0016 删除：它需要人工维护，且在迁移前已经与实际依赖的 tag 不一致。

---

## 4. 里程碑

下表概括提议的新口径，完整范围与证据以 [PRD §13](./PRD.md#13-里程碑)和[验收规范](./docs/quality/acceptance.md)为准，避免在总览维护第二份标准。

| 里程碑 | 交付目标 | 必要证据与结论边界 |
|--------|----------|--------------------|
| **M0** | 工程基线 | 真实 CI、骨架/许可证/约束；不代表运行能力 |
| **M1** | Headless 渲染/生命周期 | 输出与像素断言、失败清理、信号退出；不代表真机 |
| **M2** | 窗口/输入语义 | 真实客户端生命周期、popup/焦点、布局边界、坏客户端隔离 |
| **M2.5** | 真实设备闭环 | 后端 CI 覆盖 + 人类真机、可见指针、VT/设备恢复、硬件建档 |
| **M3** | X11/剪贴板/拖放 | Inox 会话真实互通、MIME/取消/异常退出；跨 X11/Wayland 边界的拖放上游阻塞（[ADR 0017](./docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md)） |
| **M4** | Shell 任务/双屏/配置 | 任务/焦点恢复、热插拔/混合 DPI、配置应用/回退、双语/实际语义 |
| **M5** | 安全会话与日常可用 | 锁屏/认证/挂起故障、权限拒绝、进程监督与稳定性开始验收 |
| **M6** | 发布候选 | 性能/长稳原始数据、支持矩阵、升级/回退、两种分发渠道与人类批准 |

以下“已完成”仅保留历史标准下的进展；新标准的差距与阻塞见 [ADR 复核清单](./docs/adr/README.md#既有提案的复核与实施阻塞)，本次未追认通过。

当前状态：**M0、M1、M2 已完成，M3 已实现（X03 仅部分交付）**。M1 的 headless 合成器实现（创建输出、渲染纯色帧、收到终止信号后干净退出，退出码 0）已合入 `inox-compositor` 的 `main` 且 CI 全绿，渲染路径见 [ADR 0005](./docs/adr/0005-headless-compositor-runtime.md)（现状态 `accepted`）；协议绑定集中于 `inox-protocols`（标准 `wl_*` 再导出 + 私有 `inox-ipc`，见 [ADR 0006](./docs/adr/0006-protocol-strategy.md)，状态 `accepted`），合成器以其为唯一协议来源。M2 在同一 headless 路径上扩展 `inox-compositor`：经 `wl_seat` 处理键盘/指针事件、以 `xdg-shell` toplevel 管理 ≥2 个窗口、按 master/stack 平铺布局（`proptest` 验证），决策见 [ADR 0008](./docs/adr/0008-m2-input-windows-tiling.md)；真实输入设备与 DRM/GBM 输出随后已按获批 ADR 0012 接入，限单 GPU/一个活动输出/scale 1；E4 真机验收仍未执行。同期 `inox-xwayland` 实现 PRD M3 的 XWayland 侧：以**监督外部 `xwayland-satellite` 进程**（运行时系统依赖）的方式集成无根 XWayland，不链接其 crate，决策见 [ADR 0007](./docs/adr/0007-xwayland-satellite-integration.md)；其 M1 范围的端到端验证在 headless 第三方合成器（weston）上进行，保留为宿主无关的 E2 检查。

`inox-common` 随 M1 新增 `logging` 模块。上述组件此前以多仓库形式开发，并在组件仓库中打过 `v0.1.0` / `v0.2.0` / `v0.1.0` 的 tag；[ADR 0016](./docs/adr/0016-single-repository-cargo-workspace.md) 把这些仓库的源码合并进本工作区（工作区统一版本 `0.1.1`），组件仓库待归档；`inox-common` 的 `v0.2.0` 已重切为 `v0.1.1` 并退役。

**M3**（[ADR 0017](./docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md)，状态 `proposed`）在 Inox 自己的合成器上完成 X11/数据互通：合成器广告 `wp_viewporter`、`wl_data_device_manager`（v3）与 `zwp_primary_selection_device_manager_v1`，剪贴板的读取权**跟随键盘焦点**（`set_data_device_focus`），Wayland↔Wayland 拖放与 X11 内部拖放可用，每次渲染后发送 frame callback，`inox-xwayland` 的就绪判定改为真实的 X11 连接建立往返；E3 用例移入 `inox-integration`，以子进程方式驱动工作区构建的二进制，不新增依赖边。**跨 X11/Wayland 边界的拖放被上游阻塞**（`xwayland-satellite` v0.8.3 未实现 XDND），X02 的锁屏剪贴板断言与 `wlr-data-control` 推迟到 M5，`-listenfd` 按需激活仍不在范围内 —— 因此 M3 不是 X03 的完整通过。

**M4 部分交付**（[ADR 0018](./docs/adr/0018-l3-component-architecture.md)，状态 `proposed`）：L3 的七个组件首次有了实现，`inox-config` 的 `shell.toml`、`notifications.toml` 与 `config.toml` 的 `[locale]` 随各自的消费者落地。证据等级为 **E0/E1 加离屏渲染**：纯逻辑与不变量的单元/属性测试，加上用 iced 自己的运行时与 tiny-skia 渲染器在固定尺寸下绘制的**真实控件树**的像素与事件断言 —— 后者不需要合成器，也不需要 GPU。**E2/E3 未取得**，合成器的 `zwlr_layer_shell_v1` v4 宿主现已补齐，真实协议客户端测试纳入合成器 CI 分片；L3 的 Iced 客户端接线与 GUI 驱动仍未完成。此前 `iced_layershell` 0.19.1 与 0.19.0 都无法编译（其 `iced_exdevtools` 对 `winit-core` 0.31.0-beta.3 的 `non_exhaustive` 枚举做穷尽匹配）。因此 W05、O02–O03、D01–D05 与 Q01–Q03 的宿主侧证据均未取得，M4 的 DoD 不成立：这些组件目前以普通窗口运行，layer-shell 放置策略只被建模和测试，未被应用。锁屏的协议腿按 [ADR 0022](./docs/adr/0022-m5-session-lock-notification-bus-and-first-privilege-boundary.md) 已经落地（见下一段），通知的 D-Bus 服务按 PRD §13 属 M5。

**M5 进行中**（[ADR 0022](./docs/adr/0022-m5-session-lock-notification-bus-and-first-privilege-boundary.md)，状态 `proposed`）：会话锁的**协议腿两侧**有了实现与证据 —— 合成器广告 `ext_session_lock_manager_v1`（v1），并以「会话生成的一次性令牌 + `inox_ipc` 上的 `claim_lock_role` 认领」默认拒绝授权；锁定期间只合成锁表面（没有锁表面时输出不透明底色），键盘与选区焦点清空、指针只命中锁表面、合成器快捷键不再拦截，`locked` 只在每个输出都渲染过一帧锁定帧之后发送，锁屏 UI 崩溃保持锁定。`inox-lock --take-lock` 真实取锁、遮蔽每个输出、把锁屏界面画上去、**没有解锁路径**（信号不解锁，释放靠注销）；默认不带该开关时行为不变，仍以非零退出说明认证边界尚未评审。`inox-ipc` 升到 v2，新增 `claim_lock_role` 与 `session_lock_state`；`inox-session` 生成令牌并只向可信锁屏客户端投递（`INOX_SESSION_TAKE_LOCK=1` 时才启动它）。证据等级为 **E1+E2**（两侧各自的真实客户端/真实服务端 peer 用例）加一条 **E3** 会话用例。通知守护进程现在真的提供 `org.freedesktop.Notifications`（D06）：`zbus` 阻塞 API + 独立传输线程，一次只服务一个调用（有界等待），能力集、脱敏、有界队列与动作至多一次沿用既有模型，锁屏脱敏由 `inox_ipc` 的锁状态事件驱动；证据为 p2p 连接上 18 个真实 D-Bus 用例（含每秒 100 条持续 10 s）。**锁屏界面**由 [ADR 0025](./docs/adr/0025-lock-screen-surface-ui-and-login-interface.md)（状态 `accepted`）落地：`inox-lock` 用 `iced_runtime::UserInterface` + `iced_tiny_skia` 把 `app::view` 画到 configure 尺寸并提交为 `wl_shm` 帧，协议腿第一次由 crate 自己的状态机驱动（`take_outgoing` 是唯一出口，不再有第二份覆盖簿记）；本构建没有经过评审的认证边界，因此状态里的真话是 `ServiceFailure::NotProvided`，屏幕上出现的是「这里没法校验任何东西」而不是请人输密码 —— 键鼠路径与认证边界一起落地。**仍未交付**：真实认证、logind 挂起、完整会话 ready 握手、锁屏 UI 崩溃后的接管、会话总线上的名字所有权测试、idle 侧的锁状态订阅，以及 H02 的全部真机步骤 —— M5 的 DoD 尚不成立。

**L4 首版**（[ADR 0021](./docs/adr/0021-l4-session-services.md)，状态 `proposed`）：会话管理器通过真实 Wayland 往返启动并监督桌面子进程，支持重启限流、故障隔离和限时注销；空闲组件消费标准通知并提供锁前挂起策略；可选登录组件提供有代次/限流的状态机和阶段驱动的登录界面：界面为六个生命周期阶段各有一幅画面，键盘与禁用原因都已建模并有测试覆盖，但 `--preview` 只能到达本构建真实处于的那一个阶段——没有认证边界，所以它显示的是「无法登录」、身份字段不可输入（见 [ADR 0025](./docs/adr/0025-lock-screen-surface-ui-and-login-interface.md)）。真实认证、greeter 宿主、logind 挂起和完整服务 ready 尚待审查/接线，M5 未完成。

---

## 5. 技术栈

| 类别 | 技术 |
|------|------|
| 语言 | Rust (stable, edition 2024) |
| 合成器框架 | Smithay |
| 事件循环 | calloop |
| XWayland | xwayland-satellite |
| GUI 工具包 | Iced（Elm 架构，单向数据流） |
| 配置格式 | TOML + serde |
| 日志 | tracing + tracing-subscriber |
| 错误处理 | thiserror（库）+ anyhow（应用） |
| 测试 | cargo test, proptest, waydriver |
| 代码质量 | rustfmt, clippy, cargo-deny |
| CI/CD | GitHub Actions（唯一构建验证途径） |

---

## 6. 如何贡献

Inox 采用 **AI-first** 工作流，贡献流程如下：

1. 阅读根级 [`AGENTS.md`](./AGENTS.md)，以及目标 crate 目录下的 `AGENTS.md`（crate 级规则只能收紧、不得放宽根级硬性规则）。
2. 涉及架构变更时，先在本仓库 `docs/adr/` 提交 ADR（状态 `proposed` → `accepted`）：
   新增/删除组件 crate、改变分层或依赖方向、引入新协议策略、更换核心依赖（Smithay/Iced/calloop）、改变 IPC 或配置格式的破坏性变更**必须先写 ADR**。
3. 在功能分支（`feat/xxx`、`fix/xxx`、`chore/xxx`、`docs/xxx`）上改动，提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)：`<type>(<scope>): <subject>`。
4. 创建 PR，描述包含：动机、改动、受影响的 crate、测试方式、性能影响（如有）；涉及架构变更时链接对应 ADR。
5. 等待 **CI 全绿 + 至少 1 名人类审查**；不自行合并到 `main`，不使用 `--no-verify` 绕过 CI。

### 硬性规则速查

- **不在本地编译/测试**，验收一律以 CI 为准。
- 默认禁止 `unsafe`（仅 `inox-compositor` / `inox-xwayland` 的 FFI 胶水代码可例外，需 `// SAFETY:` 注释）。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一用 `tracing`，禁止 `println!` / `eprintln!`。
- 依赖许可证白名单：仅 BSD/MIT/Apache-2.0/ISC/Zlib 等宽松许可；禁止 GPL/AGPL/SSPL。
- 用户可见文案必须走 Fluent，禁止硬编码；组件只消费主题令牌（design tokens），禁止硬编码颜色/尺寸。
- 代码注释与标识符用英文；面向人的文档（`README.md` / `AGENTS.md`）用中文。

完整规则见 [`AGENTS.md`](./AGENTS.md)，产品需求见 [`PRD.md`](./PRD.md)。

### 本地命令

> 仅供参考，不作为本地验收依据；真正的判定在 CI。

| 目的 | 命令 |
|------|------|
| 格式化 | `cargo fmt --all` |
| 静态检查 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 全部测试 | `cargo test --workspace --all-features` |
| **单个 crate 测试** | `cargo test -p <crate> --all-features` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 构建 | `cargo build --workspace --release --all-features` |
| 依赖检查 | `cargo deny check` |
| 文档站点 | `mdbook build docs/book` |

CI 的测试矩阵由层级表推导，每个分片执行的正是上表的 `cargo test -p <crate>`，因此本地命令与 CI 命令逐字一致。

### 本仓库的文档检查

`.md` 文件由 CI 工作流 `.github/workflows/docs.yml` 使用 `markdownlint-cli2` 检查（排除 `repos/` 与 `node_modules/`）；规则配置见 [`.markdownlint.json`](./.markdownlint.json)。

---

## 7. 许可证

本项目采用 **BSD 3-Clause** 许可证，版权归 Inox Desktop 贡献者所有。仓库根目录的 [`LICENSE`](./LICENSE) 覆盖工作区内全部 crate。

依赖许可证白名单与校验由 `cargo-deny` 强制执行（仅允许 BSD/MIT/Apache-2.0/ISC/Zlib 等宽松许可，禁止 GPL/AGPL/SSPL 进入核心依赖）。
