# 架构决策记录（ADR）

架构决策记录（Architecture Decision Record）存放在 `docs/adr/` 下，采用 MADR 精简格式，正文为中文。本页说明流程与索引，正文一律以 [docs/adr/README.md](https://github.com/Inox-desktop/inox/blob/main/docs/adr/README.md) 为准。

## 什么时候必须先写 ADR

依据 PRD §5.4，以下情况**必须先写 ADR 并获得批准**，再实施改动：

- 新增或删除组件 crate；
- 改变分层或依赖方向；
- 引入新的协议策略；
- 更换核心依赖（Smithay / QML + Qt Quick 与 cxx-qt / calloop）；
- 改变 IPC 或配置格式的**破坏性**变更。

根级 `AGENTS.md` §4 把这条约束复述为硬性规则：未写 ADR 就做架构、依赖方向或协议的破坏性变更，改动会被直接拒绝。

## 状态流转

```text
proposed ──► accepted ──► superseded
```

| 状态 | 含义 |
|------|------|
| `proposed` | 已提交、等待评审。**此阶段不得据此实施破坏性变更。** |
| `accepted` | 已获人类批准，成为项目约束。 |
| `superseded` | 被后续 ADR 取代。被取代的 ADR 不删除、不重写，只更新状态字段并链接到取代它的 ADR。 |

`rejected`（已评审但未采纳）与 `deprecated`（已生效但不再推荐）在需要时可用于简化状态字段，但不属于默认流转路径。

## 如何新增一个 ADR

1. 复制 [0000-template.md](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0000-template.md) 为新文件，编号取当前最大编号加一，四位零填充：`NNNN-<kebab-case-title>.md`。
2. 填写标题、状态（新建时为 `proposed`）、日期（`YYYY-MM-DD`）、决策者。
3. **背景**引用相关 PRD 章节与既有 ADR，说明问题、约束与可选方案；**决策**必须明确可执行；**后果**必须同时写出正面与负面影响。
4. 在 `docs/adr/README.md` 的索引表中登记该 ADR。
5. 通过 PR 提交，等待人类审查批准；批准后把状态改为 `accepted`。

编号 0006、0007 在索引中不存在，这是编号分配的结果，不是索引遗漏。

## 为什么 ADR 不能省

决策的正文如果只存在于 PR 讨论里，后续的代理与贡献者看不到它，只会看到「代码里已经是这样」。ADR 把「为什么」与「当时排除了哪些方案」固定在一个稳定位置：

- 每个 ADR 都要写出**考虑过但未被采纳的方案**，避免后来者重复评估已经否决过的路线；
- 都要写出**取代条件** —— 什么情况下这个决策应该被推翻。这使得推翻一个旧决策是一次可检索的、有据可依的动作，而不是一次无意中的偏离；
- 后果一节必须包含负面项，例如 ADR 0005 明确记录了「headless 渲染路径成为 Inox 自维护代码」「真实 GPU 路径要到后续里程碑才被覆盖」。

## 当前索引

| 编号 | 标题 | 状态 |
|------|------|------|
| [0001](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0001-polyrepo-strategy.md) | 采用多仓库（polyrepo）策略与初始仓库清单 | superseded（见 0016） |
| [0002](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0002-layering-and-dependency-direction.md) | L1–L4 分层与依赖只能向下 | accepted |
| [0003](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0003-technology-stack.md) | 技术栈：Rust 2024 + Smithay + calloop + Iced | accepted（Shell GUI 部分被 0033/0035 取代） |
| [0004](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0004-m0-skeleton-and-dependency-policy.md) | M0 骨架与依赖策略（不引入跨仓库/重型依赖） | accepted |
| [0005](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0005-headless-compositor-runtime.md) | M1 的 headless 渲染路径（自建 surfaceless EGL 与离屏渲染目标） | accepted |
| [0008](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0008-m2-input-windows-tiling.md) | M2 的输入事件、多窗口与平铺布局（在 headless 路径上扩展） | proposed |
| [0009](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0009-configuration-model-and-reload.md) | 配置模型、设计令牌与热重载（L1 配置格式） | proposed |
| [0016](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0016-single-repository-cargo-workspace.md) | 改为单一仓库（Cargo workspace）并取代多仓库策略 | accepted |
| [0017](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md) | M3 的 XWayland 宿主协议、剪贴板与拖放 | proposed |
| [0018](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0018-l3-component-architecture.md) | L3 组件的独立性、共享 UI 边界与 layer-shell 宿主 | proposed |
| [0033](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0033-qml-qt-quick-shell-evaluation.md) | Shell GUI 由 Iced 迁移到 QML + Qt Quick（部分取代 0003） | accepted |
| [0034](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0034-lgplv3-relicensing-evaluation.md) | 重许可为 LGPLv3（为采用 Qt 铺路） | accepted |
| [0035](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0035-cxx-qt-binding.md) | 采用 cxx-qt 作为 Rust ↔ QML 绑定 | accepted |
| [0036](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0036-component-migration-plan.md) | 逐组件迁移的实施顺序与验收（Phase 2d） | accepted |
| [0038](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0038-offscreen-qml-for-layer-and-lock-surfaces.md) | panel/lock 用离屏 QML 保留 layer-shell 与会话锁表面 | accepted |
| [0039](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0039-qml-migration-completion-and-private-abi.md) | 完成全量迁移，并确立私有 ABI / 私有协议的授权边界 | accepted |

上表只列与当前架构直接相关的条目（完整的 0001–0039 索引在 `docs/adr/README.md`）。当前 Shell 的 UI 栈
（QML + Qt Quick、cxx-qt、panel/lock 离屏渲染）由 0033/0035/0036/0038/0039 固定，
落在各 crate 的形态见[组件清单](./components.md)。

索引的权威版本是 `docs/adr/README.md`；本表若与之不一致，以它为准。

配置模型与设计令牌的决策见 ADR 0009（状态 `proposed`），应用与持久化的边界见 ADR 0014（同样 `proposed`）：它们定义了 `config.toml`、`compositor.toml`、`shell.toml`、`notifications.toml` 与 `themes/<name>/theme.toml` 的字段。这些字段模型**已实现并有测试**（`inox-config`），L3 组件已经通过它们读取配置；但决策本身尚未 `accepted`，合成器也尚未读取 `compositor.toml`，因此本站在[配置](../configuration/layout.md)一节逐页标注当前的实现状态，而不是把它们当作已发布行为。
