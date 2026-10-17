# 主题与设计令牌

> **提案，尚未发布。** 本页的令牌集合与取值来自 [ADR 0009](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0009-configuration-model-and-reload.md)（状态 `proposed`）。当前没有任何组件读取主题文件：`inox-config` 仍是骨架，Iced 组件要到 M4 才出现。

主题由一个 `theme.toml` 定义，内容是**设计令牌**（design tokens）：颜色、排版、间距、圆角与动效时长。PRD §9.2 与根级 `AGENTS.md` §4 要求组件只消费令牌，**禁止在组件中硬编码颜色与尺寸**。

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

## 自定义主题与缺失字段

- 主题文件中的 `name` 省略时取目录名。
- **缺失的令牌回退到内置的深色默认值。** 这是需要特别当心的一条：只写了一半令牌的浅色主题，剩下的一半会拿到深色取值，视觉上会明显不一致。自定义浅色主题时请把上表中 `inox-light` 的 11 个颜色逐项写全。
- 与所有配置文件一致，**未知字段是错误**：拼错的令牌名会让解析失败，而不是静默取默认值。

## 当前实现状态

- 没有任何组件读取 `theme.toml`；`inox-config` 尚未实现主题解析。
- 设计令牌的第一个真实消费者是 M4 的 Iced 组件（`inox-shell` 及其子组件）；在此之前，本页描述的是接口约定而非可用行为。
- 合成器边框等非 Iced 消费方也使用同一套令牌，因此令牌类型刻意不依赖任何图形栈。
