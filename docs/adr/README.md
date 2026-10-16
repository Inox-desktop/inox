# 架构决策记录（ADR）

本目录存放 Inox 项目的**架构决策记录**（Architecture Decision Record, ADR），采用 MADR 精简格式（中文）。

依据 PRD §5.4「变更管理与 ADR」：ADR 存放于元仓库 `docs/adr/NNNN-title.md`；以下情况**必须先写 ADR 并获得批准**：

- 新增 / 删除仓库；
- 改变分层或依赖方向；
- 引入新的协议策略；
- 更换核心依赖（Smithay / Iced / calloop）；
- 改变 IPC 或配置格式的破坏性变更。

## 状态流转

```text
proposed ──► accepted ──► superseded
```

- **proposed**：已提交、等待评审。此阶段不得据此实施破坏性变更。
- **accepted**：已获人类批准（PRD §5.5：合并批准与架构判断不可委派给 AI），成为项目约束。
- **superseded**：被后续 ADR 取代。被取代的 ADR **不删除、不重写**，只更新其状态字段为 `superseded` 并链接到取代它的 ADR；历史决策链必须保持可追溯。

其他术语：`rejected`（已评审但未采纳）与 `deprecated`（已生效但不再推荐）在需要时可用于简化状态字段，但不属于默认流转路径。

## 如何新增一个 ADR

1. 复制 [`0000-template.md`](./0000-template.md) 为新文件，编号取当前最大编号 + 1，四位零填充：`NNNN-<kebab-case-title>.md`。
2. 填写标题、状态（新建时为 `proposed`）、日期（`YYYY-MM-DD`）、决策者。
3. **背景**必须引用相关 PRD 章节与既有 ADR，说明问题、约束和可选方案；**决策**必须明确、可执行；**后果**必须同时写出正面与负面影响（含受影响仓库与后续里程碑）。
4. 在下方索引表中登记该 ADR。
5. 通过 PR 提交，等待人类审查批准；批准后将状态改为 `accepted`。

## 索引

| 编号 | 标题 | 状态 | 日期 |
|------|------|------|------|
| [0000](./0000-template.md) | 模板（Template） | — | — |
| [0001](./0001-polyrepo-strategy.md) | 采用多仓库（polyrepo）策略与初始仓库清单 | accepted | 2026-10-16 |
| [0002](./0002-layering-and-dependency-direction.md) | L1–L4 分层与依赖只能向下 | accepted | 2026-10-16 |
| [0003](./0003-technology-stack.md) | 技术栈：Rust 2024 + Smithay + calloop + Iced | accepted | 2026-10-16 |
| [0004](./0004-m0-skeleton-and-dependency-policy.md) | M0 骨架与依赖策略（不引入跨仓库/重型依赖） | accepted | 2026-10-16 |
| [0005](./0005-headless-compositor-runtime.md) | M1 的 headless 渲染路径（自建 surfaceless EGL 与离屏渲染目标） | accepted | 2026-10-16 |
| [0008](./0008-m2-input-windows-tiling.md) | M2 的输入事件、多窗口与平铺布局（在 headless 路径上扩展） | proposed | 2026-10-16 |

## 相关文档

- 产品需求：[`../../PRD.md`](../../PRD.md)
- 根级 AI 约束：[`../../AGENTS.md`](../../AGENTS.md)
- 组件兼容矩阵：[`../../compat.toml`](../../compat.toml)
