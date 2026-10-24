# ADR 0033: 决定 Shell GUI 由 Iced 迁移到 QML + Qt Quick

- 状态：accepted
- 日期：2026-10-23
- 决策者：架构组（项目所有者 rownix101）
- 批准记录：2026-10-23 项目所有者批准迁移 Qt；本文件由评估改为采纳决策（见下）。
- 关系：经项目所有者拍板，本 ADR 从「评估保留 Iced」正式改为「**采纳迁移 QML + Qt Quick、放弃 Iced**」的决策记录。这是**部分取代 ADR 0003**（技术栈，accepted）、ADR 0018（L3 component 架构）与根级 `AGENTS.md` §4 的破坏性架构变更。今后 L3 的 GUI 工具包从 Iced 改为 Qt Quick/QML；合成器仍保持 **Smithay**（绝不用 Qt Wayland Compositor）。本文件为决策记录；具体代码迁移另行实施。

---

## 背景

某次讨论提出：Inox 的 L3 桌面 Shell（面板、启动器、通知、设置、锁屏、壁纸、登录）目前用 **Iced** 实现，也许改用 **QML + Qt Quick** 会更好。这一设想有真实的现实动因，仓库里有据可查：

- **`iced_layershell` 无法编译**。ADR 0018 记录：`iced_layershell` 0.19.1 与 0.19.0 的 `iced_exdevtools` 依赖对 `winit-core` 0.31.0-beta.3 的 `non_exhaustive` 枚举 `NativeKeyCode` 做了穷尽匹配（`error[E0004]`，2026-10-16 实测），因此 `inox-panel` / `inox-launcher` / `inox-notifications` / `inox-wallpaper` 四个组件**无法获得 layer-shell 表面**，只能以普通窗口运行，放置策略虽已建模测试但未被应用（`crates/inox-panel/AGENTS.md`、`docs/book/src/usage/building.md`）。
- **AT-SPI 语义导出缺失**。ADR 0018、`crates/inox-greet/AGENTS.md`、`crates/inox-lock/AGENTS.md` 都记录：iced 0.14 没有 AT-SPI 桥，Q01 要求的 role/name/state/action 导出被阻塞。
- **waydriver / 真实宿主驱动证据缺失**。根级 `AGENTS.md` §6 与 PRD §7 要求 Shell 用 waydriver 在虚拟合成器（`weston --backend=headless`）中启动、截图、断言，但 Iced 客户端一直没有接线到 layer-shell 表面（ADR 0018、ADR 0025）。

Qt Quick 在这方面有成熟生态：Plasma/KWin 系确实使用 QML 构建 shell UI，且 Qt Wayland 对 layer-shell / 输入法 / 语义有更久的历史。这是一个值得评估的候选，而非凭空冒出来的念头。

但 Inox 当前是一套**与 Iced 深度绑定**的架构，到 2026-10-23 的状况：

- 根级 `AGENTS.md` §0/§4、PRD §4/§8/§11/§13 把 **Iced + Elm 架构（`Message`→`update`→`view` 单向数据流）** 固化为「Shell 组件必须基于」的技术栈；ADR 0003（accepted，2026-10-16）整篇建立在 Smithay + calloop + Iced 之上，并在「考虑过但未采纳」里记录了**放弃 Iced 违反该 ADR**。
- L3 七个组件 + `inox-greet` 的 `src/view.rs`（Elm 控件树）、`src/view_model.rs`（纯投影）、`src/theme.rs`（令牌→iced 主题映射）四者都是按 Iced 写的；ADR 0018 明确接受「令牌映射与宿主引导在每个组件中各有一份」的重复，但那是**在 Iced 内**的取舍。
- `inox-lock` 专门修炼了一条「离屏进 `wl_shm` 提交到会话锁表面」的路径（ADR 0025），`inox-lock/AGENTS.md` 明确**禁止** `iced_layershell`，锁表面不是窗口。
- `inox-shell`（ADR 0013）**不绘制任何内容**，明确「不得引入 Iced、事件循环或任何绘制依赖」。
- 工作区目前正运行于 `feat/settings-complete` 分支，工作树有约 2.3 万行未提交改动（[`inox-auth` 新 crate、ADR 0026–0032、wallpaper/settings/panel 大改、glass 协议、`data/` 等](https://github.com/Inox-desktop/inox/tree/feat/settings-complete)），处在一个很深的进行中 feature 状态。

本 ADR 的职责是：**客观评估**替换的可行性、成本、许可影响与替代路径。它区分「已经实现的能力 / 本文的提议 / 尚待验证的假设」，不虚构结论。

---

## 决策驱动因素与备选

驱动力（为何要重新评估）：

1. **layer-shell 表面缺失**——Iced 客户端至今拿不到 layer-shell 表面（`iced_layershell` 编不过），这是最实际的痛点。
2. **AT-SPI / 无障碍桥缺失**——Q01 阻塞至今。
3. **waydriver 宿主证据缺失**——GUI 自动化链路没有打通。
4. 单向数据流 / 隐藏复杂度——Iced 的 Elm 架构在巨型设置应用上被 ADR 0003 自认「需要靠模块拆分控制」。

制约（不变量）：

- 合成器仍需 **Smithay**（根级 §4、ADR 0003）；calloop 必须保留作主事件循环（根级 §4）。本 ADR 只评估 **Shell/L3 的 GUI 工具包**，不涉及合成器侧（合成器绝无可能改用 Qt Wayland Compositor，见许可分析）。
- 分层 L1–L4 与「依赖只能向下」的机械断言（`layering.rs`，验收 B04）不改变。
- 设计令牌 `inox_config::Theme`（ADR 0009，accepted）必须继续是 L1 的颜色/尺寸唯一来源；组件禁止硬编码。我强调这一点，因为 QML 默认鼓励代码注入样式，需要明确到底由谁把令牌变成 `QColor`（在 Rust 或 C++ 侧）。
- i18n 必须继续走 Fluent（根级 §7.3、PRD §11.2），占位符不变。QML 有 Qt 自带 Qt::qsTr 与 `.qm`/`.ts`，但这**不是** Fluent，需自建映射或桥。
- 许可白名单：根级 §3.6「禁止 GPL/AGPL/SSPL 进入核心依赖」，由 `deny.toml` 的 `[licenses] allow` 机械强制。

### Qt 许可的客观事实（依据 Qt 官方文档，2026-10-23 核实）

来源：[Qt Licensing](https://doc.qt.io/qt-6/licensing.html)（Qt 6.12）。要点：

- Qt Framework 模块以 **LGPLv3 或商业** 双许可提供。
- 以下模块**仅以 GPLv3 提供**（开源路线）：**Qt Wayland Compositor**、Qt Quick 3D、Qt Quick 3D Physics、Qt Quick Timeline、Qt Virtual Keyboard、Qt Lottie, Qt Canvas Painter、Qt CoAP、Qt Graphs、Qt GRPC、Qt HTTP Server、Qt MQTT、Qt Network Authorization、**Qt QML Compiler**、Qt Quick 3D。商业许可下则全部统一为商业许可。
- Qt 文档以 FDL 1.3；Qt 示例以 BSD-3-clause。

对照到 Inox：

| Qt 模块 | 许可（开源路线） | 在 Inox 的用途 |
|---------|------------------|----------------|
| Qt Core / Qt Gui / Qt Widgets / Qt Quick / Qt QML | LGPLv3 | **Shell UI 本体**（若走 Qt 路线）——LGPLv3 可行但有义务 |
| `QtWaylandClient`（Wayland **客户端**部分） | LGPLv3 | 普通 Qt Wayland 应用建连用——可行 |
| `QtWaylandCompositor`（Wayland **合成器**部分） | **仅 GPLv3** | 合成器（Inox 用 Smithay，**不采用**；此模块绝不可进入 Inox 依赖） |
| `Qt Virtual Keyboard` | **仅 GPLv3** | 若 Shell 要做软键盘/输入法——**禁止**（属 GPL，任何形式都违反根级 §3.6） |
| `Qt QML Compiler` | **仅 GPLv3** | 若用 QML 编译/离线 AOT——**禁止**（QML 解释运行则可避开，但性能路径受限） |

### 备选

| 方案 | 收益 | 成本 / 风险 | 选择或否决依据 |
|------|------|-------------|----------------|
| **A. 保持 Iced 现状** | 零迁移成本；已成型的 Elf 架构、离屏渲染测试装置（`iced_tiny_skia` + `iced_runtime::UserInterface`）、ADR 0025 锁屏路径全部保留；许可干净（MIT/Apache+BSD） | layer-shell 表面与 AT-SPI 桥至今缺失；`iced_layershell` 编不过需自建 sctk 表面（ADR 0018 的选项③）或等上游；waydriver 链路未打通 | **未否决**：可作为「另写一个自建 layer-shell 宿主 + 语义导出桥的原型」路线继续，成本比整机换 Qt 低一个量级。下表 B/C 的收益大多可用小幅 Iced 工作获得 |
| **B. 全部 L3 从 Iced 迁到 QML + Qt Quick（经 Rust 绑定）** | QML 声明式 UI 表达较强；Qt 生态对 layer-shell / 输入法 / 语义树有更久历史；Plasma 先例 | 见下「决策」；迁移成本巨大；**LGPLv3 引入现有纯 permissive 栈**；`inox-lock` 锁表面 + greeter 的离屏/掩码路径要推翻重建；`inox-shell` 的「不绘制」约束需重审；MSRV/CI/系统依赖（apt 装 Qt 系）全变 | **本文评估为「不推荐现在就换」，原因见下** |
| **C. 混合：让 QML 只接 layer-shell 表面，其余保留 Iced** | 主 UI 仍 Iced，Qt 只补缺 | 两个 GUI 运行时并存于同一组件，样式/主题/事件树各一套，维护两套依赖面；许可证仍引入 LGPLv3；收益有限 | **否决**：违背「一个 UI 栈」的清晰边界，且没能躲过许可成本；要补缺失的 layer-shell/AT-SPI 用方案 A 的原型更划算 |

### 为何 K（合成器）绝不可进 Qt Wayland Compositor

Qt Wayland Compositor 仅 GPLv3。如果 Inox 因为「换 QML」而顺手把合成器也迁到 Qt Wayland Compositor，那会**直接违反根级 `AGENTS.md` §3.6**（GPL 禁入核心），并且推翻 ADR 0003 的 Smithay 决策与 `inox-compositor` 的大量工作。本 ADR 明确：**QML 选项只适用于 Shell 客户端 UI；合成器永远保持 Smithay**。任何把合成器拉到 Qt 的改写都需另立一份 ADR 并接受 GPL 的架构级后果。

---

## 决策

**本 ADR 正式决定：L3 桌面 Shell 的 GUI 从 Iced 迁移到 QML + Qt Quick，放弃 Iced。合成器保持 Smithay。** 这一决定由项目所有者随「主要因为基于 Rust 的 GUI 库普遍不成熟」的考量拍板——Rust GUI 生态（Iced/egui/Slint 等）在 layer-shell 集成、AT-SPI 语义、输入法、成熟度上尚不足以支撑一套完整桌面 Shell，而 QML + Qt Quick 经过 Plasma/KWin 长期验证，是为桌面 Shell 而生的成熟栈。具体迁移执行另立实施计划，本文件为决策记录。

### 技术路线

- **QML + Qt Quick 承担七个 L3 组件的 UI + `inox-greet`**。从 Rust 使用 QML 经绑定实现：`qmetaobject-rs`（woboq，纯 Rust 注册 QMetaObject）或 `cxx-qt`（KDAB，C++/Rust 互操作 + QML）——最终选择在实施阶段按对 Qt 6.x 的稳定支持与 layer-shell 加持证据定并固定 lock。领域模型与逻辑仍留在 Rust，经绑定暴露给 QML。
- **合成器永远保持 Smithay**，绝不用 Qt Wayland Compositor（GPL-only）与 Iced 之外的变化。calloop 主事件循环、分层、`layering.rs`、Fluent i18n、设计令牌唯一来源等不变量不变。
- **layer-shell**：用 Qt Wayland Client + 自建 `wlr-layer-shell-unstable-v1` 集成获得表面；`iced` 系（`iced_layershell`）相关代码在迁移完成后移除。
- **锁屏（`inox-lock`）/ 登录（`inox-greet`）安全边界**：迁移时重新落地 ADR 0029 的 `Secret` 掩码模型（QML 的 `TextField` 默认持可复制明文，需专门处理）、「锁表面不是窗口」的要求（ADR 0025）。

### 许可影响（与 ADR 0034 联动）

- Qt 主体为 LGPLv3。经 [ADR 0034](./0034-lgplv3-relicensing-evaluation.md) 的评估，Inox 自身将**重许可为 LGPLv3** 以与 Qt 同族，并放开 `deny.toml` 白名单接受 LGPL/GPL 库。这缓解本 ADR 此前的「LGPL 触碰 permissive 白名单」阻碍。
- 即便 Inox 是 LGPL，**GPL-only 的 Qt 模块（Qt Virtual Keyboard / Qt QML Compiler / Qt Wayland Compositor）仍不可用**；若未来需要软键盘/AOT 等 GPL-only 能力，需单独裁决 GPL 或 Qt 商业许可。
- **cargo-deny 管不到系统 Qt**，需要新建「系统依赖许可审计」CI job（apt 清单 + `ldd` 断言 + LICENSE 清单）——这是实施阶段必须补的合规工具链。

### 迁移的现实成本（接受为代价）

- 七个 L3 组件 + `inox-greet` 的 `view.rs`（Elm 控件树）、`view_model.rs`（投影）、`theme.rs`（令牌→ied 映射）需按 Qt 重写；令牌 → `QColor`/`QPalette` 映射边界需明确设计（QML 内不硬编码色值）。
- `inox-lock` 的离屏 `wl_shm` 锁屏路径（ADR 0025）与 `inox-shell` 的「不绘制」约束需重新审视。
- CI 增加系统 Qt 依赖面（apt 装 `libqt6*`/`qml6-module-*`）与许可审计。
- 当前 `feat/settings-complete` 工作树约 2.3 万行未提交改动：迁移不得与其混淆，需单独 PR 序列执行。

---

## 失败、恢复与安全

因本 ADR 不实施迁移，本节点主要记录「若将来真的走 Qt，必须保持的安全边界」：

- **锁屏（`inox-lock`）+ 登录（`inox-greet`）**：ADR 0029（accepted）明确口令**只以 `Secret` 形式存在、永不进任何会保留明文的可复制编辑器**（在 Iced 里是 `iced::widget::text_input`；在 QML 里对应 `TextField`）。迁移时这条不可放宽：QML 的 `TextField`/`TextInput` 默认持有可复制、可粘贴的明文 `String`，且 Qt 的剪贴板会被应用共享——这比 Iced 的 `text_input` 更危险。若走 Qt，掩码 + `Secret` 边界必须重新设计并再过 ADR 0029 的安全审查。
- **锁表面不是窗口**：ADR 0015/0025 拒绝 layer-shell 遮罩。Qt 的 layer-shell 集成必须同样证明「锁表面是会话锁表面，不是普通窗口」，否则同样的「看起来像锁屏但不是锁屏」风险会以另一种形态回归。
- 合成器侧不采用 Qt Wayland Compositor（GPL），合成器安全模型（input seat、锁状态、`inox-glass-v1`）保持不变。

---

## 兼容、迁移与回退

### 现实成本与迁移要求

- **许可证兼容**：当前 `deny.toml` 无 LGPL/GPL。经 [ADR 0034](./0034-lgplv3-relicensing-evaluation.md)，将把 LGPLv3 纳入白名单并新建「系统依赖许可审计」（Cargo deny 管不到系统 Qt）。未核验的项不能当已批准特例。
- **Fluent i18n**：QML 的 `qsTr`/`.qm` 不是 Fluent。迁移须把 `inox_common::i18n` 的 Fluent 目录桥接到 Qt，或保留 Rust 侧查表传给 QML——需保持 `en-US`/`zh-CN` 占位符一致（根级 §7.3、Q03）。
- **设计令牌**：`inox_config::Theme` 必须保持唯一来源（ADR 0009）。Qt 侧在 Rust/C++ 的「令牌→`QColor`/`QPalette`」映射里读令牌，QML 内不得硬编码色值。
- **回退**：决策已过，无法无损回退到 Iced；实施过程中分阶段可回滚到上一稳定点（走 feature 分支 + 独立 PR），整栈回退仅存在于尚未动工前的此刻。

---

## 验收与证据

决策已通过；本条列实施阶段的验收证据计划（E2/E3，须真实取得，不虚构）：

| 验收 ID / 场景 | 责任仓库 | E0–E4 等级 | 测试与失败注入 | 通过条件 / 证据位置 |
|----------------|----------|------------|----------------|----------------------|
| QML layer-shell 兼容 | L3 组件 + `inox-compositor` | E2→E3 | 在 `sway`/`labwc` 等支持 layer-shell 的宿主或 Inox 合成器上启动一个最小 `qmetaobject-rs`/`cxx-qt` QML 层 | 真实 layer 表面出现、锚点/独占区生效、不回落为普通窗口 |
| QML AT-SPI 语义 | Q01 | E2→E3 | `at-spi2` 工具读取 QML 组件 role/name/state | role/name/state/action 可导出并可驱动 |
| LGPL 合规构建 | CI + distribute | E0→E1 | 静态 `deny` + 系统依赖许可审计；动态链接断言（`ldd`） | 合规清单、README、LICENSE 声明齐全；Qt 无 GPL-only 模块入图 |
| Rust 绑定版本锁定 | 各 L3 | E0 | 绑定固定在 Qt 6.x lock 且可复现编译 | 依赖图无 semver 漂移，无 `iced_layershell` 式无法编译 |

实施开始前需以原型取得 QML layer-shell 与 AT-SPI 的最小证据（E2），确认绑定选型后再展开全量迁移。

---

## 实施顺序

- **第一步（许可前置）**：经 [ADR 0034](./0034-lgplv3-relicensing-evaluation.md)，将 Inox 重许可 LGPLv3、放开 deny 白名单、新建系统依赖许可审计。这是 Qt 引入的前提。
- **第二步（原型验证）**：用 `qmetaobject-rs` / `cxx-qt` 做一个最小 QML layer-shell + AT-SPI 证据原型（E2），选型锁定绑定，验证 Qt 6.x 与 layer-shell 加持。
- **第三步（逐组件迁移）**：按组件独立 PR 迁移七个 L3 + `inox-greet`，每 PR 聚焦一件事（根级 §9）；先迁移无需安全边界的组件（设置/壁纸/shell 框架），锁屏（`inox-lock`）与登录（`inox-greet`）的 `Secret` 掩码与「锁非窗口」边界单独严格评审再动。
- **第四步（清理）**：待全量迁移完成后移除 Iced（`iced`/`iced_core`/`iced_runtime`/`iced_tiny_skia`）依赖与相关测试装置。
- **阻塞**：重许可 LGPL（ADR 0034）与 Qt 绑定原型若不过，Qt 迁移不能铺开。
- **未决项**：
  - cxx-qt / qmetaobject-rs 对 Qt 6.x 的当前 GA 版本、维护者、许可证、layer-shell 加持证据 —— 责任：架构组；最迟：原型选型阶段。
  - Qt 系统库许可审计在 CI 的实现方式 —— 责任：集成维护者；最迟：Qt 引入时。
  - 软键盘（`Qt Virtual Keyboard` 是 GPL）在 Inox 是否需要 —— 责任：产品；最迟：M6 无障碍项。

---

## 后果

**正面**：决定采用经过 Plasma/KWin 长期验证的成熟桌面 Shell 工具栈（QML + Qt Quick），弥补 Iced 在 layer-shell 集成、AT-SPI 语义、输入法、成熟度上的缺口；Rust 领域模型保留，UI 交给声明式 QML；合成器仍 Smithay 不受影响。

**负面**：撤销 Iced 全栈（七个 L3 组件 + `inox-greet` 的 `view.rs`/`view_model.rs`/`theme.rs` 重写）；推翻 ADR 0025 锁屏离屏路径、greeter 掩码处理、原 `inox-shell` 不绘制约束需重审；CI 增加系统 Qt 依赖面；需新建系统依赖许可审计；锁屏/登录的 `Secret` 掩码与「锁表面不是窗口」两条安全边界要在 QML 下重设计并通过 ADR 0029 安全审查；配合 [ADR 0034](./0034-lgplv3-relicensing-evaluation.md) 的重许可。

**影响面**：根级 `AGENTS.md` §4、PRD §4/§11/§13、`deny.toml`、`docs/book`、七个 L3 crate + `inox-greet` 的 `Cargo.toml`/`AGENTS.md`、CI 工作流。**本 ADR 只批准决策方向，不实施代码迁移**；具体迁移单独 PR 序列。

**复审/取代触发条件**：

1. cxx-qt 或 qmetaobject-rs 稳定支持 Qt 6 且对 layer-shell/语义有证据 → 作为实施阶段选型的依据。
2. 需用 Qt GPL-only 模块（Virtual Keyboard、QML Compiler）→ 单独裁决 GPL 或 Qt 商业许可。
3. 产品要求「全 permissive 不再分发依赖 LGPL」→ 推翻 Qt 决策。

---

*本 ADR 记录「迁移 QML + Qt Quick、放弃 Iced」的已采纳决策（2026-10-23 项目所有者拍板）。动机与成本基于 2026-10-23 仓库状态与 Qt 6.12 官方许可文档；具体代码迁移由实施 PR 执行，本文件不含虚构的通过结果。*
