# ADR 0002: L1–L4 分层与依赖方向

- 状态：accepted
- 日期：2026-10-16
- 决策者：架构组

## 背景

ADR 0001 确定了每个组件一个仓库。但如果只有仓库划分而没有明确的依赖方向约束，仓库数量只会把耦合从「crate 之间」搬到「仓库之间」，仍然可能出现反向依赖、横向依赖与循环依赖，最终导致无法独立构建与独立测试 —— 这正是 PRD §1.3「可测试性内建」所要避免的。

PRD §2 规定 Inox 采用四层架构，PRD §2.1 进一步规定「依赖只能向下：`L4 → L3 → L2 → L1`，禁止反向或跨层横向依赖」，并要求「`inox-common` 不得依赖任何其他 Inox crate（叶子节点）」。PRD §2.2 则给出运行时数据流：合成器与 Shell 之间**不通过 Rust 依赖**耦合，而是通过 Wayland 协议与 D-Bus 通信；这部分必须在架构层面被固定下来，否则后续极易被「为了省事直接 `use inox_compositor::...`」破坏。

根级 `AGENTS.md` §2 与 §4 对此给出硬性约束：禁止反向依赖、跨层横向依赖、循环依赖；需要改变分层或依赖方向时必须先写 ADR。

## 决策

### 分层与成员

| 层级 | 仓库 | 职责 |
|------|------|------|
| **L1** 基础设施与共享库 | `inox-common`, `inox-config`, `inox-docs` | 通用类型、错误、日志、配置解析、文档 |
| **L2** 合成器与窗口管理 | `inox-compositor`, `inox-protocols`, `inox-xwayland` | DRM/KMS、输入、渲染、窗口管理、协议绑定、XWayland 集成 |
| **L3** Shell 与用户界面 | `inox-shell`, `inox-panel`, `inox-launcher`, `inox-notifications`, `inox-settings`, `inox-lock`, `inox-wallpaper` | 桌面外壳与用户界面组件（Iced） |
| **L4** 会话与系统集成 | `inox-session`, `inox-idle`, `inox-greet` | 会话管理、空闲管理、显示管理器 |
| 元 / 组织 | `inox`, `.github` | 文档与组织级模板，不参与依赖图 |
| 测试 | `inox-integration` | 端到端集成测试，可依赖所有仓库 |

### 依赖规则

1. 依赖方向**只能向下**：`L4 → L3 → L2 → L1`。允许跨层向下（例如 L3 直接依赖 L1），禁止反向（L1 依赖 L3）与跨层横向（L3 依赖 L3 的其他仓库）。
2. `inox-common` 是**叶子节点**，不得依赖任何其他 Inox crate，也不得依赖 `inox-config`。
3. `inox-compositor` 与 `inox-shell`（及其 L3 子组件）之间**不建立 Rust 依赖**，只通过标准 Wayland 协议（`wlr-layer-shell`、`xdg-shell`）、Inox 自定义的 `inox-ipc` 协议（定义于 `inox-protocols`）与 D-Bus 通信。二者必须能各自独立启动与独立测试。
4. `inox-protocols` 是协议的唯一出口：任何新增协议必须先在 `inox-protocols` 中定义或引用，并说明安全与生命周期约束（PRD §2.3 脚注、根级 `AGENTS.md` §4）。
5. 循环依赖、反向依赖与横向依赖由 CI 检查拦截（`cargo tree` 审计 + `cargo deny check` 的 `[bans]` 段）。
6. L3 组件对 L2 的依赖限于明确的单点：`inox-wallpaper` 依赖 `inox-compositor` 的渲染能力（PRD §3.1）。其余 L3 组件不得直接依赖 `inox-compositor`。
7. `inox-integration` 不受上述方向限制（它是测试聚合仓库），但它只允许在各组件的**发布 tag** 上做验证。

### 变更流程

任何对分层成员、依赖方向或上述规则 1–7 的修改，必须先提交 ADR（状态 `proposed`），获人类批准（PRD §5.4、§5.5）后，同步更新 PRD §2、各仓库 `AGENTS.md` 与 `compat.toml`，再实施代码改动。

## 后果

正面：

- 每个仓库可以独立编译与独立测试，CI 反馈快；L1 变更的影响面有明确上界。
- 合成器与 Shell 的解耦使二者的失败域隔离：Shell 崩溃不会带走合成器，反之亦然，对应 PRD §8「崩溃恢复」预算。
- Shell 组件可在虚拟合成器（`weston --backend=headless`，PRD §6.1、§7）中单独做 waydriver 测试，无需构建完整合成器。
- 分层是 AI 代理判断「我可以引入什么依赖」的单条判据，降低了越界耦合的概率。

负面：

- 协议 / D-Bus 边界使部分跨组件调用需要序列化与版本协商，比直接函数调用更繁琐（但换来了独立可测试性）。
- 依赖必须通过 Git tag 逐层升级，L1 的改动传播到 L4 需要多轮 PR。
- 「跨层横向禁止」在少数场景会迫使引入新的 L1 工具 crate 而非直接复用 —— 需要 ADR 才能新增仓库，流程成本较高。
- 规则 6 对 `inox-wallpaper` 的例外是已识别的技术债：若未来更多 L3 组件需要合成器内部能力，应优先考虑把这些能力下沉到 `inox-protocols`，而不是扩大例外。

影响面：本决策约束所有里程碑（M0–M6）。M1–M3 集中在 L2，M4 集中在 L3，M5 集中在 L4，M6（PRD §13）在 `inox-integration` 中验证全链路方向正确。

取代条件：若出现必须打破「合成器与 Shell 无 Rust 依赖」的强需求（例如引入共享的合成器内部状态模型），需撰写取代本 ADR 的新 ADR，并评估对 headless 测试与失败域隔离的影响。
