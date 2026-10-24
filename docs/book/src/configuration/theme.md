# 主题与设计令牌

> **状态：模型与消费方都已落地，决策仍是提案。** 本页的令牌集合与取值来自 [ADR 0009](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0009-configuration-model-and-reload.md)（状态 `proposed`）。`inox-config` 已实现令牌类型、内置浅/深主题、主题文件解析与缺失主题的报错；L3 的六个界面组件**只消费令牌**（ADR 0018 记录了每个组件各自持有的一小段令牌 → Iced 主题映射，以及为什么它没有被抽成共享 crate）。

主题由一个 `theme.toml` 定义，内容是**设计令牌**（design tokens）：颜色、排版、间距、圆角、动效时长与玻璃材质。PRD §9.2 与根级 `AGENTS.md` §4 要求组件只消费令牌，**禁止在组件中硬编码颜色与尺寸**。

## 文件位置

```text
~/.config/inox/themes/<name>/theme.toml
~/.config/inox/themes/<name>/assets/
```

内置主题 `inox-light` 与 `inox-dark` 随二进制提供，不需要在配置目录中创建文件，但自定义主题的结构与它们完全一致 —— 可以直接复制内置取值作为起点。

## 主题选择

在 `config.toml` 的 `[theme]` 段选择：

```toml
[theme]
name = "inox-dark"                 # 省略则按 appearance 选择内置主题
appearance = "system"              # light | dark | system（默认）
```

- `name` 命中内置主题时使用内置主题；否则读取 `themes/<name>/theme.toml`。文件不存在报 `ThemeNotFound`，解析失败报 `Parse` 错误。
- `name` 省略时，按 `appearance` 选择内置主题。
- `appearance = "system"` 的实际取值由会话或 Shell 决定并注入；没有注入时回退深色。
- 主题名不接受空值、路径分隔符或 `.` / `..` 段 —— 与 `[components]` 的路径一样，避免配置把读取范围引出配置目录。

## 令牌参考

`theme.toml` 的完整结构：

```toml
name = "inox-dark"          # 省略则取目录名
appearance = "dark"         # light | dark

[colors]
background = "#1b1d23"
surface = "#23262e"
surface_variant = "#2c313b"
border = "#3a4150"
text = "#e6e9ef"
text_muted = "#a5adba"
accent = "#4c8bf5"
on_accent = "#ffffff"
danger = "#e5534b"
warning = "#e0a33e"
success = "#4caf7d"

[typography]
font_family = "Noto Sans"
font_mono_family = "JetBrains Mono"
scale = 1.0

[spacing]  # xs=2 sm=4 md=8 lg=16 xl=32
xs = 2
sm = 4
md = 8
lg = 16
xl = 32

[radii]    # sm=4 md=8 lg=12
sm = 4
md = 8
lg = 12

[motion]   # fast_ms=100 normal_ms=200 slow_ms=400
fast_ms = 100
normal_ms = 200
slow_ms = 400

[glass]    # 玻璃材质（背景模糊）；blur=0 表示只做半透明
tint = "#141822cc"
tint_strong = "#0d1018e6"
highlight = "#ffffff40"
shade = "#00000059"
blur = 128
```

### 颜色（`[colors]`）

颜色写作 `#RRGGBB` 或 `#RRGGBBAA`。以下是两个内置主题的取值：

| 令牌 | `inox-dark` | `inox-light` | 用途 |
|------|-------------|--------------|------|
| `background` | `#1b1d23` | `#f5f6f8` | 窗口与工作区底色 |
| `surface` | `#23262e` | `#ffffff` | 面板、卡片等承载内容的表面 |
| `surface_variant` | `#2c313b` | `#eceef2` | 表面上的次级区块（悬停、分组） |
| `border` | `#3a4150` | `#d6dae1` | 分隔线与描边 |
| `text` | `#e6e9ef` | `#1b1d23` | 主要文字 |
| `text_muted` | `#a5adba` | `#5c6472` | 次要文字与占位符 |
| `accent` | `#4c8bf5` | `#2f6fe0` | 强调色、选中态、焦点环 |
| `on_accent` | `#ffffff` | `#ffffff` | 铺在 `accent` 上的文字 |
| `danger` | `#e5534b` | `#c9403a` | 错误与破坏性操作 |
| `warning` | `#e0a33e` | `#b7791f` | 警告 |
| `success` | `#4caf7d` | `#2f855a` | 成功与已启用状态 |

### 排版（`[typography]`）

| 键 | 默认值 | 说明 |
|----|--------|------|
| `font_family` | `Noto Sans` | 界面字体 |
| `font_mono_family` | `JetBrains Mono` | 等宽字体 |
| `scale` | `1.0` | 字体缩放系数；应与系统字体缩放需求一致 |

### 间距（`[spacing]`）

| 键 | 默认值 |
|----|--------|
| `xs` | `2` |
| `sm` | `4` |
| `md` | `8` |
| `lg` | `16` |
| `xl` | `32` |

### 圆角（`[radii]`）

| 键 | 默认值 |
|----|--------|
| `sm` | `4` |
| `md` | `8` |
| `lg` | `12` |

### 动效（`[motion]`）

时长以毫秒计。

| 键 | 默认值 | 典型用途 |
|----|--------|----------|
| `fast_ms` | `100` | 悬停、按下等即时反馈 |
| `normal_ms` | `200` | 面板与菜单的展开收起 |
| `slow_ms` | `400` | 大范围过渡 |

### 玻璃材质（`[glass]`）

玻璃（磨砂）材质是**一个令牌组**，只用于悬浮的界面外壳（菜单栏、dock、浮动菜单、通知栈）—— 组件的 tint 只有这一处来源，不得硬编码（根级 `AGENTS.md` §4）。一块玻璃 = 半透明 `tint` + 顶边 `highlight` 发丝线 + 底边 `shade`。

| 令牌 | `inox-dark` | `inox-light` | 用途 |
|------|-------------|--------------|------|
| `tint` | `#141822cc` | `#fffbf5d9` | 铺在背景上的半透明主色（磨砂窗格本身） |
| `tint_strong` | `#0d1018e6` | `#fffdf9f2` | 强调态（如按下的 dock）的更深 tint |
| `highlight` | `#ffffff40` | `#ffffffcc` | 窗格顶边高光 —— 让它读起来像玻璃 |
| `shade` | `#00000059` | `#0000001f` | 窗格底边压暗 |
| `blur` | `128` | `128` | 向合成器请求的背景模糊半径（逻辑像素） |

`blur` 是一个**请求**：模糊本身由合成器经私有协议 `inox-glass-v1` 绘制，组件拿到的半径上限由合成器的 `capabilities` 回答，超限会被钳制。`blur = 0` 或 `capabilities = 0`（没有渲染支持、表面不是 layer 角色、会话已锁定）时，组件必须退化为**纯半透明**：`tint`/`highlight`/`shade` 照旧，只是没有背景软化，不得假装画出了模糊。决策与安全边界见 [ADR 0031](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0031-compositor-backdrop-blur-and-the-glass-protocol.md)（`proposed`）；合成器只在 **headless** 路径绘制模糊，物理（DRM）路径报 0 并保持场景锐利，因此真机上目前仍走纯半透明路径。

## 自定义主题与缺失字段

- 主题文件中的 `name` 省略时取目录名。
- **缺失的令牌回退到内置的深色默认值。** 这是需要特别当心的一条：只写了一半令牌的浅色主题，剩下的一半会拿到深色取值，视觉上会明显不一致。自定义浅色主题时请把上表中 `inox-light` 的 11 个颜色逐项写全；`[glass]` 组同理，只写 `blur` 会让其余四个 tint 令牌继承深色玻璃。
- 与所有配置文件一致，**未知字段是错误**：拼错的令牌名会让解析失败，而不是静默取默认值。

## 当前实现状态

- `inox-config` 已实现令牌类型、内置浅/深主题、`theme.toml` 解析，以及缺失主题与未知字段的报错。
- 真实消费方是 L3/L4 的 Iced 界面：`inox-panel`、`inox-launcher`、`inox-notifications`、`inox-settings`、`inox-lock`、`inox-wallpaper`、`inox-greet`。每个组件在自己的 `theme.rs` 里把令牌映射成 Iced 主题与样式（[ADR 0018](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0018-l3-component-architecture.md) 记录了这个重复的取舍）。
- 组件**如何**消费令牌 —— 类型层次、间距节奏、控件几何、五状态可见性、无障碍与缩放要求 —— 见 [Inox UI 设计规范](https://github.com/Inox-desktop/inox/blob/main/docs/quality/ui-guidelines.md)。
- 仍缺：高对比主题与 RTL 伪本地化（[验收规范](https://github.com/Inox-desktop/inox/blob/main/docs/quality/acceptance.md) Q02 的已知缺口）。
- 合成器边框等非 Iced 消费方计划使用同一套令牌，因此令牌类型刻意不依赖任何图形栈。
