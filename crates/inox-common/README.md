# inox-common

共享类型、错误处理与日志工具。属于 Inox 桌面环境的 **L1** 层。

## 状态

- 作为工作区成员以 path 依赖被 `inox-compositor`、`inox-config`、`inox-xwayland` 消费；工作区统一版本 `0.2.0`。
- `v0.1.1` 新增 `paths` 模块（0.x 阶段属兼容新增，走 patch；该版本由 `v0.2.0` 重切而来，旧 tag 已退役）：按 XDG Base Directory Specification 解析 `$XDG_CONFIG_HOME` / `$XDG_DATA_HOME` / `$XDG_STATE_HOME` / `$XDG_CACHE_HOME` / `$XDG_RUNTIME_DIR`，并给出 Inox 的 `<base>/inox` 目录，供 `inox-config` 等上层组件复用（PRD §9.1）。

当前模块：

| 模块 | 作用 |
|------|------|
| `logging` | 统一的 `tracing` 初始化入口（`RUST_LOG` 过滤，默认 `info`，不 panic），供各组件二进制复用 |
| `i18n` | Project Fluent 目录加载、语言协商（`en-US` / `zh-CN`，缺翻译回退默认语言）、以及目录 id 与占位符的一致性检查（验收 Q03）；不是 UI 运行时，因此留在 L1 |
| `paths` | XDG 基础目录解析（含 Inox 子目录）：不读全局状态，环境以 `XdgEnv` 快照注入，因此可完整单测 |

其余共享类型与工具函数在其**首个真实消费者出现时**再引入：本 crate 刻意不提前声明无人使用的抽象（依赖排期原则见 [ADR 0004](../../docs/adr/0004-m0-skeleton-and-dependency-policy.md)）。

## 开发约定

- 本项目遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，不在本地进行编译验证。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)。

## 许可证

BSD 3-Clause，见仓库根的 [`LICENSE`](../../LICENSE)。
