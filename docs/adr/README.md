# 架构决策记录（ADR）

本目录存放 Inox 项目的**架构决策记录**（Architecture Decision Record, ADR），采用中文 MADR 格式，包含生命周期、失败/安全、迁移和验证证据。

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
- **accepted**：已获明确人类批准（PRD §5.5：合并批准与架构判断不可委派给 AI），成为项目约束。新决策须链接批准 PR/评论与日期；合并 proposed 文档、CI 全绿或实现已合入均不自动等于架构接受。
- **superseded**：被后续 ADR 取代。被取代的 ADR **不删除、不重写**，只更新其状态字段为 `superseded` 并链接到取代它的 ADR；历史决策链必须保持可追溯。

若仅有个别条目被取代，旧 ADR 保持 `accepted` 并在元信息登记取代范围和后继链接，其余条目继续有效；全部条目失效时才将整份标为 `superseded`。proposed 的后继尚不改变旧决策效力。

其他术语：`rejected`（已评审但未采纳）与 `deprecated`（已生效但不再推荐）在需要时可用于简化状态字段，但不属于默认流转路径。

## 如何新增一个 ADR

1. 复制 [`0000-template.md`](./0000-template.md) 为新文件，编号取当前最大编号 + 1，四位零填充：`NNNN-<kebab-case-title>.md`。
2. 填写标题、状态（新建时为 `proposed`）、日期（`YYYY-MM-DD`）、决策者、批准记录与关联/取代范围。
3. **背景**引用 PRD 与既有 ADR，比较真实备选；**决策**明确权威状态、生命周期、错误/恢复、安全边界、版本/迁移/回退；**验收**关联场景 ID、证据等级、责任仓库与失败注入；**后果**同时包含收益、成本和复审触发条件。章节可合并，内容义务不能省略。
4. 在下方索引表中登记该 ADR。
5. 通过 PR 提交，等待人类审查批准；批准后将状态改为 `accepted`。

## 审查清单与证据约定

- 问题/范围是否具体？是否区分已实现能力、架构提议、性能预算和测量结果？备选是否含保持现状及真实可行方案？
- 谁拥有状态和数据？哪个服务执行授权？对象销毁、超时、重复/旧事件、部分失败、资源清理和恢复有无确定行为？
- 依赖方向是否满足根级规则？Cargo 依赖是否与进程/协议协作分开？具体 API 是否在固定源码中存在？许可证是否核实，是否把系统依赖遗漏在 Rust 依赖图之外？
- 配置/协议/默认值变化有无版本、历史 fixture、升级顺序和回退？headless、第三方宿主和真机的结论边界是否清楚？
- 有无[验收 ID 与证据计划](../quality/acceptance.md)，包括拒绝/故障路径？未决问题是否有责任角色、最迟解决阶段和阻塞条件？

接受状态表示架构选择获准，不表示实现完成。实现 PR、CI run、验收报告、硬件 Issue 分别链接。既有 accepted ADR 保留历史正文；新决策部分取代时须指出具体条目，接受后再更新旧决策状态/关系，不回写历史理由。

## 索引

| 编号 | 标题 | 状态 | 日期 |
|------|------|------|------|
| [0000](./0000-template.md) | 模板（Template） | — | — |
| [0001](./0001-polyrepo-strategy.md) | 采用多仓库（polyrepo）策略与初始仓库清单 | superseded（见 0016） | 2026-10-16 |
| [0002](./0002-layering-and-dependency-direction.md) | L1–L4 分层与依赖只能向下 | accepted | 2026-10-16 |
| [0003](./0003-technology-stack.md) | 技术栈：Rust 2024 + Smithay + calloop + Iced | accepted | 2026-10-16 |
| [0004](./0004-m0-skeleton-and-dependency-policy.md) | M0 骨架与依赖策略（不引入跨仓库/重型依赖） | accepted | 2026-10-16 |
| [0005](./0005-headless-compositor-runtime.md) | M1 的 headless 渲染路径（自建 surfaceless EGL 与离屏渲染目标） | accepted | 2026-10-16 |
| [0006](./0006-protocol-strategy.md) | Wayland 协议策略与 inox-ipc（协议绑定集中于 inox-protocols） | accepted | 2026-10-16 |
| [0007](./0007-xwayland-satellite-integration.md) | XWayland 集成采用监督外部 xwayland-satellite 进程（不链接其 crate） | proposed | 2026-10-16 |
| [0008](./0008-m2-input-windows-tiling.md) | M2 的输入事件、多窗口与平铺布局（在 headless 路径上扩展） | proposed | 2026-10-16 |
| [0009](./0009-configuration-model-and-reload.md) | 配置模型、设计令牌与热重载（L1 配置格式） | proposed | 2026-10-16 |
| [0010](./0010-real-backend-drm-libinput.md) | M2.5 的真实后端（libseat session + DRM/KMS 输出 + libinput 输入） | proposed | 2026-10-16 |
| [0011](./0011-evidence-based-acceptance.md) | 以用户场景、证据等级与支持矩阵验收桌面 | proposed | 2026-10-16 |
| [0012](./0012-output-input-and-frame-lifecycle.md) | 输出拓扑、窗口焦点与帧调度采用显式生命周期 | accepted | 2026-10-16 |
| [0013](./0013-shell-service-and-interaction-contracts.md) | Shell 按服务契约协作，并验收完整用户任务 | proposed | 2026-10-16 |
| [0014](./0014-configuration-apply-and-migration.md) | 配置应用、持久化与升级迁移具有失败原子性 | proposed | 2026-10-16 |
| [0015](./0015-session-lock-and-failure-boundaries.md) | 会话监督、锁屏与特权能力按失败边界设计 | proposed | 2026-10-16 |
| [0016](./0016-single-repository-cargo-workspace.md) | 改为单一仓库（Cargo workspace）并取代多仓库策略 | accepted | 2026-10-16 |
| [0017](./0017-m3-xwayland-host-clipboard-and-dnd.md) | M3 的 XWayland 宿主协议、剪贴板与拖放 | proposed | 2026-10-16 |
| [0018](./0018-l3-component-architecture.md) | L3 组件的独立性、共享 UI 边界与 layer-shell 宿主 | proposed | 2026-10-16 |
| [0019](./0019-multi-output-topology-and-multi-gpu.md) | 多输出拓扑、按输出的帧生命周期与多 GPU 设备模型 | proposed | 2026-10-17 |
| [0020](./0020-fractional-scaling.md) | 分数缩放采用 `wp_fractional_scale_v1` 与按输出的缓冲区像素 | proposed | 2026-10-17 |
| [0021](./0021-l4-session-services.md) | L4 会话监督、空闲策略与登录边界的首版实现 | proposed | 2026-10-17 |
| [0022](./0022-m5-session-lock-notification-bus-and-first-privilege-boundary.md) | M5 的会话锁协议腿、通知总线服务与首个特权边界 | proposed | 2026-10-17 |
| [0023](./0023-input-method-support.md) | 提前接入桌面输入法 | proposed | 2026-10-17 |
| [0024](./0024-settings-coverage-and-input-method-preferences.md) | 设置覆盖现有配置，并持久化输入法启动偏好 | proposed | 2026-10-17 |
| [0026](./0026-live-configuration-reload.md) | 运行中配置重载与逐消费方确认 | proposed | 2026-10-17 |
| [0027](./0027-zbus-6-migration.md) | 迁移 zbus 6 并统一提升 MSRV 至 1.89 | accepted | 2026-10-17 |
| [0035](./0035-cxx-qt-binding.md) | 决定采用 cxx-qt 作为 QML + Qt Quick 的 Rust 绑定 | accepted | 2026-10-23 |
## 既有提案的复核与实施阻塞

以下是新标准发现的差距，**不是**修改 accepted 历史正文或批准未决实现。ADR 0011、0013–0015 仍为 proposed；ADR 0012 已于 2026-10-17 获当前开发会话批准；彼此兼容性与下列事项须在人类评审中解决。

| 对象 | 要复核的具体问题 | 责任角色 / 最迟阶段 | 处理方向 |
|------|------------------|--------------------|----------|
| ADR 0001、PRD §1.1/§3、根级 AGENTS.md §2/§8 | 多仓库策略的维持成本已在 M0–M2 实测中超过模块边界收益（版本矩阵失配、57% 样板重复并已分叉、依赖方向无机械强制） | — / **已解决** | **已由 ADR 0016（accepted）解决**：改为单一 Cargo workspace，`compat.toml` 与 `repos/` 流程已删除，PRD 与根级规则已同步改写。剩余动作见 0016 实施记录「尚未完成、需要人类动作」：组件仓库归档、CODEOWNERS、分支保护检查名、CI 成本实测 |
| ADR 0002、组件表 | 禁止同层依赖与 L3→Shell 列表、wallpaper→compositor 例外存在歧义 | 架构组 / M4 实现前 | ADR 0013 提议以进程/协议协作明确边界；部分取代需显式批准 |
| ADR 0002 规则 1（层内顺序） | **四层模型无法表达层内顺序**：`inox-config → inox-common`（L1→L1）已存在，`inox-compositor → inox-protocols`（L2→L2）在组件表中列出。规则 1 的字面表述禁止同层依赖，二者矛盾。ADR 0016 实施后该冲突会由 `layering.rs` 直接变红 | 架构组 + 人类审查者 / `inox-compositor → inox-protocols` 落地前 | ADR 0016 实施记录列出三个裁决方向（层级表改为层内有序列表 / 只改规则措辞 / 维持例外登记）；裁决前同层边只能逐条登记在 `[workspace.metadata.inox.rules] same_layer_exceptions`，且每条需 ADR |
| ADR 0007 | 第三方宿主就绪、仅 X11 socket 存在不足以证明 Inox 会话互通；运行时版本/许可清单待纳入发布证据 | XWayland 维护者 / M3 | X01–X03、L01；实际 X11 往返与真实 Inox 客户端 |
| ADR 0008 | headless 注入不能证明真机；固定 US keymap 不能替代用户键位；旧两窗口断言不足以证明完整生命周期 | 合成器维护者 / M2 差距复核、M2.5 | W01–W04、O01；ADR 0012 |
| ADR 0009 | 内存快照原子替换≠设备成功应用；符号链接边界、深色令牌回退和旧配置迁移未闭环 | 配置维护者 / M4、首次格式变更前 | C01–C04；ADR 0014 |
| ADR 0010 | `Session::auto()` / `Session::Direct` 假设与 Smithay 0.7 的 `Session` trait / `LibSeatSession` 需纠正；固定后端顺序、API/feature、keycode 单位、unsafe 数量和系统许可说明须逐项核验 | 合成器维护者 + 人类审查者 / M2.5 实现前 | [Smithay 0.7 源码依据](./0012-output-input-and-frame-lifecycle.md#上游核验依据)；不以草案 API 伪代码当实现计划 |
| ADR 0010 | 设备失败静默 headless、无可见光标、VT 延后、vblank 唯一帧源、默认输出堆叠、lid close 退出与桌面可用性冲突 | 合成器/会话维护者 / M2.5，双屏 M4 | ADR 0012、0015 提议的 ready/首帧/VT/输入/电源边界优先完成评审；O01–O04、S03 |
| ADR 0012 | 复审条件「引入多 GPU / 多 seat、HDR/VRR、远程输出或新的渲染后端时，另写 ADR」已被触发：多 GPU 与多输出已实现 | 合成器维护者 + 人类审查者 / ADR 0019 评审时 | **已由 ADR 0019、0020（均 `proposed`）承接**：多输出拓扑、按输出帧生命周期、多 GPU 设备模型与分数缩放各有提案。跨 GPU 缓冲复制路径（`MultiRenderer`）明确留待 `linux-dmabuf` 落地前解决；HDR/VRR、多 seat、远程输出仍不在范围内 |
| M4 工具链 | Iced layer-shell 在本工作区**无法编译**：`iced_layershell` 0.19.1 与 0.19.0 的 `iced_exdevtools` 依赖对 `winit-core` 0.31.0-beta.3 的 `non_exhaustive` 枚举 `NativeKeyCode` 做穷尽匹配（`error[E0004]`，2026-10-16 实测）。AT-SPI 实际导出与 waydriver/宿主能力仍无证据 | Shell/测试维护者 / M4 实现前 | 本次改走可用的替代路径：**离屏渲染**（`iced_tiny_skia` + `iced_runtime::UserInterface`）在 CI 中覆盖真实控件树的布局、像素与事件，无需合成器与 GPU；layer-shell 表面、AT-SPI 语义导出与 waydriver 仍是阻塞项，见 ADR 0018 |
| L3 的 layer-shell 宿主 | `inox-compositor` 尚未广告 `zwlr_layer_shell_v1`（绑定已在 `inox-protocols`，合成器未接入），CI 也没有支持该协议的宿主（`weston` 不支持），因此面板、启动器、通知与壁纸在 Inox 会话里无法成为真实表面 | 合成器维护者 + 架构组 / E2/E3 证据之前 | 合成器侧接入 Smithay 的 `wlr_layer`（层排序、独占区、按输出分配）需要更新协议矩阵或另写 ADR；宿主选型（`labwc` / `sway`）需要一个 opt-in 的 E2 job。在此之前 L3 只交付放置策略的建模与测试，不声称放置已生效 |
| M5 认证与授权 | 真实认证边界（PAM、helper 权限、PAM 服务文件、greeter 宿主、锁屏解锁接入） | 锁屏与登录维护者 + 安全审查者 / M5 收尾前 | ADR 0015 的评审项已由 **ADR 0029（`accepted`，2026-10-17）** 记录并实施：`inox-auth` + `auth-v1` + `inox-greet` 口令字段。**仍未完成**：greeter 宿主、锁屏解锁接入本边界、真机登录验收（ADR 0029 §验收 §6） |
| CI/硬件/性能 | 新门槛尚未接线，缺固定设备、长稳/性能原始数据与 E4 支持清单 | 集成维护者 + 人类验收者 / 对应里程碑 | ADR 0011；B02、P01–P05、H01–H03、L01–L03，报告不预填 pass |

## 相关文档

- 产品需求：[`../../PRD.md`](../../PRD.md)
- 根级 AI 约束：[`../../AGENTS.md`](../../AGENTS.md)
- 组件兼容性记录：工作区唯一的 [`../../Cargo.lock`](../../Cargo.lock)（原 `compat.toml` 已随 ADR 0016 删除）
- 验收规范与报告：[`../quality/acceptance.md`](../quality/acceptance.md)、[`../quality/acceptance-report.md`](../quality/acceptance-report.md)
- Plasma 对照与参考边界：[`../quality/plasma-reference.md`](../quality/plasma-reference.md)
