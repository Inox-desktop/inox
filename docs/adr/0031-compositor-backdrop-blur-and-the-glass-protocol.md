# ADR 0031: 合成器背景模糊（frosted glass）与 `inox-glass-v1` 私有协议

- 状态：proposed
- 日期：2026-10-17
- 决策者：架构组与合成器 / 面板维护者（待人类评审）
- 批准记录：待补（人类评审 PR / 评论与日期；合并草案不等于接受）
- 关系：补充 ADR 0006（协议策略与 `inox-protocols` 是私有协议的唯一出口）、ADR 0018（L3 组件的 layer-shell 宿主边界）、ADR 0009/0030（令牌取值与「缺失即默认」的同一套做法）；细化根级 `AGENTS.md` §4「新增协议必须先在 `inox-protocols` 定义并说明安全与生命周期」与 §7.2「私有协议 unstable，破坏性变更需 ADR 并在 PRD 矩阵更新」。**不**改变分层与依赖方向，**不**采纳任何上游协议，**不**批准对普通 toplevel 的表面模糊，也**不**声称真机效果已验证。

## 背景

产品方向要求桌面首页呈现 macOS 风格的磨砂玻璃：顶部菜单栏与底部 dock 是半透明材质，透过它们能看到壁纸与被压住的内容，边缘有一道高光。这两个表面由面板建模并绘制（`crates/inox-panel/src/menubar.rs` 与 `dock.rs`，材质取自 `inox-config` 的 `[glass]` 令牌组），面板也已经通过 `crates/inox-panel/src/layer.rs` 成为真实的 `wlr-layer-shell` 表面、经 `crates/inox-panel/src/frost.rs` 请求本协议。缺的从来不是表面，也不是材质取值，而是「表面背后的东西被软化」这一件事本身：它只能由持有合成 framebuffer 的进程完成。因此本 ADR 决定的是这种**材质如何实现**、以及组件通过什么契约向合成器索取它，而不是面板布局。

「由组件自己模糊」在架构上不成立，理由有三条，且都可以在锁定版本的源码里复核：

1. **客户端无法采样自己表面背后的像素。** 那些像素在合成器的 framebuffer 里，没有任何协议会把它们交出来 —— 这正是合成器存在的意义。`crates/inox-protocols/protocol/inox-glass-v1.xml` 的接口说明把这条写成了协议存在的前提。
2. **Iced 只能模糊它已经持有的内容。** `iced_tiny_skia-0.14.1/src/engine.rs` 中出现的 `blur` 全部是 `shadow.blur_radius`，即给本进程绘制的 quad 画投影，与「表面背后的合成结果」无关。Iced 的渲染器没有、也不可能有一个跨进程背景采样入口。
3. **Smithay 0.7 没有任何模糊能力。** 对 `smithay-0.7.0` 源码做 `grep -rniE "blur"` 得到 **0** 命中；`ext-background-effect` 也没有实现（`grep -rli "background_effect"` → 0 命中）。因此不存在「打开一个开关就有背景模糊」的现成路径，模糊 pass 必须由合成器自己写。

已核验的素材（截至 2026-10-17 本 ADR 撰写时点，引用均指向本仓库固定源码）：

- **协议规范**：`crates/inox-protocols/protocol/inox-glass-v1.xml`（XML 是唯一事实来源）。`inox_glass_manager_v1` v1 有 `destroy`（析构，不带走已创建的 glass 对象）与 `get_glass(id, surface)`；错误枚举 `already_created` = 0；`inox_glass_v1` v1 有 `set_radius`、`set_region`、`destroy`（析构）与唯一事件 `capabilities(max_radius)`。
- **绑定**：`crates/inox-protocols/src/glass.rs` 用 `wayland-scanner` 在编译期从上述 XML 生成 client/server 两半（与 `ipc.rs` 同一形状），并用 6 个单元测试冻结接口名、版本、请求集、唯一析构请求、唯一事件与 `already_created` = 0；其中一个测试断言 `capabilities` 的签名只含 `Uint`、不含 `Fd`/`Array`。
- **协议往返**：`crates/inox-protocols/tests/glass_roundtrip.rs` 在真实 Unix socket pair 上驱动真实 `wayland-client` 与服务端派发 —— `capabilities` 在客户端设置任何参数之前到达；`set_radius(48)` 与 `set_region(0,0,1000,28)` 逐字节到达服务端；非正尺寸（`0,0,0,0`）原样过线（由合成器读作「清空区域」）。
- **材质令牌**：`crates/inox-config/src/theme.rs` 的 `Glass { tint, tint_strong, highlight, shade, blur }`，内置深/浅主题各一份（`blur = 128`），`#[serde(default)]` 且未知字段报错，`glass.blur = 0` 是显式的「不做背景模糊，只做半透明」。
- **合成器（headless 路径已实现，撰写时点仍在收尾）**：
  - `src/glass.rs`：纯参数层（无 Wayland 对象、无 GL）。`MAX_BLUR_RADIUS = 64`（`capabilities` 原样报告，请求超限被钳制）；`normalise_region` 把 `set_region` 的四个 `i32` 变成含义（非正尺寸清空；原点与尺寸分别封顶到 `MAX_REGION_ORIGIN`/`MAX_REGION_EXTENT`，使后续坐标运算**按构造有界**）；`capabilities(layer_role, locked, frosting_supported)` 三者缺一即报 0；`collect_panes` 先裁剪到**表面自身几何**、再裁剪到输出，按输出缩放折算物理像素，并按「从后往前」排序。11 个单元测试 + 3 个 proptest。
  - `src/render.rs`：`HeadlessRenderer` 自己实现该效果（Smithay 0.7 没有模糊）。渲染目标因此从 renderbuffer 换成可采样的 `GlesTexture`；模糊是**两个 region 大小的可分离 scratch pass**，配一段 GLSL ES 1.00 的 shader（编译一次缓存于渲染器），按 `GlassPane.at` 从场景后方向前逐 pane 执行；没有 pane 的帧与以前逐字节一致。
  - `src/state.rs`：广告全局 `inox_glass_manager_v1`（版本直接读自 `inox-protocols` 生成的接口，与 `inox_ipc` 同一做法）；`get_glass` 对已有对象的表面报致命错误 `already_created` 而不是静默替换；`capabilities` 取自 `glass_capability(surface)` = `capabilities(有 layer 角色, 已锁定, frosting_supported)`，其中 `frosting_supported()` 是 `self.renderer.is_some()`（即**只有 headless 路径**）；`scene_elements` 用 `collect_panes(已锁定, …)` 取 pane。
  - `src/backend/drm.rs` **不应用**磨砂：它照常从 `scene_elements` 取得场景，发现 `scene.glass` 非空时**每个进程只记一次** `warn!`（「该后端尚未实现磨砂，场景保持锐利，`capabilities` 报 0」），而不是每帧刷屏、也不是静默丢弃；该路径即因 `frosting_supported() == false` 而对客户端报 0（理由见第 3 条与「尚未实现」）。
  - `crates/inox-compositor/tests/glass_blur.rs`：判别性像素证据（headless、真实 `wayland-client`、一个测试一个 EGL display）。背景层画黑/白硬边，上面叠一块半透明 layer 表面（pane）并请求磨砂；同一场景渲染两次（无 glass 对象的**对照**与有效果的帧）逐列比较：pane 外硬边保持锐利（磨砂被裁剪到请求区域）、pane 内硬边变成数十列宽的斜坡（确实采样了合成后的背景而不是客户端自己的像素）、pane 区域与对照帧不同（不是恰好保留了锐利场景的空操作）。同一个测试还钉住协议前提：layer 角色表面拿到 `MAX_BLUR_RADIUS`、普通 `xdg_toplevel` 拿到 0、同一表面第二个 `get_glass` 是致命错误且违规连接被断开、存活客户端的磨砂不受影响。

## 决策驱动因素与备选

| 方案 | 收益 | 成本 / 风险 | 选择或否决依据 |
|------|------|-------------|----------------|
| 保持现状：组件只画半透明，不做背景模糊 | 零新协议、零新渲染代码 | 只是「变暗的矩形」，不是磨砂玻璃；产品方向明确要求能看见被压住的内容被软化 | 否决：需求就是背景模糊，半透明不等于磨砂 |
| 上游协议 `ext-background-effect-v1`（staging） | 语义与上游一致，将来互操作最好 | 今天没有任何一端就绪：`wayland-protocols` 0.32.13 只在 `protocols/staging/` 下带 XML（本工作区已启用 `staging` feature，所以 XML 可得），Smithay 0.7.0 **没有**服务端实现（零命中），本合成器也没有广告任何全局。staging 不等于已发布 | 否决为**首个**实现；但本决策**不排斥**将来改用它：语义（`capabilities` + `set_blur_region` + 每表面一个对象）与本协议同族，只要 Inox 需要的语义都能映射，采纳是兼容的演进 |
| 上游协议 `org_kde_kwin_blur`（KDE plasma 扩展） | KDE 生态已有实现 | XML 在 `wayland-protocols-plasma`（0.3.12）里；该 crate 只作为 `smithay-client-toolkit` 的**传递依赖**出现在 `Cargo.lock`，本工作区没有任何 crate 直接依赖它，也没有 Smithay 服务端实现 | 否决为**首个**实现：为一个私有能力引入 KDE 扩展并自写服务端，既不比私有协议省事，又把语义绑在第三方修订上 |
| **私有 `inox-glass-v1` + 合成器 GLES 背景模糊 pass** | 语义完全按 Inox 需要定义（layer 角色前提、锁定不生效、半径钳制）；与 `inox-ipc` 共用同一套绑定与不变量做法；不新增第三方依赖 | 新增一个私有协议与一条稳定性负担；模糊是一次真实的 GPU 合成成本 | **采纳** |
| 合成器隐式模糊每一个半透明表面 | 客户端零改动 | 魔法行为：客户端无法声明意图、无法关闭、无法限定区域；成本不可预测，且「半透明」被赋予一个新语义 | 否决：效果必须由客户端显式请求，成本必须可钳制、可归因 |
| 客户端自己画「壁纸 + 模糊」的整幅背景 | 不动合成器 | 客户端拿不到其它窗口的内容，也拿不到实时壁纸；内容一更新就陈旧，且等于让每个组件重复合成 | 否决：技术上做不到「背后的内容」，只能做到一张假图 |

## 决策

### 1. 协议：`inox-glass-v1`，私有、unstable、以 XML 为唯一事实来源

新增私有协议 `inox-glass-v1`，绑定与规范住在 `inox-protocols`（ADR 0006 的同一入口），**不**复制、**不**修改任何上游 XML。接口版本固定为 1，在 Inox 1.0 之前整体 unstable：消费者必须通过版本协商与 `capabilities` 事件获知能力，不得假定固定修订；破坏性变更走新接口版本 + ADR + PRD §2.3 矩阵更新（根级 `AGENTS.md` §7.2）。

### 2. 对象与请求语义（逐条按 XML，不另立规则）

- **管理者**：`inox_glass_manager_v1`。`get_glass(surface)` 创建 `inox_glass_v1`；`destroy` 只销毁管理者，已创建的 glass 对象**不**随之失效（它们属于各自的表面）。
- **一个表面至多一个对象**：对同一 `wl_surface` 第二次 `get_glass` 是**致命协议错误** `already_created`（值 0），合成器按协议断开违规客户端。理由是两份控制器会对同一块玻璃给出互相冲突的参数，且成本上限「每表面一份」会被打破。
- **新对象默认什么都不做**：初始区域为空、半径未设置，只创建不设置等于没有请求。
- **`set_region(x, y, width, height)`**：区域在**表面自己的逻辑坐标**里，相对表面左上角；越出表面的部分是**裁剪**而不是错误，表面内没有面积即「没有东西要模糊」；`width` 或 `height` **非正即清空区域**，客户端因此可以在不销毁对象的前提下停止磨砂。每个请求整体替换上一个值，效果从**下一个提交的帧**开始生效。
- **`set_radius(radius)`**：逻辑像素；**超过合成器上限的值被钳制而不是拒绝**（这样按缩放改尺寸的客户端可以一直要「合成器能给的最大值」）；`0` 关闭模糊，表面按普通半透明表面合成。
- **`capabilities(max_radius)`**：对象创建后、任何请求的答复之前**恰好发一次**，报告合成器对该表面接受的最大半径（逻辑像素）。`0` 表示这个合成器**不会**模糊该表面的背景 —— 没有渲染支持、会话已锁定，或表面没有 layer 角色。客户端必须据此降级（见第 5 条），**不得**声称一个它没有拿到的模糊。
- **layer 角色前提**：效果需要合成器在不等待窗口管理决策的情况下知道表面的位置，因此只有带 layer 角色（`wlr-layer-shell`：面板、dock、通知栈）的表面会被磨砂；**任何其它表面（普通 xdg toplevel 在内）报告 `max_radius = 0`**。客户端先建角色、后建 glass 对象；先建对象会拿到 0，可以销毁后重问。**广告一个自己不会兑现的上限是这条事件唯一绝不能做的事**，因为客户端据此决定降级路径。

### 3. 合成器侧：GLES 合成阶段的一次背景模糊 pass

模糊由合成器的 GLES 渲染阶段完成：对**已经合成到该表面之下的内容**按请求区域做一次模糊，再让客户端的半透明像素叠在结果上。参数层（`crates/inox-compositor/src/glass.rs`）把每个 layer 表面的 `Frost` 变成该输出的 pane 列表：区域裁剪到表面自身几何、再裁剪到输出，按输出缩放折算物理像素，并按「从后往前」排序，使一次磨砂不会模糊它本应位于其后的 pane。

**两条渲染路径的分工必须如实写明：**

- **headless 路径应用磨砂**（`HeadlessRenderer::render_elements` 自己实现：renderbuffer 换成可采样的 `GlesTexture`，两个 region 大小的可分离 scratch pass，按 pane 从后往前绘制）。它就是 CI 与验收证据（离屏像素读回）使用的路径，因此模糊 pass 的可见效果在这一侧可测。
- **物理（DRM）路径当前不做磨砂**：它以 `frosting_supported = false` 参与 `capabilities`，因此对该路径上的表面**报 0**、场景保持锐利。具体理由来自实现：物理路径经 `OutputDamageTracker` 呈现，它的 damage 派生自渲染元素自身的 damage 快照，而磨砂的 passthrough 目前**没有诚实的 damage 描述** —— 于是该路径选择「不广告一个不会绘制的模糊」，而不是留下无法解释的重绘/损坏。这是本 ADR 记录的**未决项**（真机效果也没有设备可验，见「尚未实现」），不是被本 ADR 认可的终态；将来给出诚实的 damage 描述并把 pass 接到物理路径时，`frosting_supported` 随之变为真。

无论哪条路径，像素**永不**离开合成器（第 4 条）。

### 4. 授权、成本上限与生命周期（评审重点）

- **模糊后的像素不离开合成器。** 客户端只收到一个整数半径：唯一的返回事件是 `capabilities(uint max_radius)`，绑定测试断言其签名不含 `Fd`/`Array`，往返测试只断言半径跨回客户端。因此这不是屏幕读取能力，**不能被用来观察其它客户端的内容**。
- **请求被裁剪到它自己的表面。** 区域在表面局部坐标里并被裁剪到表面：客户端既不能磨砂、也不能迫使合成器为它**没有绘制**的区域做工作。这是「请求的作用域＝表面」的形式化表述。
- **成本有三重上限**：半径被钳制到合成器上限（`MAX_BLUR_RADIUS = 64`）；区域被裁剪到表面、再裁剪到输出，且存下的原点/尺寸本身封顶，使坐标运算按构造有界；每个表面至多一个对象（第二个 `get_glass` 是致命错误）。因此单个客户端能触发的模糊工作量有界，且可归因到具体表面。
- **生命周期**：磨砂只在表面 mapped 且 committed 期间生效；随**表面**、**glass 对象**（`destroy`）或**连接**（客户端死亡）结束，表面本身不受影响。参数以 `WlSurface` 为键存放在 `InoxState::frost`，清除点有四处且都必须保留：`CompositorHandler::destroyed`（表面消失）、glass 对象的 `Dispatch::destroyed`、`detach_layers_of`（layer 条目被移除）与 `prune_frost`；死表面不得再被磨砂，也不得把参数留给复用的对象 id。管理者对象的 `destroy` 不影响已存在的 glass 对象。
- **锁定会话不生效，也不会泄漏锁定内容。** 锁定期间合成器的 `scene_elements` 只合成锁定场景（ADR 0022 的四处强制点之一），普通 layer 表面**根本不参与合成**，因此磨砂既不可能作用于锁定场景，也不可能把锁定内容带出去；XML 明确把「a locked session」列为 `capabilities` 报告 0 的情形，客户端据此降级。glass 对象本身不构成任何特权，它不能被用来锁定、解锁或影响会话状态。

### 5. 材质令牌与降级

模糊半径不是硬编码常量，而是 L1 主题令牌组 `Glass { tint, tint_strong, highlight, shade, blur }` 的一个字段（`crates/inox-config/src/theme.rs`，ADR 0009 的令牌模型）。面板/dock 的组件**不得**自己写死 tint：材质只有这一处来源（根级 `AGENTS.md` §4 已禁止硬编码颜色）。组件拿到 `capabilities = 0`、或绑定不到 `inox_glass_manager_v1` 时，必须退化为**纯半透明**：仍然画 `tint`/`highlight`/`shade`，只是没有背景软化；`glass.blur = 0` 就是这个降级的显式取值，而不是一个隐藏开关。

### 6. 分层与依赖

不新增任何依赖**方向**：绑定在 L2 的 `inox-protocols`（ADR 0002 规则 4 的唯一出口），合成器消费它（既有的 L2→L2 同层例外，`same_layer_exceptions` 不变）。`inox-panel` 目前**还没有**这条依赖；当它接上协议时新增的是 `inox-panel → inox-protocols` 这条 **L3→L2（向下，合法）**边，与 `inox-lock` / `inox-notifications` 在 ADR 0022 中的做法同形，且仍不得依赖 `inox-compositor`（ADR 0018）。根 `Cargo.toml` 的层表与 `[workspace.dependencies]` 的既有条目不变。

## 失败、恢复与安全

- **合成器不支持、或表面没有 layer 角色、或会话已锁定**：`capabilities` 报 0；客户端按第 5 条降级为纯半透明，不重试轰炸（重试只允许发生在表面**获得了 layer 角色**之后，即销毁旧对象再建一个新的）。
- **同一表面第二个对象**：`already_created` 是致命错误，违规客户端被断开，其余客户端不受影响（与 W04 的隔离要求同族）。
- **区域越界 / 非正尺寸**：不是错误；越界部分被裁剪，非正尺寸读作清空。这是客户端唯一的「停止磨砂」表达，必须保持幂等。
- **半径超上限**：钳制而非拒绝，客户端可以继续使用同一对象。
- **表面、对象或连接消失**：磨砂随之结束，无残留状态；对象的销毁不需要客户端先清空区域。
- **锁定**：锁定期间不合成任何普通层（ADR 0022），因此不存在「锁定前留下的 glass 对象在锁定后继续施加影响」的路径；合成器对锁定期间的新请求报 0。
- **不受本模型保护的**：模糊是一个真实 GPU 成本，本 ADR 只给出上限（半径、区域、对象数），**没有测量结果**；性能预算仍由 P01/P03 的实机测量决定，不能由本 ADR 的存在推断。协议本身不提供任何像素读取，因此不引入新的信息泄漏面。

## 兼容、迁移与回退

- **协议**：新增一个全局 `inox_glass_manager_v1` v1。旧客户端不受影响；绑定失败的客户端必须能继续以纯半透明工作（第 5 条）。这是私有协议，版本 1 尚未被任何已发布版本承诺，破坏性变更只需按 ADR 0006/§7.2 走新版本。
- **配置**：`[glass]` 令牌组是**增量**字段，`#[serde(default)]` 让所有旧 `theme.toml` 继续解析并继承内置玻璃（沿用 ADR 0009/0030 的缺失即默认语义），键名不变，因此不是配置格式的破坏性变更。
- **上游准备**：`wayland-protocols` 0.32.13 已在 `staging` feature 下带上 `ext-background-effect-v1` 的 XML（`capabilities` + `set_blur_region` + 每表面一个对象），本工作区已启用该 feature，所以**将来改用上游协议不需要新增依赖**；届时需要 Smithay 暴露服务端实现，或 Inox 在自己的协议条目里承担该角色，并另写 ADR 说明语义映射（尤其「layer 角色前提」与「锁定报 0」这两条 Inox 特有的策略）。
- **回退**：删除全局广告与 `glass` 绑定模块即回到今天之前的行为（没有任何磨砂），`inox-config` 的 `[glass]` 组可保留（字段被忽略）或整体删除；不需要迁移脚本。

## 验收与证据

| 验收 ID / 场景 | 责任仓库 | E0–E4 等级 | 测试与失败注入 | 通过条件 / 证据位置 |
|----------------|----------|------------|----------------|----------------------|
| 协议往返（`capabilities` 先到、半径/区域原样、非正尺寸清空、事件不含像素） | `inox-protocols` | E0+E1 | 真实 Unix socket pair + 真实 `wayland-client` 与服务端派发 | `crates/inox-protocols/tests/glass_roundtrip.rs`（**已存在**）；接口/请求/事件/错误码由 `src/glass.rs` 的 6 个单元测试冻结 |
| 能力与参数判定（普通 toplevel / 锁定 / 无效果路径都报 0；半径钳制；区域清空与封顶；pane 裁剪到表面与输出） | `inox-compositor` | E1 | 11 个单元测试 + 3 个 proptest（钳制幂等单调、规格化全定义且有界、pane 恒在输出内且排序） | `crates/inox-compositor/src/glass.rs`（**已存在**，纯逻辑） |
| R01+Q02 / 磨砂像素证据（判别性：pane 内硬边变斜坡、pane 外保持锐利、与无玻璃对照帧不同） | `inox-compositor` | E2 | 进程内真实客户端：背景 layer + 半透明 layer pane，同一场景渲染两次（对照 / 有效果），逐列比较；另断言 layer 角色拿 64、普通 toplevel 拿 0、第二个 `get_glass` 断开违规连接且不影响存活客户端 | `crates/inox-compositor/tests/glass_blur.rs`（**已存在**） |
| W04 同族 / 每表面一个对象 | `inox-compositor` | E1+E2 | 同一表面第二次 `get_glass`：违规连接按 `already_created` 断开，其余客户端继续渲染与输入 | 同上 |
| 磨砂的**视觉**证据（与上面同一帧，给人看） | `inox-compositor` | E2 | 同一个测试在 `INOX_SCREENSHOT_DIR` 有值时写出对照帧/磨砂帧及其 4× 最近邻放大图；放大图里 pane 内的硬边变成斜坡、对照帧仍是硬边 | `target/ui-redesign/inox-compositor/inox-glass-{control,frosted}[-zoom].png`（由 `tests/glass_blur.rs` 写出，不设变量则不落盘） |
| 桌面首页端到端（真实合成器 + 两个真实面板进程，合成帧读回为 PNG） | `inox-compositor`、`inox-panel` | E2 | `scripts/desktop-home-smoke.sh`：`--headless` 合成器 + `--test-input` 的 `dump-frame`，断言/观察两个 layer 表面的锚点、独占区与居中位置，以及菜单栏与 Dock 真的出现在合成帧里。**注意**：没有壁纸客户端时打破平衡的背景是纯色，因此这张图证明的是「首页布局与材质」，磨砂本身的可视证据是上一条 | `scripts/desktop-home-smoke.sh`（**已存在**）；本机一次运行的输出 `target/ui-redesign/inox-compositor/inox-desktop-home.png` |
| D05 / 降级 | `inox-panel`、`inox-compositor` | E1+E2 | 普通 toplevel 得到 `max_radius = 0`（**已存在**）；`capabilities = 0` 时面板不发送半径/区域、继续提交帧并只记一条 `info!`；`glass.blur = 0` 时**不创建** glass 对象；绑定不到 `inox_glass_manager_v1` 时同样纯半透明（**已存在**） | `crates/inox-panel/tests/protocol.rs`（线级：两种降级的取值断言）＋ `src/frost.rs` 的决策表单元测试；合成器侧 capability 断言在 `tests/glass_blur.rs` |
| S02 / 锁定不生效 | `inox-compositor` | E2 | 同一场景（背景层黑白硬边 + 半透明 pane + 真实 glass 对象）先渲染一次证明效果活着，再取真实会话锁：锁定帧只含锁表面（黑白图案像素 57600 → **0**，pane 内过渡列 24 → **0**，76800 个像素全部改变，逐像素与锁表面颜色的通道差 ≤1）；锁定期间为新建的 layer 表面 `get_glass` 得到 `capabilities = 0`，其后的 radius/region 请求改变锁定帧 **0** 个像素；锁持有者仍只收到 `[locked]`（无 `finished`）；解锁后同一场景回到屏幕（白色像素恢复 57600）。另做了两次变异实验确认断言是判别性的（让 `capabilities` 忽略 `locked`、去掉 `scene_elements` 的锁定提前返回，各自都让用例失败，随后按 md5 还原） | `crates/inox-compositor/tests/session_lock.rs`（**已存在**；纯逻辑侧另有 `glass::collect_panes(locked = true)` 的单元测试） |
| C01 / 令牌（`[glass]` 增量字段） | `inox-config` | E1 | 旧 `theme.toml` 无 `[glass]` 仍解析并继承内置玻璃；未知玻璃字段报错并给出字段路径 | `crates/inox-config/src/theme.rs` 的单元测试（**已存在**） |
| P01/P03 / 成本 | `inox-compositor` | E4（未取） | 固定参考设备上按 P01 的负载比较开/关玻璃的帧耗时与空闲占用 | **没有测量结果**；本 ADR 不预填 pass |
| H02 / 真机呈现 | 人类验收者 | E4（未取） | 登记硬件上开/关玻璃的可见效果与稳定性 | 本环境**没有 DRM 设备**，真机效果未验证 |

验收规范见 [`../quality/acceptance.md`](../quality/acceptance.md)。「pane 内的硬边变成斜坡、pane 外保持锐利、且与**未创建 glass 对象**的对照帧明显不同」是区分「真的有背景模糊」与「只是半透明」的判别性断言；只断言「协议往返成功」或「渲染没崩」不构成该行为已实现。

## 实施顺序与未决项

1. `inox-protocols`：XML + `glass.rs` 绑定 + `glass_roundtrip.rs`（**已完成**，见上）。
2. `inox-config`：`Glass` 令牌组（**已完成**）。
3. `inox-compositor`：纯参数层、全局广告、服务端派发（含 `already_created`、layer 角色判定、锁定报 0）、headless 的 GLES 模糊 pass 与 `tests/glass_blur.rs` 的判别性像素证据（**已落地**，见上）。物理路径的 `frosting_supported` 保持假，直到给出诚实的 damage 描述并在真机核对。
4. `inox-panel`：菜单栏/dock 材质接上 `inox-glass-v1`（先取得 layer 角色再创建对象）、`capabilities = 0` / 无全局时的纯半透明降级，以及承载这一切的 `wlr-layer-shell` 协议腿（`crates/inox-panel/src/{layer,frost}.rs`，自有连接、`Argb8888` 预乘共享内存帧、指针输入）。**已落地**，线级证据在 `crates/inox-panel/tests/protocol.rs`（真实 `wayland-server` 扮演合成器：锚点/layer/exclusive zone/margin/键盘交互/namespace/尺寸、`get_glass` 晚于 layer role、半径按 capabilities 钳制、configure 变化时重发区域、`max_radius = 0` 与 `blur = 0` 的两种降级、缺 `zwlr_layer_shell_v1` 时非零退出）。
5. 文档：PRD §2.3 与 §9.2、`docs/quality/ui-guidelines.md`、文档站点协议页与令牌页、`inox-protocols` 的 `AGENTS.md` 全局清单。

**尚未实现 / 未决项（What is NOT implemented / open）：**

- **物理（DRM）路径保持锐利，是明确的未决项。** 物理路径的 `frosting_supported()` 为假（它没有 `HeadlessRenderer`），且磨砂的 passthrough 在 `OutputDamageTracker` 上还没有诚实的 damage 描述，因此该路径报 `max_radius = 0`、场景不被磨砂，并在发现 pane 时**每进程一次** `warn!` 说明原因。把磨砂接到物理路径需要先给出诚实的 damage 描述并做真机验证 —— 本环境没有 DRM 设备，模糊在真实 scanout、damage 跟踪与功耗上的表现是 E4，未取；**不得**把 headless 的 E2 像素证据读作真机已支持。
- **非 layer（窗口）表面不模糊。** 普通 xdg toplevel、锁表面、输入法候选窗都得到 `max_radius = 0`。将来若要支持窗口级磨砂，须先解决「位置在窗口管理决策后才知道」的时序问题，并另写 ADR。
- **没有动画与放大。** 不做「液态玻璃」「鼠标附近的放大镜」等动态效果；半径是一个静态值。
- **没有性能数据。** 本 ADR 只规定了成本上限（半径钳制、区域裁剪、每表面一个对象），**没有**帧耗时、GPU 占用或空闲功耗的测量。
- **消费方已接线，但真机上拿不到模糊。** `inox-panel` 的菜单栏与 dock 是真实的 layer 表面：role 存在后创建 glass 对象、按 `capabilities` 钳制半径并发送区域，`capabilities = 0` 或拿不到全局时退化为纯半透明（`crates/inox-panel/src/frost.rs` 的决策表 + `tests/protocol.rs` 的线级断言）。但物理（DRM）路径报 0（见上一条），因此**在真机上这两个表面目前画的是纯半透明**，磨砂只在 headless 合成路径（CI 与验收证据使用的那条）成立。另外 dock 的表面宽度由 `DockContent`（图块数 + 托盘标签的**字符估算** + 面板自身句子的保底槽位）推出，所以没有窗口服务时空 dock 也是一个可读的居中胶囊（16 px 细条已修，`tests/protocol.rs` 断言 512 px 与「句子在表面内」）；但字符估算是估算而非度量：托盘里的 CJK 标签可能被低估（全宽字形约 1 em，估算按 0.5 em），比估算更宽的字体可能裁掉最后几个字形，窗格的柔和阴影也会被表面边缘裁掉（表面恰好等于窗格）。
- **协议在 Inox 1.0 之前 unstable**；上游采纳路径（`ext-background-effect-v1`）见「兼容、迁移与回退」。

## 后果

正面：磨砂玻璃第一次有了**可实现的路径** —— 模糊由唯一持有合成 framebuffer 的进程完成，客户端通过一个语义明确、成本可钳制、像素不可读回的私有协议索取它；协议与绑定有了往返测试，令牌有了 L1 来源，合成器侧有了纯参数层（能力判定、裁剪、缩放、排序都有单元测试与 proptest 钉住）、全局广告、服务端派发、headless 的 GLES 模糊 pass（目标改为可采样的 `GlesTexture`，两个 region 大小的可分离 scratch pass，逐 pane 从后往前；没有 pane 的帧逐字节不变）与判别性像素证据。安全边界（不读回、裁剪到自身表面、每表面一个对象、锁定不生效）在 XML、绑定测试与参数层测试里都有对应的可检验形式。

负面：工作区新增第二个私有 Wayland 协议与一条长期稳定性负担（unstable 直到 1.0）；合成器多了一次真实的 GLES 模糊成本与一段新渲染代码，而**它的性能与真机表现目前完全没有证据**，物理路径当前还明确不磨砂（报 0、场景锐利），因此真机上的菜单栏与 dock 目前仍是纯半透明——这是本次交付最大的缺口，而不是被接受的终态；面板一侧为此新增了一条 L3 → L2 的依赖边与一整套自有连接/共享内存帧/指针输入的协议腿（合法但不再是「只画界面」的组件）；上游采纳仍需 Smithay 或 Inox 自己补服务端实现。

影响面：`crates/inox-protocols`（`protocol/inox-glass-v1.xml`、`src/glass.rs`、`src/lib.rs`、`tests/glass_roundtrip.rs`、crate `AGENTS.md` 的全局清单）、`crates/inox-config`（`theme.rs` 的 `Glass`）、`crates/inox-compositor`（`src/glass.rs`、`render.rs` 的模糊 pass、`state.rs` 的全局广告/派发/`frosting_supported`、`backend/drm.rs` 的「不磨砂并每进程警告一次」、`tests/glass_blur.rs`、crate `AGENTS.md` 的全局清单）、`crates/inox-panel`（菜单栏/dock 与降级）、`PRD.md` §2.3/§9.2、`docs/quality/ui-guidelines.md`、`docs/book/` 的协议页与令牌页、`docs/adr/README.md` 的索引。

复审条件：上游 `ext-background-effect-v1` 离开 staging 且有可用服务端实现、Smithay 暴露背景效果能力、要支持窗口级（非 layer）磨砂、要把模糊成本纳入 P01/P03 的绝对预算、或 Inox 需要跨进程共享该效果语义时，重新评估本 ADR 的协议归属与成本模型。
