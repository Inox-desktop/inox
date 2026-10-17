# ADR 0019: 多输出拓扑、按输出的帧生命周期与多 GPU 设备模型

- 状态：proposed
- 日期：2026-10-17
- 决策者：架构组（待人类评审）
- 批准记录：待补（人类评审 PR / 评论与日期；合并草案不等于接受）
- 关系：扩展 ADR 0012 的「单一活动输出 + scale 1」条目，触发其复审条件「引入多 GPU … 时另写 ADR」；**不改变** ADR 0002 的分层与依赖方向，不引入新 crate、新第三方依赖或新不透明状态。ADR 0020 另行决定分数缩放。

## 背景

ADR 0012 已被接受，但它的实施记录把交付面显式限制为「单 GPU/一个活动输出/scale 1」，并把多 GPU、多 seat、HDR/VRR、远程输出与新渲染后端列为复审触发条件。M4 的验收项 O02/O03 要求真正的多输出：两个不同分辨率的输出（含负坐标）、拔掉主屏与全部拔掉后的 20 轮恢复、窗口与可操作入口迁移到存活屏、离线屏偏好保留、重接不重复创建面板；O04 还要求「每个输出独立进度，恢复时补一次必要重绘」。

当前实现（`crates/inox-compositor/src/state.rs`、`src/backend/drm.rs`、`src/runtime/physical.rs`）在四个地方把「一个输出」写进了类型与生命周期：

1. `InoxState` 持有单个 `output: Output`、`output_active: bool`、`output_global: Option<GlobalId>` 与一个 `work_area: Rectangle`；`replace_output(Option<Output>)` 是唯一的拓扑变更入口，语义是「全量替换」——它无条件 `remove_global` 并给所有 layer 表面发 `close`。这在单输出下正确，在多输出下会让「拔掉一块屏」毁掉另一块屏上的面板与壁纸。
2. `Workspace` 是一维的窗口顺序，`arrange` 只接受一个 `work_area`。没有「窗口属于哪个输出」的概念，因此无法表达 O02 的迁移，也无法给两个输出各自平铺。
3. `CalloopData` 持有**一个** `FrameSchedule`、一个 `paint_timer` 与一个 `watchdog`；`PhysicalState` 持有**一个** `DrmBackend`（单 `Scanout`）。ADR 0012 要求「每个输出至少区分 idle、scheduled、submitted、paused/removed」「每输出最多保留受控数量的 pending 提交」「热插拔/暂停后完成的旧事件带代次校验」——这些都需要按输出而非按进程计数。
4. `open_gpu` 只打开第一个能给出可用输出的 DRM 设备，其余设备被记录为诊断原因后丢弃。

不做这个决策的后果不是「少一个功能」：M4 的 O02/O03 无法开始，O04 的双输出补充项也无法在 CI 分片中得到任何证据；而把多输出继续塞进 `replace_output` 会让后续每一次显示管理改动都必须先拆掉这个假设。

### 已核验的上游能力（Smithay 0.7.0，锁定版本）

本次实现前核对了本机 Cargo registry 中 Smithay 0.7.0 的源码，以下能力确实存在，本文的决策不建立在草案 API 上：

- `desktop::space::Space` 本身就支持多输出：`map_output(&Output, location)`（`src/desktop/space/mod.rs:258`）把输出按逻辑坐标放进同一个空间，`output_geometry`（:288）返回「位置 + 按 scale 与 transform 折算后的逻辑尺寸」，`outputs_for_element`（:308）给出元素跨越了哪些输出，`refresh`（:330）据几何分发 `output_enter`/`output_leave`。渲染侧提供 `render_elements_for_region(renderer, region, scale, alpha)`（:381，把元素位置按 scale 折算成物理像素并相对 region 原点偏移）与 `render_elements_for_output`（:418）。
- `backend::renderer::damage::OutputDamageTracker::from_output(&Output)`（`src/backend/renderer/damage/mod.rs:286` 附近）与 `render_output`（:316）：其模式取自 `OutputModeSource::Auto(output)`，即尺寸、scale、transform 都从该 `Output` 的当前状态读；因此每个输出一个 tracker 即可，不需要自建坐标换算。
- 一个 `DrmDevice` 可以驱动多个 connector/CRTC：`DrmDevice::create_surface(crtc, mode, &[connector])`（`src/backend/drm/device/mod.rs:314`）按 CRTC 建面，`DrmDevice::crtcs()`/`planes()` 提供资源句柄。`GbmBufferedSurface` 的分配器来自该设备的 `GbmDevice`，同一设备可并存多个 scanout。
- `wayland::output::OutputManagerState::new_with_xdg_output` 按输出分发 `zxdg_output_v1`，无需为多输出新增协议。

### 未核验项（不得当作已批准）

- **跨 GPU 缓冲共享**：`backend::renderer::multigpu` 的 `GpuManager`/`MultiRenderer` 确认存在，且其 `ImportMemWl`/`ImportDmaWl` 实现不受 `use_system_lib` 门控（`src/backend/renderer/multigpu/mod.rs:1908`、:2017）。但 `MultiRenderer<'render, 'target, R, T>` 借用 `GpuManager` 的设备状态，要在 `PhysicalState` 里与 `GpuManager` 共存需要自引用或 take/put 的构造；本 ADR 不预先批准该重构，见「决策 3」的边界。
- 真实双 GPU 硬件上的 dmabuf 跨设备导入行为、VT 往返、20 轮热插拔均属 E4，本文不声称已验证。

## 决策驱动因素与备选

驱动因素：O02/O03/O04 的验收断言；ADR 0012 已有的「每输出独立生命周期」要求；保持 `layers.rs`/`layout.rs` 的纯逻辑与既有属性测试有效；不扩大 `unsafe` 面；在无 DRM 设备的 CI 上仍可验证。

| 方案 | 收益 | 成本 / 风险 | 选择或否决依据 |
|------|------|-------------|----------------|
| A. 保持单输出，`replace_output` 全量替换 | 改动最小 | O02/O03 无法实现；拔插会摧毁存活屏上的 layer 表面，直接违反 O02「重接不重复创建面板、入口迁移到存活屏」 | 否决：不满足已列出的验收项 |
| B. 多输出，但每输出一个独立的 `Space` 与独立的窗口集合 | 每个输出互不干扰，局部推理简单 | 全局逻辑坐标（含负坐标）与跨屏移动窗口/popup/指针要求一个连续坐标空间；多个 `Space` 会让 `element_under`、`outputs_for_element`、迁移都退化成手写映射 | 否决：ADR 0012 已定「统一使用有符号逻辑全局坐标」 |
| C. 单一 `Space` + 输出表 + 每个输出一份帧生命周期（本方案） | 复用一个已被上游验证的多输出空间；跨屏几何、命中与迁移都由 Smithay 的同一套逻辑得出；纯逻辑（放置、迁移、钳制）可属性测试 | 需要把 `InoxState` 的输出字段改成表并重写拓扑变更入口；物理后端要按设备与按输出两层重构 | 选择 |
| D. 每个 GPU 各自渲染自己的输出（本方案的设备模型） | 不需要把渲染器类型泛化到 `MultiRenderer`，`SceneElement` 仍是 `GlesRenderer` 的元素；每个 GPU 一个 EGL/GBM 上下文本来就是 `OutputDamageTracker` 与 GBM 分配所要求的 | 客户端缓冲若无法被目标 GPU 导入就画不出来；混厂商多 GPU 需要跨 GPU 复制路径 | 选择，并显式限定边界（见决策 3） |
| E. 全部输出由主 GPU 渲染，用 `MultiRenderer` 跨 GPU 输出 | 对所有缓冲区组合都正确 | `MultiRenderer` 的生命周期与 `GpuManager` 共存需要自引用/take-put 构造；当前客户端缓冲面只有 `wl_shm`（合成器尚未广告 `linux-dmabuf`），先做复制路径成本高且无法在 CI 验证 | 暂缓：作为 `linux-dmabuf` 落地时的后续 ADR，见「实施顺序与未决项」 |

## 决策

### 决策 1：输出的权威状态与身份

`inox-compositor` 是输出拓扑的唯一权威。运行时输出身份与持久化显示偏好身份继续分离（ADR 0012）：运行时身份是本次进程内铸造的 `OutputId`，进程结束即失效；偏好按 connector 名与用户配置匹配，不把 connector 序号或列表下标当稳定标识。

`InoxState` 用一张有序的输出表取代单输出字段。每个条目持有：`Output`、其 `wl_output` 全局、在全局逻辑坐标中的位置、该输出的逻辑工作区（扣除 layer 独占区后），以及一个由后端给出的**运行时键**。表的顺序即「主输出优先」的顺序，第 0 项是缺省主输出。

运行时键**不是** connector 名：一台机器上两块 GPU 可以各有 `DP-1`，只用名字做键会把它们并成一个输出。名字只用于对客户端广告与用户配置匹配，键用于识别「这是不是同一个输出实例」。DRM 后端用「显卡 + connector」组成键，headless 后端用配置名。已知限制：两块 GPU 的同名 connector 会在客户端看到两个同名 `wl_output`（对象不同、名字相同），消歧属于显示管理（`wlr-output-management`）的范围，不在本 ADR 内。

### 决策 2：拓扑变更是一个入口、一次事务

唯一的拓扑变更入口接受「期望的输出集合」，与当前集合求差，然后：

1. 名字与身份未变的输出保留其全局、其 layer 表面与其窗口，只更新几何（位置、mode、scale）；
2. 新增输出创建全局、映射进 `Space`、按顺序计算未显式指定的位置；
3. 消失的输出结束其运行时实例：属于它的 layer 表面收到 `close`（协议要求客户端为新输出重建），属于它的窗口**迁移到存活的主输出**，其偏好保留；
4. 全部变更应用完毕后一次性重算每个输出的工作区、重新安排窗口、重设指针位置与焦点、并调度一次重绘。

用户可见的语义：拔掉一块屏不会影响另一块屏上的 layer 表面、窗口或焦点；重接不会为同一块屏重复创建面板，因为同名输出走「保留」分支。

自动放置规则（未在配置中显式给出坐标的输出）：按输出表顺序，放在已放置集合的包围盒右侧、与包围盒顶部对齐；显式坐标优先。规则是纯函数，可属性测试「显式坐标不被改写」「自动放置的输出不与已放置输出重叠」。

### 决策 3：多 GPU 的设备模型与边界

枚举当前 libseat seat 上的**全部** DRM 设备，主 GPU 优先；每个设备打开自己的 libseat 租约、`DrmDevice`、`GbmDevice` 与 EGL/GLES 渲染器，驱动挂在自己名下的 connector。设备之间不共享渲染器。

边界必须写清楚，不得读作完整 PRIME：

- 支持：一台机器上有多块 GPU，每块驱动自己的输出；输出按设备分组，互不影响；设备级的断连/恢复不牵动其它设备。
- 暂不支持：跨 GPU 的缓冲**复制**路径（`MultiRenderer`）。当前合成器只广告 `wl_shm`，客户端缓冲全在主存，任何 GPU 都能导入，因此这一版的跨 GPU 行为是完整的。
- 已知限制：一旦 `linux-dmabuf` 全局落地，客户端缓冲可能位于某一块 GPU 上；届时目标 GPU 无法导入的组合（典型是混厂商与不兼容 modifier）会画不出来，必须在此之前引入复制路径。**这是 `linux-dmabuf` 的前置条件，不是可选优化**，登记在「实施顺序与未决项」。

### 决策 4：按输出的帧生命周期

`FrameSchedule` 的粒度从「每进程一个」改为「每输出一个」，代次、重试预算、空闲/调度/提交中/暂停/移除状态与陈旧完成校验全部按输出独立。headless 后端为每个输出保留独立的离屏渲染目标，`render_frame` 以输出为单位渲染。

保留既有不变量：无 damage 不提交；提交期间最多一个 pending；可重试失败有界（≤3）且不阻塞 calloop；page-flip 完成有超时；全部输出消失时停止重绘并等待设备事件，不忙循环。

### 决策 5：layer-shell 的按输出放置

`LayerEntry` 记录其所属输出。放置与独占区计算继续使用 `src/layers.rs` 的纯函数，但以「该输出的逻辑区域」为输入、以全局坐标输出，因此每个输出各有一份工作区，普通窗口只平铺在自己所属输出的工作区内。`new_layer_surface` 在客户端未指定输出时绑定主输出；指定了不存在的输出时按协议发 `close`。

## 失败、恢复与安全

- **部分失败**：新增输出创建全局或映射失败时，该次变更整体回退到变更前的输出集合并在日志中给出原因；不得只成功一半。物理路径下「没有任何输出可用」不是失败：保留会话与客户端连接，停止提交，等待 udev 事件。
- **陈旧事件**：DRM 完成事件的代次校验保留，并在多输出下按（设备, CRTC）匹配；已被移除的输出上的完成事件不得复活任何对象。
- **暂停/恢复**：libseat pause 停止全部设备的提交与输入分发，resume 重建每个设备的 swapchain 状态并各调度一次恢复帧；暂停期间到达的完成事件由代次校验丢弃。
- **资源**：每个输出的 pending 提交数上限为 1（与现状一致）；每个设备最多保留受控的 EGL/GBM 资源，设备消失时按「scanout → EGL/GBM → DRM master → libseat 租约」顺序释放。
- **安全边界**：本决策不新增任何协议全局、不新增权限、不引入网络行为。layer-shell 仍是普通桌面层，不提供锁屏保证；`inox_ipc` 的能力面不因多输出扩大。不新增 `unsafe`：物理后端的 EGL/GLES 构造仍是既有命名函数中的局部 `allow` 加 `SAFETY` 注释。

## 兼容、迁移与回退

- **配置格式**：`compositor.toml` 的 `[[outputs]]` 由「按顺序匹配的输出列表」扩展为可带 `position` 的多输出列表；新增键全部有默认值（缺席即自动放置），既有配置文件继续解析成功，属于**兼容扩展**而非破坏性变更（ADR 0009）。`scale` 的取值域在本 ADR 与 ADR 0020 中收紧为有界正数，解析期报错，不静默 clamp。
- **命令行**：`--output-size`/`--refresh-rate` 继续描述缺省的单输出；新增可重复的 `--output <SPEC>` 用于 headless 建立多输出虚拟拓扑（O02 要求「E1+E3 虚拟拓扑」证据）。不带 `--output` 时行为与今天完全一致。
- **Cargo 依赖**：不新增任何第三方或工作区内依赖，不改变分层与依赖方向（无新 crate、无 `[workspace.dependencies]` 变更）。
- **回退**：单输出的配置与命令行得到的拓扑与今天等价（一个输出、位置 (0,0)）。若多输出路径出现不可恢复的问题，可通过配置退回单输出而不需要回滚代码。

## 验收与证据

| 验收 ID / 场景 | 责任仓库 | E0–E4 等级 | 测试与失败注入 | 通过条件 / 证据位置 |
|----------------|----------|------------|----------------|----------------------|
| O02：2 个不同分辨率输出（含负坐标）；拔掉主屏、全部拔掉后逐个恢复，共 20 轮 | `inox-compositor` | E1+E3 虚拟拓扑 + E4 热插拔 | 进程内双输出集成测试：窗口落在各自输出、拔掉承载窗口的输出后窗口迁移到存活输出、layer 表面只在所属输出上；纯逻辑属性测试覆盖放置/迁移 | 迁移后无窗口失去归属、存活输出的 layer 表面未被 `close`、无忙循环；E4 部分另行登记 |
| O03（几何部分）：双屏不同 scale；跨屏移动窗口、popup 与指针 | `inox-compositor` | E1+E3 几何/像素 + E4 | 进程内双输出几何断言 + 指针跨屏命中；分数缩放的像素断言见 ADR 0020 | 逻辑/物理坐标、命中与光标对齐；不可用 mode 被拒绝且保留旧状态 |
| O04（补充项）：每个输出独立进度 | `inox-compositor` | E1+E2 | `backend::frame` 的按输出状态机单元测试；双输出下断言一个输出移除不影响另一个的调度 | 无无限重绘/重试；移除后另一输出继续呈现 |
| 多 GPU 设备枚举 | `inox-compositor` | E1 | 设备分组的纯逻辑测试；真实双 GPU 属 E4 | 每块设备只驱动自己的 connector；单设备机器行为与今天一致 |

本文档不预填任何 E4 结果；O01–O04 的真机步骤仍按 [硬件验收](../quality/l2-hardware-validation.md) 执行并由人类记录。

## 实施顺序与未决项

1. 先落地纯逻辑（放置、迁移、钳制）与其属性测试，再改 `InoxState`，最后改物理后端；每一步都保持 headless 回归全绿。
2. `linux-dmabuf` 落地**之前**必须解决跨 GPU 缓冲：要么采用 `MultiRenderer`（需要把渲染元素类型泛化到 `MultiRenderer` 并解决其与 `GpuManager` 的借用共存），要么按设备引入显式的复制路径。责任角色：合成器维护者；最迟阶段：`linux-dmabuf` 全局写入 `state.rs` 的全局清单之前。未解决前不得声称「多 GPU 已支持任意客户端缓冲」。
3. 显示试用（15 s 未确认自动恢复）与运行中的显示配置事务属于 ADR 0014 的配置应用范围，不在本 ADR 内。
4. 旋转、HDR/VRR、多 seat、远程输出仍不在范围内；它们的复审触发条件照 ADR 0012 执行。

## 后果

- 正面：M4 的 O02/O03/O04 有了实现基础；输出拔插不再具备摧毁存活屏的能力；帧生命周期按输出独立，故障不再全局化；多 GPU 在「每设备驱动自己的输出」这一模型下可用，且模型与今天的缓冲面（`wl_shm`）一致。
- 负面：`InoxState` 的输出相关字段从单值变成表，`replace_output` 被更复杂的事务入口取代；物理后端重构为「设备 → 输出」两层；多输出的真机行为（20 轮热插拔、跨屏 popup、光标对齐）需要 E4 才能声称通过，CI 只能给虚拟拓扑证据。
- 影响面：`inox-compositor`（主要）、`inox-config`（`OutputConfig` 扩展）、`inox-protocols`（无新增全局）、`inox-panel`/`inox-wallpaper`（按输出创建表面的前提）；PRD §2.3 协议矩阵与 §13 里程碑表的状态说明、根级 `AGENTS.md` 无需改动（不涉及分层与硬性规则）、`crates/inox-compositor/AGENTS.md` 与 README 需要同步。
- 复审条件：引入多 seat、HDR/VRR、远程输出、新的渲染后端，或广告 `linux-dmabuf`（触发决策 3 的跨 GPU 复制路径要求）时，另写 ADR。

## 实施记录（2026-10-17）

本次在用户明确要求「支持多 GPU、非整数缩放与多显示器」后实现。**ADR 状态仍为 `proposed`**：
本文的架构选择尚未经人类评审批准，实现合入不等于架构接受。

落地的内容：

- `crates/inox-compositor/src/topology.rs`（新）：纯几何——显式/自动放置、跨输出指针钳制、逻辑尺寸折算，
  含单元测试与 proptest（显式坐标不被改写、自动放置不与已放置输出重叠、钳制结果落在包围盒内）。
- `crates/inox-compositor/src/workspace.rs`：每个窗口带一个 `Option<OutputId>` 归属，
  `arrange` 改为按输出分别平铺，`reassign` 支持「输出消失→迁移」与「输出出现→收养孤儿窗口」。
- `crates/inox-compositor/src/state.rs`：输出表取代单输出字段，`apply_outputs` 成为唯一拓扑变更入口
  （保留/新增/替换/移除四条分支），layer 放置与工作区按输出独立，scene elements 按输出生成，
  frame callback 按输出发送，指针钳制走 `topology::clamp`。
  `OutputDescriptor::key` 是识别输出实例的键（DRM 后端为「显卡 + connector」），
  因此两块 GPU 上的同名 connector 不会互相顶替；`tests/multi_output.rs` 末尾有一条覆盖该碰撞的断言。
- `crates/inox-compositor/src/backend/drm.rs`：`GpuBackend` 改为「一个 DRM 设备 → 每 connector 一个
  `Scanout`」，`rescan` 增量重建并保留未变 connector 的 `Output` 实例；CRTC 被已用集合去重，
  不可用 mode 只影响该输出并保留其原状态。
- `crates/inox-compositor/src/runtime/physical.rs`：枚举并打开 seat 上每一块能驱动输出的 GPU，
  每块一个 libseat 租约/DRM 通知源/渲染器；每输出一份 `FrameSchedule`、一个 paint 定时器与一个
  page-flip 看门狗；单输出失败只撤下该输出，设备失败只撤下该设备。
- `crates/inox-compositor/src/render.rs`：headless 渲染目标改为每输出一个，按需分配与重建。
- `crates/inox-config`、`crates/inox-compositor/src/config.rs`：`[[outputs]]` 的 `position` 与可重复的
  `--output <SPEC>`，全部带解析期校验与测试。

已验证（E0/E1）：`cargo fmt`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`
与 `cargo test -p inox-compositor -p inox-config -p inox-protocols --all-features` 本地通过。
新增 `tests/multi_output.rs`：双输出广告与几何、窗口在主输出平铺、主输出消失后迁移、
全部输出消失后窗口仍托管、恢复后被收养、O02 的 20 轮拔插往返、重复应用同一集合为无操作（E1+E3）。

独立评审（针对本改动的多视角对抗式复核）在物理后端发现并修复了四处缺陷，均已确认不是误报：

1. **presented 通知丢失**：匹配到的 page-flip 完成事件不再调用 `InoxState::frame_presented`，
   于是真机上 `wl_surface.frame` 回调从不发送、`frames_rendered` 不增长——等待回调的客户端会停在
   空白窗口。已恢复为「只有匹配上的完成才算呈现」。
2. **拓扑刷新无条件 resume**：`refresh_topology` 会重置**每个**输出的帧代次，包括仍有 flip 在飞的输出，
   使该 flip 的完成永远匹配不上；它还忽略 `PhysicalState::active`，在 libseat 暂停期间
   （VT 切换/挂起时掉设备）重新启用提交与输入。改为只对新增输出建状态、对其余输出用 `damage` 排帧，
   并在会话暂停时只更新拓扑。
3. **首帧失败不再致命**：渲染失败与 page-flip 超时改走 `drop_output`，绕过了 `ready` 门禁，
   于是「请求了 DRM 却从未呈现一帧」会静默变成「没有输出」，既没有诊断也不退出（ADR 0012 要求失败并诊断）。
   新增 `fail_output`：首帧之前且这是最后一个输出时按启动失败处理。
4. **输出身份用了 connector 名**：两块 GPU 可以各有 `DP-1`，只按名字识别会把它们并成一个输出，
   帧表也按名字索引，导致第二块屏永远不被绘制。改为 `OutputDescriptor::key`（DRM 为「显卡 + connector」），
   `tests/multi_output.rs` 末尾新增一条同名 connector 的碰撞断言。

尚未完成：跨 GPU 缓冲复制路径（`MultiRenderer`，见「实施顺序与未决项」第 2 条）、旋转、
显示试用、以及 O02/O04 的全部 E4 真机步骤（真实热插拔、真实翻页）。E4 结果不得由上述测试推断。
