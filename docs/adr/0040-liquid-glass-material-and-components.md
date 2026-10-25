# 0040 — 液态玻璃材质、`inox-glass-v1` v2 与系统玻璃组件库

- **状态**：proposed
- **日期**：2026-10-24
- **决策者**：YG（人类）/ Inox contributors
- **相关**：PRD §2.3（协议矩阵）、§3.1（组件）、§5.4（ADR 门槛）；ADR 0002（分层）、0009（配置与设计令牌）、0033/0035/0038（QML、cxx-qt、离屏表面）、私有协议策略（`inox-glass-v1` v1 的落地记录）

## 背景

`inox-glass-v1` v1 已经让客户端请求合成器模糊自己表面背后的区域（毛玻璃）。它解决的是「半透明窗格背后的细节太吵」，但没有解决「窗格看起来像一块玻璃」：真实液态玻璃的读感来自**边缘折射**——圆角矩形边缘把背后的画面按法线方向位移，最强的位移发生在轮廓上、向窗格内部在一条窄带内衰减；开启色散时不同波长位移不同，边缘因此出现极细的彩色分离；再叠上客户端自己画的 tint、顶部高光与底部阴影。

参考实现（Kyant0/AndroidLiquidGlass 的 `backdrop` 库）把这一套拆成可组合的 effect：`Blur`、`Lens`（圆角矩形 SDF + 折射带 + 可选 depth/色散）、`Highlight`。它对我们的约束是：**折射必须采样客户端表面背后的已合成像素**。客户端做不到——那些像素在合成器的 framebuffer 里，没有任何协议把它们交出来（这正是合成器的意义）。因此液态玻璃不是 QML 能独立完成的视觉效果，它和 v1 的模糊一样，必须由合成器绘制。

同时，L3 的每个组件（面板、启动器、通知、锁屏……）都需要同一套判断：材质令牌怎样映射成协议参数、合成器报 0 时怎样降级、尺寸变化后区域/圆角怎样重发。目前这段判断只存在于 `inox-panel/src/frost.rs`。再复制三份必然漂移：每份都会自己 clamp、自己决定何时创建对象、自己对 `capabilities` 做出略有不同的解释。需要把它下沉成一个可复用的 crate，并给 QML 侧一套共享的玻璃组件（窗格、按钮、开关、滑杆、标签栏），否则每个组件都会重画一遍 tint/高光/阴影。

`inox-glass-v1` 在 1.0 前显式声明为 unstable、允许向后不兼容地变更，并要求消费者通过版本与 `capabilities` 协商；这份 ADR 使用这条授权，把协议升到 **v2**，而不是另开一个平行的私有协议。

## 决策

### 1. 协议：`inox-glass-v1` 升到 v2（加法式）

接口名与全局名不变，`inox_glass_manager_v1` 与 `inox_glass_v1` 的版本都升到 **2**。旧请求/事件语义不动，v1 客户端继续工作。

新增请求（均在 `inox_glass_v1` v2，参数为**逻辑像素**，在 `set_region` 的坐标系内解释）：

| 请求 | 参数 | 语义 |
|------|------|------|
| `set_corner_radii` | `top_left, top_right, bottom_right, bottom_left: uint` | 折射轮廓的四个圆角半径；0 表示直角。合成器 clamp 到 `MAX_CORNER_RADIUS` 且不超过区域短边的一半 |
| `set_refraction` | `height: uint, amount: uint, flags: uint` | `height` 是折射带厚度，`amount` 是轮廓处的峰值位移；任一为 0 关闭折射。`flags` 是位域：`depth`=1（叠加指向中心的球面分量）、`dispersion`=2（色散） |

新增事件（仅发给 v2 对象）：`liquid_capabilities(max_radius, max_refraction_height, max_refraction_amount, flags)`。`flags` 报告合成器实际支持的折射特性（当前恒为 `depth | dispersion`，或在不支持的合成路径上为 0）。`capabilities`（v1 事件）仍然先发一次，保证只监听旧事件的客户端不会觉得对象哑了；v2 客户端以 `liquid_capabilities` 为准。两个事件都必须在任何效果生效前到达。

冻结上限（合成器的诚实上限，协议只作为 clamp 依据）：

- `MAX_BLUR_RADIUS = 64`（沿用 v1，不变）
- `MAX_CORNER_RADIUS = 16384`
- `MAX_REFRACTION_HEIGHT = 128`
- `MAX_REFRACTION_AMOUNT = 64`

`capabilities` 的「不广告做不到的事」规则不变：只有「表面有 layer 角色」∧「会话未锁定」∧「合成路径真的会画」三者同时成立，才报非零上限；否则 `liquid_capabilities` 全 0，客户端降级为纯半透明。

协议不新增任何读回路径：折射采样在合成器内部完成，客户端只拿到上限与是否接受，拿不到像素。

### 2. 合成器渲染：折射 pass

纯逻辑（参数 clamp、`Lens` 缺省值、区域/圆角/上限折算为物理像素、pane 收集）继续放在 `crates/inox-compositor/src/glass.rs`，有单元测试与 proptest。渲染在 `render.rs`，只影响 **headless** 路径；DRM 路径继续报 0 并保持场景锐利（见「限制」）。

单个 pane 的液态玻璃 pass：

1. 计算源矩形：pane 区域向外扩 `ceil(blur_radius) + ceil(refraction_amount)`，再裁到输出边界。扩边是为了让折射能采到轮廓外侧的像素，同时让模糊在 pane 边缘有真实邻域。
2. 模糊：`radius > 0` 时对源矩形做水平+垂直两趟可分离高斯（沿用 v1 的 9-tap kernel）；`radius == 0` 时用同一 shader、`step = 0` 做一次恒等拷贝。结果落在 scratch 纹理，**不**回写目标。
3. 折射：一个全屏（pane 区域大小）片元 pass，采样 scratch。对每个像素：
   - 用区域坐标算圆角矩形 SDF；`sd > 0` 的像素输出 alpha 0（圆角外不得被玻璃覆盖）。
   - `-sd < height` 的带内：`t = 1 + sd/height`，`d = (1 - sqrt(1 - t²)) * amount`，沿 SDF 梯度（加 `depth` 时再加径向分量）位移采样坐标。
   - `dispersion` 开启时按参考实现的 7-tap 权重分别取 R/G/B，未开启时单 tap。
   - 输出预乘颜色，alpha 乘以 `clamp(0.5 - sd, 0, 1)` 得到 1px 抗锯齿的轮廓覆盖。
4. 把折射结果画回 pane 区域；客户端自己的半透明表面随后照常合成在上面。

shader 仍是 GLSL ES 1.00、仍带 `//_DEFINES_` 标记（Smithay 会替换出 plain/NO_ALPHA/EXTERNAL 变体）、仍只用一个 sampler（scratch），所有采样 clamp 到半texel 内以避免 GL 默认 `GL_REPEAT` 把对边卷进来。没有 lens 的 pane 走 v1 的旧路径，像素逐字节不变。

判别性证据：`tests/glass_blur.rs` 增加一个透镜场景——背景是锐利的竖条纹，pane 开 `blur = 0`、`height/amount > 0`。断言 pane 外的条纹位置不变；pane 内靠近轮廓的条纹被向外推移（与无 glass 对象的对照帧相比位移列数 > 0）；pane 内部（离轮廓超过 `height` 处）与对照帧一致（证明只折射边缘带）；圆角外像素等于对照（证明 SDF 遮罩生效）；开启 `dispersion` 时同一边缘的红/蓝通道位移不同。删掉折射 pass 必然让这些断言失败。

### 3. 新 crate `inox-glass`（L3）：材质决策 + 玻璃组件库

内容分两半：

**Rust（纯逻辑，无 Qt）**

- `Capabilities`：v1/v2 capabilities 的解析（`from_legacy` / `from_liquid`）、`can_blur` / `can_refract`。
- `Corners`、`Refraction`、`Material`：设计令牌的材质表达。
- `Region`、`Request`：协议请求的纯数据形态（半径、区域、四角半径、折射参数）。
- `decide(material, capabilities, region) -> Outcome`：唯一的 clamp/降级入口，返回 `Effect(Request)` 或 `Plain(reason)`；`Plain` 区分「令牌要求关闭模糊」「合成器做不到」「区域为空」。
- `Request::apply(&impl GlassTarget)`：把请求写到真实协议对象（`GlassTarget` trait 让它可以被测试替身记录）；对 v1 对象自动跳过 `since=2` 的请求。
- 常量（`MAX_*`）与 `inox-protocols` 生成的值有一致性测试，禁止两边各写一份悄悄漂移。

**QML（无状态渲染器）**

`qml/` 下提供 `GlassPane`、`GlassButton`、`GlassToggle`、`GlassSlider`、`GlassTabBar` 五个组件，外加 `CatalogWindow.qml` 目录页。规则：

- 组件只接收属性（tint、高光、阴影、圆角、文字、状态位），**不接输入、不持业务状态、不做决策**；点击命中和状态仍由 Rust 投影负责（与现有 L3 组件的约定一致）。
- 颜色、尺寸、圆角、动效时长全部由上层的令牌投影传入；QML 只允许结构性数值（1px 高光、AA 用值）。
- 每个可交互组件设置 `Accessible.role` / `Accessible.name` / `Accessible.enabled`，名称由调用方传入。
- 组件文件由宿主 crate 的 `build.rs` 通过 `inox_glass::qml_files()` 收进自己的 QML 模块（Qt 资源按二进制归属，跨 crate 的 QML 插件静态注册不可靠）；面板的离屏测试渲染 `CatalogWindow.qml`，对窗格 tint、圆角外透明、开关选中色、滑杆填充宽度做像素断言。这是组件库的 E1 + 离屏证据。

**分层例外**：`inox-panel -> inox-glass` 是 L3 → L3。按分层规则 1，它登记进根 `Cargo.toml` 的 `same_layer_exceptions`，并由本 ADR 授权；`inox-glass` 本身只向下依赖 `inox-config`（L1）与 `inox-protocols`（L2）。

### 4. 设计令牌：`[glass]` 增加折射字段

`inox_config::Glass` 增加四个带 `#[serde(default)]` 的字段，旧主题文件照常解析：

```toml
[glass]
refraction = 24          # 折射带厚度（逻辑像素），0 关闭折射
refraction_amount = 12   # 轮廓峰值位移（逻辑像素）
depth = false            # 叠加球面深度分量
dispersion = true        # 边缘色散
```

内置明暗两套主题给出保守默认值；面板把 `radii.md` 作为四角半径。组件与面板都只能从令牌取值，不得自选折射量。

### 5. 面板：v2 客户端

- 绑定 `inox_glass_manager_v1` 时请求 `min(advertised, 2)`；拿到 v1 就不发 `since=2` 请求，只是没有折射。
- 只要「模糊 > 0 或折射 > 0」就创建 glass 对象（此前只看模糊）。
- 收到 `capabilities` 与 `liquid_capabilities` 后都经 `inox_glass::Capabilities` 归一，`decide` 算出的 `Request` 由 `apply` 发到线上；尺寸/令牌变化时重发区域、圆角与折射参数。
- `max_radius = 0` 且 `max_refraction_* = 0` 时退回纯半透明并只 `info!` 一次；任何情况下不得声称画了自己没画的折射。
- 面板既有的投影测试保持通过；新增一条「玻璃令牌变化 → 请求变化」的断言与一条 QML 目录页像素断言。

## 后果

**正面**

- 桌面第一次有真正的液态玻璃材质：边缘折射、可配色散、与令牌一致，且客户端无法伪造或读取他人像素。
- 材质决策只有一份实现，面板与后续组件共享；协议参数、clamp、降级理由都可单元测试。
- 协议 v2 是加法式的，v1 客户端（现网面板旧版本）不受影响。
- QML 组件库让新组件的玻璃外观不必重画；目录页就是设计参照。

**负面 / 限制**

- **DRM（真机）路径仍不实现玻璃**：`OutputDamageTracker` 的 damage 来自渲染元素自身的 damage snapshot，折射/模糊 passthrough 还没有诚实的 damage 描述。因此本 ADR 交付的是 headless 路径上的完整效果与像素证据，真机验收（E4）仍未取得；不得把 CI 像素测试读作「真机已支持」。
- 液态玻璃 pass 比 v1 模糊更贵：多一趟全 pane 的片元 pass，色散开启时每个像素 7 次采样；CI 用 llvmpipe，预算按小 pane 设计。
- 折射参数是「看起来对」的美术参数，不是物理正确的光学模型（参考实现同样如此）；色散强度不可调，只有开关。
- 新 crate 与同层依赖增加了分层图的复杂度；例外是显式登记的，不是自由的。
- 组件库只覆盖五种基础组件，菜单、对话框等仍各自实现；不追求与 Kyant 库对等。

## 考虑过但未被采纳

- **新开 `inox-liquid-glass-v1` 平行协议**：会把 region 裁剪、每表面一个控制器、capabilities 诚实性、锁定语义整套复制一份，并产生「两个协议都能磨砂同一表面」的竞争；升级现有协议更小。
- **客户端在 Qt 里画折射**：Qt 拿不到表面背后的像素；即便拿得到，等于给客户端开放屏幕读取能力。
- **合成器画 tint 与高光**：客户端本来就要画自己的内容，重复画会双重叠色，也会把材质从设计令牌里搬进合成器；合成器只做它唯一能做的那件事——采样背后的像素。
- **现在就给 DRM 路径接入**：需要先给 passthrough 定义可信 damage，否则会带来花屏/残影，超出本 ADR 的范围。
- **独立 bin crate 做组件目录**：cxx-qt 的静态 QML 模块注册跨 crate 不可靠（KDAB/cxx-qt#770），且材质逻辑会被迫复制；改为宿主编译 QML 文件、面板测试提供证据。

## 取代条件

出现以下任一情况时，本决策应被重新评估或取代：

- 上游 wayland-protocols 出现可用的背景模糊/折射标准协议（则应迁移，私有协议退役）；
- Smithay 提供有 damage 语义的 blur/lens 渲染元素（则应删除 headless 专用实现并接入 DRM）；
- 液态玻璃在真机上被证明不可用（性能或合成正确性），届时降级为 v1 毛玻璃并记录反例；
- 1.0 发布时协议需要冻结：v2 若被接受，应把 unstable 标记移除并重新编号。
