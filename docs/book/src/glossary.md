# 术语表

## 项目术语

| 术语 | 含义 |
|------|------|
| **Inox** | 本项目，一个用 Rust 编写的模块化 Wayland 桌面环境 |
| **合成器（compositor）** | 负责输出管理、输入分发与渲染的服务；Inox 中即 `inox-compositor` |
| **Shell** | 用户可见的桌面外壳（面板、启动器、通知、设置等），与合成器以协议/DBus 通信而非 Rust 依赖 |
| **crate** | Rust 编译单元。Inox 中每个组件是一个 crate，位于 `crates/` |
| **工作区（workspace）** | 本仓库的 Cargo workspace，包含全部组件 crate |
| **层级表** | 根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`，声明每个 crate 属于 L1–L4 还是测试层，是分层成员的唯一事实来源 |
| **分层检查（B04）** | `cargo test -p inox-integration --test layering`，读取真实依赖图断言 ADR 0002 的依赖方向规则 |
| **`inox-ipc`** | Inox 私有的不稳定 Wayland 协议，用于合成器与 Shell 之间的控制与配置重载 |
| **ADR** | Architecture Decision Record，架构决策记录，见 `docs/adr/` |
| **DoD** | Definition of Done，里程碑验收标准 |
| **MSRV** | Minimum Supported Rust Version，最低支持的 Rust 版本；本工作区统一为 1.86 |
| **headless** | 无真实显示设备的运行/测试模式 |
| **E0–E4** | 验收证据等级，见[验收规范](https://github.com/Inox-desktop/inox/blob/main/docs/quality/acceptance.md) |
| **Portal** | `xdg-desktop-portal`，桌面集成标准接口 |
| **layer-shell** | `wlr-layer-shell-unstable-v1`，面板/壁纸等桌面层所用协议 |
| **satellite** | `xwayland-satellite`，被 `inox-xwayland` 监督的外部 XWayland 集成进程 |

## 已不再使用的说法

以下术语出现在 ADR 0016 之前的文档与 ADR 历史正文中，保留在此以便阅读旧记录：

| 术语 | 现状 |
|------|------|
| **元仓库（meta repo）** | 原指只承载 PRD、ADR 与兼容矩阵的 `inox` 仓库。ADR 0016 后 `inox` 是同时承载产品代码与文档的**单一仓库**，不再有独立的元仓库 |
| **组件仓库 / variant repos** | 原指每个组件一个独立 GitHub 仓库。现为 `crates/` 下的工作区成员；旧仓库待归档 |
| **`compat.toml`** | 原组件兼容矩阵，需人工登记且迁移前已与实际 tag 不符。已删除，兼容性记录改为工作区唯一的 `Cargo.lock` |
| **polyrepo / 多仓库策略** | ADR 0001 的决策，已被 ADR 0016 取代 |
