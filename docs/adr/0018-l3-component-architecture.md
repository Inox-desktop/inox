# ADR 0018: L3 组件的独立性、共享 UI 边界与 layer-shell 宿主

- 状态：proposed
- 日期：2026-10-16
- 决策者：架构组（待人类评审）
- 批准记录：待补
- 关系：补充 ADR 0003（技术栈）、ADR 0013（Shell 服务契约，`proposed`）、ADR 0014（配置应用）与 ADR 0015（锁屏与失败边界，`proposed`）。**不改变** ADR 0002 的分层与依赖方向：规则 1–7 与例外登记一律照旧。

## 背景

L3 的实现（PRD §13 的 M4）需要一次性决定三件相互牵连、且都会长期约束七个组件的事：组件之间共享什么、共享的东西放在哪一层、以及 layer-shell 表面由谁提供。三件事都不能靠「实现时顺手决定」，因为它们的代价都不可逆：加一条 Cargo 边需要 ADR 才能撤销，把 GUI 工具包牵进 L1 会让顶层基础设施依赖 UI 生态，而选错宿主方案会让四个组件在 Inox 自己的合成器上跑不起来。

现有约束把这三点夹得很紧：

- ADR 0002 规则 1 禁止同层横向依赖（L3 → L3），规则 3 禁止合成器与 Shell 之间的 Rust 依赖，规则 6 只允许 `inox-wallpaper` 单点访问合成器。这些规则由 `crates/inox-integration/tests/layering.rs`（验收项 B04）逐条机械断言。
- [ADR 0013](../adr/0013-shell-service-and-interaction-contracts.md) 明确：L3 组件不建立到其他 L3 仓库或 `inox-compositor` 的新 Cargo 依赖；共享数据只在既有 L1 的真实职责范围内提供；**不能把 UI 运行时塞进 `inox-common` 规避分层**。需要共享 UI crate 时**另写 ADR**。该 ADR 仍是 `proposed`。
- ADR 0014 要求 `inox-config` 不依赖 Iced、calloop 或设备 API —— 配置是数据与校验，不是 UI 运行时。
- [ADR 0015](../adr/0015-session-lock-and-failure-boundaries.md) 拒绝用 layer-shell 遮罩代替安全锁屏，并要求锁屏实现前完成人类安全审查。
- 根级 `AGENTS.md` §4 要求 Shell 组件基于 Iced 并遵循 Elm 架构、只消费主题令牌；§7.3 与 PRD §11.2 要求用户可见文案走 Fluent。

同时，本次实现必须面对两个**尚不具备**的前提：CI 里没有任何支持 `wlr-layer-shell-unstable-v1` 的宿主（`weston` 不支持它），而 `inox-compositor` 目前也**没有广告** `zwlr_layer_shell_v1`。ADR 0013 与 PRD §7 要求 M4 之前先用原型在 CI 中证明 Iced + layer-shell 可行；本 ADR 不声称该原型已完成。

考虑过但未被采纳的方案：

- **新增共享 L3 UI crate（例如 `inox-ui`）承载令牌映射、Fluent 装配与 layer-shell 引导**：技术上是本问题的正解，但它同时是新增 crate 与新增同层边，属 PRD §5.4 的破坏性架构变更，须先获人类批准。本 ADR 把它作为**提案**记录，本次不实施。
- **把共享 UI 辅助放进 `inox-common`**：复用面最广，但直接违反 ADR 0013 的明文禁止，并会让 L1 叶子依赖 GUI 工具包与 Wayland 客户端栈。不采用。
- **放弃 Iced、各组件自绘**：可省掉 `iced_layershell`，但违反 ADR 0003（Shell 组件必须基于 Iced 与 Elm 架构）。不采用。
- **等合成器支持 layer-shell 后再实现 L3**：顺序上最干净，但会让已验证的纯逻辑（任务模型、XDG 条目解析、通知队列策略、锁屏状态机）继续以骨架形式存在，无法被测试覆盖。不采用。

## 决策

### 组件独立，不新增任何 Cargo 边

L3 仍是七个独立进程，彼此之间以及与 `inox-compositor` 之间只通过 Wayland 协议、`inox-ipc` 与 D-Bus 协作。本次实现**不新增**同层边，也不新增 L3 → 合成器的边；`[workspace.metadata.inox.rules]` 未改动，`layering.rs` 的期望值不变。

### 共享能力下沉到既有 L1 职责内

- **国际化**：`inox-common::i18n` 新增 Fluent 目录加载、语言协商、默认语言回退与目录一致性检查（验收 Q03）。它**不是** UI 运行时：没有 Iced 依赖、不接触事件循环、不绘制任何东西，职责是「把消息 id 变成字符串」，与 `logging`、`paths` 同级。
- **配置 schema**：`shell.toml` 与 `notifications.toml` 的字段模型进入 `inox-config`（`shell.rs`、`notifications.rs`），`config.toml` 增加 `[locale]`。`inox-config` 因此成为消费方唯一的配置入口；它仍然不依赖 Iced、calloop 或任何设备 API（ADR 0014 不变）。
- **主题令牌**：已由 `inox-config::theme` 提供（ADR 0009），本次不新增令牌。

### 由此产生的重复是权衡，不是疏忽

没有共享 UI crate，就意味着七个组件各自持有一小段相同形状的代码：令牌 → `iced::Theme::Palette` 的映射、Fluent 目录装配、以及 layer-shell 宿主引导（每个约 100–150 行）。**这是本 ADR 明确接受的成本**，代价换来的是一条不需要 ADR 就能守住的分层边界：任何组件都可以独立编译、独立测试、独立失败，`layering.rs` 不必新增例外。

复审条件写在「后果」一节：当重复开始成为实际缺陷来源（同一处令牌语义被修 N 次、宿主引导行为分叉），应当以真实瓶颈与数量为依据，提请批准共享 crate 或登记同层例外 —— 而不是继续复制。

### Iced 版本与渲染器

- `iced` 0.14，`default-features = false`，显式启用 `tiny-skia`、`wayland`、`thread-pool`、`crisp`、`image`。**不启用 `wgpu`**：`tiny-skia` 是纯软件渲染，无 GPU 的 CI 与 llvmpipe 环境都可呈现，而 `wgpu` 会引入第二个没有任何 Inox 配置会选中的渲染器。软件渲染是 CI 与回退路径，不是性能验收基准（PRD §8）。
- MSRV 由 1.86 提升到 **1.88**：`iced` 0.14 与 `image` 0.25 都声明 `rust-version = "1.88"`。只移动声明的下限，CI 仍跟随 `stable`。
- `image` 只启用 `png` 与 `jpeg`：壁纸与图标路径需要的两种格式；不引入相机、AVIF、TIFF 等解码器。

### layer-shell 宿主

- 四个 layer-shell 客户端（`inox-panel`、`inox-launcher`、`inox-notifications`、`inox-wallpaper`）使用 `iced_layershell` 0.19（MIT，与 iced 0.14 同一代）获得 `zwlr_layer_shell_v1` 表面。`inox-settings` 是普通 `xdg-shell` 窗口，不需要它。`inox-shell` 本身不持有 layer 表面。
- 启动前先做**显式自检**：连接 Wayland、查询注册表里是否存在 `zwlr_layer_shell_v1`；缺失时以可操作的错误信息与非零退出码结束，而不是把控制权交给第三方 crate 后收获一个内部 panic。inbox 的每个 layer-shell 客户端都能在**没有**该全局的合成器上启动并给出这条信息，这一点是可测的（E1/E2）。
- `ext-session-lock-v1` 与 layer-shell 是两件事：`inox-lock` 不提供 layer-shell 遮罩（ADR 0015 明确拒绝该方案），它的表面协议腿在合成器广告该全局并通过人类安全审查之前不实现。

### 本次交付的边界（不声称 M4 通过）

本次实现交付各组件**权威职责的领域模型、服务契约与 Iced 界面**，即验收规范 §1 的 **E0/E1**（编译、lint、文档、单元与属性测试），状态层按 ADR 0013 的跨进程契约建模：状态快照 + 事件、请求代次、`sent/accepted/applied/rejected` 区分、loading/empty/error/disabled/retry 状态。**不交付**：

- E2/E3 的 GUI 证据：需要支持 layer-shell 的宿主，CI 目前没有（`weston` 不支持该协议），本地也无法验证；
- 合成器侧的 `zwlr_layer_shell_v1` 广告：合成器尚未支持，没有它，任何 L3 组件在 Inox 会话里都无法成为可见表面；
- 锁屏的协议腿与通知守护进程的 D-Bus 服务：前者被 ADR 0015 的安全审查与合成器能力阻塞，后者按 PRD §13 属 M5（D06）；
- Iced + layer-shell + 语义导出 + GUI 驱动链路的 CI 原型（ADR 0013、PRD §7 要求先于 M4 完成）。

因此本 ADR 与本次实现都**不构成 M4 的通过声明**；roadmap 与组件清单按此口径标注。

## 验收与迁移

- 本次实现的证据等级为 E0/E1：`fmt`、`clippy -D warnings`、`cargo doc -D warnings`、`cargo test -p <crate>` 全绿；每个组件的纯逻辑有单元测试，任务分组、布局几何、搜索排序与队列策略有 `proptest`（验收 B03 的 ≥1000 样本要求）；`en-US`/`zh-CN` 目录用 `inox_common::i18n::compare_catalogs` 断言 id 与占位符一致（Q03）。
- 迁移：`config.toml` 新增 `[locale]`、`shell.toml` 与 `notifications.toml` 首次获得字段模型 —— 全部字段带 `#[serde(default)]` 与完整默认值，既有的空文件与缺字段配置无需改动即可继续工作；未知字段仍报错（ADR 0009/0014）。本次不引入 schema 版本号，因为没有破坏性变更。
- 回退：删除 `crates/<l3>/*` 的实现与根 `Cargo.toml` 中的 L3 依赖块，即回到骨架状态；`inox-common::i18n` 与 `inox-config` 的新模块是纯增量，卸载后没有调用方。MSRV 回退到 1.86 需要同时移除 iced 依赖。
- 未决阻塞：合成器的 layer-shell 支持（L2 工作，需要自己的 ADR 或扩展现有 ADR 0006 的协议矩阵）；CI 的 layer-shell 宿主选择（`labwc` / `sway`，需要一个 opt-in 的 E2 job）；`iced_layershell` 的能力边界（弹出层、键盘交互、fractional scale）需要真实宿主验证后才能写入能力声明。

## 后果

正面：分层边界与依赖方向完全不动，`layering.rs` 无需新例外；七个组件保持独立编译、独立测试与独立失败；国际化与配置有唯一入口，消费方不再各自解析 `.ftl` 或 TOML；软件渲染让 CI 与无 GPU 环境都能构建与测试 UI 代码；layer-shell 自检把「宿主不支持」变成一条可读的启动错误。

负面：令牌映射与宿主引导在每个组件中各有一份（见「决策」与复审条件）；`iced_layershell` 是第三方 crate，其 API 与 iced 0.14 的代际对齐需要跟随上游；MSRV 提升到 1.88 会排除 1.86/1.87 工具链的使用者；`tiny-skia` 的软件渲染能力上限低于 GPU 渲染，分数缩放与动画效果需要真实宿主实测才能声明。

影响面：`crates/inox-common`（`i18n`）、`crates/inox-config`（`shell.rs`、`notifications.rs`、`root.rs`、`store.rs`）、七个 L3 crate、根 `Cargo.toml`（依赖与 MSRV）、仓库根 `AGENTS.md` §2 的层级说明与 PRD §3.1 的组件表状态、文档站点。根级硬性规则（unsafe、unwrap、日志、许可证、i18n、令牌）一律不放宽。

复审条件：① 共享 UI crate 提案需要人类批准，触发条件是重复数量或分叉缺陷成为实际成本；② 合成器广告 `zwlr_layer_shell_v1` 后重审 E2/E3 的可行性与宿主要求；③ `iced_layershell` 若停滞或与 iced 主版本脱节，重审宿主方案（自建 sctk 表面 + iced 离屏渲染）；④ 分数缩放、分数刷新率与多输出 DPI 混合进入验收前，需重审 `tiny-skia` 是否满足视觉质量要求。
