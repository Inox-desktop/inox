# 面板、启动器与壁纸配置（`shell.toml`）

> **状态：字段模型已实现，消费方为部分实现。** `shell.toml` 的模型位于 `inox-config` 的 `src/shell.rs`，
> 默认值、范围校验与错误定位都有单元测试（本地 `cargo test -p inox-config` 全绿；CI 为唯一验收途径）。
> 消费它的 L3 组件按 [ADR 0018](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0018-l3-component-architecture.md)
> （状态 `proposed`）交付到 **E0/E1**（编译、lint、文档、单元与属性测试）；需要 layer-shell 宿主的
> **E2/E3** 证据尚未取得，理由见该 ADR 与[里程碑与路线图](../roadmap.md)。

`shell.toml` 由 L3 的面板、启动器与壁纸读取，文件名在 `config.toml` 的 `[components] shell` 中给出
（见[配置目录布局](./layout.md)）。文件缺席时所有字段取默认值；**未知字段与越界值在解析期报错**，
错误信息包含文件、行号与点分字段路径（ADR 0009 决策 4）。

```toml
[panel]
position = "bottom"        # top | bottom
height = 32                # 8..=256
margin = 0                 # 0..=64
exclusive = true           # 为面板保留一条独占区
autohide = false
show_clock = true
clock_format = "%H:%M"
max_task_buttons = 12      # 1..=64
icon_size = 16             # 8..=128

[launcher]
max_results = 8            # 1..=64
icon_size = 48             # 8..=256
terminal = []              # 以 argv 形式给出的终端前缀，例如 ["alacritty", "-e"]
include_hidden = false
max_query_chars = 128      # 1..=512

[wallpaper]
mode = "fill"              # fill | fit | stretch | center | tile
# image = "wallpapers/steel.png"   # 相对配置目录的路径；省略则使用主题背景色

[wallpaper.per_output]
"*" = "wallpapers/steel.png"       # 未单独指定的输出
"HDMI-A-1" = "wallpapers/wide.png"
```

## `[panel]`

| 键 | 类型 | 默认值 | 范围 | 说明 |
|----|------|--------|------|------|
| `position` | 字符串 | `"bottom"` | `top` / `bottom` | 面板贴哪条边；同时决定独占区的方向 |
| `height` | 整数 | `32` | 8..=256 | 面板高度（像素）。这是布局尺寸，取自已解析的配置，不以字面量写进视图 |
| `margin` | 整数 | `0` | 0..=64 | 与屏幕边缘的间距。窗口**仍由配置给出**，不是硬编码的 |
| `exclusive` | 布尔 | `true` | — | 为面板保留一条屏幕边缘的独占区（layer-shell 的 exclusive zone） |
| `autohide` | 布尔 | `false` | — | 隐藏直到指针触及该边缘 |
| `show_clock` | 布尔 | `true` | — | 是否显示时钟 |
| `clock_format` | 字符串 | `"%H:%M"` | 见下 | 时钟格式，**不是** strftime 透传 |
| `max_task_buttons` | 整数 | `12` | 1..=64 | 任务按钮数上限，超出时折叠为一个「更多」入口 |
| `icon_size` | 整数 | `16` | 8..=128 | 任务与托盘图标尺寸 |

### 时钟格式是一个受支持的子集

`clock_format` 只接受下列指令，其余一律在解析期报错，不会原样透传给面板：

| 指令 | 含义 | 指令 | 含义 |
|------|------|------|------|
| `%H` | 两位小时（24 小时制） | `%a` / `%A` | 星期简称 / 全称 |
| `%I` | 两位小时（12 小时制） | `%b` / `%B` | 月份简称 / 全称 |
| `%M` / `%S` | 分钟 / 秒 | `%e` | 不补零的日 |
| `%p` | AM / PM | `%Z` | 时区名（如 `CST`） |
| `%Y` / `%m` / `%d` | 年 / 月 / 日 | `%%` | 字面百分号 |

`%a`、`%A`、`%b`、`%B` 的名称来自面板自己的 Fluent 目录，而不是英文常量表：切换到 `zh-CN` 时星期与
月份随之切换（验收 Q03）。

## `[launcher]`

| 键 | 类型 | 默认值 | 范围 | 说明 |
|----|------|--------|------|------|
| `max_results` | 整数 | `8` | 1..=64 | 搜索结果上限（有界结果集，见 ADR 0013 决策 4） |
| `icon_size` | 整数 | `48` | 8..=256 | 结果行图标尺寸 |
| `terminal` | 字符串数组 | `[]` | — | 以 **argv** 形式给出的终端前缀，用于 `Terminal=true` 的桌面条目 |
| `include_hidden` | 布尔 | `false` | — | 是否包含 `Hidden` / `NoDisplay` 的条目 |
| `max_query_chars` | 整数 | `128` | 1..=512 | 搜索词长度上限 |

`terminal` 刻意是**数组而不是字符串**：条目以 `--icon <Icon>`、`Terminal` 前缀与条目自身 argv
依次拼接启动，因此整条链路上没有任何地方需要解析或引用一段 shell 命令（验收 D02 明确要求不得把
搜索词或条目内容变成 shell 命令）。前缀为空而条目又是 `Terminal=true` 时，启动以一条可读错误失败，
而不是静默起一个没有界面的进程。

## `[wallpaper]` 与 `[wallpaper.per_output]`

| 键 | 类型 | 默认值 | 说明 |
|----|------|--------|------|
| `mode` | 字符串 | `"fill"` | `fill`（裁剪填满，保持宽高比）/ `fit`（完整可见）/ `stretch` / `center` / `tile` |
| `image` | 路径 | 省略 | 相对配置目录的图片路径，与主题资源使用同一套路径策略（拒绝绝对路径与 `..`） |
| `per_output` | 表 | 空 | 按输出名覆盖图片；键 `*` 是未单独指定输出的回退 |

`image` 省略、文件不存在或无法解码时，壁纸回退到**主题的背景令牌**并给出可诊断的日志，而不是黑屏或
崩溃：显式选择却读不到的资源必须被报告（ADR 0014），而「没有配置图片」是正常状态。

## 相关页面

- 顶层 `config.toml`（组件文件路径、环境变量、主题、语言）见[配置目录布局](./layout.md)；
- 设计令牌（颜色、间距、圆角、动效）见[主题与设计令牌](./theme.md)；
- 通知守护进程的配置见[通知配置](./notifications.md)。
