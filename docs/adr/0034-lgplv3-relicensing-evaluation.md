# ADR 0034: 决定将 Inox 重许可为 LGPLv3

- 状态：accepted
- 日期：2026-10-23
- 决策者：架构组（项目所有者 rownix101）
- 批准记录：2026-10-23 项目所有者批准重许可 LGPLv3；本文件由评估改为采纳决策（见下）。
- 关系：经项目所有者拍板，本 ADR 从「评估 BSD→LGPLv3」正式改为「**采纳重许可 LGPLv3**」的决策记录。它与 [ADR 0033](./0033-qml-qt-quick-shell-evaluation.md)（迁移 Qt）联动：Inox 为采用 Qt 而重许可 LGPLv3。将取代根级 `AGENTS.md` §0/§3.6、PRD §15、`LICENSE`、`deny.toml` `[licenses] allow` 与各 crate `AGENTS.md` 的许可证陈述。具体重许可落地由执行 PR 实施，本文件为决策记录。

---

## 背景

紧随 [ADR 0033](./0033-qml-qt-quick-shell-evaluation.md)（评估 Shell GUI 由 Iced 迁到 QML + Qt Quick）的结论，讨论走向许可证层面：若 Inox 要采用 Qt Quick/QML，其许可结构需要重新审视（ADR 0033 §「Qt 许可的客观事实」）。用户在决策中选择评估方向为 **LGPLv3**。

当前许可证相关事实（2026-10-23 从仓库核实）：

- **`LICENSE`**：BSD 3-Clause，`Copyright (c) 2026, Inox Desktop Contributors`。
- **工作区许可证陈述**：
  - 根级 `AGENTS.md` §0：许可证 BSD 3-Clause；§3.6 硬性规则：「仅允许 BSD / MIT / Apache-2.0 / ISC / Zlib 等宽松许可证。**禁止 GPL/AGPL/SSPL** 进入核心依赖。由 `cargo-deny` 强制。」
  - `PRD.md` §15（L579–591）：采用 BSD 3-Clause，`LICENSE` 一份覆盖全工作区，依赖白名单由 cargo-deny 强制。
  - `README.md` §7（L230–234）：同 PRD。
  - `deny.toml` `[licenses] allow`：只有 permissive（Apache-2.0、BSD-2/3、ISC、MIT、Zlib、Unicode、Unlicense、CC0、BSL-1.0、MPL-2.0），**不含 LGPL/GPL**。
- **著作权人**：`git shortlog -sne HEAD` 显示全部 96 次提交仅来自两个同名身份 `rownix101` / `Rownix`，**同一邮箱** `rownix101@gmail.com`。即作者事实上为一人（项目自述 "0 fork"）。因此「重许可需全体著作权人同意」的前置条件在当前是**满足的**（需在评审中由作者书面确认）。

### Qt + LGPLv3 的许可联动（来源：Qt 官方 [Licensing](https://doc.qt.io/qt-6/licensing.html)，Qt 6.12）

- Qt **主体框架**（Qt Core/Gui/Widgets/Quick/QML）以 **LGPLv3 或商业** 双许可提供。
- 部分模块**仅 GPLv3**：**Qt Wayland Compositor、Qt Virtual Keyboard、Qt QML Compiler、Qt Quick 3D、Qt Lottie、Qt Graphs 等**，商业许可下统一为商业许可。
- **关键澄清（本 ADR 必须写明的边界）**：把 Inox **自身**改用 LGPLv3，只能让 Inox 与 Qt 主体的 LGPL 兼容、并允许 cargo-deny 白名单中加入 LGPL/GPL 依赖。它**并不会**解锁上述 GPL-only 模块——因为一个 LGPLv3 应用不能合法地链接 GPLv3-only 的 Qt 模块（LGPL 与 GPL 的结合需要 GPL 侧兼容；GPL-only 模块要求整个派生物以 GPL 发布才可链接）。要用 Qt Virtual Keyboard / Qt QML Compiler 等 GPL-only 能力，需要 GPLv3 或 Qt 商业许可，LGPLv3 不够。

也就是说，**LGPLv3 的价值是「与 Qt 主体同族、允许依赖图出现 LGPL/GPL 库、且保持下游可链接性（弱 copyleft）」**，而不是「解锁全部 Qt」。这与用户引用的 KDE/GNOME 先例一致：**工具库保持弱 copyleft（LGPL），让别人不必 GPL**。

---

## 决策驱动因素与备选

驱动力：

1. 若走 Qt 路线，Inox 本身采用 LGPLv3 与 Qt 主体同族，避免「BSD 项目 + LGPL Qt」跨许可族的不一致。
2. 现状 `deny.toml` 连 LGPL/GPL 都不允许——即便只引一个 LGPL 库都过不了 cargo-deny。改 LGPLv3 需要先决定整个依赖白名单方向。
3. 保持开源友好 + 允许下游链接（KDE/GNOME 的「库 LGPL」智慧）。

不变量：

- 合成器仍 Smithay（根级 §4）；Qt 只可能影响 Shell/L3 主体或未来 GPL-only 模块，且合成器绝不碰 Qt Wayland Compositor（GPL，见 ADR 0033）。
- 分层、`layering.rs`、i18n 走 Fluent、设计令牌唯一来源，一律不变。

### 备选

| 方案 | 收益 | 成本 / 风险 | 选择或否决依据 |
|------|------|-------------|----------------|
| **保持 BSD 3-Clause** | 零变更；最宽松、可再许可、商业友好；下游集成零门槛 | `deny.toml` 不容 LGPL/GPL 库；若 Qt 主体自带少量 LGPL 传递依赖需个案豁免；与 Qt 主体异族 | **未被否决**：若 Inox 只做自选工具栈（保留 Iced），BSD 很健康。只有当确实要链接 LGPL/GPL 三方库时才需要改 |
| **改 LGPLv3（本案评估）** | 与 Qt 主体（LGPL）同族；cargo-deny 可放开 LGPL/GPL 依赖；弱 copyleft，下游仍可链接；匹配 KDE/GNOME 的「库弱 copyleft」智慧 | 不再是「全 permissive」；下游若整体集成 Inox 需遵守 LGPL 义务；需一套「系统依赖许可」审计补上 cargo-deny 管不到的 Qt 系统库 | **采用（作为评估方向）**，前提是作者确认重许可授权 + 未来确实要链接 Qt/LGPL 库 |
| **改 GPLv3（strong copyleft）** | 解锁全部 GPL-only Qt 模块（Virtual Keyboard、QML Compiler）；许可最「严」 | 下游任何集成者须整体 GPL；与 Qt 主体 LGPL 兼容但比它严格；自断随发行版/商用采用；与 KDE/GNOME 先例相悖（它们的库是 LGPL） | **否决（本方向）**：为工具栈买断整个项目 copyleft，成本远超收益；用户已在决策选择 LGPLv3 |

---

## 决策

**本 ADR 正式决定：将 Inox 的项目许可证从 BSD 3-Clause 重许可为 LGPLv3。** 这是为采用 Qt 路线（[ADR 0033](./0033-qml-qt-quick-shell-evaluation.md)）而同步做的许可对齐。具体判断：

1. **这是弱 copyleft，不是 GPL。** LGPLv3 允许下游把 Inox 作为库/组件链接而不整体 GPL（只要履行「可提供源码、可替换」义务）。这与 Qt 主体同族，也符合 KDE/GNOME「工具库弱 copyleft」的业界智慧——它们的工具库（Qt/GTK）都是 LGPL。
2. **LGPLv3 不自动解锁 GPL-only Qt 模块。** 若要 Qt Virtual Keyboard / Qt QML Compiler，仍需 GPL 或 Qt 商业许可。因此本重许可定位为「对齐 Qt 主体 + 放开依赖白名单」，**不是**「拿到全部 Qt」。
3. **著作权前置条件满足。** 单作者（同一邮箱），"0 fork"。重许可的「全体著作权人同意」可实现，需在落地时书面确认。
4. **重许可范围**：替换 `LICENSE` 为 LGPLv3、根级 `AGENTS.md` §0/§3.6、`PRD.md` §15、`README.md` §7、`deny.toml` `[licenses] allow`（加入 `LGPL-3.0`）、各 crate `AGENTS.md`、文档站，并新建「系统依赖许可审计」CI job。以聚焦的「重许可」PR 落地（`chore`/`license` scope）。本文件为决策记录，不在此实施文件改动。

### 触发条件（已满足）

此次重许可的触发条件是「决定走 Qt 路线」（ADR 0033），该条件已由项目所有者确认；同时项目所有者认同长期采用弱 copyleft（对齐 Qt 生态）。因此从 BSD 切到 LGPLv3 的决定成立，进入落地阶段。

---

## 失败、恢复与安全

- **不可逆性**：许可证切换是**单向**的。GPL/LGPL 之后不能无损回退到 BSD（被传染的下游作品无法追回）。一旦接受，旧 BSD 版本的历史副本仍由 Git 保留，但当前主线将永久 LGPL。评审时必须以「可观察但不是今天就做」的方式记录此不可逆性。
- **著作权**：若未来引入任何第三方贡献者，重许可能再一次需要其授权。单作者现状不保证永续；应在 `CONTRIBUTING`/`AGENTS.md` 中约定贡献者同意 DCO/CLA 允许重许可。
- 需要补的**系统依赖许可审计**：cargo-deny 只扫 Cargo 依赖图，管不到系统 Qt .so。下落 Qt 时需新的 CI 工具（apt 清单 + `ldd` 断言 + LICENSE 清单），否则许可义务无法机械验证。

---

## 兼容、迁移与回退

- **兼容**：LGPLv3 与 Qt 主体（LGPL）、Smithay（MIT）、Iced（MIT/Apache）、全部现有 permissive 依赖向前兼容；只要 `deny.toml` 白名单加入 `LGPL-3.0`（可把 GPL-only 依赖单独管控），现有依赖图可继续通过。
- **迁移**：改 `LICENSE` → 根级 `AGENTS.md` §0/§3.6 → `PRD.md` §15 → `README.md` §7 → `deny.toml` `[licenses] allow` → 全部 crate `AGENTS.md` → 文档站。全部改成 LGPLv3 陈述，推出一个「重许可」commit（Conventional Commits，`chore`/`license` scope），CI 需新增许可审计 job。
- **回退**：license 单向不可回退；若有备份，Git 分支可作为实验回退点，但会留下「曾经 LGPL」的历史。评审时应以「不急着做」换取先决条件成熟。

---

## 验收与证据

决策已通过；本条列重许可落地阶段的验收依赖：

| 验收场景 | 责任角色 | E0–E4 | 通过条件 |
|----------|----------|-------|----------|
| 作者书面授权重许可（含 DCO/CLA 同意） | 项目所有者 | —（非技术） | 授权记录归档 |
| `LICENSE` 替换为 LGPLv3 文本且署名正确 | 架构组 | E0 | 文件替换、diff 可见 |
| cargo-deny 全绿（加入 `LGPL-3.0` 白名单后） | 集成 | E0 | `cargo deny check` 通过 |
| 系统依赖许可审计 job | 集成 | E0→E1 | CI 校验 Qt/系统库许可 & LDD 动态链接 |
| 文档 sync（AGENTS/PRD/README/book） | 架构组 | E0 | 无残留 BSD/GPL 禁入矛盾陈述 |

---

## 实施顺序

- **第一步**：项目所有者书面授权重许可（DCO/CLA 同意记录）。
- **第二步**：改 `LICENSE` → 根级 `AGENTS.md` §0/§3.6 → `PRD.md` §15 → `README.md` §7 → `deny.toml`（加 `LGPL-3.0`）→ 各 crate `AGENTS.md` → 文档站；合并一个聚焦的「重许可」PR。
- **第三步**：建系统依赖许可审计 CI job（apt 清单 + `ldd` 断言 + LICENSE 清单）。
- **第四步（联动 Qt）**：交付 ADR 0033 的 Qt 原型，验证 QML + LGPL 合法性。
- **未决项**：Qt GPL-only 模块（Virtual Keyboard / QML Compiler）在 LGPL 下仍不可用——若不需软键盘/AOT 可接受；否则需 Qt 商业许可或 GPL。责任：产品；最迟：M6 无障碍项。

---

## 后果

**正面**：与 Qt 生态（LGPL）同族；cargo-deny 可放开 LGPL/GPL 依赖，为未来链接 Qt 铺路；弱 copyleft 允许下游链接，不牺牲采用面；与 KDE/GNOME「库 LGPL」的业界智慧一致。

**负面**：不再是 BSD 的「全 permissive 可再许可」；下游若整体分发需遵守 LGPL 义务；许可证切换不可逆；需要新建「系统依赖许可审计」来补 cargo-deny 的盲区；可持续的三方贡献者将引入复杂的重许可授权问题。

**影响面**：`LICENSE`、根级 `AGENTS.md` §0/§3.6、`PRD.md` §15、`README.md` §7、`deny.toml`、全部 crate `AGENTS.md`、文档站、CI（新增许可审计 job）、（若走 Qt）cxx-qt/qmetaobject 绑定依赖。

**复审 / 取代触发条件**：

1. 引入 Qt GPL-only 模块（Virtual Keyboard / QML Compiler）→ 需额外评估 GPL 或商用许可，LGPL 不够。
2. 引入任何非作者贡献者 → 先确认其 DCO/CLA 允许重许可。
3. 产品希望保持「全 permissive 可再许可」→ 推翻重许可决策。

---

*本 ADR 记录「将 Inox 重许可为 LGPLv3」的已采纳决策（2026-10-23 项目所有者拍板，作为 ADR 0033 采用 Qt 的许可对齐）。动机与影响基于 2026-10-23 仓库状态（单作者、BSD LICENSE、deny.toml 无 LGPL/GPL）与 Qt 6.12 官方许可文档。重许可文件改动由后续执行 PR 实施。*
