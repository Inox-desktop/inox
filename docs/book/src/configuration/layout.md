# 配置目录布局

> **状态：`inox-config` 已实现，消费方接入中。** 本页与[合成器配置](./compositor.md)、[面板、启动器与壁纸配置](./shell.md)、[通知配置](./notifications.md)、[主题与设计令牌](./theme.md)描述的字段与默认值来自 [ADR 0009](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0009-configuration-model-and-reload.md)（状态 `proposed`）与 [ADR 0014](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0014-configuration-apply-and-migration.md)（状态 `proposed`）。`inox-config` 的模型、解析、错误定位与热重载语义均已实现并有测试；L3 组件通过 `InoxConfig` 读取配置。`inox-compositor` 的物理后端启动时读取 TOML，运行中应用范围见本页热重载部分；各组件的行为验收等级见 [ADR 0018](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0018-l3-component-architecture.md)。

目录布局本身由 PRD §9.1 规定：

```text
~/.config/inox/
├── config.toml          # 顶层：指向各组件配置、环境变量、主题选择
├── compositor.toml      # 输出、输入、平铺布局
├── shell.toml           # 面板、启动器、壁纸
├── notifications.toml   # 通知显示、超时、队列与策略
└── themes/
    └── <name>/
        ├── theme.toml   # 颜色、排版、间距、圆角、动效时长
        └── assets/
```

## 谁来读哪个文件

| 文件 | 读取者 | schema 状态 |
|------|--------|-------------|
| `config.toml` | 所有组件（入口：组件文件、环境变量、主题、语言） | 已实现 |
| `compositor.toml` | `inox-compositor` | 已实现，但合成器**尚未读取** |
| `shell.toml` | `inox-panel`、`inox-launcher`、`inox-wallpaper` | 已实现，见[该页](./shell.md) |
| `notifications.toml` | `inox-notifications` | 已实现，见[该页](./notifications.md) |
| `themes/<name>/theme.toml` | 所有消费设计令牌的组件 | 已实现 |

`config.toml` 是**入口文件**：它用 `[components]` 段以**相对配置目录的文件名**指向各组件的配置文件，而不是让每个组件自己去猜路径。

```toml
[environment]                      # 注入会话的环境变量，键值均为字符串
MOZ_ENABLE_WAYLAND = "1"

[components]                       # 相对配置目录的文件名，省略则用默认值
compositor = "compositor.toml"
shell = "shell.toml"
notifications = "notifications.toml"

[theme]
name = "inox-dark"                 # 省略则按 appearance 选择内置主题
appearance = "system"              # light | dark | system（默认）

[locale]
language = "zh-CN"                 # 省略则跟随环境（LC_ALL / LC_MESSAGES / LANG）
```

`[components]` 的值在进入文件系统前会被校验：绝对路径与包含 `..` 的路径一律拒绝，避免配置把读取范围引出 `~/.config/inox/`。三个路径的校验都在 `InoxConfig::load` 内完成，因此某个组件的配置文件尚未被使用，也照样会在 Inox 启动期失败，而不是等该组件启动才失败。`shell.toml` 与 `notifications.toml` 的字段模型已随各自的消费者落地，见[面板、启动器与壁纸配置](./shell.md)与[通知配置](./notifications.md)。

## `[locale]`

| 键 | 类型 | 默认值 | 说明 |
|----|------|--------|------|
| `language` | 字符串 | 省略 | BCP-47 语言标签；省略表示跟随环境 |

`[locale]` 是会话**唯一的语言解析点**：省略时按 `LC_ALL` → `LC_MESSAGES` → `LANG` 的顺序读取环境（`en_US.UTF-8` 这样的取值会被规整为 `en-US`，POSIX 的 `C`/`POSIX` 视为「没有语言」），显式给出时先解析标签再**协商**到 Inox 实际提供的目录上（当前为 `en-US` 与 `zh-CN`）。任何输入最终都落到一个已有译文目录，因此组件不需要自己决定「这个语言没有译文时怎么办」；标签本身不合法则在解析期报错（字段 `locale.language`），不静默回退。POSIX 的 `C` 与 `POSIX` 在配置文件里**也是错误**：它们表示「没有设置语言」而不是「名为 posix 的语言」，想要跟随环境请省略 `[locale]`，而不是写 `POSIX`。`en_US`、`zh_cn` 这类写法会被规整为规范拼写（环境变量同样如此），这是有意的宽容。

## 配置目录在哪里

配置目录遵循 XDG Base Directory Specification，由 `inox-common` 的 `paths` 模块解析（已实现），各组件不自行拼路径：

| 用途 | 取值 | 回退 |
|------|------|------|
| 配置 | `$XDG_CONFIG_HOME/inox` | `$HOME/.config/inox` |
| 数据 | `$XDG_DATA_HOME/inox` | `$HOME/.local/share/inox` |
| 状态 | `$XDG_STATE_HOME/inox` | `$HOME/.local/state/inox` |
| 缓存 | `$XDG_CACHE_HOME/inox` | `$HOME/.cache/inox` |
| 运行时 | `$XDG_RUNTIME_DIR` 与其下的 `inox` 子目录 | 无回退 |

按规范，空值与相对路径的环境变量视为无效并回退到默认值；`$XDG_RUNTIME_DIR` 例外，规范没有为它定义回退，因此它必须是已存在的绝对路径目录。

## 缺失文件与未知字段

ADR 0009 对四种情况给出了明确不同的行为，取舍是「首次运行不报错，写错配置必须报错」：

| 情况 | 行为 |
|------|------|
| 文件不存在 | 使用默认值，以 `debug!` 记录。这是首次运行的正常路径 |
| 字段缺失 | 使用默认值，不报错 |
| 字段拼错（未知字段） | **报错**。所有结构体启用 `deny_unknown_fields`，拼写错误在启动期暴露，而不是静默取默认值 |
| 解析失败 | **报错**，并给出文件路径、`行:列` 与字段路径 |

未知字段被拒绝这一点值得强调：它意味着**升级后可能出现「新版本组件 + 旧版本配置」之外的另一种组合** —— 旧版本组件 + 新版本配置会直接报错。跨版本混用时需要同时更新配置。

## 热重载

在设置中点击「应用」会先原子保存，再请求受影响的运行中组件重载。
只有消费方实际应用并回读当前候选后才显示「已生效」；组件未运行、超时或拒绝会分别报告。
手工编辑后可运行 `inoxctl reload`，或向指定消费方发送 `SIGHUP`。
解析失败保留当前配置；壁纸读取/解码失败保留旧帧。修改生效不重建现有应用连接或通知。

目前支持合成器的主题、键盘和触摸板偏好、平铺布局，以及面板内容、高度、启动器偏好、
通知策略和壁纸预览。**显示模式、输出缩放与位置可以在运行中切换，但必须先确认**：
合成器以「试用」应用新值，设置页弹出确认框并倒计时 30 秒，未确认就恢复试用前的偏好
并把 `compositor.toml` 的 `outputs` 改回原值；期限由合成器持有，设置窗口消失也会回退。
headless 后端的输出来自 `--output`，该路径仍拒绝运行时改动。输入法进程替换与进程环境
仍需下次会话。依赖 layer-shell 的面板摆放、自动隐藏和通知桌面角落仍受宿主限制，
会拒绝实时应用。

配置 schema 与快照在 L1 的 `inox-config`；D-Bus、SIGHUP 和资源提交在消费方二进制。
协议契约位于 `inox-protocols/protocol/configuration-v1.md` 与
`configuration-v2.md`（显示试用的 `Trial`/`Confirm`/`Rollback`），不新增 Cargo 层级边。
详见 [ADR 0026](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0026-live-configuration-reload.md)
与 [ADR 0028](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0028-display-trial-confirm-and-rollback.md)。

## 当前实现状态

- `inox-config`：目录发现、四个文件的模型与解析、字段路径与行列定位、主题解析、语义快照与 `ConfigStore`。
- `inox-compositor`：物理后端启动时读取 TOML；Wayland、会话 D-Bus 与 SIGHUP 可触发运行中配置应用。headless 默认仍保持命令行给定的输出，只有明确重载才读取配置。
- 面板、启动器、壁纸与通知：接入独立配置回执，保留原有运行状态；它们当前仍以普通窗口/预览运行，配置确认不证明桌面摆放已实现。
- 设置：逐消费方请求、语义回读、错误呈现与 `inoxctl reload`；仍保留原子写入和外部编辑检测。
- 锁屏、登录、会话环境、空闲服务等其他组件的运行中配置应用不在本次范围。
