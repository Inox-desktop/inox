# ADR 0036: Phase 2d — L3 Shell 组件与 inox-greet 从 Iced 迁移到 QML + Qt Quick 的实施计划

- 状态：accepted
- 日期：2026-10-23
- 决策者：架构组（项目所有者 rownix101）
- 批准记录：2026-10-23 项目所有者在本开发会话明确批准本实施计划（「我已批准 docs/adr/0036-component-migration-plan.md，开始实现，可以并行」），并指示按计划**并行**启动各组件迁移 PR；批准记录随首个实施 PR 链接补全。本记录是架构/实施顺序批准，不表示任何迁移步骤的 CI 验收已通过。
- 关系：补充 [ADR 0033](./0033-qml-qt-quick-shell-evaluation.md)（Shell GUI 迁移 QML + Qt Quick）与 [ADR 0035](./0035-cxx-qt-binding.md)（采用 cxx-qt 绑定）。ADR 0033/0034/0035 已批准决策方向；本 ADR 记录**实施顺序与每步验收**，不重复已批准的选型结论。

## 背景

[ADR 0033](./0033-qml-qt-quick-shell-evaluation.md) 决定 L3 Shell 的 GUI（七个 L3 组件 + `inox-greet`）由 Iced 迁移到 QML + Qt Quick、放弃 Iced；[ADR 0035](./0035-cxx-qt-binding.md) 拍板绑定用 **cxx-qt**、并确立「QML 窗口 + LayerShellQt（LGPL）」作 layer-shell 宿主、AT-SPI 走完整 Qt 应用的 QPA 无障碍桥。ADR 0033/0035 均明确「具体代码迁移另行实施」「按组件逐个迁移，每步一个聚焦 PR」，留待本实施计划对齐顺序与验收。

Phase 1（LGPLv3 重许可，ADR 0034）、ADR 0035 与 headless 原型（`crates/inox-qml-prototype`，PR #25）已落地。PR #25 已由人类评审并合并（merge commit `fbd0c64`），原型栈已入 `main`。**Phase 2d 由此可启动。**

当前工作区状态（2026-10-23，本会话从 `origin/main` 核对）：

- 依赖 `iced` 的 crate：`inox-greet`（L4）、`inox-launcher`、`inox-lock`、`inox-notifications`、`inox-panel`、`inox-settings`、`inox-wallpaper`（L3）。`inox-shell` 无 UI、不依赖 `iced`，不受影响。合成器 `inox-compositor` 不改，始终是 Smithay。
- 这些 crate 的 `src` 累计约 5.2 万行（本会话 `wc -l` 实测：panel 6247、launcher 7464、notifications 12035、settings 12320、lock 6994、wallpaper 6507、greet 598）。
- **合成器已接入 `zwlr_layer_shell_v1`**（已提交、已测试）：`inox-compositor` 用 Smithay 的 `WlrLayerShellState` + `delegate_global_dispatch!` 广告该全局，并已有 `tests/layer_shell.rs`（真实客户端覆盖四层像素顺序、独占区/平铺、焦点、卸载重映射、未确认 configure 隔离）。即 layer-shell 的**服务端能力已经在位**。**尚缺的是 waydriver 宿主**：CI 的 weston / M3 E2E job 都在进程内/第三方宿主上驱动合成器或其协议测试，仓库没有「启动真实 L3 Qt 表面 → 截图 → 断言」的 waydriver 宿主（README 提及 waydriver 但仓库无对应实现或测试）。
- 每个含 cxx-qt bridge 的 crate 已按 ADR 0035 §实施执行约束，须在各 crate 的 `[lints.rust]` 将 `unsafe_code = "forbid"` 覆盖为 `"allow"`，并在 crate 级 `AGENTS.md` 记录该绑定例外。

## 决策驱动因素与备选

迁移必须服从 ADR 0035 §实施执行约束与根级硬性规则：领域逻辑留 Rust、经绑定暴露 QML；安全边界（`inox-lock` 锁表面、`inox-greet` 登录表面）保留；只消费设计令牌、禁止硬编码颜色/尺寸；用户文案走 Fluent；保持键盘可导航 + AT-SPI 语义树；系统依赖只允许 LGPL+permissive（`license-audit` 继续校验）。每个组件独立聚焦 PR，CI 为唯一判定。

| 方案 | 收益 | 成本 / 风险 | 选择或否决依据 |
|------|------|-------------|----------------|
| **逐组件迁移，每步独立 PR（采用）** | 每步可评审、可回滚到上一稳定点（ADR 0033 §回退）；失败可归因到单 crate；CI 矩阵按 crate 分片已有基础设施 | 步骤多、周期长；迁移期间 Iced 与 QML 共存 | **采用**：既符合 ADR 0033「每步一个聚焦 PR」「复审条件 3—不与其它工作混在一起」，也符合根级 §9.2。 |
| 一次性替换全部 7 个 crate | 一次到位、避免长期共存 | 单 PR 巨大、不可评审、失败无法定位；违反 ADR 0033 复审条件 3 | **否决** |
| 保持现状（不迁移） | 无迁移成本 | 与已批准决策（ADR 0033/0034/0035）矛盾；Iced layer-shell 无法编译（ADR 0018） | **否决**（决策已过） |

### 组件迁移顺序（本 ADR 建议）

依据「先建公共宿主与验证基座，再按依赖复杂度由小到大」：

1. **公共基座**：合成器侧 layer-shell **已在位**（`WlrLayerShellState` + `tests/layer_shell.rs`，见背景）。公共基座指的是补上 **waydriver 宿主**：启动真实 L3 Qt 表面、截图、断言的真实合成器宿主 job（opt-in E2 job），并接入各组件迁移时复用的 waydriver 基础设施——这是后续 L3 组件成为真实 layer 表面、被截图断言的前提。可在单个 PR 内先建该基座。
2. **wx-simple 先行确认模式（建议）**：`inox-greet`（L4，598 行，最小；不依赖 layer-shell，是可 headless 运行的登录预览）作为**首个完整迁移样本**，端到端确立「cxx-qt bin-only + QML + Fluent + 令牌 + 测试」的模式，供后续组件复用。
3. **layer-shell 组件按依赖/复杂度升序**迁移：`inox-panel` → `inox-wallpaper` → `inox-launcher` → `inox-notifications` → `inox-settings` → `inox-lock`（锁表面，安全边界最敏感）。
4. **收尾 PR**：迁移全部完成后，从依赖图移除 `iced` / `iced_core` / `iced_runtime` / `iced_tiny_skia`（及其 `iced_layershell`），删除 Iced 测试 harness；同步更新 PRD §3.1 / §9、crate 级 `AGENTS.md`、`docs/book`。

> **注意**：具体顺序在人类评审时可调整；本 ADR 只固定「先建基座、greet 先行、按组件逐 PR、安全边界保留、收尾移除 Iced」这几条不可违反的约束。

## 决策

本 ADR **批准并锁定 Phase 2d 的执行方式与顺序约束**（不重新选型）：

1. 迁移一律使用已验证的 **cxx-qt bin-only 模式**（ADR 0035 §实施执行约束 + PR #25 原型的 bin-only 修复）：bridge 通过 `#[cxx_qt::bridge]` 定义，`build.rs` 用 `CxxQtBuilder` + `.file(...)` 声明桥源码；含 bridge 的 crate 在各 `[lints.rust]` 覆盖 `unsafe_code = "allow"`（本绑定下的根级 §3.2 例外），Inox 亲自写的 Rust 不写显式 `unsafe`，FFI 局限在受控 bridge 模块，安全逻辑仍在安全 Rust 侧。
2. **先建公共基座**（独立 PR）：**补 waydriver 宿主**——启动真实 L3 Qt 表面的真实合成器宿主 job（opt-in E2），接入 waydriver 基础设施：截图、断言 L3 Qt 表面。合成器侧 `zwlr_layer_shell_v1` 已在位，不需新增合成器工作（layer-shell 服务端已由 `WlrLayerShellState` + `tests/layer_shell.rs` 覆盖）。LayerShellQt 在 Qt 宿主层用 `LayerShellQt::Window::get(window)` 包装 QML 窗口，其基准数据在真实宿主上以截图断言校验。
3. **`inox-greet` 首迁移**（独立 PR）：最小、可 headless 运行、不依赖 layer-shell；作为端到端模式样本。
4. **layer-shell 组件逐个迁移**（每步一个独立、聚焦、可评审的 PR），顺序 panel → wallpaper → launcher → notifications → settings → lock；`inox-lock` 的会话锁表面（ADR 0025）与其安全/生命周期边界必须以「锁表面，不是普通窗口」对待。
5. **收尾 PR**：全部迁移后移除 Iced 依赖与 Iced 测试 harness，更新 PRD / crate AGENTS.md / docs/book。
6. 每步只改动该 crate 及必要的公共基座，不顺手重构无关代码（根级 §5）。

## 失败、恢复与安全

- **安全边界不变式**：`inox-lock`（会话锁，ADR 0025）与 `inox-greet`（登录表面）在迁移中不得放宽结算/认证/生命周期边界；它们的 QML 层只做渲染与事件，认证/会话决策始终在安全 Rust / `inox-auth` 侧。FFI 只出现在受控 bridge，不把凭据或敏感状态经 QML 属性暴露给不可信 QML。
- **layer-shell 基座失败**：若基座 PR（waydriver 宿主）在 CI 失败或宿主不可用，迁移工作停在基座之后；不以「普通窗口」行为伪装成已生效的放置（ADR 0018 启动自检要求：无可用显示/协议时必须可读报错 + 非零退出码，不得把放置策略当作已应用）。
- **逐组件回退**：每个迁移 PR 独立，迁移后可回滚到上一稳定点（ADR 0033 §回退）；组件级 `git revert` 只影响该 crate，不影响已迁移的兄弟组件。

## 兼容、迁移与回退

- 新依赖（`cxx-qt` / `cxx-qt-build` / `cxx-qt-lib`，MIT OR Apache-2.0）与 Qt 宿主（Qt Core/Gui/Qml/Quick/WaylandClient、LayerShellQt，均 LGPL）进出 `license-audit` / `deny.toml` 白名单；不引入 GPL-only Qt 模块。系统 Qt 包沿用已验证的 `system-deps` qt 分支。
- 迁移期间 Iced 与 QML 并存于不同 crate；`cargo-deny` 与 `license-audit` 全程校验依赖图。
- QML 端设计令牌/颜色：只在 Rust 或 C++ 侧由令牌显式转 `QColor`，QML 内禁止硬编码（根级 §4）。用户文案走 Fluent，随组件一并迁移。
- 无配置/协议格式破坏性变更；私有协议矩阵不因迁移改变。

## 验收与证据

所有证据以 CI 为准（根级 §3.1）；waydriver 属于真实宿主级证据。复合迁移验收：

本 ADR 的验收 ID 使用 **`M`（migration）前缀**，与 `docs/quality/acceptance.md` 中既有的 `D01–D06`（Shell 与设置的用户任务）**刻意分开**：两者编号体系不同，本文件的 `M02`（waydriver 宿主）与 §6 的 `D02`（`inox-launcher`）是不同项，引用时必须带前缀。

| 验收 ID / 场景 | 责任仓库 | E0–E4 等级 | 测试与失败注入 | 通过条件 / 证据位置 |
|----------------|----------|------------|----------------|----------------------|
| M01 合成器 layer-shell（已在位） | `inox-compositor` | E1 | 真实客户端在进程内合成器上覆盖四层像素/独占区/焦点/隔离 | `tests/layer_shell.rs` 通过（已提交、CI 已验证） |
| M02 基座：waydriver 宿主可启动 L3 Qt 表面 | `inox-*`（宿主 job） | E1/E2 | waydriver 在真实合成器宿主启动 L3 Qt 表面，截图、断言 | opt-in E2 job 截图断言通过 |
| M03 greet 迁移端到端 | `inox-greet` | E1 | 迁移后 headless 测试（登录预览、键盘导航）通过；非测试路径无显式 `unsafe` | `cargo test -p inox-greet` + `cargo clippy -D warnings` + release build 全绿 |
| M04 每个 layer-shell 组件迁移 | 对应 crate | E1/E2 | waydriver 截图断言 + 该 crate 单测 | 该 crate CI（test/clippy/doc/build）全绿；放置策略在真实 layer 表面生效 |
| M05 锁表面边界保持 | `inox-lock` | E1/E2 | 锁表面非普通窗口；认证仍由 `inox-auth`；QML 不暴露敏感态 | lock 单测 + 安全审查（ADR 0015 前提） |
| M06 收尾移除 Iced | 工作区 | E0 | 依赖图无 Iced；`cargo deny` / `license-audit` 全绿 | 移除后全工作区 CI 全绿，无 `iced*` 依赖 |

> 以 ADR 0033/0035 的复审条件与 docs/quality/acceptance.md 为准；上表为新迁移行为补充的验收 ID，随具体 PR 细化。未实现前只列证据计划，不虚构结果。

## 实施顺序与未决项

- **第一步先建公共基座**（独立 PR）：补 **waydriver 宿主**（当前所有 L3 组件成为真实 layer 表面、被截图断言的前提）。合成器侧 layer-shell 已在位，故基座聚焦于 waydriver 基础设施与其真实合成器宿主 job 的选型/接线（`labwc` / `sway` 或直接用 `inox-compositor` 自身作宿主，二选一并 opt-in E2 job）。
- **第二次序**：`inox-greet` 首迁移确立模式 → layer-shell 组件 panel → wallpaper → launcher → notifications → settings → lock → 收尾移除 Iced。

**未决项 / 责任 / 最迟解决阶段**：

| 未决项 | 责任角色 | 最迟阶段 |
|--------|----------|----------|
| waydriver 宿主选型（`labwc` / `sway` / `inox-compositor` 自身）与 E2 job 接线 | 合成器维护者 + 人类审查 | M02 基座 PR |
| waydriver 测试基座实现（仓库当前无该基础设施） | Shell/集成维护者 | M02 基座 PR |
| `inox-lock` / `inox-greet` 安全审查通过迁移边界（ADR 0015 前提） | 锁屏/登录维护者 + 安全审查者 | M05 前 |

## 后果

**正面**：确定可评审、可回退的逐组件迁移路径；公共基座（waydriver 宿主）为 L3 真实 Qt 表面补上截图断言能力；在不放松安全边界的前提下按已批准决策把 Iced 换成 QML+Qt Quick。合成器侧 layer-shell 已在位，无需改动合成器。

**负面**：7 个 crate + 公共基座的迁移工作量与周期大；迁移期间 Iced 与 QML 共存增加维护面；需要为每个 crate 覆盖 `unsafe_code = allow` 并守护「Inox 手写 Rust 不写 unsafe」的纪律；waydriver/真实宿主基座本身从零搭建。

**影响面**：七个 L3/L4 crate、CI（waydriver job、layer-shell e2e；合成器 layer-shell 已在位不新增工作）、`deny.toml` / `license-audit`（缺 Iced 收尾）、PRD §3.1 / §9、crate 级 `AGENTS.md`、`docs/book`。合成器仍为 Smithay，不改、不回退到 Qt Wayland Compositor。

**复审触发**：若迁移中发现某组件与 layer-shell/QML 的接合在真实合成器上无法成立，或某个安全边界在绑定层出现绕行可能，须停下该组件并回退到上一稳定点，向人类报告，再决定是否另写 ADR。

*本 ADR 为 Phase 2d 实施计划（accepted，2026-10-23 项目所有者批准）。表中 E0–E4 证据在对应 PR 的 CI 运行前保持未验证；不虚构已验证结果。*
