# ADR 0020: 分数缩放采用 `wp_fractional_scale_v1` 与按输出的缓冲区像素

- 状态：proposed
- 日期：2026-10-17
- 决策者：架构组（待人类评审）
- 批准记录：待补（人类评审 PR / 评论与日期；合并草案不等于接受）
- 关系：细化 ADR 0012 的坐标与缩放条目（该 ADR 已定「M4 至少支持异构分辨率、负坐标、100%/200% 整数缩放与 90° 旋转；分数缩放在协议与像素测试完成前仅为扩展配置」，并把引入新协议列为复审触发条件）；依赖 ADR 0019 的多输出拓扑；不改变 ADR 0002 的分层与依赖方向。部分取代 ADR 0012 中「分数缩放在协议与像素测试完成前仅为扩展配置」这一句的延期状态——本 ADR 补齐它要求的协议与像素测试口径。

## 背景

PRD §2.3 把 `fractional-scale-v1` 列为「计划」，并写明「宣称支持前需完成混合 DPI 与像素/命中测试」。验收项 O03 要求双屏分别为 100%/200%，并明确「125%/150% 为显式扩展配置，宣称支持时同样必测」。因此分数缩放不是一个可以「顺手打开」的开关：不开协议而只改配置值，客户端拿不到正确的缓冲区缩放，结果是模糊或错位；开了协议而没有像素测试，则是把「绑定存在」当成「支持」。

现状（`crates/inox-compositor/src/state.rs`、`src/backend/drm.rs`）：

- 输出一律 `Scale::Integer(1)`；DRM 后端在 `rescan` 里对 `preference.scale != 1.0` 直接返回 `Unavailable` 错误，即配置里写分数缩放会让物理后端拒绝启动。
- `scene_elements` 把所有元素位置按 `1.0` 折算成物理像素（`to_physical(1.0)`、`to_physical_precise_round(1.0)`），光标同样硬编码 `1.0`。
- 合成器创建的是 `CompositorState::new`，即 `wl_compositor` v5；v6 才有 `preferred_buffer_scale`/`preferred_buffer_transform`。
- 没有 `wp_fractional_scale_v1` 全局。

### 已核验的上游能力（Smithay 0.7.0，锁定版本）

- `output::Scale` 是 `Integer(i32) | Fractional(f64) | Custom { advertised_integer, fractional }`（`src/output.rs:174`），并提供 `integer_scale()`（不支持分数缩放的协议向上取整）与 `fractional_scale()`（`src/output.rs:194`、:205）。`Scale::Custom` 的存在说明「对整数协议广告一个值、对分数协议广告另一个值」是被显式建模的，不是本 ADR 的发明。
- `CompositorState::new_with_version::<D>(display, 6)` 创建 `wl_compositor` v6（`src/wayland/compositor/mod.rs:687`），`compositor::send_surface_state(surface, data, scale, transform)`（:409）按需发送 `preferred_buffer_scale`/`preferred_buffer_transform`，且内部对 v6 以下客户端直接返回。
- `wayland::fractional_scale` 提供 `FractionalScaleManagerState`、`FractionalScaleHandler`、`with_fractional_scale`，以及 `FractionalScaleState::set_preferred_scale(f64)`（`src/wayland/fractional_scale/mod.rs:238`，按 1/120 单位发送）与 `preferred_scale()`（:248）。
- 渲染与命中都不需要自建换算：`Space::render_elements_for_region`（`src/desktop/space/mod.rs:381`）与 `render_elements_for_output`（:418）都接受 scale 并把逻辑坐标折算为物理像素；`OutputDamageTracker` 的模式取自 `OutputModeSource::Auto(output)`，从 `Output` 当前状态读尺寸、scale 与 transform。
- 命中测试的 `Space::element_under`/`output_under`（:185、:203）工作在逻辑坐标，与 scale 无关；`wl_output.scale` 与 `zxdg_output_v1` 的 `logical_size` 由 Smithay 按 `Scale` 发送。

### 未核验项

- 真实客户端（GTK/Qt/wlroots 应用）在 125%/150% 下的实际缓冲区尺寸与模糊度属 E4；CI 只能断言合成器一侧的几何与像素。
- 后端为整数缩放自动选择的「向上取整」是否与所有目标客户端一致，未做跨工具包实测。

## 决策驱动因素与备选

驱动因素：O03 的混合 DPI 断言；PRD §2.3 对 `fractional-scale-v1` 的「宣称支持前需完成混合 DPI 与像素/命中测试」；不引入自建坐标换算（ADR 0012 要求「物理像素转换在后端边界执行并定义舍入规则」）；CI 无 GPU 也要能验证。

| 方案 | 收益 | 成本 / 风险 | 选择或否决依据 |
|------|------|-------------|----------------|
| A. 只支持整数缩放，`scale != 1` 继续拒绝 | 零风险，与现状一致 | O03 的 125%/150% 永远无法交付；PRD 已把它列为扩展配置 | 否决：不满足明确的验收项 |
| B. 只广告 `wp_fractional_scale_v1`，不给整数客户端任何提示 | 协议面最小 | v6 之前或未绑定该协议的客户端不知道应该渲染多大，会出现缩放错误或模糊；协议本身也要求与 `preferred_buffer_scale` 配合 | 否决：协议语义不完整 |
| C. `wl_compositor` v6 + `preferred_buffer_scale` + `wp_fractional_scale_v1`，渲染按输出 scale（本方案） | 两类客户端都拿到正确提示；渲染复用 Smithay 已有的按 scale 折算路径；像素证据可在 headless 下得到 | 需要新增一个协议全局并扩充协议面清单与测试；`Scale` 的取舍（向上取整 vs 品牌值）成为一个必须写明的产品行为 | 选择 |
| D. 引入 `wlr-output-management` 一并做显示管理 | 长线目标 | 该协议是「计划」级别，且它属于显示配置事务（ADR 0014），与渲染正确性无关 | 否决：与本决策无关的授权面扩张 |

## 决策

### 决策 1：缩放的表示与广告

输出的缩放因子是配置里的一个有界正数，运行时表示为 Smithay 的 `Scale`。本 ADR 选择 `Scale::Custom { advertised_integer, fractional }`：`fractional` 是配置值，`advertised_integer = ceil(fractional)`。

理由与后果：`wl_output.scale` 只能表达整数，协议规定分数缩放客户端通过 `wp_fractional_scale_v1` 取得精确值。向上取整保证不支持分数缩放的客户端按更大的缓冲区渲染，合成器再降采样——代价是显存与带宽，收益是不模糊。这与 Smithay `Scale::Fractional` 的默认行为一致；显式写成 `Custom` 是为了让「对整数客户端广告什么」成为代码里可见的一行，而不是库的隐式选择。

`1.0` 等整数取值仍表示为 `Scale::Custom { advertised_integer: 1, fractional: 1.0 }` 还是 `Scale::Integer(1)` 不影响协议输出（两者的 `integer_scale()` 都返回 1）；实现选择让 `advertised_integer` 与 `fractional` 由同一个函数推导，避免两处判断。

### 决策 2：协议面

- `wl_compositor` 提升到 v6（`CompositorState::new_with_version`），并在表面的相关状态变化时调用 `send_surface_state(surface, data, advertised_integer, Transform::Normal)`。
- 新增全局 `wp_fractional_scale_manager_v1` v1，实现 `FractionalScaleHandler`。客户端为表面创建分数缩放对象时，按该表面当前所在输出的 `fractional` 值 `set_preferred_scale`；表面跨输出移动或输出 scale 变化时重新设置。`set_preferred_scale` 只在值变化时发送，因此重复调用是安全的。
- 两个协议都进入 `state.rs` 的「已广告全局」模块文档清单，并由 `tests/protocol_surface.rs` 断言可绑定——这是 PRD §2.3「只有绑定或广告全局不能标记为支持」之外的**必要条件**，不是充分条件。

### 决策 3：渲染、命中与光标

- 每个输出的渲染以其 `Output` 为单位：元素位置、光标位置与光标缓冲都按该输出的 `fractional_scale()` 折算成物理像素，相对该输出的原点；不再有任何硬编码的 `1.0`。
- 命中测试、`pointer_location`、窗口几何与 work area 一律留在逻辑坐标；跨输出移动指针不需要坐标转换，因为输出被映射进同一个 `Space`（ADR 0019 决策 1）。
- 指针在无输出覆盖处的钳制按 ADR 0019 的纯逻辑函数执行，与 scale 无关。

### 决策 4：配置与取值域

- `OutputConfig.scale` 在解析期校验为 `0.25..=8.0` 的正数，越界报 `Parse` 并指出字段路径（ADR 0009 决策 4：越界必须在解析期报错，不得静默 clamp）。下界保证逻辑尺寸不会因除以极小数而溢出，上界远高于任何真实显示器。
- 物理后端**不再拒绝**非 1 的 scale。它仍然拒绝无法驱动的 mode（既有行为）。
- headless 后端接受同一组取值，使 O03 的虚拟拓扑证据可以在 CI 上取得。

### 决策 5：支持声明

在像素与几何测试到位之前，PRD §2.3 的 `fractional-scale-v1` 行保持「计划」，不升为「必须」。本 ADR 落地后它升级为「M4 必须」，前提是：几何/像素/命中测试在 CI 通过（E1+E3），且 E4 在真实混合 DPI 双屏上记录结果。协议已广告而 E4 未取时，支持矩阵中该行必须写明「已实现，E4 未取」。

## 失败、恢复与安全

- **输入校验**：`scale` 的取值域在解析期与构造期各控一次——解析期拒绝越界值；构造期对非配置来源的值做防御性处理（与 `LayoutConfig::sanitised` 同一模式）。
- **部分失败**：设置输出 scale 失败（例如后端拒绝）时，该输出保留原 scale 并记录原因，不使整个拓扑变更失败——scale 是显示属性，不影响输出是否存在。输出移除时，属于它的分数缩放偏好按 ADR 0019 的迁移规则随窗口迁移。
- **协议边界**：`wp_fractional_scale_v1` 与 `preferred_buffer_scale` 都只是提示，客户端可以不遵从。合成器必须在客户端给出任意缓冲区尺寸时仍然正确合成（既有的 viewporter/缩放路径已经如此），不得假定缓冲区尺寸等于逻辑尺寸乘以 scale。这是本决策最重要的失败模式：**不得把客户端的遵从当作正确性前提**。
- **安全**：本决策不新增权限、不新增输入源、不扩大 `inox_ipc` 能力面；一个只影响像素几何的协议全局不构成新的攻击面，但坐标换算必须使用饱和运算，避免客户端提交的极端尺寸在乘法中溢出（沿用 `layers.rs` 中已有的 i64 中间量做法）。

## 兼容、迁移与回退

- **配置**：`scale` 此前接受任意正有限数，现在收窄到 `0.25..=8.0`。这是**破坏性变更**：写出 `scale = 100` 的配置文件现在会在解析期报错。依据 ADR 0009 的「越界必须在解析期报错」这是期望行为，但必须在 PR 描述与文档中声明；`scale = 1.0`（默认）与所有 0.25–8 之间的既有取值不受影响。
- **协议**：`wl_compositor` 从 5 升到 6 是向后兼容的（客户端绑定低版本时使用 v5 语义）。新增全局不破坏任何既有客户端。
- **回退**：把 `scale` 写成 `1` 即可得到与今天逐像素一致的行为；不需要回滚代码。若 `wp_fractional_scale_v1` 出现不可恢复的问题，可先只保留 v6 的 `preferred_buffer_scale`（整数语义）。

## 验收与证据

| 验收 ID / 场景 | 责任仓库 | E0–E4 等级 | 测试与失败注入 | 通过条件 / 证据位置 |
|----------------|----------|------------|----------------|----------------------|
| O03：双屏 100%/200%；125%/150% 为显式扩展配置，宣称支持时必测 | `inox-compositor`、`inox-config` | E1+E3 几何/像素 + E4 | 配置解析用例（含越界拒绝）；进程内双输出几何断言；headless 像素断言：同一逻辑几何在 1.0/1.25/1.5/2.0 下产生预期物理像素的位置与尺寸 | 逻辑/物理坐标、命中、光标与 popup 对齐；越界值报 `Parse` 且保留旧状态 |
| 协议面：`wp_fractional_scale_manager_v1` 可绑定，`wl_compositor` ≥ 6 | `inox-compositor` | E1 | `tests/protocol_surface.rs` 扩展 | 全局清单与 `state.rs` 模块文档一致 |
| 客户端不遵从 scale 时的正确性 | `inox-compositor` | E1+E2 | 集成测试：客户端提交与逻辑尺寸不匹配的缓冲，断言合成器仍正确缩放/裁剪且不越界 | 不依赖客户端遵从 |
| E4：真实客户端在 125%/150% 下的模糊度与缓冲尺寸 | 人类验收者 | E4 | 混合 DPI 双屏实际操作 | 按 [硬件验收](../quality/l2-hardware-validation.md) 记录 |

## 实施顺序与未决项

1. 先落配置与协议全局（可绑定即通过协议面测试），再改渲染与光标，最后补像素断言。
2. 与 ADR 0019 的依赖关系：分数缩放要求「按输出渲染」已经成立，因此 0019 的实现先于本 ADR 的决策 3。
3. 未决项：PRD §2.3 中 `fractional-scale-v1` 行从「计划」升为「M4 必须」需要 E1+E3 证据到位；E4 由人类在混合 DPI 硬件上记录。责任角色：合成器维护者 + 人类验收者。

## 后果

- 正面：O03 的混合 DPI 验收有了实现与 CI 证据路径；DRM 后端不再因为配置里写了 1.5 而拒绝启动；缩放的取值域与舍入规则被写成代码里可见的一行，而不是库的隐式行为。
- 负面：新增一个协议全局与一个 v6 协议版本需要长期维护与测试；`scale` 取值域收窄是一次破坏性配置变更；像素测试无法替代真实客户端的模糊度验收。
- 影响面：`inox-compositor`（主要）、`inox-config`（取值域）、`inox-protocols`（重新导出 `wp::fractional_scale`，不新增 XML）、`inox-settings`（显示配置 UI 的取值域）；PRD §2.3 协议矩阵与 §13 里程碑说明、`crates/inox-compositor/AGENTS.md` 与 README、mdBook 的配置与架构页面需要同步。
- 复审条件：引入 `wlr-output-management`、每表面独立的缩放策略、HDR 或按表面动态切换缩放时，另写 ADR。

## 实施记录（2026-10-17）

本次在用户明确要求「支持非整数缩放」后实现。**ADR 状态仍为 `proposed`**：本文的架构选择
尚未经人类评审批准，实现合入不等于架构接受。

落地的内容：

- `crates/inox-protocols/src/standard.rs`：新增 `wp::fractional_scale` 再导出（不新增 XML）。
- `crates/inox-compositor/src/state.rs`：`CompositorState::new_v6`，新增
  `FractionalScaleManagerState` 全局与 `FractionalScaleHandler`；`update_surface_scale` 沿表面树
  向上找到所属输出，按需发送 `preferred_buffer_scale` 并 `set_preferred_scale`；
  `update_surface_scales` 在拓扑变更后刷新全部已显示表面。
- 渲染：`scene_elements` 与 layer/光标放置按输出 `fractional_scale()` 折算物理像素；
  headless 渲染器把该 scale 传给 `draw_render_elements`（元素**位置**是物理像素、**尺寸**由
  逻辑尺寸乘 scale 得出——这是本次实现中唯一一处必须显式传递的隐式约定）。
- `crates/inox-config`、`crates/inox-compositor/src/config.rs`：`scale` 取值域收紧为 `0.25..=8.0`
  并在解析期报错；`backend/drm.rs` 不再拒绝非 1 的 scale，改为 `Scale::Custom`（整数客户端取上取整）。

已验证（E0/E1）：本地 `fmt` / `clippy -D warnings` / 三 crate 测试全绿。
新增 `tests/fractional_scale.rs`：1.5x 输出的物理 mode、`zxdg_output_v1` 逻辑尺寸与
`wl_output.scale == 2` 分别断言；真实客户端提交 64x64 固体红缓冲后读回物理像素，
断言红块覆盖物理 `[12, 108)`（判别性：忽略 scale 的实现会得到 `[8, 72)`）、
其外是背景色、整帧是 960x540 物理像素（E1+E3）。
`tests/protocol_surface.rs` 断言 `wl_compositor` ≥ 6 与 `wp_fractional_scale_manager_v1` 可绑定。

尚未完成：真实混合 DPI 客户端在 125%/150% 下的模糊度与缓冲尺寸（E4）、
PRD §2.3 该行从「计划」升为「M4 必须」所需的人类评审确认。
