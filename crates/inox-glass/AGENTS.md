# AGENTS.md — inox-glass

本 crate 遵循根仓库的**全部硬性规则**（[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)），本文件只补充本 crate 特有内容，不放宽任何硬性规则。

## 职责

`inox-glass` 是液态玻璃（liquid glass）在 Inox 里的**唯一下沉实现**，交付两半：

1. **纯 Rust 材质 API（无 Qt、无 cxx-qt）**：把 `[glass]` 设计令牌翻译成协议参数，负责 clamp、v1/v2 协商、降级理由与 `Request::apply`。面板与后续每个 L3 组件都消费它，而不是各写一份（那正是 `inox-panel/src/frost.rs` 在 ADR 0040 之前的状态）。
2. **QML 玻璃组件库（资产，不是 QML 模块）**：`qml/` 下五个无状态渲染器 + `CatalogWindow.qml` 目录页。宿主 crate 在自己的 `build.rs` 里用 `inox_glass::qml_files()` 把它们编进**自己的** QML 模块。

所属层级：**L3**。依赖只有向下：`inox-config`（L1）、`inox-protocols`（L2）与第三方 `wayland-client`、`serde_json`。

## 分层例外（唯一）

`inox-panel -> inox-glass` 是 **L3 → L3**，由 [ADR 0040](../../docs/adr/0040-liquid-glass-material-and-components.md) 明确授权，并登记在根 `Cargo.toml` 的 `[workspace.metadata.inox.rules] same_layer_exceptions`。规则：

- 例外是**登记制**，不是自由：别的 L3 crate 想依赖 `inox-glass` 必须先在根 `Cargo.toml` 里加一条并说明 ADR 依据，否则 `cargo test -p inox-integration --test layering` 变红。
- `inox-glass` 自己**不得**新增任何 Inox 依赖到 L3 及以上。它不能依赖 `inox-panel`、`inox-compositor`，也不能依赖任何 Qt 绑定。
- 删除面板那条 edge 时，`same_layer_exceptions` 里的 `["inox-panel", "inox-glass"]` 必须同时删除。

## crate 特有约束

- **无 Qt / cxx-qt**。本 crate 不出现 `cxx`、`cxx-qt`、`cxx-qt-build`、`cxx-qt-lib`，也不注册 QML 类型。跨 crate 的静态 QML 模块注册不可靠（KDAB/cxx-qt#770），所以 QML 由宿主编译、由宿主测试——`inox-glass` 只提供路径与投影。
- **`unsafe`**：继承工作区 `unsafe_code = "forbid"`，本 crate 不设任何 `allow`。
- **禁 `unwrap()` / `expect()` / `panic!()` / `todo!()`（非测试路径）**。合法的是 `unwrap_or` / `unwrap_or_else`（例如 `Region::pane` 的 `i32::try_from(width).unwrap_or(i32::MAX)`）。
- **日志**：本 crate 是纯逻辑，不写日志（`tracing` 也不依赖）。降级理由通过 `Outcome::Plain(Plain::…)` 交回调用方，由调用方决定那一条 `info!`。
- **令牌**：
  - Rust 侧只从 `inox_config` 取值：`Material::from_tokens(&tokens.glass, radius)`、`catalog::from_theme(&Theme)`。本 crate 不写死任何设计颜色、尺寸、圆角或动效时长。
  - QML 侧只接收属性。`tests/qml_assets.rs` 静态断言：`qml/` 里除了 `transparent` 与结构性哨兵 `#00000000` 之外**不允许**出现颜色字面量；`text:` / `label:` / `accessibleName:` / `name:` 后面不允许出现字符串字面量。
  - 允许的结构性数值：`0`（"还没有投影"）、`hairline` 的默认 `1`、`/ 2` 这类 AA/几何运算、以及 `catalog::from_theme` 里由令牌推导出的布局尺寸。
- **无输入处理**：QML 里禁止 `MouseArea` / `TapHandler` / `Keys.` / `onClicked` 等（同样由 `tests/qml_assets.rs` 断言）。命中测试、焦点顺序与状态机属于 Rust 投影。
- **Accessible**：
  - 交互组件必须设 `Accessible.role` 与 `Accessible.name`（名称由调用方按 Fluent 文案传入，本 crate 不写死）。
  - **`enabled` 用 `Item.enabled`**：Qt 6 的 `Accessible` attached 类型**没有** `enabled` 属性（只有 role/name/description/checkable/checked/selected/pressed/focusable/…）。Qt Quick 直接把 item 的 `enabled` 映射成可访问性 disabled 状态，所以调用方投影 `enabled: <token 状态>` 即满足 ADR 的「Accessible role/name/enabled」要求；视觉上的禁用外观由调用方选择的 tint 表达，本 crate 不自造透明度。
- **协议版本门**：`since=2` 的请求（`set_corner_radii`、`set_refraction`）只在 `version >= REFRACTION_SINCE_VERSION` 的对象上发。适配器（`src/protocol.rs`）与 `Request::apply` **两处都**做这个判断：适配器可以单独使用，把协议错误留给调用方的纪律是不能接受的。
- **API 是契约**：`Capabilities` / `Corners` / `Refraction` / `Material` / `Region` / `Request` / `Outcome` / `Plain` / `GlassTarget` / `decide` / `apply` / `qml_files` 的签名是面板的集成契约。改签名必须同步 `crates/inox-panel`（ADR 0040 的 panel 任务），并在报告里写明改动。
- **诚实性**：`decide` 不得返回「看起来有、其实没有」的效果。合成器报 0 就降级；`max_refraction_*` 为 0 就不发 `set_refraction`；只广告了 depth 时不得申称色散。

## 目录结构

```
crates/inox-glass/
  Cargo.toml          # 只有向下依赖；[lints] workspace
  src/lib.rs          # crate 文档、常量（MAX_*、flag 位、REFRACTION_SINCE_VERSION）、再导出
  src/capabilities.rs # Capabilities：v1/v2 capabilities 的归一
  src/material.rs     # Corners / Refraction / Material（含 from_tokens）与圆角上限推导
  src/region.rs       # Region（表面局部逻辑坐标）
  src/request.rs      # RefractionRequest / Request / Plain / Outcome / decide / GlassTarget / apply
  src/protocol.rs     # 唯一出现 wayland 类型的地方：InoxGlassV1 的 GlassTarget 适配器
  src/qml.rs          # QML_FILES / qml_files() / qml_sources() / catalog_qml()（路径用 env!("CARGO_MANIFEST_DIR")）
  src/catalog.rs      # catalog_model(&Theme)：目录页的 JSON 投影
  qml/                # GlassPane/Button/Toggle/Slider/TabBar.qml + CatalogWindow.qml
  tests/negotiation.rs    # proptest：clamp 幂等/单调/诚实/安静
  tests/protocol_drift.rs # 与生成绑定的一致性（flag 位、since、接口版本、ADR 上限）
  tests/qml_assets.rs     # 资产规则：无输入、无颜色/文案字面量、Accessible、目录页结构
  README.md / AGENTS.md
```

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。**不要**在本 crate 上跑 workspace 级命令。

| 目的 | 命令 |
|------|------|
| 格式化 | `cargo fmt -p inox-glass` |
| 静态检查 | `cargo clippy -p inox-glass --all-targets --all-features --offline -- -D warnings` |
| 测试 | `cargo test -p inox-glass --offline` |
| 分层检查 | `cargo test -p inox-integration --test layering --offline` |
| 本 crate 文档 | `cargo doc -p inox-glass --no-deps` |

QML 语法/属性检查（本机装了 Qt 6 时）：

```sh
qmllint crates/inox-glass/qml/*.qml
```

`qmllint` 必须零警告：它抓过 `Accessible.enabled`（Qt 6 不存在）这类真实错误。

## 测试要求

- **单元测试**（`src/**` 内联）覆盖每一个 clamp 与每一条降级分支，以及 `Request::apply` 的调用顺序与 v1 版本门。
- **proptest**（`tests/negotiation.rs`）覆盖四条不变量：幂等（用协商结果再决定一次不变）、单调（能力更强不会得到更小请求）、诚实（不超过任何上限、不申称没广告的色散）、安静（零能力/空区域/关闭的材质都给 `Plain`）。
- **漂移测试**（`tests/protocol_drift.rs`）把 flag 位、`since` 版本、接口版本钉在生成绑定上，并钉住 ADR 0040 §1 的四个上限值。协议 XML 本身不携带上限数字（上限是合成器运行时的诚实广告），这一点在测试注释里写明，不假装测了它没测的东西。
- **资产测试**（`tests/qml_assets.rs`）：`qml_files()` 每个路径存在且以 `.qml` 结尾（`src/qml.rs` 内联测试），外加无输入、无颜色/文案字面量、Accessible role/name、目录页由 `modelJson` 驱动。
- **没有的东西要说清楚**：本 crate 不做 socket 级协议测试（那需要 wayland-server 扮演合成器），线上证据在 `crates/inox-protocols/tests/glass_roundtrip.rs` 与面板的 `src/tests/protocol.rs`；组件像素证据在面板的离屏测试（渲染 `CatalogWindow.qml`）。

## 已知限制

- **DRM（真机）路径没有玻璃**：合成器侧尚未给 passthrough 定义可信 damage（ADR 0040「限制」）。本 crate 的能力判断与之无关——它只诚实地转述合成器的上报。
- **组件像素证据不在本 crate**：`inox-glass` 不链接 Qt，不能自己出帧；`CatalogWindow.qml` 由面板的离屏宿主渲染并断言。
- **目录页的文案为空**：`catalog::from_theme` 投影出的 `text` / `label` / `accessibleName` / `labels` 都是空串——用户可见文案只能来自 Fluent，调用方填充。
