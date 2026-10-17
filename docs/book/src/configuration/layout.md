# 配置目录布局

> **提案，尚未发布。** 本页与[合成器配置](./compositor.md)、[主题与设计令牌](./theme.md)描述的文件名、字段与默认值来自 [ADR 0009](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0009-configuration-model-and-reload.md)（状态 `proposed`）。就当前代码而言，`inox-config` 仍是 M0 骨架，`inox-compositor` 只解析命令行参数，**没有任何组件读取 TOML**。请勿把本页当作已发布行为。

目录布局本身由 PRD §9.1 规定：

```text
~/.config/inox/
├── config.toml          # 顶层：指向各组件配置、环境变量、主题选择
├── compositor.toml      # 输出、输入、平铺布局
├── shell.toml           # 面板、启动器、主题引用（schema 见 M4）
├── notifications.toml   # 通知（schema 见 M5）
└── themes/
    └── <name>/
        ├── theme.toml   # 颜色、排版、间距、圆角、动效时长
        └── assets/
```

## 谁来读哪个文件

| 文件 | 读取者 | schema 状态 |
|------|--------|-------------|
| `config.toml` | 会话与所有组件（入口） | 由 ADR 0009 定义 |
| `compositor.toml` | `inox-compositor` | 由 ADR 0009 定义 |
| `shell.toml` | `inox-shell` 及各 L3 组件 | 计划随 M4 定义 |
| `notifications.toml` | `inox-notifications` | 计划随 M5 定义 |
| `themes/<name>/theme.toml` | 所有消费设计令牌的组件 | 由 ADR 0009 定义 |

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
```

`[components]` 的值在进入文件系统前会被校验：绝对路径与包含 `..` 的路径一律拒绝，避免配置把读取范围引出 `~/.config/inox/`。`shell.toml` 与 `notifications.toml` 在当前提案中只保留路径、不定义字段，因此这两个文件即使格式有误也不会在启动期报错 —— 这是有意的安排，等 M4/M5 定义 schema 时再收紧。

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

- `inox-config`：M0 骨架，尚未实现任何解析逻辑。
- `inox-compositor`：只解析命令行参数，不读取 TOML（见[合成器配置](./compositor.md)）。
- `inox-shell`、`inox-notifications`：crate 仍为骨架，`shell.toml` / `notifications.toml` 的字段尚未定义。
- `inox-common` 的 `paths` 模块已实现 XDG 解析，但尚未打 tag 发布。
