# 合成器配置

> **状态：模型已实现，物理后端已接入，headless 启动时不读 TOML。** 本页的字段与默认值来自配置模型的决策（状态 `proposed`）。`inox-config` 已实现 `compositor.toml` 的模型、范围校验与错误定位并有测试；`inox-compositor` 的**物理后端**启动时读取 `compositor.toml` 的 `[[outputs]]`、`[input]` 与 `[layout]`，运行中可按下面的热重载规则应用其中一部分；**headless 后端启动时只解析命令行参数，只有显式重载才读取配置文档**（输出改动仍被拒绝）。L3 组件已通过 `InoxConfig` 读取各自的配置文件。

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

[layout]                            # master/stack 平铺 + 工作区数量
master_ratio = 60                   # 10..=90，master 列占工作区宽度的百分比
master_count = 1
gap = 8
workspace_count = 1                 # 1..=9，工作区数量
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

平铺策略是 master/stack：master 列在工作区左侧，其余窗口在右侧堆叠。窗口不只有平铺一种形态——每个窗口在
平铺（`tiled`）、浮动（`floating`）、最大化（`maximized`）与全屏（`fullscreen`）之间切换，最小化与这四种模式正交。
`workspace_count` 给出窗口可以分布在几个工作区上；工作区编号从 0 开始，`Super+1`..=`Super+9` 正好覆盖 1..=9 个工作区，
因此上限是 9 而不是一个更大的数：没有按键可达的工作区是不可到达的状态。工作区数量在会话启动时固定，
运行中不动态增删（重载配置只会把落在消失工作区上的窗口移到最后一个仍然存在的工作区）。

| 键 | 类型 | 默认值 | 取值范围 | 说明 |
|----|------|--------|----------|------|
| `master_ratio` | 整数 | `60` | `10..=90` | master 列占工作区宽度的百分比 |
| `master_count` | 整数 | `1` | ≥ 1 | master 列中的窗口数 |
| `gap` | 整数 | `8` | ≥ 0 | 工作区边缘与窗口之间、相邻窗口之间的间隙（逻辑像素） |
| `workspace_count` | 整数 | `1` | `1..=9` | 工作区数量；编号从 0 开始，`Super+1`..=`Super+9` 逐个切换 |

越界的平铺参数与越界的 `workspace_count` 都在**解析期报错**并指出字段路径（`layout.master_ratio`、`layout.workspace_count`…），
不会被静默修正。注意当前合成器内部另有一层防御性的参数 clamp（`LayoutConfig::sanitised`，越界值会被夹到合法区间而不是报错）；
那是运行期的兜底，不应作为配置校验的替代。

### 窗口模式与工作区

每个窗口在任一时刻只处于一种模式，模式是互斥枚举而不是可以叠加的标志位：

| 模式 | 几何 | 说明 |
|------|------|------|
| `tiled` | master/stack 平铺 | 默认形态；回到平铺时重新进入它原有的布局顺序 |
| `floating` | 自己的几何，不参与平铺 | 客户端 `move`/`resize` 请求会把窗口转为浮动；首次浮动时使用工作区的 2/3 居中矩形 |
| `maximized` | 该输出的**工作区**（`work_area`） | 独占区面板预留的空间不被盖住，与 O02 的面板保留区规则一致 |
| `fullscreen` | 该输出的**整个** area | 面板被盖住，这是全屏的定义 |

最小化与模式**正交**：最小化保留模式与几何，激活时原样恢复。窗口只在**当前**工作区被合成，其他工作区的窗口保留状态但不可见。

已知限制：**浮动窗口的几何不持久化**——会话重启后回到默认位置，不写任何文件（持久化属于 `inox-settings` 的后续工作）；
工作区数量来自启动配置且会话期间固定。**指针交互式移动/缩放尚未实现**：`move`/`resize` 请求只把窗口转成浮动，
合成器没有接管拖动与缩放，因此没有一个把浮动窗口真正拖到新位置的路径。

### 快捷键

键位绑定是纯逻辑（`src/input.rs`），有单元测试，与客户端/任务栏请求走同一个状态机：

| 快捷键 | 动作 |
|--------|------|
| `Alt+Tab` / `Alt+Shift+Tab` | 焦点前后移动 |
| `Super+F` | 浮动 / 平铺切换 |
| `Super+M` | 最大化 / 还原切换（**切换**，不是单向最大化） |
| `Super+Shift+F` | 全屏 / 还原切换 |
| `Super+H` | 最小化焦点窗口 |
| `Super+1`..=`Super+9` | 切换到第 1..=9 个工作区（内部编号 0..=8） |
| `Super+Shift+1`..=`Super+Shift+9` | 把焦点窗口移到第 1..=9 个工作区 |
| `Super+Shift+Q` | 向焦点窗口发 `xdg_toplevel.close`（是请求，客户端可以保存、确认或拒绝；合成器不替它拆窗口） |
| `Super+Q` | 退出合成器 |

切换到不存在的工作区（例如 `workspace_count` 配置为 3 时按 `Super+5`）与越界的 `move_to_workspace` 一样被拒绝，
不会创建工作区。会话锁定时这些快捷键不再被拦截，按键全部属于锁表面。

关闭窗口只有「请求」这一种语义：`Super+Shift+Q` 与任务栏的中键关闭走同一条路径，都只发 `xdg_toplevel.close`。
客户端可以保存、弹窗确认或拒绝；合成器**不会**在客户端销毁 toplevel 之前拆掉窗口，任务栏也只在收到
`window_removed` 之后才移除按钮。

## 与当前实现的差距

两条后端路径读取配置的方式不同，下表把本页的字段分别对应到它们**现在**实际支持的输入方式：

- **物理后端**（`--backend auto` / `drm`）启动时用 `InoxConfig` 读取 `compositor.toml` 的 `[[outputs]]`、`[input]` 与 `[layout]`，运行中经会话 D-Bus 与 `SIGHUP` 的应用范围见[配置目录布局](./layout.md)的热重载一节；
- **headless 后端**启动时只解析命令行参数、**不读取 TOML**（`inox-compositor --help` 可查看）。headless 是 CI 与本地无显示环境用的确定性后端，它的输出与布局由命令行给定；运行中经会话 D-Bus / `SIGHUP` 的**显式重载**仍会读取配置文档并应用其中的 `[layout]`（输出改动继续被拒绝，headless 的输出只来自 `--output`）。

| 配置项 | headless（命令行） | 物理后端（`compositor.toml`） |
|--------|--------------------|------------------------------|
| 输出尺寸 | `--output-size <WxH>`（默认 `1280x720`，最大 `16384`），作用于第一个输出 | `[[outputs]] width` / `height` |
| 刷新率 | `--refresh-rate <HZ>`（默认 `60`，最大 `1000`），作用于第一个输出 | `[[outputs]] refresh_hz` |
| 输出名 | `--output name=…`（默认 `headless-1`） | `[[outputs]] name` |
| `scale` | `--output scale=…`（默认 `1.0`，范围 `0.25..=8.0`） | `[[outputs]] scale` |
| 输出位置 | `--output x=…,y=…`（省略则自动放置） | `[[outputs]] position` |
| 多输出 | `--output <SPEC>` 可重复；第一次使用**替换**默认输出，之后依次追加 | `[[outputs]]` 可重复（表数组） |
| Wayland 套接字 | `--socket <NAME>`（默认自动选择下一个空闲名字） | 同左（`--socket` 是进程参数，不从 TOML 读） |
| `[layout] master_ratio` / `master_count` / `gap` | 启动时**无对应参数**，固定为默认值 `60 / 1 / 8`；只有显式重载才从配置文档读取 | **已接入**：启动时读取，运行中重载会应用 |
| `[layout] workspace_count` | 启动时用 `--workspaces <N>`（范围 `1..=9`，默认 `1`；越界是**解析错误**而不是夹取）；显式重载时从配置文档读取 | **已接入**：启动时读取，重载时应用（数量仍固定，只在重载时随配置变化） |
| `[input] keyboard_layout` / `keyboard_variant` | **尚未接入**：固定为 `rules=evdev, model=pc105, layout=us`，且不读取 `XKB_DEFAULT_*` 环境变量 | **已接入**：编译 XKB keymap 时使用 |
| `[input]` 的重复速率、`tap_to_click`、`natural_scroll` | **不适用**：headless 没有真实输入设备链路 | **已接入**：libinput 设备配置，热重载可改 |
| 输出数量 | 任意多个（`--output` 可重复） | 任意多个（`[[outputs]]` 可重复） |

因此今天把 `[layout] workspace_count` 写进 `~/.config/inox/compositor.toml`，只有物理后端在**启动时**会读它；headless 启动时要设工作区数量请用
`--workspaces <N>`，两者表达的是同一件事（范围与默认值相同，且都指向同一个 `Workspace` 上限）。
两条路径都支持运行中的**显式重载**（会话 D-Bus 的 `org.inox.Configuration1.Reload` 或 `SIGHUP`），重载会读取配置文档并应用其中的 `[layout]`
（headless 还会重建键盘 keymap，所以 `[input] keyboard_layout`/`keyboard_variant` 在显式重载后同样生效）。但 headless 的文档一旦带
`[[outputs]]`、输入法或环境变量，整次重载被拒绝——`--output` 仍是它输出的唯一来源；`tap_to_click` / `natural_scroll` 在 headless 上没有
libinput 设备可应用，实际无效。
