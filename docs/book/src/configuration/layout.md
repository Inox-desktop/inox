# 配置目录布局

> **状态：`inox-config` 已实现，消费方接入中。** 本页与[合成器配置](./compositor.md)、[面板、启动器与壁纸配置](./shell.md)、[通知配置](./notifications.md)、[主题与设计令牌](./theme.md)描述的字段与默认值来自 [ADR 0009](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0009-configuration-model-and-reload.md)（状态 `proposed`）与 [ADR 0014](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0014-configuration-apply-and-migration.md)（状态 `proposed`）。`inox-config` 的模型、解析、错误定位与热重载语义均已实现并有测试；L3 组件通过 `InoxConfig` 读取配置。**`inox-compositor` 仍只解析命令行参数**，尚未读取 `compositor.toml`；各组件的行为验收等级见 [ADR 0018](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0018-l3-component-architecture.md)。

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

- 组件在收到 `SIGHUP` 或 `inoxctl reload` 后重新读盘。
- 重载失败时**保留旧配置**并以 `warn!` 记录原因，返回错误但不退出、不半应用。
- `inox-config` 只提供确定性的重载语义（重新读盘、成功则原子替换、失败则保留旧值），**不引入信号依赖、也不实现 IPC 服务端**：SIGHUP 到 calloop 事件源的接线属于各组件，`inoxctl reload` 的服务端属于 L2 的 `inox-protocols` + `inox-ipc`。这一分工来自 ADR 0002 的依赖方向（L1 不得依赖 L2）。
- 只有调用同一个重载入口的组件才享有「失败保留旧配置」的保证；自行读取配置的组件会绕过这条语义。

## 当前实现状态

- `inox-config`：已实现目录发现、四个文件的模型与解析、字段路径与行列定位、主题解析与 `ConfigStore` 热重载语义，并有单元测试。
- `inox-common`：`logging`、`paths` 已实现；`i18n` 提供 Fluent 目录加载、语言协商与目录一致性检查。
- `inox-compositor`：**只解析命令行参数，不读取 TOML**（见[合成器配置](./compositor.md)），也没有广告 layer-shell。
- L3 组件（面板、启动器、壁纸、通知、设置、锁屏）：通过 `InoxConfig` 读取各自的配置；交付等级与阻塞项见 [ADR 0018](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0018-l3-component-architecture.md)。
- L4 组件（`inox-session`、`inox-idle`、`inox-greet`）：仍为骨架。
