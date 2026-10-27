# inox-glass

Inox 的**液态玻璃材质**：一块只做一件事的纯 Rust 决策库，配一套无状态 QML 玻璃组件。

液态玻璃不是一个客户端能独立画完的效果（[ADR 0040](../../docs/adr/0040-liquid-glass-material-and-components.md)）：

- **材质**（半透明 tint、顶边发丝高光、底边阴影、圆角形状）由组件自己画，值全部来自设计令牌；
- **背景**（磨砂与边缘折射）只能由合成器画——客户端拿不到自己表面背后的像素。组件经 `inox-glass-v1` **v2** 请求它，合成器用 `capabilities` / `liquid_capabilities` 回答自己真正会画什么。

这个 crate 就是这两半相遇的地方：令牌怎样变成协议参数、超限怎样 clamp、合成器做不到时怎样降级、尺寸/令牌变了怎样重发。在 ADR 0040 之前这段判断只存在于 `inox-panel/src/frost.rs`，再复制几份必然漂移。

组件库目前有**两个消费者**，两条都是 L3 → L3 边，登记在根 `Cargo.toml` 的 `same_layer_exceptions`：

| 消费者 | 用哪些组件 | 依赖形态 | 授权的 ADR |
|--------|------------|----------|------------|
| `inox-panel` | 窗格（菜单栏与 Dock）用 `GlassPane` | **运行期**用材质 API（`frost.rs` 的 `decide`）+ **构建期**读 QML 资产 | 0040 |
| `inox-notifications` | 通知卡片用 `GlassPane` | **只在构建期**读 QML 资产（运行期不链接本 crate） | 0044 |

新增消费者仍须单独写 ADR 并单独登记。

## Rust API

```rust
use inox_config::Theme;
use inox_glass::{Capabilities, Material, Outcome, Region, decide};

let tokens = Theme::builtin_dark();

// 1. 令牌 -> 材质（blur、四角半径、折射带全部取自令牌）
let material = Material::from_tokens(&tokens.glass, tokens.radii.md);

// 2. 合成器的回答 -> 能力（v2 用 liquid_capabilities 的四个参数）
let capabilities = Capabilities::from_liquid(64, 128, 64, 0b11);

// 3. 唯一的 clamp/降级入口
match decide(material, capabilities, Region::pane(640, 68)) {
    Outcome::Effect(request) => {
        // 4. 写到真实的 inox_glass_v1 对象（或测试替身）
        // request.apply(&mut glass);
        let _ = request;
    }
    Outcome::Plain(reason) => {
        // 纯半透明：只画 tint/highlight/shade，打一条 info! 说明原因
        let _ = reason;
    }
}
```

要点：

- `Capabilities::from_legacy(max_radius)` 是 v1 `capabilities`；`from_liquid(max_radius, max_refraction_height, max_refraction_amount, flags)` 是 v2 `liquid_capabilities`（`flags` 是 `refraction_flag` 位域，`WEnum<RefractionFlag>` 用 `u32::from(flags)` 转换）。
- `Outcome::Plain` 四种理由：`BlurDisabled`（令牌要求关闭）、`RefractionDisabled`（只要折射、但合成器不折射）、`CompositorCannotBlur`（合成器什么都画不了）、`EmptyPane`（没有面积）。
- 折射要么 `height` 要么 `amount` 为 0 就不生效；色散只有在合成器广告了才会上线（`dispersion` 位），depth 位照发（协议规定合成器忽略不认识的位）。
- 四角半径同时被 clamp 到协议上限 16384 与「区域短边的一半」。
- `GlassTarget` 是落点抽象：五个方法，`supports_refraction()` 为假时 `apply` 不发 `since=2` 请求（版本 1 对象只收到 `set_radius` + `set_region`）。
- 常量：`MAX_CORNER_RADIUS = 16384`、`MAX_REFRACTION_HEIGHT = 128`、`MAX_REFRACTION_AMOUNT = 64`、`REFRACTION_FLAG_DEPTH = 1`、`REFRACTION_FLAG_DISPERSION = 2`、`REFRACTION_SINCE_VERSION = 2`。

## QML 组件（资产，不是 QML 模块）

`qml/` 下六个文件：`GlassPane`、`GlassButton`、`GlassToggle`、`GlassSlider`、`GlassTabBar`、`CatalogWindow`。

它们**没有**自己的 QML 模块：Qt 的 QML 类型按二进制注册，跨 crate 的静态注册不可靠；宿主 crate 在自己的 `build.rs` 里把它们编进自己的模块（面板与通知两个宿主都是这样做的）：

```rust
// 宿主 crate 的 build.rs（例如 inox-panel）
let mut files: Vec<std::path::PathBuf> = vec![std::path::PathBuf::from("qml/Main.qml")];
files.extend(inox_glass::qml_files());            // 绝对路径，来自 env!("CARGO_MANIFEST_DIR")

CxxQtBuilder::new_qml_module(QmlModule::new("com.inox.panel.qml").qml_files(files))
    // …
    .build();
```

`qml_files()` 返回的路径以**本 crate** 的 manifest 目录为基准，所以从宿主的 build script 调用也不会错位；`catalog_qml()` 单独给出目录页路径。

> `cxx-qt-build` 会把绝对路径的 alias 挂在模块前缀下面（`/qt/qml/<uri>/home/…`）：六个文件仍落在**同一个**资源目录里，兄弟组件的相对 import 照常解析，只是资源路径不好看。想要干净路径的 build script 可以用 `inox_glass::qml_sources()`（文件名 + 源文本，`include_str!` 编进来），把它们写进自己的 `OUT_DIR` 再把相对路径交给模块构建器。两条路产出同样的六个文件，`tests/qml_assets.rs` 断言两份清单一致。

组件规则（由 `tests/qml_assets.rs` 与 `qmllint` 双重检查）：

| 组件 | 根 | 关键属性 | Accessible |
|------|----|----------|------------|
| `GlassPane` | `Rectangle` | `tint` `highlight` `shade` `radius` `hairline` `highlighted` `shaded` | —— |
| `GlassButton` | `Item` | `text` `tint` `tintPressed` `highlight` `shade` `textColor` `focusRing` `focusRingWidth` `radius` `hairline` `fontFamily` `fontSize` `pressed` `focused` | role `Button`，name = `text` |
| `GlassToggle` | `Rectangle`（轨道本身） | `checked` `label` `trackTint` `trackTintChecked` `knobColor` `radius` | role `CheckBox`，name = `label`，`checkable`/`checked` |
| `GlassSlider` | `Item` | `value`(0..1) `label` `trackTint` `fillTint` `knobColor` `trackRadius` `knobSize` | role `Slider`，name = `label` |
| `GlassTabBar` | `Item` | `labels` `currentIndex` `accessibleName` `tint` `selectedTint` `highlight` `shade` `textColor` `selectedTextColor` `radius` `hairline` `fontFamily` `fontSize` | role `PageTabList`，name = `accessibleName`；每个 tab 是 `PageTab` |
| `CatalogWindow` | `Window` | `modelJson`、`hostBackdrop` | —— |

- 全部无输入处理：`pressed` / `focused` / `checked` / `value` / `currentIndex` 是调用方（Rust）的投影，命中测试不在 QML。
- `enabled` 用 `Item.enabled`（Qt 6 没有 `Accessible.enabled`；Qt Quick 把它映射成可访问性 enabled 状态）。禁用外观由调用方的 tint 决定，组件不自己加透明度。
- 禁止颜色/尺寸/文案字面量；`transparent` 与结构性哨兵 `#00000000` 是唯一例外。

### 目录页与投影

`CatalogWindow.qml` 的根是 `Window`（宿主 `QQmlApplicationEngine` 拿到的 `QQuickWindow` 才能被 `grabWindow()`），由 `modelJson` 驱动，和面板的 `Main.qml` 是同一套约定（宿主还用 `hostBackdrop` 交换清屏色来还原 alpha 通道）。

投影由 `inox_glass::catalog_model(&theme)` 生成（`src/catalog.rs` 是 schema 的权威）：

```text
{ width, height, background, fontFamily, fontSize, hairline,
  pane:   { x, y, width, height, radius, tint, highlight, shade },
  button: { x, y, width, height, radius, text, tint, tintPressed, highlight, shade,
            textColor, focusRing, focusRingWidth, pressed, focused, enabled },
  toggle: { x, y, width, height, radius, label, trackTint, trackTintChecked,
            knobColor, checked, enabled },
  slider: { x, y, width, height, trackRadius, label, trackTint, fillTint,
            knobColor, knobSize, value, enabled },
  tabBar: { x, y, width, height, radius, accessibleName, labels, currentIndex,
            tint, selectedTint, highlight, shade, textColor, selectedTextColor, enabled } }
```

每个矩形自带 `x/y/width/height`，像素测试可以直接从投影里读出它要断言的位置，不需要猜布局。颜色是 `#AARRGGBB`（Qt 的顺序，alpha 在前）。`text` / `label` / `accessibleName` / `labels` 是空串：用户可见文案只能来自 Fluent，由调用方填。

## 测试

```sh
cargo test -p inox-glass --offline
```

- 单元测试：每个 clamp、每条降级分支、`apply` 的调用顺序与 v1 版本门。
- `tests/negotiation.rs`（proptest）：幂等、单调、诚实（不超上限、不申称没广告的色散）、安静（零能力/空区域/关闭的材质 → `Plain`）。
- `tests/protocol_drift.rs`：flag 位、`since` 版本、接口版本钉在生成绑定上；ADR 上限值钉住。
- `tests/qml_assets.rs`：资产规则（无输入、无颜色/文案字面量、Accessible、目录页结构），以及 `qml_files()` 的每个路径存在且以 `.qml` 结尾。

组件像素证据在**面板**的离屏测试里，有两个点：Dock 帧（`render_a_translucent_dock_frame` 断言窗格那一行逐通道等于 `glass.tint` 的预乘值、±1）与目录页（`CatalogWindow.qml` 的 tint、圆角外透明、开关选中色、滑杆填充宽度）——本 crate 不链接 Qt，不能自己出帧。**通知栈**的卡片没有像素证据：它只有投影断言与「QML 模块在离屏平台加载成功」这条加载证据。

## 已知限制

- **DRM（真机）路径没有玻璃**：合成器侧尚未定义可信 damage（ADR 0040）。本 crate 只转述合成器上报的能力。
- **通知栈只有纯半透明**：`inox-notifications` 是普通窗口，拿不到 layer 角色，合成器对它报 0；它也没有创建 glass 对象。改完卡片只意味着「材质同源」，不意味着通知有模糊（ADR 0044 的「限制」）。
- **组件只覆盖五种基础组件**：菜单、对话框等仍各自实现。
- **不做 socket 级协议测试**：线上证据在 `inox-protocols` 的往返测试与面板的协议测试里。

## License

LGPL-3.0。
