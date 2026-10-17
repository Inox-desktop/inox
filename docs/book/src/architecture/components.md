# 组件清单

Inox 采用**单一仓库（Cargo workspace）**：全部组件都是本仓库 `crates/` 下的工作区成员，彼此通过 path 依赖连接，版本由 `[workspace.package]` 统一（见[依赖管理](../development/dependencies.md)）。决策见 [ADR 0016](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0016-single-repository-cargo-workspace.md)（取代 ADR 0001 的多仓库策略）。组件清单以 PRD §3.1 为准。

**层级成员的唯一事实来源**是根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`，并由 `cargo test -p inox-integration --test layering` 断言（验收项 B04）——本页下表与它不一致时以它为准。

## 状态说明

本页的「当前状态」按以下口径标注，均可在 `crates/<name>/` 中核对：

| 标注 | 含义 |
|------|------|
| **已实现** | 该 crate 中已有可用实现，且有对应测试与 CI |
| **骨架** | 只有 crate 结构、CI 与文档，`src/` 中尚无业务逻辑 |
| **计划中** | 尚未创建，仅在 PRD 中规划 |

## 全部组件

| crate | 层级 | 路径 | 职责 | 发布物 | 当前状态 |
|-------|------|------|------|--------|----------|
| `inox-common` | L1 | `crates/inox-common` | 共享类型、错误处理、日志、工具函数 | crate | 已实现（`logging`、`paths`） |
| `inox-config` | L1 | `crates/inox-config` | 配置模型、TOML 解析、默认配置 | crate | 已实现（模型、解析、错误定位；ADR 0009 仍为 `proposed`） |
| `inox-protocols` | L2 | `crates/inox-protocols` | 标准协议绑定与自定义 Wayland 协议扩展 | crate | 已实现（标准 `wl_*` 再导出 + 私有 `inox-ipc`） |
| `inox-compositor` | L2 | `crates/inox-compositor` | 基于 Smithay 的 Wayland 合成器（广告 `wl_output`/`wl_compositor` 与私有 `inox_ipc`） | 二进制 `inox-compositor` | 已实现（M1、M2；真实后端为脚手架） |
| `inox-xwayland` | L2 | `crates/inox-xwayland` | XWayland 集成（监督 xwayland-satellite 进程） | 二进制/服务 | 已实现（进程监督、就绪探测、生命周期） |
| `inox-shell` | L3 | `crates/inox-shell` | Shell 主框架与插件系统 | 二进制 `inox-shell` | 骨架 |
| `inox-panel` | L3 | `crates/inox-panel` | 面板（任务栏、系统托盘、时钟） | 二进制 `inox-panel` | 骨架 |
| `inox-launcher` | L3 | `crates/inox-launcher` | 应用启动器 | 二进制 `inox-launcher` | 骨架 |
| `inox-notifications` | L3 | `crates/inox-notifications` | 通知守护进程 | 二进制 `inox-notifications` | 骨架 |
| `inox-settings` | L3 | `crates/inox-settings` | 系统设置应用 | 二进制 `inox-settings` | 骨架 |
| `inox-lock` | L3 | `crates/inox-lock` | 锁屏（`ext-session-lock-v1`） | 二进制 `inox-lock` | 骨架 |
| `inox-wallpaper` | L3 | `crates/inox-wallpaper` | 壁纸设置与渲染 | 二进制 `inox-wallpaper` | 骨架 |
| `inox-session` | L4 | `crates/inox-session` | 会话管理器，拉起合成器与 Shell | 二进制 `inox-session` | 骨架 |
| `inox-idle` | L4 | `crates/inox-idle` | 空闲管理 | 二进制 `inox-idle` | 骨架 |
| `inox-greet` | L4 | `crates/inox-greet` | 显示管理器（可选） | 二进制 `inox-greet` | 骨架 |
| `inox-integration` | 测试 | `crates/inox-integration` | 端到端集成测试与分层方向检查（B04） | 测试报告 | 部分实现（分层检查已落地，E2E 待 M6） |

工作区之外还有两处非 crate 位置：

- **本站点**位于 `docs/book/`（mdBook），无 `Cargo.toml`，不参与依赖图与 `cargo deny`；
- 组织仓库 `.github` 承载组织级工作流模板与 Issue/PR 模板。

规划中、**尚未创建**的组件：`inox-portal`（`xdg-desktop-portal` 后端）、`inox-screenshot`（截图/录屏）、`inox-osd`（屏幕提示）、`inox-cli`（`inoxctl` 命令行工具）。新增 crate 属破坏性架构变更，须先写 ADR。

## 需要注意的几处

### `inox-common` 是叶子节点

`inox-common` 不得依赖任何其他 Inox crate，它是 L1 的依赖终点，因此只收纳**不依赖 Inox 其他部分**的共享能力。当前工作区版本为 `0.1.1`，提供 `logging`（统一的 `tracing` 初始化入口）与 `paths`（XDG 基础目录解析）模块。

这条规则由 `layering.rs` 机械断言：即使把 `inox-common` 加进同层例外表也无法放行，叶子规则先于例外检查。

### `inox-compositor` 的二进制名

PRD §3.1 曾把发布物记作 `inox-comp`；根级 `AGENTS.md` §5 说明这只是简写，实际二进制与你运行的命令都是 `inox-compositor`。

当前 `inox-compositor` 实现的是 **M1 + M2**：headless 输出创建、键盘与指针事件处理、以 `xdg-shell` toplevel 为窗口的 master/stack 平铺。`src/backend/drm.rs` 是 M2.5 真实后端的脚手架（ADR 0010 状态 `proposed`）：后端特性已在 `Cargo.toml` 中启用并被 CI 编译检查，但真实 DRM/KMS 与 libinput 路径尚未在运行中打通，也无 XWayland、剪贴板与拖放。详见[构建与运行](../usage/building.md)。

### L2 内部有一条已登记的同层边

`inox-compositor` 依赖 `inox-protocols`（核心 `wl_*` 绑定与私有 `inox-ipc`），而两者同属 L2。这既是 ADR 0002 规则 4（协议只能从 `inox-protocols` 出口）的要求，又与规则 1（禁止同层依赖）冲突——四层模型表达不了层内顺序。该边登记在 `[workspace.metadata.inox.rules] same_layer_exceptions` 中，删掉条目 CI 会立刻变红。

同样的情况在 L1 已存在（`inox-config → inox-common`）。规则 1 该如何表达层内顺序仍是待裁决项；现在它阻塞的是已经落地的代码，不再是假设。

### 合成器与 Shell 之间没有 Rust 依赖

L2 与 L3 之间的边界不是「暂时没有依赖」，而是 ADR 0002 规则 3 明确规定的设计：两者只能通过 Wayland 协议、`inox-ipc` 与 D-Bus 通信。这意味着 Shell 组件可以在虚拟合成器（`weston --backend=headless`）中单独测试，Shell 崩溃也不会带走合成器。该规则同样由 `layering.rs` 断言，双向都不允许。

### L3 访问合成器只有一个登记的单点

ADR 0002 规则 6 允许 `inox-wallpaper` 直接依赖 `inox-compositor`，其余 L3 组件不得如此。这一单点登记在根 `Cargo.toml` 的 `[workspace.metadata.inox.rules] l3_dependencies_on_compositor` 中；[ADR 0013](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0013-shell-service-and-interaction-contracts.md) 提议取消该例外，接受后删掉登记条目即收紧规则。

## 相关页面

- 每层的依赖规则见[总体架构](./overview.md)；
- 各组件的 CI 门禁见[持续集成](../development/ci.md)；
- 兼容性记录见[依赖管理](../development/dependencies.md)（工作区唯一的 `Cargo.lock`）。
