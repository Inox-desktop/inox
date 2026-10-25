# 合成器配置

> **状态：模型已实现，合成器尚未接入。** 本页的字段与默认值来自配置模型的决策（状态 `proposed`）。`inox-config` 已实现 `compositor.toml` 的模型、范围校验与错误定位并有测试；`inox-compositor` **仍只解析命令行参数、不读取 TOML**，合成器接入 `compositor.toml` 尚未实现。L3 组件已通过 `InoxConfig` 读取各自的配置文件。

涉及两个文件：顶层 `config.toml` 给出环境变量、组件文件位置与主题选择；`compositor.toml` 给出输出、输入与平铺参数。目录位置与通用规则见[配置目录布局](./layout.md)。

## `config.toml`

```toml
[environment]                      # 注入会话的环境变量，键值均为字符串
MOZ_ENABLE_WAYLAND = "1"

[components]                       # 相对配置目录的文件名，省略则用默认值
compositor = "compositor.toml"
shell = "shell.toml"               # 见「面板、启动器与壁纸配置」
notifications = "notifications.toml"  # 见「通知配置」

[theme]
name = "inox-dark"                 # 省略则按 appearance 选择内置主题
appearance = "system"              # light | dark | system（默认）

[locale]
language = "zh-CN"                 # 省略则跟随环境
```

| 段 | 作用 |
|----|------|
| `[environment]` | 会话范围内注入的环境变量（如 `MOZ_ENABLE_WAYLAND`）。键与值都必须是字符串 |
| `[components]` | 各组件的配置文件路径，取值是**相对配置目录**的文件名；绝对路径与 `..` 会被拒绝 |
| `[theme]` | 主题选择，详见[主题与设计令牌](./theme.md) |
| `[locale]` | 会话语言；省略则跟随环境，详见[配置目录布局](./layout.md) |

## `compositor.toml`

```toml
[[outputs]]                         # 可重复；整个列表省略时等价于一个默认输出
name = "DP-1"                       # 省略则匹配前面条目没有认领的输出
width = 1920
height = 1080
refresh_hz = 60
scale = 1.5                         # 0.25..=8.0，允许非整数
position = { x = 1920, y = 0 }      # 省略则由合成器自动放置；允许负坐标

[input]
keyboard_layout = "us"
keyboard_variant = ""
keyboard_repeat_delay_ms = 600
keyboard_repeat_rate_hz = 25
tap_to_click = true
natural_scroll = false

[layout]                            # master/stack 平铺
master_ratio = 60                   # 10..=90，master 列占工作区宽度的百分比
master_count = 1
gap = 8
```

### 输出（`[[outputs]]`）

`[[outputs]]` 是可重复的表数组。带 `name` 的条目匹配同名 connector；不带 `name` 的条目按出现顺序
认领前面条目没有认领的输出；没有任何条目认领的输出使用其原生首选模式与 1.0 缩放。

| 键 | 类型 | 默认值 | 说明 |
|----|------|--------|------|
| `name` | 字符串 | 无（省略） | 输出连接器名，如 `HDMI-A-1`；省略则按顺序认领未被认领的输出 |
| `width` | 整数 | `1280` | 输出宽度（物理像素），1..=16384 |
| `height` | 整数 | `720` | 输出高度（物理像素），1..=16384 |
| `refresh_hz` | 整数 | `60` | 刷新率（Hz），1..=1000 |
| `scale` | 浮点 | `1.0` | 缩放因子，`0.25..=8.0`，允许非整数 |
| `position` | 表 | 省略（自动放置） | `{ x = …, y = … }`，全局逻辑坐标，允许负数；每个分量在 ±4194304 内 |

越界的 `width` / `height` / `refresh_hz` / `scale` / `position` 都在**解析期报错**并指出字段路径。
输出位置是全局逻辑坐标：第二个显示器放在第一个右边就写 `x = <第一个显示器的逻辑宽度>`，
放在左边就写负数。省略 `position` 时由合成器自动放置到已放置输出的包围盒右侧。

缩放是每个输出独立的：逻辑尺寸 = 物理尺寸 ÷ 缩放。只理解整数缩放的客户端从
`wl_compositor` v6 的 `preferred_buffer_scale` 拿到缩放的**上取整**，理解
`wp_fractional_scale_v1` 的客户端拿到精确值；命中测试与窗口几何始终用逻辑坐标。

### 输入（`[input]`）

| 键 | 类型 | 默认值 | 说明 |
|----|------|--------|------|
| `keyboard_layout` | 字符串 | `us` | 键盘布局 |
| `keyboard_variant` | 字符串 | `""`（空） | 布局变体 |
| `keyboard_repeat_delay_ms` | 整数 | `600` | 开始重复前的延迟（毫秒） |
| `keyboard_repeat_rate_hz` | 整数 | `25` | 重复速率（Hz） |
| `tap_to_click` | 布尔 | `true` | 轻触点击 |
| `natural_scroll` | 布尔 | `false` | 自然滚动方向 |

### 布局（`[layout]`）

平铺策略是 master/stack：master 列在工作区左侧，其余窗口在右侧堆叠。

| 键 | 类型 | 默认值 | 取值范围 | 说明 |
|----|------|--------|----------|------|
| `master_ratio` | 整数 | `60` | `10..=90` | master 列占工作区宽度的百分比 |
| `master_count` | 整数 | `1` | ≥ 1 | master 列中的窗口数 |
| `gap` | 整数 | `8` | ≥ 0 | 工作区边缘与窗口之间、相邻窗口之间的间隙（逻辑像素） |

越界的平铺参数在**解析期报错**，不会被静默修正。注意当前合成器内部另有一层防御性的参数 clamp（`LayoutConfig::sanitised`，越界值会被夹到合法区间而不是报错）；那是运行期的兜底，不应作为配置校验的替代。

## 与当前实现的差距

下表把本页的字段对应到 `inox-compositor` **现在**实际支持的输入方式。CLI 参数是当前唯一可用的配置途径（`inox-compositor --help` 可查看）：

| 配置项 | 现在的对应物 |
|--------|--------------|
| 输出尺寸 | `--output-size <WxH>`（默认 `1280x720`，最大 `16384`），作用于第一个输出 |
| 刷新率 | `--refresh-rate <HZ>`（默认 `60`，最大 `1000`），作用于第一个输出 |
| 输出名 | `--output name=…`（默认 `headless-1`） |
| `scale` | `--output scale=…`（默认 `1.0`，范围 `0.25..=8.0`） |
| 输出位置 | `--output x=…,y=…`（省略则自动放置） |
| 多输出 | `--output <SPEC>` 可重复；第一次使用**替换**默认输出，之后依次追加 |
| Wayland 套接字 | `--socket <NAME>`（默认自动选择下一个空闲名字） |
| `[layout]` 的全部键 | 固定为默认值 `60 / 1 / 8`，无对应参数 |
| `[input]` 的全部键 | **尚未接入**：键盘布局固定为 `rules=evdev, model=pc105, layout=us`，不读取 `XKB_DEFAULT_*` 环境变量；重复速率、轻触点击与自然滚动都还没有实现，因为真实输入设备链路（libinput/udev）尚未引入 |
| 输出数量 | 任意多个（`--output` 可重复）；headless 与物理路径都支持多输出 |

物理后端（`--backend auto` / `drm`）**会**读取 `compositor.toml` 的 `[[outputs]]`，消费其中的
`name`、`width`、`height`、`refresh_hz`、`scale` 与 `position`。headless 路径仍只接受命令行参数，
因此上表里的 `[input]`、`[layout]` 与 headless 的 `[[outputs]]` 今天写进 `~/.config/inox/` 仍然无效。
