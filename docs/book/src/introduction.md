# 简介

Inox 是一个用 Rust（stable，2024 edition）编写、基于 Smithay 与 Iced 的模块化 Wayland 桌面环境，采用 AI-first 方式开发，CI（GitHub Actions）是唯一的构建与测试验证途径。

本站点是 Inox 的架构说明、组件职责、配置参考与开发约定入口，源文件位于 `docs/book/`，使用 mdBook 构建，CI 通过 `mdbook build` 验证。

## 本站点的范围

- 说明整体分层、各组件职责与运行时数据流；
- 汇总 Wayland 协议支持矩阵与配置参考；
- 记录开发流程、CI 门禁、测试策略与依赖管理约定；
- 提供里程碑状态与术语表。

本站点**不复制**产品需求与架构决策的原文，只做索引与解释：需求与决策的正文始终留在仓库根的 `PRD.md` 与 `docs/adr/`。

## 权威来源

文档之间出现冲突时，一律以下列仓库内原文为准：

| 内容 | 权威来源 |
|------|----------|
| 产品需求与验收标准 | [PRD.md](https://github.com/Inox-desktop/inox/blob/main/PRD.md) |
| 架构决策 | [docs/adr/](https://github.com/Inox-desktop/inox/tree/main/docs/adr) |
| AI 代理与贡献者的硬性规则 | [根级 AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) |
| 里程碑的当前状态 | [README 第 4 节](https://github.com/Inox-desktop/inox/blob/main/README.md) |
| 组件版本与兼容性记录 | [Cargo.toml](https://github.com/Inox-desktop/inox/blob/main/Cargo.toml) 的 `[workspace.package]` 与 [Cargo.lock](https://github.com/Inox-desktop/inox/blob/main/Cargo.lock) |
| 单个组件的实现现状 | `crates/<name>/` 的代码、`README.md` 与 `AGENTS.md` |

## 如何阅读

- 想了解整体设计：从[总体架构](./architecture/overview.md)开始，再看[组件清单](./architecture/components.md)。
- 想实际跑起来：直接看[构建与运行](./usage/building.md) —— 目前只有 headless 合成器可以运行，还不是日常可用的桌面。
- 想参与贡献：先读[协作流程](./development/workflow.md)与[持续集成](./development/ci.md)。
- 想查配置项：[配置目录布局](./configuration/layout.md)、[合成器配置](./configuration/compositor.md)、[主题与设计令牌](./configuration/theme.md)。
- 想知道项目走到哪一步：[里程碑与路线图](./roadmap.md)。

## 文档约定

- 正文使用中文；命令、标识符、路径、TOML 键名与代码保持英文原样。
- 关于组件状态的描述必须能在 `crates/<name>/` 中核对。尚未实现的组件会明确标注为**骨架**、**计划中**或**尚未实现**，不把规划中的能力写成已有能力。
- 描述配置项时，若对应实现尚未发布，页面开头会标明它依据的 ADR 与其状态。
