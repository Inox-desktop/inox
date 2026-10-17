# ADR 0016: 改为单一仓库（Cargo workspace）并取代多仓库策略

- 状态：accepted
- 日期：2026-10-16
- 决策者：架构组
- 批准记录：2026-10-16 仓库所有者（架构组）在开发会话中明确批准并指示直接实施（“直接开改吧，然后整体虽然在一起，但需要模块化，方便局部测试”）。本次改动尚未开 PR；**合并前须在 PR 上补链接与日期**，并把本条替换为 PR 引用。
- 关系：**取代 ADR 0001 全部条目**（其状态已更新为 `superseded`，历史正文未改写）；**部分取代 ADR 0002** —— 仅取代其借「仓库边界」强制依赖方向的手段与「负面」中「依赖必须通过 Git tag 逐层升级」一项，ADR 0002 的分层成员与依赖规则 1–6 **继续有效**，改由 `layering.rs` 强制（见实施记录中的新问题）。

## 背景

ADR 0001 自述的取代条件已经触发。原文：「若跨仓库依赖成本显著超过模块边界带来的收益（例如出现难以收敛的版本矩阵维护负担），应重新评估 monorepo 并撰写取代本 ADR 的新 ADR。」以下是截至 2026-10-16（M0 骨架完成、M1 部分落地、M2 进行中）的实测状态。

**组件数量与实际代码量严重失配。** 组织下有 17 个组件仓库加 1 个组织仓库，而真实产品代码只存在于 5 个 crate：`inox-common` 618 行、`inox-config` 3320 行、`inox-compositor` 3345 行、`inox-protocols` 545 行、`inox-xwayland` 2570 行，合计约 10,400 行。其余 11 个仓库（`inox-shell`、`inox-panel`、`inox-launcher`、`inox-notifications`、`inox-settings`、`inox-lock`、`inox-wallpaper`、`inox-session`、`inox-idle`、`inox-greet`、`inox-integration`）目前都是 17 行的 `fn main()` 骨架，且这 11 个骨架内容彼此相同。

**版本矩阵在第一天就已失配。** `inox-common` 当前版本为 `0.2.0`；`inox-config` 依赖 `tag = "v0.2.0"`，而 `inox-compositor` 与 `inox-xwayland` 仍依赖 `tag = "v0.1.0"`；元仓库 `compat.toml` 只登记 `inox-common = "v0.1.0"`。同一时刻存在三份互相矛盾的「当前版本」，而 PRD §3.3 把 `compat.toml` 定义为兼容性的权威记录、验收项 L01 以它解析发布清单。三处不一致意味着 L01 的证据链在当前状态下不可信。

**组织级配置已开始漂移。** 214 个非 `target` 文件中有 121 个（57%）是逐仓库重复的样板：`LICENSE`、`rust-toolchain.toml`、`deny.toml`、`AGENTS.md`、`README.md`、`.gitignore`、`.github/workflows/ci.yml`。其中 18 份 `LICENSE` 字节完全相同、18 份 `rust-toolchain.toml` 完全相同，但 17 份 `ci.yml` 中已有 3 个不同版本、18 份 `deny.toml` 中已有 4 个不同版本。也就是说，本应靠「同一份模板复制」保证一致性的文件，在一个月内已经分叉，且没有任何机制能在分叉时报警。

**MSRV 已分裂。** `inox-compositor` / `inox-protocols` / `inox-xwayland` 为 `rust-version = "1.86"`，其余 11 个 crate 为 `1.85`。根级 `AGENTS.md` §8 要求升级需说明破坏性影响，但没有任何检查在断言「同一依赖图中不存在两个 MSRV」。

**项目的硬性规则放大了 polyrepo 的主要成本。** 根级 `AGENTS.md` §3.1 规定「不在本地编译/测试，验收一律以 CI 为准」，§8 规定跨仓库依赖只能锁定 `tag` / `rev`。两者叠加后的实际链路是：修改 `inox-common` 的一个公开签名 → 上游合 PR → 等上游 CI → 打 tag → 逐个下游仓库改 tag → 各自再等一次完整 CI。ADR 0002 的「负面」已经把这一点记为既定成本（「依赖必须通过 Git tag 逐层升级，L1 的改动传播到 L4 需要多轮 PR」）；ADR 0013 又提议重画 L3 的进程/协议边界，ADR 0011–0015 五份提案全部处于 proposed 且 PRD §3.1 的依赖表已自述「不作为同层 Rust 依赖的授权」—— 一次跨层的边界调整因此在 polyrepo 下必然展开为 N 个仓库的 N 个 PR 与 N 次独立 CI。

**关键反证：仓库边界并没有换来依赖方向的强制。** ADR 0001 的首要正面理由是「仓库边界即模块边界，PRD §2.1 的『依赖只能向下』『禁止循环依赖』可在仓库粒度上被审查与自动化检查」。但实际上，17 个仓库各自只看到自己的依赖子图，当前没有任何 CI 检查在断言 ADR 0002 的规则 1–6：没有检查禁止同层横向依赖，没有检查禁止 L1 反向依赖，也没有检查保证 `inox-compositor` 与 `inox-shell` 之间不存在 Rust 依赖（ADR 0002 规则 3）。仓库数量买到的不是约束执行，而是样板复制。

**尚无外部消费者。** 当前没有任何 Inox crate 被组织外项目消费；PRD §3.3 的「独立版本、独立发布」在 1.0 之前没有真实需求方，它换来的独立性是纸面上的。

**未知项与未验证假设。** 单一工作区 CI 的分区收益需要以实测确认，不能在本文预设结论：（1）GitHub Actions 上的工作区缓存命中率与共享 `Cargo.lock` 是否真的低于 17 条独立流水线的分钟数；（2）按路径过滤与按 crate 矩阵两种策略在此仓库规模下的 job 时长与排队行为。这两项列为实施顺序中的首个验证点。

## 决策驱动因素与备选

可衡量的驱动因素：（1）跨层改动所需的人工步骤数与 CI 运行次数；（2）依赖方向规则能否被机械强制而不仅是文档约定；（3）组织级配置（lint、MSRV、许可证白名单）的单一事实来源数量；（4）发布一致性证据（当前由 `compat.toml` 承担）的可信度；（5）代理并行工作时共享文件的冲突面。

| 方案 | 收益 | 成本 / 风险 | 选择或否决依据 |
|------|------|-------------|----------------|
| **A. 保持 polyrepo（ADR 0001 现状）** | 组件 CI 相互独立，单组件失败不阻塞其他；并行代理在不同仓库无文件冲突；独立 SemVer 与独立发布；18 份 `AGENTS.md` 可承载 crate 级差异 | 跨层改动需 N 轮 tag 升级；57% 文件是重复样板且已分叉；版本矩阵已失配；依赖方向无机械强制；18 条流水线各自从零编译 smithay/iced，无跨仓库缓存共享 | **否决**：ADR 0001 自述的取代条件（「难以收敛的版本矩阵维护负担」）在当前 10,400 行、17 仓库的规模上已经成立，且失配发生在第一天，而非长期演化之后。继续维持意味着在组件数继续增长（另有 4 个规划中仓库）时固化为 23 份样板与 23 条流水线 |
| **B. 单一仓库 + Cargo workspace（建议采纳）** | 一次 PR 内完成跨层原子改动与一次 CI 验证；依赖方向、MSRV、许可证白名单、协议出口各自只剩一份权威配置；单一 `Cargo.lock` 本身就是兼容矩阵与 L01 的不可变证据；工作区内 path 依赖消除 tag 往返延迟，代理可对完整依赖图执行 `cargo check -p <crate>`；共享 target 与缓存使全图编译成本低于多次独立全量编译 | 根的 `Cargo.toml` / `Cargo.lock` / 工作流变更是全仓爆炸半径；需要新增机械化的分层方向检查（否则失去 ADR 0002 的执行）；MSRV 必须收敛为 1.86；根级 `AGENTS.md` §2/§8 与 PRD §1.1/§3 需同步修订；失去按仓库的版本自治 | **采纳**：以 (2)(3)(4) 的收益换 (1) 的成本，并用 B04 的分层检查把原本缺失的强制机制补齐；爆炸半径由代码审查与路径过滤控制，不构成阻塞 |
| **C. 粗粒度三仓库（`libs` / `compositor` / `desktop`）** | 减少样板至 3 份；保留较粗的失败域隔离；比 A 少的 tag 往返 | `libs` 与两个消费仓库之间仍保留 tag 锁定延迟，只是跳数变少；`desktop` 内部同时容纳 L3 与 L4，ADR 0002 的「禁止同层横向」在仓库内退化为不可检查的隐式约定；新增 `inox-portal` / `inox-cli` 等规划中仓库时需要重新决定归属 | **否决**：它保留了 A 的主要成本（tag 往返、版本矩阵），同时放弃 A 的边界清晰度；对驱动因素 (2) 没有改善，属于两个方案的折中失败 |

## 决策

采用**单一仓库（Cargo workspace）**，取代 ADR 0001 的 polyrepo 策略。具体约定如下。

1. **落地位置**：元仓库 `inox` 升级为唯一的代码与文档仓库，根目录同时承载 `PRD.md`、`docs/`（ADR 与质量文档）与 `crates/`（全部组件）。组织仓库 `.github` 保留，只承载 ISSUE/PR 模板与组织级工作流模板。承载形态的最终选择（元仓库自体升级，还是新建 `inox-desktop` 代码仓库并保留 `inox` 为纯文档）列为未决项 1，需在批准时确定。
2. **工作区结构**：全部组件为 `crates/inox-<component>/` 下的 workspace 成员，crate 名与二进制名保持现有命名（`inox-compositor` 产出 `inox-compositor` 二进制，`inox-comp` 别名不变）。层级不再由仓库归属表达，改由根 `Cargo.toml` 中一份显式的分层成员表表达（见第 3 条），该表是 ADR 0002 分层成员的唯一事实来源。
3. **分层与依赖方向保持不变，但改为机械强制**：ADR 0002 的规则 1–6 逐条保留，包括 `inox-common` 的叶子性、L4→L3→L2→L1 的单向性、禁止同层横向依赖，以及 `inox-compositor` 与 `inox-shell`（及其 L3 子组件）之间不存在 Rust 依赖、只经 Wayland 协议 / `inox-ipc` / D-Bus 通信。强制方式改为一个新的 E0 检查：以 `cargo metadata` 读取真实依赖图，对上述规则逐条断言并给出违例路径；该检查与 `cargo deny check` 的 `[bans]` 段共同取代原「仓库边界」的作用。ADR 0002 规则 6 的 `inox-wallpaper` 例外在 ADR 0013 获批前维持现状，本 ADR 不改变其效力。
4. **单一样板**：`Cargo.lock`、`rust-toolchain.toml`、`deny.toml`、CI 工作流、根 `AGENTS.md` 各一份。crate 级差异（`unsafe` 例外、MSRV 之外的 lint 覆盖）通过 workspace 默认值加单包 `[lints]` 覆盖表达，不通过复制文件表达。
5. **MSRV 收敛为 1.86**：因为 `inox-compositor` / `inox-protocols` / `inox-xwayland` 已要求 1.86，工作区统一取 1.86，11 个曾为 1.85 的 crate 一并升级。根级 `AGENTS.md` §8 的 MSRV 说明随之改写为单一值。
6. **`unsafe` 策略不变**：根级 `AGENTS.md` §3.2 的例外范围与白名单逐字保留。`inox-compositor` / `inox-xwayland` 继续使用 `unsafe_code = "deny"` 加已批准的局部 `allow` 与 `// SAFETY:` 注释，其余 crate 继续 `forbid`；工作区默认值为 `forbid`，例外包在自身 `Cargo.toml` 中覆盖。新增 `allow` 仍须先提交 ADR。
7. **版本与发布**：crate 版本由工作区统一推进（1.0 之前不做独立 SemVer 编排）。需要单包发布时使用 `cargo release -p <crate>`，tag 格式 `<crate>-vX.Y.Z`。`compat.toml` 与 `repos/` 克隆流程**删除**：单一 `Cargo.lock` 就是兼容矩阵，验收项 L01 的不可变清单由该 lock 文件与工作区 tag 共同给出。
8. **`inox-integration`** 保留为工作区成员（`crates/inox-integration/`），从 path 依赖驱动全部组件，继续承担 E3 端到端职责；ADR 0002 规则 7 中「只允许在各组件的发布 tag 上做验证」需相应改写为「在不可变 commit 或发布 tag 上验证」，这项改写列为批准后的同步项。
9. **`AGENTS.md` 的层级**：根 `AGENTS.md` 保留全部硬性规则，§2 的「仓库地图」改写为「工作区 crate 地图」，§8 的「跨仓库依赖使用锁定 tag 的 Git 依赖」改写为「工作区内使用 path 依赖，工作区外依赖仍锁定 tag / rev」；`crates/inox-*/AGENTS.md` 继续存在，承载本已存在的 crate 级差异（`unsafe` 白名单、MSRV 说明、测试要求、目录结构）。

## 失败、恢复与安全

- **爆炸半径**：根 `Cargo.toml`、`Cargo.lock`、工作流与工作区 lint 配置的改动会影响全部 crate。缓解：这些文件在 CODEOWNERS 中登记为需人类审查路径；CI 按路径过滤，使不触碰根配置的 PR 只重建受影响 crate。这是本决策引入的主要新增风险，必须随迁移一并落地，不能只作为文档承诺。
- **`Cargo.lock` 冲突**：并行代理同时新增依赖时会产生 lock 冲突。该类冲突是机械性的（重新执行任一 `cargo` 命令即按 manifest 重新解析），不是语义冲突；PR 模板要求代理在提交前重新生成并确认 diff 仅含预期变更。此风险不改变依赖方向，也不影响运行时可复现性，因为锁定来源仍只有一份。
- **单 crate 编译失败阻塞整仓 `main`**：这是 polyrepo 明确要避免的场景。缓解为路径过滤加 `-p` 定位；同时应认识到「整仓可构建」本就是 L01 与内存中任一发布组合的前提，polyrepo 只是把这个失败推迟到集成阶段并隐藏了不一致。
- **供应链检查变严（净收益）**：`inox-common` 等 crate 的真实依赖图此前被 18 份 `deny.toml` 分别观察，每份只覆盖自己的子图；单一 `deny.toml` 与单一 lock 使 license 白名单、RUSTSEC 与重复版本检查覆盖完整图。预期会出现新的 deny 告警，这属于此前被分割掩盖的真实问题，应在迁移 PR 中逐条处理并在 PR 说明中列出，不得以放宽白名单的方式消除。
- **运行时边界完全不变**：本决策不改变任何进程边界、Wayland 协议、`inox-ipc`、D-Bus 服务、配置格式或特权能力。ADR 0002 规则 3 的「合成器与 Shell 不通过 Rust 依赖耦合」在机械检查下只会更具体：验收项 S04 的授权与版本协商、S05 的失败域隔离、ADR 0015 的特权边界均不受影响。
- **不引入静默网络行为或遥测**：本决策不新增任何网络面。工作区 CI 的缓存若使用第三方 Action，需在迁移 PR 中登记来源与锁定版本，符合根级 `AGENTS.md` §8 与 §3.9。

## 兼容、迁移与回退

- **外部消费者**：当前无组织外消费者，不存在需要在迁移中维持的公开 API 兼容承诺。
- **已完成历史**：5 个有真实历史的 crate（`inox-common` 6 个 commit、`inox-config` 3 个、`inox-compositor` 15 个、`inox-protocols` 6 个、`inox-xwayland` 10 个）；11 个骨架仓库各 1 个 commit。骨架部分直接导入即可；有历史部分建议用 `git subtree` 或 `git filter-repo` 保留提交历史，是否保留列为未决项 2，不构成阻塞。
- **已发布 tag**：`v0.1.0` / `v0.1.1` 保留在归档仓库中，不重写、不强推；`v0.2.0` 已被重切为 `v0.1.1` 并退役。迁移后新 tag 采用 `<crate>-vX.Y.Z`。
- **迁移顺序**：（1）本 ADR 获批；（2）在选定仓库建立工作区骨架（根 `Cargo.toml` 含分层成员表与 `[workspace.lints]`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`.github/workflows/ci.yml`、`crates/`）；（3）逐 crate 导入源码并把 git 依赖改为 path 依赖，版本由工作区统一；（4）MSRV 收敛至 1.86；（5）接入分层方向检查（B04）并注入反向依赖与同层横向依赖的负向用例确认其会失败；（6）删除 `compat.toml`、`repos/` 流程与 `repos/` 的 `.gitignore` 条目，更新根 `AGENTS.md` §2/§8、PRD §1.1/§3/§6.1/§12、`README.md`、`docs/adr/README.md` 索引与 ADR 0002 的关系登记；（7）把 17 个组件仓库归档（archive）并留说明指向新仓库。步骤 3–6 可在同一 PR 或数个聚焦 PR 中完成，但**中间态不得双向提交**：旧仓库在迁移开始后只读。
- **回退**：若工作区 CI 实测（见实施顺序）显示无法接受的退化，回退路径是把 path 依赖改写回 git 依赖并按现有目录结构拆回多个仓库 —— 相比反向迁移成本更低，因为 crate 内部结构与 `AGENTS.md` 都已按可拆分的方式保留。迁移分支在 CI 全绿并被批准前不合并。
- **新依赖**：本决策不新增第三方 crate，因此无新增许可证审查对象；唯一的外部依赖面变化是 CI Action 与缓存组件的登记。

## 验收与证据

| 验收 ID / 场景 | 责任仓库 | E0–E4 等级 | 测试与失败注入 | 通过条件 / 证据位置 |
|----------------|----------|------------|----------------|----------------------|
| B04（已加入 `docs/quality/acceptance.md`） | 单一仓库 CI | E0 | CI 中运行 `cargo metadata` 驱动的分层方向检查；注入一个 L1→L3 反向依赖、一个 L3→L3 同层横向依赖、一个 `inox-shell` → `inox-compositor` 的 Rust 依赖，三者都必须使检查失败 | 每条规则有独立断言与违例路径输出；负向注入确实失败；正向全图通过。证据：CI job URL + 失败输出 |
| B04b（已加入 `docs/quality/acceptance.md`） | 单一仓库 CI | E0 | 工作区 `fmt --check`、`clippy --all-targets --all-features -D warnings`、`test --all-features`、`build --release`、`doc --no-deps`、`deny check` 在**一次**运行内覆盖全部 crate；注入一个仅供单 crate 使用的 GPL 依赖必须被 `deny` 拦截 | 与 B01 的检查集合一致，只是执行范围改为工作区；证据：单次运行的全部 job |
| B01（修订） | 单一仓库 | E0 | PR 描述列出「受影响 crate + 适用验收 ID + 回归用例」 | 与现行 B01 相同，仅把「变更仓库」改为「受影响的 crate」 |
| B04c（已加入 `docs/quality/acceptance.md`） | 单一仓库 CI | E0 | 对规则逐条注入反例（反向边、同层边、合成器/Shell 互依、未登记的 L3→合成器边、依赖聚合 crate），每条都必须让检查失败 | 反例由 `layering.rs` 的 `tests` 模块覆盖，共 15 个规则单元测试；不注入反例的检查按「未验证」处理 |
| L01（修订） | 单一仓库 | E3/E4 | 以单一 `Cargo.lock` 与工作区 tag 解析不可变 SHA 清单，替代按 `compat.toml` 解析 | 清单可由 lock 文件与 tag 机械重建且与实测一致；SBOM 要求不变 |
| R01–R03、W01–W05、O01–O04、X01–X03、D01–D06、C01–C04、S01–S05、Q01–Q03、P01–P05、H01–H03 | 迁移后的对应 crate | 不变 | 全部既有验收项在迁移后必须重新通过 | 本决策不改变任何运行时行为，故不新增运行时场景；既有项必须在迁移 PR 中重跑并提供证据，不能以「只改了构建布局」为由跳过 |

B04 / B04b 已随本 ADR 的批准落地（见「实施记录」）：`layering.rs` 位于 `crates/inox-integration/tests/`，工作区 `fmt`/`clippy`/`deny`/`doc`/`build` 与按 crate 分片的测试矩阵均已在 `.github/workflows/ci.yml` 中接线。`docs/quality/acceptance.md` 的 B04/B04b 行已登记。**分支保护尚未按新 job 集合更新**，在更新之前不能声称门禁已生效。

## 实施顺序与未决项

原计划先做最小原型实测 CI 分区收益再决定是否迁移。批准后的指示是直接实施，因此该实测被推迟到迁移之后。**已回填**，见「首次 CI 运行的结果」；回退判据第 1 条未触发。

未决项：

| 未决项 | 责任角色 | 最迟解决阶段 | 状态 |
|--------|----------|--------------|------|
| 1. 承载形态：元仓库 `inox` 自体升级为 monorepo，还是新建 `inox-desktop` 代码仓库并保留 `inox` 为纯文档 | 架构组 | 本 ADR 批准时 | **已定**：`inox` 自体升级（见实施记录） |
| 2. 历史保留程度：5 个有历史的 crate 是否用 `subtree` / `filter-repo` 保留提交历史 | 架构组 + 实施者 | 迁移 PR 提出时 | **已定**：不保留，直接导入当前工作树 |
| 3. CI 分区策略：路径过滤 vs crate 矩阵，以及是否引入 merge queue | 集成维护者 | 迁移完成后 | **已定**：按 crate 分片矩阵（由层级表推导）；路径过滤与 merge queue 待实测后评估 |
| 4. ADR 0013 的 L3 边界重画与本次迁移的先后顺序 | 架构组 | ADR 0013 评审时 | **未决**。两者都会改动 L3 成员与层级表例外，需明确谁先落地以免两次改写同一张表 |
| 5. 是否保留 `crates/inox-*/AGENTS.md`，以及根 `AGENTS.md` §2/§8 的最终措辞 | 架构组 | 批准后的文档同步 PR | **已定**：保留 crate 级 `AGENTS.md`；根 `AGENTS.md` §2/§8/§12 已同步改写 |
| 6. 第三方 CI Action / 缓存组件的来源与锁定版本登记 | 集成维护者 | 迁移完成时 | **已定**：沿用既有 Action，未新增供应商（见实施记录） |
| 7. **ADR 0002 规则 1 与层内顺序的冲突**（本次实施发现，见实施记录） | 架构组 + 人类审查者 | **现已阻塞**（原定 M4 之前） | **未决**。`inox-compositor → inox-protocols`（L2→L2）已随「合成器消费协议绑定」落地并登记在 `same_layer_exceptions`，因此规则 1 的裁决不再是假设性工作。在裁决前，同层边只能逐条登记，且每条都需要 ADR |
| 8. 组件仓库归档、CODEOWNERS、分支保护检查名更新 | 仓库所有者 | 本工作区首次 CI 全绿并合并后 | **未决，新增**。均需仓库管理权限或不可逆的对外操作，未自行执行 |
| 9. 工作区统一版本的最终取值：`0.1.1` 是否合适 | 「版本号语义与发布流程」ADR 的所有者 + 架构组 | 该 ADR 离开 `proposed` 时 | **未决，新增**。该 ADR（PR #10，`proposed`）规定 0.x 阶段 patch 为兼容段、**提升 MSRV 属 minor**；本迁移把 11 个 crate 的 MSRV 由 1.85 提至 1.86，按该规则应是 minor（`0.2.x`），但 `v0.2.0` 恰好已被退役，两个约束直接冲突。本 ADR 取 `0.1.1`（不复用退役号、与现行发布线一致）并在此登记冲突，**不擅自裁定版本语义**；若该 ADR 最终采用 MSRV=minor，改的是一行 |
| 10. 仓内存在**两个 ADR 0010** | 架构组 | 本 PR 合并前 | **未决，新增**。`docs/adr/0010-real-backend-drm-libinput.md` 已在 `main`，而开放中的 PR #10 要新增 `docs/adr/0010-versioning-and-release-policy.md`。`main` 的 `compat.toml` 已经在引用后者（“ADR 0010 第 10 条”），索引与引用链因此有歧义。编号冲突属 PR #10 的范围，本 ADR 不改写他人提案，仅登记 |

上游发布 tag → 下游更新 → 集成验证的现有流程随本决策作废，替换为「工作区内一次 PR 一次 CI」。批准前，ADR 0011–0015 的 proposed 状态与既有 accepted ADR 的效力均不受影响。

## 后果

正面：

- 跨层改动从「N 个 PR × N 次 CI 加 tag 往返」降为一次 PR 内的一次验证；ADR 0002 已经记录的成本（「L1 的改动传播到 L4 需要多轮 PR」）被消除。
- 组织级配置（lint、MSRV、许可证白名单、协议出口、CI 检查集合）从 17–18 份副本收敛为各一份，消除已在发生的漂移。
- 依赖方向规则首次获得机械强制：`cargo metadata` 驱动的检查可以在一次运行中断言 ADR 0002 规则 1–6，这是 polyrepo 的仓库边界**从未**提供过的能力。
- 兼容性证据从人工维护的 `compat.toml` 变为构建系统自身产出的单一 `Cargo.lock`，L01 的证据链不再可能与其声称的组件版本矛盾。
- `cargo-deny` 的许可证、RUSTSEC 与重复版本检查覆盖完整依赖图而非 18 个子图。
- 代理可在完整依赖图上执行 `cargo check -p <crate>` / `cargo test -p <crate>`，使「改动是否真的编译并被测试覆盖」在提交前即可判定，与 CI 作为唯一验收途径的规则相容。

负面：

- 根配置文件成为全仓爆炸半径；需要 CODEOWNERS 与路径过滤两道缓解，二者必须在迁移中一并落地。
- 失去按仓库的版本自治与独立发布；`inox-integration` 规则 7 的「只在发布 tag 上验证」需改写为「不可变 commit 或 tag」。
- `Cargo.lock` 在并行代理场景下成为共享冲突点，需要 PR 模板与重新生成约定来消化。
- MSRV 必须收敛至 1.86，11 个此前为 1.85 的 crate 一并提升；若某 crate 有维持在 1.85 的强理由，需在该 crate 的 `AGENTS.md` 中说明并作为例外登记。
- 18 个仓库的归档是单向操作，组织页面与外部链接需要指引；此成本一次性发生。
- 迁移期内旧仓库只读，跨仓库的进行中 PR 需要在迁移前收敛。

影响面：本决策影响全部 17 个组件仓库、组织仓库与元仓库，涉及 PRD §1.1、§1.3、§3（含 §3.1 表、§3.2 依赖管理、§3.3 发布策略、§3.4 目录结构）、§5.1、§6.1、§12、§14，根级 `AGENTS.md` §2、§3.1 的表述、§8、§12，`README.md`、`compat.toml`（删除）、`docs/adr/README.md` 索引，以及 ADR 0002（部分取代登记）与 ADR 0004（骨架阶段依赖策略的适用性需复核）。里程碑影响：M0/M1/M2 的既有证据在迁移后需重跑；M3–M6 的计划工作量因消除 tag 往返而减少。**批准后**需把 ADR 0001 状态更新为 `superseded` 并链接本文，并按索引规则保留其历史正文不改写。

复审 / 取代触发条件：

1. 最小原型实测显示工作区 CI 的 job 时长或排队成本显著高于 17 条独立流水线，且路径过滤与矩阵策略都无法改善 —— 则本决策应回退。
2. 出现真实的外部消费者要求某个 crate 独立于 Inox 的发布节奏演进 —— 此时应评估把该 crate 拆出，而不是整体回到 polyrepo。
3. 单一工作区的根配置冲突成为代理并行工作的主要瓶颈（以实测的冲突率与返工量为据）—— 应评估拆分粒度。
4. 组件数量与代码量增长到单次工作区 CI 无法在可接受时间内完成，且路径过滤失效 —— 应评估按层拆分而非按组件拆分。

---

## 实施记录（2026-10-16）

获批后于同日实施。本节记录实际做了什么、哪些判断由实施者代行、以及**实施中发现的新问题**；它不是决策正文，而是本决策的可核对证据。

### 已落地

| 项 | 位置 |
|----|------|
| 工作区根清单 | `Cargo.toml`：`members = ["crates/*"]`、`resolver = "3"`、`[workspace.package]`（统一版本 `0.1.1`、MSRV 1.86）、`[workspace.dependencies]`、`[workspace.lints]`、`[workspace.metadata.inox]`（层级表与规则例外） |
| 统一样板 | `rust-toolchain.toml`（1 份，原 18 份完全相同）、`deny.toml`（1 份，原 18 份且已分叉成 4 版）、`LICENSE`（1 份，原 18 份完全相同）、`.github/workflows/ci.yml`（1 份，原 17 份且已分叉成 3 版） |
| 组件导入 | 16 个 crate → `crates/`；mdBook → `docs/book/`（原 `inox-docs`，无 `Cargo.toml`，不参与依赖图与 `cargo deny`） |
| 清单改写 | 16 个 manifest 全部改为工作区继承：`version`/`edition`/`rust-version`/`license`/`repository`/`readme`/`keywords`/`categories` 取 `*.workspace = true`；内部依赖改为 path 依赖（`inox-common.workspace = true`）；第三方依赖版本收敛到 `[workspace.dependencies]`，成员只追加自己的 feature |
| 分层机械检查 | `crates/inox-integration/tests/layering.rs`：17 个测试 = 15 个规则单元测试（对合成图）+ 2 个真实工作区断言；数据源为 `cargo metadata --no-deps` 与 `[workspace.metadata.inox.layers]` |
| CI | `.github/workflows/ci.yml` 的 9 个 job：`fmt`、`clippy`、`deny`、`doc`、`build`、`plan`、`test`（按 crate 分片）、`xwayland-e2e`、`book` |
| 删除 | `compat.toml`；`repos/` 目录流程（`repos/` 的 `.gitignore` 条目保留到本地克隆删除为止） |
| 系统库单一来源 | `.github/actions/system-deps/action.yml`：apt 包列表从「4 个工作流 job 各抄一份」改为一个复合 action。理由是包列表是事实清单，抄本分叉会让某个 job 静默缺库——这与本 ADR 收敛样板是同一个理由。首次 CI 运行证明 `clippy` / `doc` **也**需要这些库（见「首次 CI 运行的结果」） |
| 连带修正 | 根 `.gitignore` 增加 `/docs/book/book/`：文档站点原先自带 `.gitignore`（`book/`），而本 ADR 取消了逐组件忽略文件，根忽略文件必须接管。`docs/book/src/SUMMARY.md` 增加 `markdownlint-disable MD025`：该文件用多个 `#` 表示 mdBook 分卷标题，此前它位于被排除的 `repos/` 下、不在 lint 范围内，迁入 `docs/` 后进入范围 |

`unsafe` 例外的表达方式有一个必须记录的实现细节：cargo **不允许**成员同时写 `lints.workspace = true` 与本地 `[lints]` 覆盖（`cannot override workspace.lints in lints`）。因此 `crates/inox-compositor/Cargo.toml` 无法继承工作区 lint 集，只能逐字重述后再改写 `unsafe_code = "deny"`。该 crate 的 manifest 中已注明这一约束；**`[workspace.lints]` 与它的副本之间的一致性没有机械检查，是审查项**。`inox-xwayland` 无 `unsafe`（源码中无 `#[allow(unsafe_code)]`）且源码注释明确声明无例外，故维持工作区默认的 `forbid`，不进入例外名单。

### 由实施者代行的判断

对应原「未决项」表的 1、2、3、5、6 项，逐项处置与依据见更新后的未决项表；其中三项取值值得单独说明：

- **工作区统一版本取 `0.1.1`**。初稿曾取 `0.2.0`（当时 `inox-common` 名义上的最高版本），**该选择已被推翻**：本迁移期间 `main` 上的 `compat.toml` 记录 `inox-common` 的现行发布线是 `v0.1.1`（`paths` 属兼容新增，0.x 阶段走 patch），且 `v0.2.0` 已重切为 `v0.1.1` 并退役——继续用 `0.2.0` 等于复用已退役的版本号。见下方未决项 9。
- **MSRV 统一为 1.86**，11 个此前为 1.85 的 crate 一并提升；1.86 本就是 `inox-protocols`（wayland-protocols 0.32）与 `inox-compositor`（calloop 0.14 via smithay 0.7）的既有要求。
- **`inox-docs` 不再是 workspace 成员**，而是 `docs/book/`。它是本 ADR 中唯一「从 L1 成员变为非成员」的组件：没有 `Cargo.toml`，不参与依赖图、层级表与 `cargo deny`。若这一点被认为改变了 ADR 0002 的分层成员，需要人类在评审时确认——本 ADR 只主张它本来就已经不参与任何 Rust 依赖关系。

### 导入来源缺陷与修复（2026-10-17 补记）

本节记录一次**已合并到本 ADR 的工作被静默丢弃**，以及它是怎么被发现的。它不是流程说明，而是一个已经发生过的错误。

**发现方式。** 在回答「组件仓库是否可以删除了」时，把工作区与各仓库的**远端 ref** 做内容比对（`scripts/verify-import-parity.py`），而不是依赖本地自检。本地 216 个测试全绿**完全没有暴露这个问题**——它们只证明导入进来的东西自洽，不证明导入完整。

**缺陷 1：克隆陈旧。** 5 个克隆落后远端 `main`（`inox-compositor` 落后 6 个提交，`inox-common` 3 个，`inox-config` / `inox-xwayland` / `inox-docs` 各 1 个）。后四个的内容差异为零（落后的是合并提交），但 `inox-compositor` 落后的是**已合并的 PR #4「以锁定 tag 消费 inox-protocols 并广告 inox-ipc」**，实质内容为 `src/state.rs` 的 `inox_ipc` 全局与方法、新测试 `tests/ipc_global.rs`、以及清单/文档改动。该 PR 于 `2026-10-16T19:25:59Z`（本地 12:25）合并，而导入发生在本地 13:39——**提交已在远端一个多小时，导入却用了没 fetch 过的克隆**。

**缺陷 2：克隆工作区脏。** `repos/inox-compositor` 的工作区有 231 行未提交改动，外加 untracked 的整目录 `src/backend/`——即 M2.5 真实后端脚手架（`BackendKind`、`--backend` 解析、`smithay` 的 `backend_*` features）。这部分**从未提交到任何分支，也从未推送**，只存在于那份本地磁盘上。`tar` 导入工作树把它带了进来：既是侥幸保住（否则本轮才发现丢失），也说明导入来源**本来不可审计**。

**根因。** 「以当前工作树导入」这句实施指令同时缺少两个约束：(a) 工作树是否等于远端 ref，(b) 工作树是否干净。前者丢掉已合并的提交，后者把未提交的本地状态变成唯一副本。

**修复。** 把两份来源合并进工作区：B = 远端 `main` 的 ipc 工作，C = 克隆未提交的 M2.5 工作。除 `src/state.rs`（两边都改过）外，工作区逐文件等于 C。`src/state.rs` 已机械验证等于「远端 `main` 版本 + C 的两行文档注释改动」，是单一 hunk，可肉眼复核。

**新增的可复现校验。** `scripts/verify-import-parity.py`：对每个源文件，把工作区副本与组件仓库的 committed ref 比较，再确认工作区副本本身已提交在本仓库的某个 ref 中（`--committed-ref`，默认 `HEAD`，可用 `origin/main` 证明已推送）。分类为「丢失」「在本仓库未提交」「此处有意取代」「与 ref 一致」四种。**关键是最后那一步**：「已提交在本仓库」与「已推送到 GitHub」是两个不同断言，把这个区分交给显式的 ref 参数，而不是靠约定。

脚本初版用克隆的工作树作为第二个来源，按「是否等于克隆工作树」分类——那个判据只在克隆还存在时成立，而且**不是真正要紧的属性**。现版本不依赖 `repos/`，克隆存在时其未提交状态只作为诊断信息输出。该脚本是迁移工具，组件仓库归档后应随它们一起删除。

首次运行结果：**无文件丢失**；4 个文件与组件仓库的 ref 有意不同（其中 3 个是 C 的内容、1 个为手工合并），且全部已提交在本仓库。

**教训。** 「本地全部测试通过」与「导入完整」是两件不同的事。任何从另一份代码库拷贝内容的迁移，都必须把**内容与来源的比对**作为验收项，而不是把目标仓库的自检当作完整性证据。

### 本次发现、需要人类裁决的新问题

`layering.rs` 上线后立即指出了 **ADR 0002 规则 1 的字面表述与 PRD 依赖表冲突**，且不止一处：

| 边 | 层级关系 | 现状 |
|----|----------|------|
| `inox-config → inox-common` | L1 → L1（同层） | **实际存在**，且 PRD §3.1 与 README 组件表都明确 `inox-config` 依赖 `inox-common`。规则 1 的字面表述（「依赖方向只能向下」「禁止…跨层横向」，例证为 L3→L3）禁止同层依赖，两者矛盾。当前登记在 `[workspace.metadata.inox.rules] same_layer_exceptions`，检查仍按规则原文执行 |
| `inox-compositor → inox-protocols` | L2 → L2（同层） | **PRD 与 README 的组件表列出，代码尚未实现**。实现该依赖时检查会变红，并强制先裁决 |

根因是四层模型无法表达**层内顺序**：`inox-common` 排在 `inox-config` 之前、`inox-protocols` 排在 `inox-compositor` 之前，都是健康的层内顺序，但规则 1 只承认层级之间。这一歧义在 ADR 索引的复核清单中**早有登记**；本 ADR 的价值在于把它从「评审时容易被忽略的一句话」变成「CI 会红的具体边」。

可选裁决方向（本 ADR 不擅自选择，因为它修改的是 ADR 0002，属于独立决策）：

1. **层级表改为层内有序列表**（`l1 = [inox-common, inox-config]`、`l2 = [inox-protocols, inox-compositor, inox-xwayland]`、`l3 = [inox-shell, …]`），规则 1 改为「依赖只能指向排序更靠前的位置」；`inox-common` 的叶子性成为推论而非单独规则条目。收益是层内顺序成为显式数据；代价是层级表语义比「四层」复杂。
2. **只改规则 1 的措辞**，明确层内顺序由组件表给出。但这与 PRD §3.1 自述「本表不作为同层 Rust 依赖的授权」冲突，需要同时修改 PRD。
3. **维持现状**：每条实际需要的同层边登记为例外，由 ADR 索引持续跟踪。

在裁决前，`same_layer_exceptions` 是唯一的放行手段，且**任何条目都必须先有 ADR**——检查的失败信息已写明这一点，对应的单元测试（`same_layer_edge_fails` / `a_planned_same_layer_l2_edge_fails_until_an_adr_permits_it`）也把该行为固定下来。

### 迁移中发现的连带影响

文档站点从被 lint 排除的 `repos/` 迁入 `docs/` 后，**进入了 markdownlint 的检查范围**（`.github/workflows/docs.yml` 检查 `**/*.md` 并只排除 `repos/`），因此暴露出它在旧位置从未被检查过的格式问题。已按「文件格式要求」而非「放宽规则」处理：mdBook 的章节目录必须用多个一级标题分卷，故在 `SUMMARY.md` 内以行内指令局部豁免 MD025，其余问题直接修正，未改动仓库级规则配置。同类问题会在任何组件从被排除路径迁入检查范围时再次出现，处理原则相同。

**已知未纳入检查的项**：`docs/book/` 的构建只验证「能构建」，不验证链接有效性（mdBook 默认不检查章节互链）；本次以脚本核对了全仓库 Markdown 的相对链接（0 处失效），但该检查尚未接入 CI。

### 首次 CI 运行的结果（PR #13，2026-10-17）

共 25 个检查（9 个 job + 16 个 crate 测试分片）：**23 通过、2 失败**。

**失败的是 `clippy` 与 `doc`，原因与 lint 无关**：两者都以 `--workspace` 运行且未安装系统库，而 `libudev-sys` 的 build script 会执行 `pkg-config --libs --cflags libudev` 并在缺 `libudev.pc` 时 panic（exit 101）。这推翻了被取代的 17 份工作流里那条注释——「clippy 不链接最终二进制，所以不需要系统库」：**build script 在 `cargo check` 类构建中同样会运行**，是否需要系统库不能只看链接。

修复：把 apt 包列表从「多处各抄一份」收敛为 `.github/actions/system-deps` 复合 action，`clippy` / `doc` / `build` 共用基础集，`test` 仅对 `graphics` 分片追加 EGL/Mesa。这一处理与本 ADR 收敛样板是同一个理由（包列表是事实清单，抄本分叉会让某个 job 静默缺库）。

**通过并因此获证的部分**：16 个 crate 的测试分片全过，其中 `test (inox-compositor)` 确实装上了 `libudev-dev`、编译通过并运行了 50 个 lib 测试 —— 说明 `plan` 由 manifest 推导 `graphics` 的判定是正确的；`plan` 本身成功产出 16 项矩阵（本 ADR 点名的首个未验证环节）；`build (release)`、`xwayland-e2e`、`fmt`、`cargo-deny`、`markdownlint`、`mdbook` 均通过。

**CI 成本（回填「实施顺序与未决项」的阻塞项）**：整条流水线 wall clock ≈ **2 分 07 秒**；最长的 job 为 `xwayland-e2e` 2.0 min、`build (release)` 1.6 min、`test (inox-compositor)` 1.2 min，13 个桩 crate 分片各 0.3–0.4 min。相比 17 条独立流水线（各自从零编译同一批依赖、无跨仓库缓存共享），单一工作区流水线在墙钟时间上更好，**回退判据第 1 条未触发**。注意这是首个运行的观测值，且同一次运行内的分片会互相预热 rust-cache，稳态成本需在后续运行中复核。

### 尚未完成、需要人类动作

- **组件仓库尚未归档，且归档前有两件事必须先做。**
  1. ~~处理只存在于未提交工作树的文件~~ —— **本项基于一个错误判断，已撤销。** 本节初稿称 `inox-compositor` 的 5 个文件（`src/backend/drm.rs`、`src/backend/mod.rs`、`src/config.rs`、`src/lib.rs`、`src/runtime.rs`）「只存在于一台机器的磁盘上」。核实后不成立：这些内容的副本已随 PR #13 提交并推送到本仓库（`git cat-file -e origin/main:crates/inox-compositor/...` 全部存在，且与克隆未提交工作树字节相同）。当时把脚本的「不等于组件仓库的 ref」误读成了「不在任何 GitHub 仓库」。**正确的表述是：这些内容从未推送进*组件仓库*，但已推送进本仓库**——这是迁移的预期结果，不是待办。归档前无需为此做任何处理。
  2. **跑一次比对并留证**，确认没有文件丢失：`python3 scripts/verify-import-parity.py`（默认对 `HEAD`；用 `--committed-ref origin/main` 可额外证明已推送）。
- **建议归档（archive），不建议删除（delete）。** 本 ADR 未迁移组件仓库的提交历史（未决项 2），因此那 17 个仓库是下列内容的**唯一副本**：`inox-compositor` 15 个提交、`inox-xwayland` 10 个、`inox-common` 6 个、`inox-protocols` 6 个、`inox-config` 3 个、`inox-docs` 5 个，以及 `v0.1.0` / `v0.1.1` tag。归档是可逆的、保留历史与 tag，且不阻止任何人读代码；删除是不可逆的，并且**并不比归档多解决任何问题**。若确实要删除，应先决定是否要把历史导入本仓库。
- **CODEOWNERS 未落地。** 「失败、恢复与安全」把「根 `Cargo.toml` / `Cargo.lock` / 工作流变更需人类审查」列为控制爆炸半径的主要缓解手段，但本仓库目前没有 `.github/CODEOWNERS`，文档中也没有可确认的所有者 handle。**在补上该文件并把根配置路径登记为需审查之前，这一缓解手段不存在**——不应以本 ADR 提到它就认为已经生效。
- **分支保护未按新 job 集合更新。** 受保护检查名从各仓库的 `fmt / clippy / test / build / doc / deny` 变为工作区 job（含分片矩阵的 `test (<crate>)`）。在更新之前，门禁与新工作流的对应关系不成立。
- ~~**CI 成本未实测。**~~ 已在首个 CI 运行中观测并回填（见「首次 CI 运行的结果」）。仍需**稳态**数据：首个运行受益于同一次运行内互相预热的 rust-cache，不能代表缓存冷启动或长期成本。
- **提交历史未迁移。** 16 个 crate 以当前工作树导入，组件仓库的提交历史留在（待归档的）旧仓库中；`git log` 在本仓库无法追溯到组件仓库的历史。
