# ADR 0001: 采用多仓库（polyrepo）策略与初始仓库清单

- 状态：accepted
- 日期：2026-10-16
- 决策者：架构组

## 背景

Inox 需要在项目伊始确定代码组织形态，因为它直接决定 CI 结构、依赖管理方式、版本策略与 AI 代理的工作边界。可选方案有三类：

1. **单仓库 monorepo（Cargo workspace + 单一 crate 树）**。跨组件重构成本低，但所有组件共享一个 CI 流水线：任何一个组件的编译失败都会阻塞全部组件，且 `cargo-deny`、lint 与 MSRV 策略只能取全集，无法按组件差异化。
2. **少量粗粒度仓库（例如 compositor / shell / libs 三仓库）**。折中方案，但每个仓库内部仍会退化为隐式分层，PRD §2.1 的「依赖只能向下」无法由 CI 在仓库边界上强制。
3. **多仓库 polyrepo（每个组件一个独立仓库）**。边界最清晰，代价是跨仓库依赖需要显式版本锁定。

PRD §1.1 明确要求「通过多仓库拆分实现清晰的模块边界，便于 AI 独立开发与测试」，PRD §1.2 与 §1.3 强调「单一职责的 crate」与「依赖方向严格自底向上」，PRD §3 直接规定「所有仓库位于 GitHub 组织 `Inox-desktop` 下，采用多仓库策略，每个仓库独立版本、独立 CI」。同时 PRD §14 已识别 polyrepo 的主要风险为「跨仓库依赖管理复杂」，并给出的缓解手段是 Git tag 锁定 + 集成仓库统一测试 + 维护兼容矩阵。

PRD §5.1 的 AI-first 原则要求「小步提交、每个 PR 聚焦单一模块，便于 AI 理解和 CI 快速反馈」——这一点在 polyrepo 下天然成立：不同代理可以在不同仓库上并行工作而不产生文件冲突。

## 决策

采用**多仓库（polyrepo）策略**，每个组件对应 GitHub 组织 `Inox-desktop` 下的一个独立仓库，各自独立 `Cargo.toml`、独立 SemVer 版本、独立 CI 流水线与独立 `LICENSE` / `AGENTS.md`。

约定如下：

- 元仓库 `inox` 不包含产品代码，只承载 PRD、ADR、兼容矩阵 `compat.toml` 与项目总览；组织级仓库 `.github` 只承载工作流模板与 ISSUE/PR 模板。二者均无 Rust crate。
- 跨仓库依赖**只允许**使用锁定 `tag`（或 `rev`）的 Git 依赖，禁止未锁定的 `branch = "main"`（根级 `AGENTS.md` §8，PRD §3.2）。
- 每个组件仓库的 `AGENTS.md` 继承根级硬性规则，只允许追加/收紧，不得放宽（根级 `AGENTS.md` §1、§12）。
- 本地并行开发时，各组件仓库的克隆聚合在元仓库的 `repos/` 目录下；`repos/` 全部被元仓库 `.gitignore` 忽略，因此组件仓库的 git 状态与元仓库互不干扰。
- 新增或删除仓库属于破坏性架构变更，必须**先写 ADR**（PRD §5.4、根级 `AGENTS.md` 常见任务配方「新增一个仓库」），并在合并后同步更新 PRD §3.1 与本 ADR 的清单。

### 初始仓库清单

| 仓库名 | 层级 | 职责 | 发布物 |
|--------|------|------|--------|
| `.github` | 组织 | 组织级工作流模板、ISSUE/PR 模板、`CONTRIBUTING` | — |
| `inox` | 元仓库 | 项目总览、路线图、PRD、ADR、兼容矩阵、文档索引 | 文档 |
| `inox-common` | L1 | 共享类型、错误处理、日志、工具函数 | crate |
| `inox-config` | L1 | 配置模型、TOML 解析、默认配置 | crate |
| `inox-docs` | L1 | 用户与开发者文档（mdBook，无 Rust crate） | 文档站点 |
| `inox-compositor` | L2 | Wayland 合成器，基于 Smithay | 二进制 `inox-comp` |
| `inox-protocols` | L2 | 标准协议绑定与自定义 Wayland 协议扩展（含 `inox-ipc`） | crate |
| `inox-xwayland` | L2 | XWayland 集成（基于 xwayland-satellite） | 二进制/服务 |
| `inox-shell` | L3 | Shell 主框架与插件系统 | 二进制 `inox-shell` |
| `inox-panel` | L3 | 面板（任务栏、系统托盘、时钟） | 二进制 `inox-panel` |
| `inox-launcher` | L3 | 应用启动器 | 二进制 `inox-launcher` |
| `inox-notifications` | L3 | 通知守护进程 | 二进制 `inox-notifications` |
| `inox-settings` | L3 | 系统设置应用 | 二进制 `inox-settings` |
| `inox-lock` | L3 | 锁屏（`ext-session-lock-v1`） | 二进制 `inox-lock` |
| `inox-wallpaper` | L3 | 壁纸设置与渲染 | 二进制 `inox-wallpaper` |
| `inox-session` | L4 | 会话管理器，启动合成器与 Shell | 二进制 `inox-session` |
| `inox-idle` | L4 | 空闲管理 | 二进制 `inox-idle` |
| `inox-greet` | L4 | 显示管理器（可选） | 二进制 `inox-greet` |
| `inox-integration` | 测试 | 端到端集成测试，聚合所有组件 | 测试报告 |

规划中但**尚未创建**的仓库（1.0 之后或按需提前，创建前需各自通过 ADR）：`inox-portal`、`inox-screenshot`、`inox-osd`、`inox-cli`。

## 后果

正面：

- 仓库边界即模块边界，PRD §2.1 的「依赖只能向下」「禁止循环依赖」可在仓库粒度上被审查与自动化检查（`cargo tree` / `cargo-deny`）。
- 每个仓库的 CI 独立，单个组件的失败不会阻塞其他组件的迭代，符合 PRD §5.1「CI 快速反馈」。
- 多个 AI 代理可严格并行工作于互不重叠的目录，天然无文件冲突。
- 独立版本与独立发布（PRD §3.3）使组件可被外部项目单独消费。

负面：

- 跨仓库依赖管理复杂度上升（PRD §14 已列为中等风险）：每次上游发布必须先打 tag，下游才能升级（根级 `AGENTS.md` §8 的升级流程）。
- 跨仓库的原子性重构不可能：一个跨 3 个仓库的重构需要 3 个 PR 与 3 次 CI，且中间态可能短暂不可编译。
- 版本漂移风险：必须依赖 `compat.toml` 与 `inox-integration` 的端到端测试来发现不一致的组合。
- 组织级配置（lint、MSRV、许可证白名单）需要在每个仓库重复维护，一致性依赖 `.github` 的 workflow 模板与根级 `AGENTS.md` 的约束力。

影响面：本决策是 M0（PRD §13）「组织与仓库初始化」阶段的全部骨架工作前提；M6 的集成测试与打包依赖 `inox-integration` 与 `compat.toml` 的稳定维护。

取代条件：若跨仓库依赖成本显著超过模块边界带来的收益（例如出现难以收敛的版本矩阵维护负担），应重新评估 monorepo 并撰写取代本 ADR 的新 ADR。
