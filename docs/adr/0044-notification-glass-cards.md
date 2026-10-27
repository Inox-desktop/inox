# 0044 — 通知卡片采用共享玻璃组件（授权第二条 L3 → L3 边）

- **状态**：proposed
- **日期**：2026-10-26
- **决策者**：YG（人类）/ Inox contributors
- **相关**：[ADR 0040｜液态玻璃材质、`inox-glass-v1` v2 与系统玻璃组件库](./0040-liquid-glass-material-and-components.md)
  （本记录扩展它的组件库消费者范围，不改变它的协议、材质、渲染与降级决策）、
  ADR 0018（L3 组件的独立性、共享 UI 边界：需要共享 UI crate 时另写 ADR，本次是那条条款的第二次行使）、
  ADR 0033/0035（QML + Qt Quick 与 cxx-qt 绑定）、ADR 0002（分层规则 1：禁止同层依赖）、
  `docs/quality/ui-guidelines.md` §5、§12、
  `crates/inox-notifications/{Cargo.toml,build.rs,src/theme.rs,src/view.rs,qml/Main.qml}`、
  `crates/inox-panel/{src/theme.rs,src/view.rs,qml/Main.qml}`

## 背景

ADR 0040 把液态玻璃的**材质决策**（参数 clamp、v1/v2 协商、降级理由）与**五个无状态 QML 组件**
（`GlassPane` / `GlassButton` / `GlassToggle` / `GlassSlider` / `GlassTabBar`）下沉到 L3 的 `inox-glass`，
但它只授权了**一个**消费者：`inox-panel`（菜单栏与 Dock 的窗格）。其余 L3 组件仍然各画各的表面。

`docs/quality/ui-guidelines.md` §12 把玻璃限定在**悬浮的界面外壳**上，并明确点名三种允许的表面：
面板的菜单栏与 dock、浮动菜单、**通知栈**。也就是说「通知栈可以用玻璃」这条许可在 0040 落地时
就已经存在，只是实现没跟上——通知卡片至今仍是自绘的**不透明**矩形：`surface` 色填充 +
一条 hairline `border`（critical 时是 danger 色），见 `crates/inox-notifications/src/view.rs` 投影出的
`card.background` / `card.edge` 与 `qml/Main.qml` 里那个 `Rectangle` delegate。

通知卡片正是 §12 说的那种表面：它浮在用户的窗口之上，承载一段短句与最多几个动作，不是大面积正文；
它背后本来就有东西，玻璃在那里才有意义。

但把玻璃画进 notifications 只有两条路，两条现在都不可接受：

- **在本 crate 里再写一份 tint/highlight/shade 的取法**：`inox-glass` 存在的理由就是这件事只做一次。
  ADR 0040 记录的漂移（每个组件自己 clamp、自己决定何时创建对象、对 `capabilities` 各自解释）
  会原样发生在**材质**上：通知的 tint 与面板的 tint 会在某次令牌改动后分叉，而没有人会注意到。
- **让 notifications 直接 `import com.inox.glass`**：Qt 的 QML 类型按二进制注册，跨 crate 的静态模块
  注册不可靠（KDAB/cxx-qt#770）。面板已经因此选择「宿主 crate 在自己的 `build.rs` 里把组件编进
  自己的模块」（ADR 0040 §3），通知栈只能走同一条路。

于是这次改动同时是**新增一条 L3 → L3 边**（分层规则 1 禁止同层依赖）与**一次材质迁移**。
两件事都不能靠实现时顺手决定。

## 决策

### 1. 新增 `inox-notifications -> inox-glass`，且只在 build 期

- `inox-notifications` 在 `[build-dependencies]` 里依赖 `inox-glass`（以及 `qt-build-utils`，
  因为 `cxx-qt-build` 重新导出了 `QResource` / `QResources`，但没有导出 `QResourceFile`）。
- `build.rs` 遍历 `inox_glass::qml_files()`（路径以**对方 crate** 的 `CARGO_MANIFEST_DIR` 解析，
  从宿主的 build script 调用也不会错位），对每个文件用
  `QResourceFile::new(&path).alias(format!("qml/{name}"))` 挂进 `.qrc_resources(...)`，
  alias 是文件名，因此五个组件与 `CatalogWindow` 落在与 `Main.qml` 同一个资源目录里，
  成为**同目录隐式类型**（`GlassPane { … }`）。拿不到文件名的条目不 panic，跳过。
- **运行期不新增任何 Rust 依赖**：守护进程里没有一处 `use inox_glass`，也不创建任何
  `inox_glass_v1` 对象。这条边的作用只是让 `build.rs` 能读资产路径。
  （对比面板那条：`inox-panel` 在**运行期**也用它——`src/frost.rs` 调 `inox_glass::decide` 做降级
  决策。`inox-notifications` 不做这个决策：它没有 glass 对象可以决定。)
- 这条边登记进根 `Cargo.toml` 的 `[workspace.metadata.inox.rules] same_layer_exceptions`
  （`["inox-notifications", "inox-glass"]`，第四条例外）。`crates/inox-integration/tests/layering.rs`
  把 **build 依赖也当作边**，所以「只是构建期」不等于「不用登记」：删掉条目 CI 立刻变红。
  删除这条边时必须同时删除该条目。

### 2. 通知卡片改为 `GlassPane`，窗口清屏色改为透明

- **投影**：`card` 从 `{ background, edge, radius, inset, rail }` 变为
  `{ tint, highlight, shade, hairline, radius, inset, rail }`。三个颜色就是 `tokens.glass` 的
  `tint` / `highlight` / `shade`，`hairline` 复用原本画 `border.width` 的 `BORDER_WIDTH`；
  `theme::card_edge` 连同它的测试一起删除。投影里不再有 `card.background` 与 `card.edge`。
- **QML**：卡片 delegate 的根从自绘 `Rectangle` 换成 `GlassPane`，绑定上面五个属性。
  以下全部保持不变：`required property var modelData`、`popupCard` / `popup` 两个标记属性、
  `Accessible.role: Accessible.Pane` 与 `Accessible.name: popup.summary`、进场动画（目标仍是
  `cardItem`，静止值仍是 `opacity: 1` / `y: 0`）、urgency rail 的 `Rectangle`、关闭与动作按钮的
  `MouseArea`。卡片区域仍然没有颜色/尺寸字面量，也没有用户可见字符串。
- **窗口清屏色**：`Window.color` 从 `root.vm.palette.background` 改为 `"transparent"`。
  这是结构性哨兵，不是设计色：玻璃只有在背后能透出东西时才是玻璃，§12 把「拿不到效果」的形态
  定义为**纯半透明**。清屏色因此不是令牌，也不新增令牌。

### 3. 同一次迁移的另一半：面板的窗格也改由 `GlassPane` 绘制

面板原本把窗格画成**自己的**分档矩形（`theme::glass_stops` 的若干色带 + `Stop` 结构）。
本 ADR 一并把它换成 `theme::pane_style(tokens, strong, radius, shaded) -> PaneStyle`
（`tint` / `highlight` / `shade` / `hairline` / `radius` / `highlighted` / `shaded`）与投影里的一种
新图元 `kind: "pane"`，由 `qml/Main.qml` 实例化 `GlassPane` 渲染。

面板没有因此新增依赖边（`inox-panel -> inox-glass` 早已由 ADR 0040 授权并登记），但这是同一个
组件库从「面板自己画」切到「面板用组件」的那一半。只记录通知那一半，读者会以为面板没变。

### 4. 这次交付的证据边界

- **单元/投影（E0/E1）**：卡片投影的三种颜色等于 `tokens.glass` 的对应令牌，`hairline` 等于
  `BORDER_WIDTH`；面板的投影测试断言 `kind: "pane"` 携带的值来自 `pane_style`。
- **加载证据（E1）**：通知的离屏自检（`QT_QPA_PLATFORM=offscreen` 子进程走真实 `--self-check`）
  会加载模块并等 `reportReady`。`GlassPane` 没有被编进模块时，这一步会以
  `is not a type` 之类的 QML 错误失败——这正是「组件真的被编进了本 crate 的模块」的证据。
  实测把 `.qrc_resources(glass_resources())` 拿掉后正是这条失败并报 `GlassPane is not a type`。
- **像素证据在面板侧，且本次改动之后仍然成立**：面板的离屏帧测试
  `render_a_translucent_dock_frame`（`crates/inox-panel/src/headless_tests/qml.rs`）断言
  Dock 窗格内部某一行（`y = ceil(edge_height)`，正好在顶部 highlight 发丝之下）的像素**逐通道
  等于 `premultiplied(tokens.glass.tint)`，容差 ±1**，并断言窗格下方 margin 条带 alpha 为 0、
  帧里存在 `0 < alpha < 255` 的像素。在本次改动**之前**，这条断言断言的是面板自己铺的色带；
  **之后**它断言的是 `GlassPane` 真的把 tint 画对了——同一个断言换了绘制者，原样通过。
  这是「组件与旧绘制等价」最直接的证据，也是组件库的第二个像素证据点（第一个仍是目录页）。
  该测试的保护是真实的：把 `Main.qml` 里的 `GlassPane` 改成不存在的类型名，它会立刻失败。
  但**通知栈自身仍然没有像素测试，也没有截图**，这一点不因上面的证据而改变。
- **分层证据**：`cargo test -p inox-integration --test layering` 断言这条边已登记。实测把条目从
  `same_layer_exceptions` 删掉后的报错是
  `rule 1 (same layer): inox-notifications (L3) depends on inox-glass (L3); … inox-notifications -> inox-glass (build dependency)`
  ——末尾的 `(build dependency)` 正是「构建期也算边」的证据。
- 以上全部是 E0/E1。**没有 E2、E3 或 E4 证据。**

## 后果

### 正面

- 通知栈成为共享玻璃组件库的**第二个消费者**，材质（tint、两道发丝边、圆角）只有一个实现，
  令牌改动不会在两个 crate 里各自落地成两个不同的玻璃。
- `ui-guidelines` §12 的许可与实现第一次对齐：被点名的三种表面里，两种（面板窗格、通知卡片）
  现在真的用同一套材质绘制。
- 「宿主在自己的 qrc 里编组件」这条路线从「只有面板验证过」变成「两个宿主都验证过」，
  跨 crate 复用 QML 资产不再是一个只在文档里成立的说法。
- **组件与旧绘制等价的直接证据**：面板从「自己铺色带」切到「用 `GlassPane`」之后，
  它原有的像素断言（`render_a_translucent_dock_frame`：窗格那一行逐通道等于 `glass.tint`
  的预乘值、±1）**原样通过**。同一个断言换了绘制者仍然成立，比「新写一条断言证明新组件对」
  更能说明两者画的是同一个材质；把 `GlassPane` 换成不存在的类型名，这条断言会立刻失败。
- 通知侧不新增运行期依赖，也不新增协议对象：这条边的成本被限制在构建期（面板那条不同——它在
  运行期用材质 API 做降级决策）。

### 负面 / 限制

- **critical 卡片失去 danger 边框色**。`GlassPane` 的契约里没有描边：它接收 `tint` / `highlight` /
  `shade` / `hairline` / `radius`（根是 `Rectangle`，所以 `border` 属性在语言上仍存在，但组件从不设置它，
  投影也不再有 `edge` 键），`theme::card_edge` 因此被删除。紧急度改由 urgency rail（leading edge 上的
  色条）与卡片上的**文字句子**共同承载——这与 §5「颜色不能是唯一线索」一致，也确实是一次
  **可见的设计变化**：critical 与 normal 卡片的外壳现在一样，区别只剩 rail 与句子。
- **通知栈拿不到模糊与折射，真实会话里看到的是纯半透明，不是玻璃**。原因有两层：
  通知栈运行在普通 `xdg_toplevel` 窗口里（`src/placement.rs` 里的 layer-shell 放置策略只是建模，
  没有宿主应用它），而合成器只在「表面此刻有 layer 角色 ∧ 会话未锁定 ∧ 该表面的合成路径真的会施加
  效果」三者同时成立时才广告非零能力（`crates/inox-compositor/src/glass.rs`），普通 toplevel
  走平铺路径、不满足第一条；并且本 crate 根本没有创建 `inox_glass_v1` 对象，连「请求效果」这一步
  都没走到。要看到模糊，得先给通知栈接上 layer-shell 宿主，再补 `inox-glass` 的 `decide`/`apply`
  调用；即使那时，DRM（真机）路径仍按 ADR 0040 的限制报 0。
- **窗口透明需要 E4 真机验证**。`Window.color: "transparent"` 只是让 QML 不清屏；能不能真的透出
  背后的桌面取决于 Qt 平台插件的 ARGB visual 与会话合成。离屏测试**不能**证明这一点，
  公开的 CI 像素断言也覆盖不到它。
- **加上该边后 `same_layer_exceptions` 有四条**（`inox-config -> inox-common`、
  `inox-compositor -> inox-protocols`、`inox-panel -> inox-glass`、本条的
  `inox-notifications -> inox-glass`）。例外仍然是**登记制**：每新增一条都要先有 ADR 并登记，
  否则 `layering.rs` 会红。四条不是预算，只是当前的事实。
- **一个栈里混了两种材质**：卡片是玻璃，而守护进程自己的状态行（空态、勿扰、溢出句子的
  `notice`）仍是 `surfaceVariant` 的不透明填充；卡片内部的图标占位块同样是不透明的
  `surfaceVariant`。玻璃目前只覆盖卡片外壳，不是整个窗口。
- **通知栈自身仍没有像素证据**：`GlassPane` 现在有两个像素证据点（面板的 Dock 帧与目录页），
  但两者都是**面板**的测试；「通知卡片在屏幕上实际是什么样子」仍然只有投影断言与离屏加载证据，
  没有图像证据，也没有截图。

## 考虑过但未被采纳

- **把 `GlassPane` 提升为真正的 QML 模块（`import com.inox.glass`）**：跨 crate 的静态 QML 模块
  注册不可靠（KDAB/cxx-qt#770），面板已因此选择宿主编译；改成运行时磁盘搜索路径会把资产从
  二进制里搬出来，并让「装了哪个版本的 QML 文件」变成新的失败面。
- **把玻璃组件复制进 notifications**：正是 ADR 0040 要消灭的漂移。复制出来的 tint 取法与
  面板分叉之后，没有任何测试会发现，只有在用户屏幕上才会。
- **只改通知、不写 ADR**：`layering.rs` 会红（未登记的同层边），而分层方向的改变按 PRD §5.4
  本来就必须先有 ADR；登记条目自身也要求给出 ADR 依据。
- **同时把 settings / launcher 迁到玻璃组件**：它们的行与控件是有状态的（悬停/按下/焦点/禁用
  各一套投影，命中与焦点顺序也在 Rust），而 `inox-glass` 的组件按契约只接收属性、不持状态、不接输入；
  通知卡片是唯一「一个表面就是一块玻璃、内部不带交互控件」的现成对象，因此先只迁它。
- **现在就给通知栈加 glass 对象与协议腿（不等 layer 宿主）**：没有 layer 角色时合成器只会报 0，
  绑定一个必然降级、且没有任何可见收益的对象，等于白拿一条协议依赖。
- **保持窗口不透明、只把卡片画成半透明**：视觉上是「玻璃贴在不透明底板上」，
  比现在的纯半透明更假，也违背 §12 的「一块玻璃 = 铺在透明表面上的半透明 tint」。

## 取代条件

出现以下任一情况时，本决策应被重新评估或取代：

- 上游 wayland-protocols 出现可用的标准背景模糊协议（私有 `inox-glass-v1` 退役；这条边若还要
  存在，应改为依赖标准协议的绑定，仍需重新登记）；
- Smithay 给 passthrough 定义了可信 damage，使 DRM 路径能画玻璃（届时通知栈若接上 layer 宿主，
  本记录的「限制」之模糊一半会改变，但边本身不变）；
- 通知栈被改成**非玻璃**设计（`ui-guidelines` §12 收回「通知栈」这一项）：应删除这条边、
  删除登记条目，并让卡片退回普通表面；
- QML 跨 crate 模块注册在 cxx-qt 上游被解决：应删除这条边与登记条目，改为 `import`；
- L3 → L3 边继续增长到无法靠逐条登记管理：应先裁决分层规则 1 对**层内顺序**的表达
  （ADR 0002/0016 记录的未决项），而不是继续往 `same_layer_exceptions` 里加行。
