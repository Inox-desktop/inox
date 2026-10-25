# inox-config

配置模型与 TOML 解析。属于 Inox 桌面环境的 **L1** 层。

## 状态

已实现配置发现、模型、主题与热重载（PRD §9.1 / §9.2）：

- 通过 `inox-common` 的 XDG 快照定位 `$XDG_CONFIG_HOME/inox`；
- `config.toml`、`compositor.toml` 与主题的 TOML 模型，所有字段都有完整默认值；
- 解析错误带文件、行、列与字段路径；
- 内置 `inox-light` / `inox-dark` 主题与用户主题加载；
- `ConfigStore` 热重载：加载成功才替换，失败保留旧配置并 `warn!`。

行为要点：

- 未知字段、类型错误、颜色字面量错误、**平铺参数越界**（`master_ratio` 不在 10..=90、
  `master_count` < 1、`gap` < 0）与**输出参数越界**（尺寸不在 1..=16384、刷新率不在 1..=1000、
  `scale` 非正或非有限）都在解析期报错，不做静默钳制；`LayoutConfig::sanitised()`
  只是消费方对非配置来源的防御兜底。
- `[input]` 目前只校验类型不校验范围：真实输入后端尚未落地，范围校验随它一起到来。
- 文件缺失只记 `debug!`，所以排查「配置没生效」时用 `RUST_LOG=inox_config=debug` 运行。
- `[components]` 的三个路径在**启动时**全部校验，且在任何组件文件被读取之前完成；
  空值、绝对路径与越出配置目录的路径一律拒绝。
- 主题文件的 `appearance` 只接受 `light` / `dark`：主题只描述一种配色，
  `system` 只属于 `config.toml` 的选择层。

## 配置布局

```
~/.config/inox/
├── config.toml          # 顶层：组件文件、环境变量、主题选择、语言
├── compositor.toml      # 输出、输入、平铺布局
├── shell.toml           # 面板、启动器、壁纸
├── notifications.toml   # 通知显示、超时、队列与策略
└── themes/
    └── <name>/
        ├── theme.toml   # 颜色、排版、间距、圆角、动效时长
        └── assets/
```

- `config.toml` 与三个组件配置文件（`compositor.toml`、`shell.toml`、`notifications.toml`）
  缺失不是错误：回退到默认值。文件存在却无法读取或解析则是错误。
- **主题文件缺失是错误**：`themes/<name>/theme.toml` 不存在返回 `ThemeNotFound`——
  显式选了一个主题却读不到，不能悄悄换成深色。
- `shell.toml`（`src/shell.rs`）与 `notifications.toml`（`src/notifications.rs`）的字段模型随各自的消费者
  落地：面板、启动器、壁纸与通知守护进程。`[locale]` 是会话唯一的语言解析点，消费方从
  `InoxConfig::locale` 取已协商的语言，不自行读取环境变量。
- `compositor.toml` 的输出默认值（1280x720@60、scale 1.0）与平铺默认值（`master_ratio` 60、
  `master_count` 1、`gap` 8）与 `inox-compositor` 当前实现一致；这一致性目前靠人工核对，
  计划在 M6 由 `inox-integration` 的对照测试钉住（该测试尚不存在）。

## 模块

| 模块 | 职责 |
|------|------|
| `error` | `ConfigError`，以及把 I/O 与解析失败统一成该错误的两个辅助函数 |
| `paths` | `ConfigPaths`：PRD §9.1 的目录布局、组件路径与主题名的合法性检查 |
| `root` | `config.toml`：`RootConfig`、`ComponentFiles`、`ThemeSelection`、`Appearance` |
| `theme` | 设计令牌、内置主题、主题解析与加载 |
| `compositor` | `compositor.toml`：输出、输入、平铺布局 |
| `wallpapers` | Inox 随发行版附带的内置壁纸清单（标识、文件名、作者、来源、许可），以及默认与「主题背景」两个保留取值 |
| `store` | `InoxConfig::load` 与 `ConfigStore` 热重载 |

所有模型都带 `#[serde(deny_unknown_fields)]`：配置里的拼写错误必须报错，不允许静默忽略。

## 开发约定

- 本项目遵循[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，不在本地进行编译验证。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)。

## 许可证

LGPL-3.0，见仓库根的 [`LICENSE`](../../LICENSE)。
