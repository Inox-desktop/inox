# 合成器配置

> **提案，尚未发布。** 本页的字段与默认值来自 [ADR 0009](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0009-configuration-model-and-reload.md)（状态 `proposed`）。当前 `inox-config` 仍是 M0 骨架，`inox-compositor` **只解析命令行参数、不读取 TOML**。合成器接入 `compositor.toml` 尚未实现。

涉及两个文件：顶层 `config.toml` 给出环境变量、组件文件位置与主题选择；`compositor.toml` 给出输出、输入与平铺参数。目录位置与通用规则见[配置目录布局](./layout.md)。

## `config.toml`

```toml
[environment]                      # 注入会话的环境变量，键值均为字符串
MOZ_ENABLE_WAYLAND = "1"

[components]                       # 相对配置目录的文件名，省略则用默认值
compositor = "compositor.toml"
shell = "shell.toml"               # 该文件的 schema 将在 M4 定义
notifications = "notifications.toml"  # 该文件的 schema 将在 M5 定义

[theme]
name = "inox-dark"                 # 省略则按 appearance 选择内置主题
appearance = "system"              # light | dark | system（默认）
```

| 段 | 作用 |
|----|------|
| `[environment]` | 会话范围内注入的环境变量（如 `MOZ_ENABLE_WAYLAND`）。键与值都必须是字符串 |
| `[components]` | 各组件的配置文件路径，取值是**相对配置目录**的文件名；绝对路径与 `..` 会被拒绝 |
| `[theme]` | 主题选择，详见[主题与设计令牌](./theme.md) |

## `compositor.toml`

```toml
[[output]]                          # 可重复；整个列表省略时等价于一个默认输出
name = "headless-1"                 # 省略则按输出顺序匹配
width = 1280
height = 720
refresh_hz = 60
scale = 1.0

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

### 输出（`[[output]]`）

`[[output]]` 是可重复的表数组，按出现顺序与输出匹配；整个列表省略时等价于一个默认输出。

| 键 | 类型 | 默认值 | 说明 |
|----|------|--------|------|
| `name` | 字符串 | 无（省略） | 输出连接器名，如 `HDMI-A-1`；省略则按输出顺序匹配 |
| `width` | 整数 | `1280` | 输出宽度（像素） |
| `height` | 整数 | `720` | 输出高度（像素） |
| `refresh_hz` | 整数 | `60` | 刷新率（Hz） |
| `scale` | 浮点 | `1.0` | 输出缩放 |

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
| 输出尺寸 | `--output-size <WxH>`（默认 `1280x720`，最大 `16384`） |
| 刷新率 | `--refresh-rate <HZ>`（默认 `60`，最大 `1000`） |
| 输出名 | 固定为 `headless-1`（无对应参数） |
| `scale` | 固定为整数缩放 1（无对应参数） |
| Wayland 套接字 | `--socket <NAME>`（默认自动选择下一个空闲名字） |
| `[layout]` 的全部键 | 固定为默认值 `60 / 1 / 8`，无对应参数 |
| `[input]` 的全部键 | **尚未接入**：键盘布局固定为 `rules=evdev, model=pc105, layout=us`，不读取 `XKB_DEFAULT_*` 环境变量；重复速率、轻触点击与自然滚动都还没有实现，因为真实输入设备链路（libinput/udev）尚未引入 |
| 输出数量 | 只有一个 headless 输出，无多输出支持 |

也就是说：**今天把上表里的 TOML 写进 `~/.config/inox/` 不会有任何效果**，合成器既不读它、也不会因为字段拼错而报错。
