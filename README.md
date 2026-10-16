# Inox Desktop

**Inox** 是一个用 **Rust (stable, 2024 edition)** 编写、基于 **Smithay + Iced** 的模块化 Wayland 桌面环境。

名称取自「不锈钢」(Inox)，与 Rust 的「锈」形成反差，寓意坚固、耐腐蚀、长期可维护。

- **组织**：<https://github.com/Inox-desktop>
- **许可证**：BSD 3-Clause（见 [`LICENSE`](./LICENSE)）
- **本仓库**：`inox` 元仓库（项目总览、路线图、PRD、ADR、组件兼容矩阵）

产品质量要求见 [PRD 1.2](./PRD.md)、[桌面验收规范](./docs/quality/acceptance.md)与[验收报告模板](./docs/quality/acceptance-report.md)。本轮参考 [Plasma Desktop 固定快照](./docs/quality/plasma-reference.md)，新增 [ADR 0011–0015](./docs/adr/README.md) 均为 proposed，待人类评审；文档中的新门槛不代表 CI 已接线或产品已通过。

> 本仓库是**元仓库**：它不包含产品代码，只承载跨仓库的架构决策与文档。
> 各组件仓库是组织 `Inox-desktop` 下彼此独立的 GitHub 仓库；本地并行开发时它们聚合在本仓库的 `repos/` 目录中，该目录已被本仓库 `.gitignore` 忽略，不进入元仓库的版本控制。

---

## 1. 项目简介

Inox 是一个现代 Wayland 桌面环境，目标是构建模块化、可维护、高性能的桌面体验。项目采用 **AI-first 开发模式**：AI 生成绝大部分代码，人类负责产品决策、架构验收与真实硬件测试。**所有编译、测试、lint 均在 GitHub Actions 中完成，不在本地进行编译验证**。

### 目标

- 构建一个模块化、可维护、高性能的 Wayland 桌面环境。
- 最大化利用 Rust 生态中成熟的合成器库与 GUI 框架（Smithay / Iced / calloop）。
- 通过多仓库拆分实现清晰的模块边界，便于 AI 独立开发与测试。
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
7. **AI 可读性**：命名、目录、文档保持一致性，任何仓库的约定不得与根级 [`AGENTS.md`](./AGENTS.md) 冲突。

---

## 2. 架构分层

Inox 采用四层架构，**依赖只能向下**（`L4 → L3 → L2 → L1`），禁止反向依赖、跨层横向依赖与循环依赖。

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
│  inox-common, inox-config, inox-docs         │
└─────────────────────────────────────────────┘
```

- **Layer 1**：配置解析、通用类型、日志、工具函数，被所有上层依赖。`inox-common` 是叶子节点，不得依赖任何其他 Inox crate。
- **Layer 2**：核心 Wayland 合成器，基于 Smithay，处理 DRM/KMS、输入、渲染、窗口管理与 XWayland 集成。
- **Layer 3**：桌面 Shell，使用 Iced 构建面板、启动器、通知、设置等用户界面组件。
- **Layer 4**：会话管理、显示管理器、空闲管理等系统级服务。

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

## 3. 仓库清单

所有仓库位于 GitHub 组织 `Inox-desktop` 下，采用**多仓库策略**：每个仓库独立版本、独立 CI，跨仓库依赖通过锁定 `tag` 的 Git 依赖管理。

| 仓库名 | 层级 | 职责 | 主要依赖 | 发布物 |
|--------|------|------|----------|--------|
| `.github` | 组织 | 组织级工作流模板、ISSUE/PR 模板、CONTRIBUTING | — | — |
| `inox` | 元仓库 | 项目总览、路线图、PRD、ADR、集成测试入口、文档索引 | 所有子仓库 | 文档 |
| `inox-common` | L1 | 共享类型、错误处理、日志、工具函数 | — | crate |
| `inox-config` | L1 | 配置模型、TOML 解析、默认配置 | `inox-common`, serde | crate |
| `inox-docs` | L1 | 用户与开发者文档 | — | 文档站点（mdBook） |
| `inox-compositor` | L2 | Wayland 合成器，基于 Smithay | `inox-common`, `inox-config`, `inox-protocols` | 二进制 `inox-comp` |
| `inox-protocols` | L2 | 标准协议绑定与自定义 Wayland 协议扩展 | `wayland-rs` | crate |
| `inox-xwayland` | L2 | XWayland 集成（基于 xwayland-satellite） | `inox-compositor` | 二进制/服务 |
| `inox-shell` | L3 | Shell 主框架，插件系统 | `inox-common`, `inox-config`, Iced | 二进制 `inox-shell` |
| `inox-panel` | L3 | 面板（任务栏、系统托盘、时钟） | `inox-shell` | 二进制 `inox-panel` |
| `inox-launcher` | L3 | 应用启动器 | `inox-shell` | 二进制 `inox-launcher` |
| `inox-notifications` | L3 | 通知守护进程 | `inox-shell` | 二进制 `inox-notifications` |
| `inox-settings` | L3 | 系统设置应用 | `inox-shell` | 二进制 `inox-settings` |
| `inox-lock` | L3 | 锁屏 | `inox-shell` | 二进制 `inox-lock` |
| `inox-wallpaper` | L3 | 壁纸设置与渲染 | `inox-compositor` | 二进制 `inox-wallpaper` |
| `inox-session` | L4 | 会话管理器，启动合成器与 Shell | `inox-compositor`, `inox-shell` | 二进制 `inox-session` |
| `inox-idle` | L4 | 空闲管理 | `inox-session` | 二进制 `inox-idle` |
| `inox-greet` | L4 | 显示管理器（可选） | `inox-session` | 二进制 `inox-greet` |
| `inox-integration` | 测试 | 端到端集成测试，聚合所有组件 | 所有仓库 | 测试报告 |

**规划中的仓库**（1.0 之后或按需提前）：`inox-portal`（PORTAL 后端）、`inox-screenshot`（截图/录屏）、`inox-osd`（屏幕提示）、`inox-cli`（`inoxctl` 命令行工具）。

### 组件兼容矩阵

集成仓库 `inox-integration` 消费本仓库根目录的 [`compat.toml`](./compat.toml) 以获知各组件当前兼容版本。组件发布后在 `[components]` 段登记 `<name> = "<tag>"`。

---

## 4. 里程碑

下表概括提议的新口径，完整范围与证据以 [PRD §13](./PRD.md#13-里程碑)和[验收规范](./docs/quality/acceptance.md)为准，避免在总览维护第二份标准。

| 里程碑 | 交付目标 | 必要证据与结论边界 |
|--------|----------|--------------------|
| **M0** | 工程基线 | 真实 CI、骨架/许可证/约束；不代表运行能力 |
| **M1** | Headless 渲染/生命周期 | 输出与像素断言、失败清理、信号退出；不代表真机 |
| **M2** | 窗口/输入语义 | 真实客户端生命周期、popup/焦点、布局边界、坏客户端隔离 |
| **M2.5** | 真实设备闭环 | 后端 CI 覆盖 + 人类真机、可见指针、VT/设备恢复、硬件建档 |
| **M3** | X11/剪贴板/拖放 | Inox 会话真实互通、MIME/取消/异常退出 |
| **M4** | Shell 任务/双屏/配置 | 任务/焦点恢复、热插拔/混合 DPI、配置应用/回退、双语/实际语义 |
| **M5** | 安全会话与日常可用 | 锁屏/认证/挂起故障、权限拒绝、进程监督与稳定性开始验收 |
| **M6** | 发布候选 | 性能/长稳原始数据、支持矩阵、升级/回退、两种分发渠道与人类批准 |

以下“已完成”仅保留历史标准下的进展；新标准的差距与阻塞见 [ADR 复核清单](./docs/adr/README.md#既有提案的复核与实施阻塞)，本次未追认通过。

当前状态：**M0、M1、M2 已完成**。M1 的 headless 合成器实现（创建输出、渲染纯色帧、收到终止信号后干净退出，退出码 0）已合入 `inox-compositor` 的 `main` 且 CI 全绿，渲染路径见 [ADR 0005](./docs/adr/0005-headless-compositor-runtime.md)（现状态 `accepted`）；协议绑定集中于 `inox-protocols`（标准 `wl_*` 再导出 + 私有 `inox-ipc`，见 [ADR 0006](./docs/adr/0006-protocol-strategy.md)，状态 `accepted`），合成器以其为唯一协议来源。M2 在同一 headless 路径上扩展 `inox-compositor`：经 `wl_seat` 处理键盘/指针事件、以 `xdg-shell` toplevel 管理 ≥2 个窗口、按 master/stack 平铺布局（`proptest` 验证），决策见 [ADR 0008](./docs/adr/0008-m2-input-windows-tiling.md)；真实输入设备（libinput/udev）与 DRM/KMS 真实输出仍待后续里程碑。同期 `inox-xwayland` 实现 PRD M3 的 XWayland 侧：以**监督外部 `xwayland-satellite` 进程**（运行时系统依赖）的方式集成无根 XWayland，不链接其 crate，决策见 [ADR 0007](./docs/adr/0007-xwayland-satellite-integration.md)；其端到端验证在 headless 第三方合成器（weston）上进行，剪贴板/拖放与 `-listenfd` 按需激活仍待后续里程碑。`inox-common` 随 M1 新增 `logging` 模块，成为首个跨仓库依赖；`inox-common` `v0.1.0`、`inox-compositor` `v0.1.0` 与 `inox-protocols` `v0.1.0` 均已发布并登记于 [`compat.toml`](./compat.toml)。

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

1. 阅读根级 [`AGENTS.md`](./AGENTS.md)，以及目标仓库根目录的 `AGENTS.md`（仓库级规则只能收紧、不得放宽根级硬性规则）。
2. 涉及架构变更时，先在本仓库 `docs/adr/` 提交 ADR（状态 `proposed` → `accepted`）：
   新增/删除仓库、改变分层或依赖方向、引入新协议策略、更换核心依赖（Smithay/Iced/calloop）、改变 IPC 或配置格式的破坏性变更**必须先写 ADR**。
3. 在功能分支（`feat/xxx`、`fix/xxx`、`chore/xxx`、`docs/xxx`）上改动，提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)：`<type>(<scope>): <subject>`。
4. 创建 PR，描述包含：动机、改动、影响的仓库、测试方式、性能影响（如有）；涉及架构变更时链接对应 ADR。
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

### 本仓库的文档检查

`.md` 文件由 CI 工作流 `.github/workflows/docs.yml` 使用 `markdownlint-cli2` 检查（排除 `repos/` 与 `node_modules/`）；规则配置见 [`.markdownlint.json`](./.markdownlint.json)。

---

## 7. 许可证

本项目采用 **BSD 3-Clause** 许可证，版权归 Inox Desktop 贡献者所有。每个仓库根目录均包含 `LICENSE` 文件，见本仓库 [`LICENSE`](./LICENSE)。

依赖许可证白名单与校验由 `cargo-deny` 强制执行（仅允许 BSD/MIT/Apache-2.0/ISC/Zlib 等宽松许可，禁止 GPL/AGPL/SSPL 进入核心依赖）。
