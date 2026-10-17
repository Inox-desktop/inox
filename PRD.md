# Inox Desktop 产品需求文档（PRD）

**版本**：1.2
**日期**：2026-10-16
**状态**：草案
**组织**：https://github.com/Inox-desktop
**许可证**：BSD 3-Clause

---

## 修订记录

| 版本 | 日期 | 变更摘要 |
|------|------|----------|
| 1.0 | 2026-10-16 | 初稿 |
| 1.1 | 2026-10-16 | 补充 Wayland 协议支持矩阵、非功能需求与性能预算、配置与 IPC 模型、安全/Portal、无障碍与国际化、打包与分发、里程碑验收标准、ADR 流程、命名与版本策略、术语表 |
| 1.2 | 2026-10-16 | 参考 Plasma Desktop 的实际测试与状态处理，提出分级证据、完整用户任务、多屏/故障/迁移/锁屏验收、性能测量口径和发布支持矩阵；新增 ADR 0011–0015。新增规范与架构调整待人类评审，不代表已开启门禁或已通过验收 |
| 1.3 | 2026-10-16 | 按 [ADR 0016](docs/adr/0016-single-repository-cargo-workspace.md) 把多仓库策略改为单一仓库（Cargo workspace）：§3 由「仓库架构」改为「工作区与组件架构」，依赖改为工作区内 path 依赖，删除 `compat.toml` 与 `repos/` 流程；§2.1 的依赖方向改由 `layering.rs`（B04）机械强制。ADR 0001 已由 ADR 0016 取代 |

---

## 1. 项目概述

**Inox** 是一个用 Rust 编写的现代 Wayland 桌面环境。名称取自“不锈钢”（Inox），与 Rust 的“锈”形成反差，寓意坚固、耐腐蚀、长期可维护。项目采用 **AI-first 开发模式**：AI 生成绝大部分代码，人类负责产品决策、架构验收与硬件测试。所有编译、测试、lint 均在 GitHub Actions 中完成，不在本地进行编译验证。

### 1.1 目标

- 构建一个模块化、可维护、高性能的 Wayland 桌面环境。
- 最大化利用 Rust 生态中成熟的合成器库与 GUI 框架。
- 通过单一工作区内的单一职责 crate 实现清晰的模块边界，便于局部开发、局部测试与并行代理协作。
- 建立以 GitHub Actions 为核心的持续集成流水线，作为唯一的构建验证途径。
- 采用 BSD 3-Clause 许可证，保持开源友好。

### 1.2 非目标

- 不在项目初期支持所有 Wayland 协议或 X11 全部特性。
- 不追求与 GNOME/KDE 功能对等，优先保证核心体验稳定。
- 不进行本地编译验证；开发者与 AI 均依赖 CI 反馈。
- 不商业化，暂不进行商标注册（但需注意 INOX 商标风险）。
- 1.0 之前不追求全部辅助技术、语言与主题市场；核心任务的键盘操作、实际可读语义、字体缩放、高对比及 en-US/zh-CN 文案从 M4 起必须验收，不能仅保留空接口。

### 1.3 设计原则

1. **正确性优先于功能广度**：宁可少一个特性，不可留一个崩溃或卡顿路径。
2. **单一职责的 crate**：每个 crate 只解决一类问题，依赖方向严格自底向上。
3. **可测试性内建**：核心逻辑必须可在 headless 环境下被测试，副作用集中于边界层。
4. **配置即数据**：所有用户可见行为由 TOML 驱动，可热重载，AI 可生成与校验。
5. **可观测性内建**：使用 `tracing` 结构化日志，默认无遥测上传。
6. **无静默失败**：错误必须显式传播或记录，禁止吞掉错误。
7. **AI 可读性**：命名、目录、文档保持一致性，任何仓库的约定不得与根级 `AGENTS.md` 冲突。

---

## 2. 总体架构

Inox 采用分层架构，自底向上分为四层：

```
┌─────────────────────────────────────────────┐
│ Layer 4: 会话与系统集成                      │
│  inox-session, inox-greet, inox-idle         │
├─────────────────────────────────────────────┤
│ Layer 3: Shell 与用户界面                    │
│  inox-shell, inox-panel, inox-launcher,      │
│  inox-notifications, inox-settings,          │
│  inox-lock, inox-wallpaper                   │
├─────────────────────────────────────────────┤
│ Layer 2: 合成器与窗口管理                    │
│  inox-compositor, inox-xwayland,             │
│  inox-protocols                              │
├─────────────────────────────────────────────┤
│ Layer 1: 基础设施与共享库                    │
│  inox-common, inox-config                    │
└─────────────────────────────────────────────┘
```

- **Layer 1**：提供配置解析、通用类型、日志、工具函数，被所有上层依赖。
- **Layer 2**：核心 Wayland 合成器，基于 Smithay，处理 DRM/KMS、输入、渲染、窗口管理、XWayland 集成。
- **Layer 3**：桌面 Shell，使用 Iced 构建面板、启动器、通知、设置等用户界面组件。
- **Layer 4**：会话管理、显示管理器、空闲管理等系统级服务。

L1–L4 都是本仓库 `crates/` 下同一个 Cargo workspace 的成员（[ADR 0016](docs/adr/0016-single-repository-cargo-workspace.md)）。文档站点 `inox-docs` 不属于任何层：它是 `docs/book/` 下的 mdBook，无 `Cargo.toml`，不参与依赖图。

### 2.1 依赖方向约束

- 依赖只能向下：`L4 → L3 → L2 → L1`，禁止反向依赖、同层横向依赖与循环依赖。
- `inox-common` 不得依赖任何其他 Inox crate（叶子节点）。
- `inox-compositor` 与 `inox-shell` 之间**不通过 Rust 依赖**耦合，而是通过 Wayland 协议 / IPC 通信，保证可独立运行与测试。
- 以上规则不再只靠约定：层级成员登记在根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`，由 `crates/inox-integration/tests/layering.rs`（验收项 B04）在 CI 中读取真实依赖图逐条断言，失败会指明具体的边与被违反的规则。**新增 crate 未登记层级会直接失败**，因此不存在绕过规则的路径。
- 规则例外同样登记在 `[workspace.metadata.inox.rules]` 中，当前有两条：
  1. `same_layer_exceptions`：`inox-config → inox-common`（L1 内部顺序）与 `inox-compositor → inox-protocols`（L2 内部顺序）。**四层模型无法表达层内顺序**：这两条边都是健康的实现方向，但 ADR 0002 规则 1 的字面表述禁止同层依赖，而规则 2（`inox-common` 为叶子）与规则 4（`inox-protocols` 是协议唯一出口）又要求它们存在。该歧义由 ADR 索引登记，`inox-compositor → inox-protocols` 已随合成器消费协议绑定而**落地**，因此规则 1 的裁决已是阻塞项而非可选项。
  2. `l3_dependencies_on_compositor`：ADR 0002 规则 6 允许的 L3→合成器单点（当前只有 `inox-wallpaper`）。
  删除例外条目即收紧规则，收紧需要相应 ADR。

### 2.2 运行时数据流与 IPC

```
 应用 (Wayland/X11 客户端)
        │  wayland socket
        ▼
 ┌───────────────────┐   inox-ipc (自定义 Wayland 协议)
 │  inox-compositor  │◄───────────────────────────┐
 └───────────────────┘                             │
        ▲  layer-shell / xdg-shell              ┌──┴────────────┐
        │                                       │  inox-shell   │
 ┌──────┴────────┐   D-Bus (notifications,      │  及 L3 组件   │
 │  inox-session │   settings, portals)         └───────────────┘
 └───────────────┘
```

- **Wayland 协议**：合成器与 Shell 组件之间使用 `wlr-layer-shell`、`xdg-shell` 等标准协议，以及 Inox 自定义的 `inox-ipc` 协议（见 `inox-protocols`）。
- **D-Bus**：通知、设置、xdg-desktop-portal 等跨进程服务使用 D-Bus 会话总线。
- **配置热重载**：组件监听 `SIGHUP` 或 `inox-ipc` 配置重载消息，重新读取 TOML 并原子应用。

### 2.3 Wayland 协议支持矩阵

| 协议 | 级别 | 说明 |
|------|------|------|
| `wl_compositor` / `wl_subcompositor` | 必须 | 基础表面与子表面 |
| `wl_shm` | 必须 | 共享内存缓冲（软件渲染回退） |
| `wl_seat` / `wl_pointer` / `wl_keyboard` | 必须 | 输入设备 |
| `wl_output` | 必须 | 输出设备 |
| `wl_data_device`（`wl_data_device_manager` v3） | M3 必须 | 剪贴板与拖放；读取权跟随键盘焦点（`set_data_device_focus`），见 [ADR 0017](docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md) |
| `xdg-shell` | 必须 | 标准窗口 |
| `xdg-decoration` | 必须 | 服务端/客户端装饰协商；M3 不广告，X11 窗口标题栏由 satellite 自绘（[ADR 0017](docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md)） |
| `wlr-layer-shell-unstable-v1`（`zwlr_layer_shell_v1` v4） | 必须 | 已接入合成器：四层合成、锚点/边距/独占区、configure/ack 与卸载清理、键盘交互；真实客户端协议/像素测试交由 CI。仅普通桌面层，安全锁屏另用 session-lock；L3 的 Iced 客户端接线尚未完成 |
| `viewporter`（`wp_viewporter`） | M3 必须 | 缩放与裁剪；`xwayland-satellite` 的宿主硬前提（缺失即启动失败），见 [ADR 0017](docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md) |
| `linux-dmabuf` | 必须 | GPU 缓冲零拷贝 |
| `presentation-time` | 必须 | 帧同步反馈 |
| `idle-inhibit` / `ext-idle-notify-v1` | 必须 | 空闲管理 |
| `pointer-constraints` / `relative-pointer` | 计划 | 游戏与 3D 应用 |
| `fractional-scale-v1` | M4 必须 | 分数缩放，见 [ADR 0020](docs/adr/0020-fractional-scaling.md)（状态 `proposed`）。已实现：`wl_compositor` v6 的 `preferred_buffer_scale` 与 `wp_fractional_scale_manager_v1` v1，渲染/光标/layer 按输出 scale 折算物理像素，几何、命中与**离屏像素**断言在 CI（E1+E3）。真实混合 DPI 客户端在 125%/150% 下的模糊度与缓冲尺寸仍是 E4，**未取的 E4 不因 CI 全绿而追认** |
| `wlr-output-management` | 计划 | 显示器管理 |
| `wlr-screencopy` / `ext-image-copy-capture` | 计划 | 截图与录屏 |
| `ext-session-lock-v1` | M5 必须 | 安全锁屏，不能以 layer-shell 遮罩替代；见 ADR 0015 |
| `zwp_primary_selection_device_manager_v1`（本表此前记作 `gtk-primary-selection`） | M3 必须 | 主选区（X11 中键粘贴兼容）；与 `wl_data_device_manager` 一并广告，见 [ADR 0017](docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md) |
| `xwayland-shell` | 计划 | XWayland 集成 |
| `wlr-virtual-keyboard` / `input-method-v2` | 仅规划 | 输入法（1.0 之后） |

> 新增协议必须先在 `inox-protocols` 定义或引用，并说明安全与生命周期约束；破坏性变更需 ADR。
> `wlr-data-control`（全局、无焦点限制的剪贴板读取路径，供剪贴板管理器使用）不在 M3 范围内：开放它需要按客户端凭据做授权过滤，因此与剪贴板管理器一并推迟到 M5（[ADR 0017](docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md)）。

本表是目标范围，**不是已实现声明**。每个受支持版本须另列：XML/接口版本、绑定来源、服务端广告版本、行为覆盖、权限规则、客户端互操作与验收报告。只有绑定或广告全局不能标记为“支持”；未验证能力保持 planned/experimental。输出管理、捕获、输入注入、私有控制与锁屏须由服务端授权，见 [ADR 0015](docs/adr/0015-session-lock-and-failure-boundaries.md)。layer-shell 用于面板/壁纸等普通桌面层，不提供安全锁屏保证。

---

## 3. 工作区与组件架构

全部组件都是本仓库 `crates/` 下同一个 Cargo workspace 的成员。本仓库同时承载 PRD、ADR 与文档站点；组织仓库 `.github` 只承载组织级工作流与 ISSUE/PR 模板。跨 crate 的依赖在工作区内使用 path 依赖（[ADR 0016](docs/adr/0016-single-repository-cargo-workspace.md)，取代 ADR 0001 的多仓库策略）。

### 3.1 组件清单

**层级成员的唯一事实来源是根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`**；本表必须与它一致，`cargo test -p inox-integration --test layering`（验收项 B04）会断言这一点。

| crate | 层级 | 路径 | 职责 | 发布物 |
|-------|------|------|------|--------|
| `inox-common` | L1 | `crates/inox-common` | 共享类型、错误处理、日志、工具函数 | crate |
| `inox-config` | L1 | `crates/inox-config` | 配置模型、TOML 解析、默认配置 | crate |
| `inox-protocols` | L2 | `crates/inox-protocols` | 标准协议绑定与自定义 Wayland 协议扩展 | crate |
| `inox-compositor` | L2 | `crates/inox-compositor` | Wayland 合成器，基于 Smithay | 二进制 `inox-compositor` |
| `inox-xwayland` | L2 | `crates/inox-xwayland` | XWayland 集成（监督 xwayland-satellite 进程） | 二进制/服务 |
| `inox-shell` | L3 | `crates/inox-shell` | Shell 主框架，插件系统 | 二进制 `inox-shell` |
| `inox-panel` | L3 | `crates/inox-panel` | 面板（任务栏、系统托盘、时钟） | 二进制 `inox-panel` |
| `inox-launcher` | L3 | `crates/inox-launcher` | 应用启动器 | 二进制 `inox-launcher` |
| `inox-notifications` | L3 | `crates/inox-notifications` | 通知守护进程 | 二进制 `inox-notifications` |
| `inox-settings` | L3 | `crates/inox-settings` | 系统设置应用 | 二进制 `inox-settings` |
| `inox-lock` | L3 | `crates/inox-lock` | 锁屏（`ext-session-lock-v1`） | 二进制 `inox-lock` |
| `inox-wallpaper` | L3 | `crates/inox-wallpaper` | 壁纸设置与渲染 | 二进制 `inox-wallpaper` |
| `inox-session` | L4 | `crates/inox-session` | 会话管理器，启动合成器与 Shell | 二进制 `inox-session` |
| `inox-idle` | L4 | `crates/inox-idle` | 空闲管理 | 二进制 `inox-idle` |
| `inox-greet` | L4 | `crates/inox-greet` | 显示管理器（可选） | 二进制 `inox-greet` |
| `inox-integration` | 测试 | `crates/inox-integration` | 端到端集成测试与分层方向检查（B04） | 测试报告 |

工作区之外还有两处非 crate 位置：

- `docs/book/`：用户与开发者文档站点（mdBook，原 `inox-docs` 仓库）。无 `Cargo.toml`，不参与依赖图，也不参与 `cargo deny`。
- 组织仓库 `.github`：组织级工作流模板、ISSUE/PR 模板与 `CONTRIBUTING`，无 Rust 代码。

**规划中的组件**（1.0 之后或按需提前；新增 crate 属破坏性架构变更，须先写 ADR）：

| crate | 层级 | 职责 |
|-------|------|------|
| `inox-portal` | L3/L4 | `xdg-desktop-portal` 后端（截图、文件选择、屏幕共享） |
| `inox-screenshot` | L3 | 截图/录屏前端工具 |
| `inox-osd` | L3 | 音量/亮度等屏幕提示 |
| `inox-cli` | L1 | 统一的 `inoxctl` 命令行控制工具（IPC 客户端） |

组件之间的实际依赖以根 `Cargo.toml` 与各 crate manifest 为准，本表不再声称依赖关系——旧表的「主要依赖」列混有 Cargo 与运行时协作关系，且**不作为同层 Rust 依赖的授权**。[ADR 0013](docs/adr/0013-shell-service-and-interaction-contracts.md) 提议取消 wallpaper 直连合成器的例外，明确 L3 独立进程与协议边界；该提案接受后须同步更新层级表的例外登记、ADR 0002 与 crate 级约束，实施前继续遵守根级规则。

### 3.2 依赖管理

- 工作区**内部**的 Inox crate 之间使用 **path 依赖**，在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明一次，成员用 `{ workspace = true }` 引用（根级 `AGENTS.md` §8）。
- 第三方依赖的版本同样只在 `[workspace.dependencies]` 声明；成员只追加自己需要的 feature。
- 工作区**外部**的依赖仍必须锁定 `tag` 或 `rev`，禁止使用未锁定的 `branch = "main"`。
- path 依赖必须带显式 `version`，否则其隐式 `*` 会被 `deny.toml` 的 `[bans] wildcards = "deny"` 拒绝。
- 版本由工作区统一（`[workspace.package] version`）；1.0 之前不做按 crate 的独立 SemVer 编排。需要单包发布时用 `cargo release -p <crate>`，tag 格式 `<crate>-vX.Y.Z`。
- 唯一的 `Cargo.lock` 就是组件兼容性记录，也是验收项 L01 的不可变清单来源；原 `compat.toml` 与 `repos/` 目录流程随 ADR 0016 删除。
- MSRV 由 `[workspace.package] rust-version` 统一给出（当前 1.86）。
- 依赖安全由单一 `deny.toml` 针对**完整依赖图**检查（许可证白名单 + RUSTSEC 漏洞 + 重复版本）；此前 18 份配置各自只看子图。

### 3.3 分支与发布策略

- 主分支：`main`，保护分支，要求 CI 通过。
- 功能分支：`feat/xxx`、`fix/xxx`、`chore/xxx`、`docs/xxx`。
- 发布：使用 `cargo-release` 打 tag；工作区统一版本时为 `vX.Y.Z`，单 crate 发布时为 `<crate>-vX.Y.Z`。
- 跨 crate 的改动在**一个 PR、一次 CI** 内原子完成；不再存在「先升上游并发布 tag，再逐个更新下游」的中间态。

### 3.4 工作区目录结构

```
inox/
├── Cargo.toml           # workspace：members、workspace.package、workspace.dependencies、
│                        #            workspace.lints、workspace.metadata.inox（层级表与规则例外）
├── Cargo.lock           # 唯一锁文件，即组件兼容性记录
├── deny.toml            # cargo-deny：针对完整依赖图的许可证/漏洞/重复版本策略
├── rust-toolchain.toml  # 固定 stable
├── AGENTS.md            # 根级 AI 约束（crate 级 AGENTS.md 只能追加/收紧）
├── PRD.md
├── LICENSE              # BSD 3-Clause，全仓库一份
├── README.md
├── docs/
│   ├── adr/             # 架构决策记录
│   ├── quality/         # 验收规范与报告
│   └── book/            # 用户与开发者文档站点（mdBook，无 Cargo.toml）
├── crates/
│   └── inox-<component>/
│       ├── Cargo.toml   # 字段一律 *.workspace = true
│       ├── AGENTS.md    # crate 级 AI 约束
│       ├── README.md
│       ├── src/
│       └── tests/
└── .github/workflows/   # ci.yml（含按 crate 分片的测试矩阵）、docs.yml
```

---

## 4. 技术栈

| 类别 | 技术 | 说明 |
|------|------|------|
| 语言 | Rust (stable) | 2024 edition |
| 合成器框架 | Smithay | Wayland 合成器底层抽象 |
| 事件循环 | calloop | 基于回调的事件循环 |
| 显示后端 | Smithay `backend_drm` | DRM/KMS 控制 |
| 输入后端 | Smithay `backend_libinput` | 输入设备抽象 |
| 会话管理 | libseat | 无 root 权限设备访问 |
| XWayland | xwayland-satellite | 无根 XWayland 集成 |
| GUI 工具包 | Iced | Elm 架构，单向数据流 |
| 配置格式 | TOML + serde | 人类可读，AI 友好 |
| 日志 | tracing + tracing-subscriber | 结构化日志 |
| 错误处理 | thiserror + anyhow | 库用 thiserror，应用用 anyhow |
| 测试 | cargo test, proptest, waydriver | 单元/属性/GUI 测试 |
| 性能分析 | Tracy (tracy-client) | 帧分析与输入延迟 |
| CI/CD | GitHub Actions | 唯一构建验证途径 |
| 代码质量 | rustfmt, clippy, cargo-deny | 强制检查 |
| 许可证 | BSD 3-Clause | 仓库根目录一份 LICENSE |

---

## 5. 开发流程与 AI 协作

### 5.1 核心原则

- **AI 生成代码，人类验收**：AI 负责实现、测试、文档、重构；人类负责架构决策、PR 审查、硬件验收。
- **CI 是唯一真相**：不在本地编译或运行测试。所有验证通过 GitHub Actions 完成。
- **小步提交**：每个 PR 聚焦单一模块，便于 AI 理解和 CI 快速反馈。

### 5.2 AI 代理工作流

1. 人类编写任务描述（Issue 或 PRD 片段）。
2. AI 代理读取根级 `AGENTS.md` 与目标 crate 目录下的 `AGENTS.md`，了解架构、命令、风格、禁止事项。
3. AI 生成代码，推送到功能分支，创建 PR。
4. GitHub Actions 自动运行：fmt、clippy、test、build、doc、deny。
5. AI 读取 CI 日志，自动修复错误，更新 PR。
6. 人类审查架构合理性与关键逻辑，批准合并。
7. 合并到 `main` 后，触发发布流程（可选）。

### 5.3 AGENTS.md 规范

根级 `AGENTS.md` 之外，每个 crate 目录（`crates/<name>/AGENTS.md`）必须包含 `AGENTS.md`，内容至少包括：

- 项目简介与职责
- 架构约束（如“禁止 unsafe”“必须使用 Smithay 抽象”）
- 本地命令（但强调不在本地运行，仅作参考）
- CI 命令与预期
- 代码风格
- 测试要求
- 依赖管理规则
- 提交信息格式（Conventional Commits）

各 crate 的 `AGENTS.md` 以根级为基础，仅补充该 crate 特有的约束（`unsafe` 白名单、MSRV 说明、测试要求、目录结构等），不得放宽根级的硬性规则。

### 5.4 变更管理与 ADR

- 架构决策记录（ADR）存放于本仓库 `docs/adr/NNNN-title.md`，采用 MADR 精简格式。
- 以下情况**必须**先写 ADR 并获得批准：新增/删除组件 crate、改变分层或依赖方向、引入新协议策略、更换核心依赖（Smithay/Iced/calloop）、改变 IPC 或配置格式的破坏性变更。
- ADR 状态：`proposed` → `accepted` → `superseded`。
- ADR 必须包含决策驱动因素、可行备选与取舍、状态/数据所有权、失败和恢复、安全边界、影响仓库、版本/迁移/回退、可执行验收 ID 与证据计划、未决阻塞及复审条件，不能仅重述技术选型。模板与审查清单见 [ADR 索引](docs/adr/README.md)。
- 架构接受、实现合入、CI 通过和产品验收分别记录；合并 proposed 文档不自动构成人类接受。状态变为 accepted 必须链接明确的人类批准记录；具体 API、依赖版本与许可证需有可核实来源。ADR 0001–0015 的历史正文保持不变，被取代的决策只更新状态字段与链接。

### 5.5 人类验收职责

人类负责且仅负责以下不可委派的工作：

- 架构合理性与边界划分的最终判断。
- 真实硬件上的功能验收（GPU 兼容、输入延迟、多显示器、XWayland 应用）。
- 许可证/商标/合规决策。
- 合并批准。

---

## 6. CI/CD 策略

### 6.1 工作区 CI 工作流

触发条件：`push` 到 `main`、`pull_request`。单一工作流 `.github/workflows/ci.yml` 取代原先的 17 份按仓库副本（其中 3 份已经分叉）。

| job | 内容 |
|-----|------|
| `fmt` | `cargo fmt --all -- --check` |
| `clippy` | `cargo clippy --workspace --all-targets --all-features -- -D warnings`（不链接，故不装系统库） |
| `deny` | `cargo deny check`，针对完整依赖图 |
| `doc` | `cargo doc --workspace --no-deps --all-features`，`RUSTDOCFLAGS=-D warnings` |
| `build` | `cargo build --workspace --release --all-features`，安装链接所需的系统库 |
| `plan` | 由 `[workspace.metadata.inox.layers]` 推导测试矩阵；覆盖不全即失败，不允许空矩阵 |
| `test` | **按 crate 分片**的矩阵，每片执行 `cargo test -p <crate> --all-features`；`fail-fast: false` |
| `xwayland-e2e` | `INOX_XWAYLAND_E2E=1 cargo test -p inox-xwayland --test headless_x11_mapping`；宿主为第三方 headless 合成器，仅 E2 |
| `book` | `mdbook build docs/book` |

要点：

- **分片由层级表推导**，不手工维护：crate 只要登记了层级就必然被构建与测试，因此不会出现「新增 crate 却漏测」的静默缺口。矩阵为空或未覆盖全部成员时 `plan` job 直接失败。
- 每个分片的命令与开发者本地执行的命令**逐字一致**（`cargo test -p <crate>`），失败可归因到具体 crate，且便于局部复现。
- 系统库只装在需要链接的分片（manifest 声明 `smithay` 的 crate）上，桩 crate 保持廉价。
- **模块特定测试**（沿用原要求）：
  - 合成器：headless 集成测试，软件渲染（`LIBGL_ALWAYS_SOFTWARE=1`），验证输出创建、渲染、输入事件、干净退出。
  - Shell：使用 `waydriver` 在虚拟 Wayland 合成器（如 `weston --backend=headless`）中启动应用，截图并验证。
  - 分层方向：`inox-integration` 的 `layering.rs` 逐条断言 ADR 0002 规则（验收项 B04）。
- 缓存：`Swatinem/rust-cache@v2`，工作区共享一份 target 与 registry 缓存，替代原先 17 条流水线各自从零编译同一批依赖。

### 6.2 CI 门禁规则

- 任一检查失败即阻止合并；不提供 `--no-verify` 等绕过手段。
- PR 必须通过分支保护规则：至少 1 名人类审查、CI 全绿、无未解决评论。
- 覆盖率阈值由各 crate 在 `AGENTS.md` 中声明（默认对核心 crate 行覆盖 ≥ 70%）。
- 1.2 提议采用[验收规范](docs/quality/acceptance.md)与 [ADR 0011](docs/adr/0011-evidence-based-acceptance.md)：PR 标明适用场景 ID，保留首次失败、用例数量、超时与环境证据；必需测试缺依赖、跳过、零用例或缺报告均不计通过。对应 job 补齐后才可声称自动门禁已实施。
- 只运行受影响场景与既有回归用于 PR 反馈；里程碑和发布运行累计完整套件。不能以反复重跑、删除断言或未经审查更新基准图绕过失败。

### 6.3 集成测试

`inox-integration` 仓库负责端到端测试：

- 通过 Git 依赖拉取所有组件。
- 在 GitHub Actions 中启动 `inox-session`（headless 后端 + 软件渲染）。
- 运行 `waydriver` 测试套件，模拟用户操作，验证窗口管理、面板、启动器等功能。
- 生成测试报告与截图作为 artifact。
- E3 必须以 Inox 合成器运行完整会话；第三方 headless 宿主测试仅为 E2 组件证据，且需核实宿主实际具备所需协议。使用私有 XDG 目录/总线、事件等待与有限超时，检查退出和残留资源。
- 正式验收锁定发布 tag 及解析后的 SHA；候选组合可锁 rev 预检。提交 [验收报告](docs/quality/acceptance-report.md)，保留环境、依赖、断言、失败日志与 GUI 语义/图像证据。

### 6.4 硬件验收

- CI 无法替代真实硬件测试。
- 人类使用对应提交的 CI 发布物，不在本地构建；验收 GPU/输入、VT、热插拔、混合 DPI、锁屏和挂起恢复，并记录硬件/驱动/内核/设备/校验和。
- 结果以硬件 Issue 关联 H01–H03 及各场景，逐配置声明 supported/experimental/unsupported。M6 最低支持矩阵包含 Intel Mesa、AMD Mesa、双屏不同 DPI、笔记本挂起/恢复；不能由一个设备推导所有驱动均支持。
- CI 失败不得由硬件验收覆盖；缺硬件证据时标记待验收，不能将真实后端编译通过视为设备支持。

### 6.5 发布流程

- 通过 `cargo-release` 打 tag 触发 GitHub Release：工作区统一版本时为 `vX.Y.Z`，单 crate 发布时为 `<crate>-vX.Y.Z`。
- 兼容性证据是唯一的 `Cargo.lock`：其中的提交与版本组合即「已发布组件组合」，不再有人工维护的兼容矩阵。
- 发布候选须完成 L01–L03：无未解决 P0/P1、支持矩阵与兼容应用清单、升级/回退证据、系统运行依赖/SBOM/校验和及人类批准。CI 自动证据至少保留 30 天，发布证据另行归档，不能只链接临时 artifact。

---

## 7. 测试策略

| 测试类型 | 工具 | 范围 |
|----------|------|------|
| 单元测试 | `cargo test` | 所有纯逻辑模块 |
| 属性测试 | `proptest` | 窗口管理、布局算法 |
| 集成测试 | `cargo test` + headless | 合成器输出、输入、协议 |
| GUI 测试 | `waydriver` | Shell 组件交互 |
| 性能测试 | Tracy | 帧时间、输入延迟 |
| 依赖安全 | `cargo-deny` | 许可证、漏洞 |
| 静态检查 | clippy, fmt | 全部代码 |

各层测试的结论边界、场景操作和证据以[验收规范](docs/quality/acceptance.md)为准。单元/属性测试不模拟核心逻辑；协议测试使用真实客户端；GUI 同时检查实际行为、焦点/语义和图像。真实后端提交失败、设备移除、服务崩溃、无效配置、迁移中断和权限拒绝必须覆盖，不能只测成功路径。

M4 前先用 CI 原型证明 Iced layer-shell、语义导出/AT-SPI、GUI 驱动与焦点链路可行；工具缺能力列为阻塞项，不能假定框架天然支持。完整任务基线至少覆盖：启动器搜索并启动→任务列表出现→切窗→配置应用/取消→服务重启重建→锁定/解锁→注销。

---

## 8. 非功能需求与性能预算

| 指标 | 目标 | 测量方式 |
|------|------|----------|
| 输入到屏幕延迟 | < 20 ms（P95） | P02：真实输入到像素变化，≥200 样本/轮，3 轮；Tracy 只辅助定位 |
| 合成帧 CPU 预算 | < 8 ms（P95，60 Hz）且负载期错过呈现截止帧 <1% | P01：固定分辨率/负载/设备，预热后 10 min，3 轮 |
| 空闲 CPU 占用 | 全部 Inox 进程合计均值 < 单核 1% | P03：预热后采样 5 min，3 轮 |
| 完整会话空闲内存 | RSS 总和峰值 < 300 MiB | P03：固定服务集合与 RSS 口径，另披露共享页重复计入 |
| 冷启动到可用会话 | < 3 s（P95） | P04：20 次，会话进程开始至核心服务 ready 且输入往返成功，披露缓存状态 |
| 打开启动器 / 搜索更新 | 软件响应 < 100 ms（P95） | P04：1000 条目、各 100 次 |
| 故障隔离 | Shell 重启保留应用连接；合成器故障明确结束/重建会话 | S05：不承诺恢复已断开的客户端或未保存数据，见 ADR 0015 |
| 稳定性 | E3 8 h / E4 24 h 无崩溃、挂死或焦点失联，资源增长受控 | P05：持续操作、锁屏/重载、RSS/FD 起终值与超时记录 |
| 日志开销 | 默认 `info` 级，无高频刷屏 | 代码审查 |

- 上述是产品预算，尚非实测结论；详细测量条件与稳定性容限见 [P01–P05](docs/quality/acceptance.md)。参考硬件和采样流程先登记再测量，每轮都应合格。
- 候选/基线同机交替测量；相同统计量回退 >5% 必须调查与复测，确认后提供原始数据和人类审查的取舍；绝对预算超限仍不通过。
- 软件渲染（llvmpipe）仅作为 CI 与回退路径，不作为性能验收基准。

---

## 9. 配置与主题

### 9.1 配置布局

```
~/.config/inox/
├── config.toml          # 顶层：指向各组件配置、环境变量
├── compositor.toml      # 输出、输入、窗口规则、快捷键
├── shell.toml           # 面板、启动器、主题引用
├── notifications.toml
└── themes/
    └── <name>/
        ├── theme.toml   # 颜色、圆角、间距、字体
        └── assets/
```

- 所有配置使用 TOML + serde，提供 `#[serde(default)]` 与完整默认值，缺失字段不报错。
- 配置解析错误必须给出可读的定位信息（文件、行号、字段路径）。
- 支持 `SIGHUP` 与 `inoxctl reload` 触发的热重载；重载失败时保留旧配置并记录警告。
- 解析成功、保存成功与组件已生效分别报告；消费方先准备/应用再推进代次，并发旧结果不能覆盖新请求。显示试用 15 s 未确认由服务端恢复；不承诺跨进程全局原子提交，见 [ADR 0014](docs/adr/0014-configuration-apply-and-migration.md)。
- 保存使用目标目录内的临时文件与原子替换/同步策略，检测外部编辑冲突；无屏/设备缺失不删除偏好。首次破坏性版本变更前必须有 schema 版本、历史 fixture、幂等迁移、备份及可复现回退。
- 路径安全除词法校验外还需符号链接/安全打开策略与竞态测试；不能声称仅拒绝 `..` 即保证读取范围。

### 9.2 主题系统

- 主题由 `theme.toml` 定义设计令牌（design tokens）：颜色、排版、间距、圆角、动效时长。
- Iced 组件只消费令牌，不得硬编码颜色/尺寸。
- 支持浅色/深色与跟随系统；默认提供 `inox-light`、`inox-dark`。
- 高对比、字体缩放与减少动态偏好从首个 Shell 阶段验证；自定义主题回退按所选外观匹配，不能以缺令牌为由产生不可读混色。涉及 ADR 0009 默认值语义的调整须先接受 ADR 0014 再迁移。

---

## 10. 安全、权限与 Portal

- **权限模型**：合成器不以 root 运行，设备访问通过 `libseat`。所有客户端按 Wayland 安全模型隔离，无全局输入注入给未授权客户端。
- **剪贴板/拖放**：仅在获得焦点的客户端间传递，遵守 `wl_data_device` 语义。
- **锁屏**：使用 `ext-session-lock-v1`，合成器拥有锁定状态；所有输出安全遮蔽后确认 locked，UI 崩溃或认证失败保持锁定，新增输出先遮蔽。挂起须等待锁定完成，见 [ADR 0015](docs/adr/0015-session-lock-and-failure-boundaries.md) 与 S02–S03。
- **特权能力**：输出管理、配置控制、捕获/注入和锁屏分别登记可信调用方、授权来源、撤销和版本范围；校验在服务端执行，不能用 app_id/PID 或同 UID 直接授予能力，见 S04。
- **Portal**：通过 `inox-portal` 提供 `xdg-desktop-portal` 后端，覆盖截图、屏幕共享、文件选择、设置；前端应用通过标准 Portal 接口访问，避免合成器直接暴露特权接口。
- `inox-portal` 仍是规划仓库，创建与后端方案须另有 ADR；未实现时明确披露，拒绝未经授权的捕获/注入，不能用默认开放私有接口替代。提供能力时须验证授权、拒绝、撤销和锁屏中止。
- **依赖安全**：`cargo-deny` 强制许可证白名单（BSD/MIT/Apache-2.0/ISC/Zlib 等），禁止 GPL/AGPL 依赖进入核心；定期扫描 RUSTSEC 漏洞。
- **禁止项**：不得引入远程代码执行、不得默认开启网络监听、不得上传遥测。

---

## 11. 无障碍与国际化

### 11.1 无障碍（Accessibility）

- 目标：键盘可完成主要操作（启动器、面板、设置、锁屏）。
- Iced 组件实际导出 role/name/state/action，经评审的桥接路径供 AT-SPI/屏幕阅读器读取；核心任务从 M4 起验证，完整生态后续扩展。桥接归属与可行性须通过原型确认，不把空接口或框架宣传当作证据。
- 尊重系统字体缩放与高对比度主题令牌。
- 支持屏幕放大所需的 `wlr-screencopy`/`ext-image-copy-capture`（计划）。
- Q01–Q03 要求唯一可见焦点、无键盘陷阱、200% 字体缩放、高对比、减少动态与 RTL 伪本地化布局测试；无需等到宣称完整无障碍后再补。

### 11.2 国际化（i18n / l10n）

- Shell 面向用户的字符串统一走 Project Fluent（`fluent-rs`），禁止硬编码 UI 文案。
- 默认语言 `en-US`，提供 `zh-CN`；支持 RTL 布局预留。
- 组件文案与代码分离，便于 AI 批量修订与校验占位符一致性。

---

## 12. 打包与分发

| 渠道 | 说明 | 优先级 |
|------|------|--------|
| 源码构建 | `cargo build --release`，最小系统依赖 | 必须 |
| Arch AUR | `inox-desktop` 包 | 高 |
| Nix Flake | `inox` flake 输出 | 高 |
| Fedora COPR | 面向 Fedora/不可变系统 | 中 |
| Flatpak | 仅用于 Shell 应用（合成器不适用） | 中 |
| Debian/Ubuntu | `.deb` + 后续 PPA | 低（1.0 后） |

- 发布物由 CI 生成（`--all-features`、strip、校验和与 SBOM）。
- 每个 Release 附带 `build-info`（commit、工具链版本、特性开关）。

---

## 13. 里程碑

本表是 1.2 提议的新验收口径，完整前置、断言、证据与累计规则见[验收规范](docs/quality/acceptance.md)。历史已完成项只代表原标准通过；本次不自动追认新标准验收。架构提案仍须按 ADR 状态获得批准后实施。

| 里程碑 | 目标 | 涉及 crate | 验收标准（DoD） |
|--------|------|----------|-----------------|
| M0 | 可追溯的工程基线 | 工作区骨架、`.github` | B01：骨架、约束、许可证与首次真实 CI；不宣称运行能力 |
| M1 | Headless 输出/渲染/生命周期 | `inox-compositor` | B01–B03、R01–R03：真实输出枚举、像素断言、启动失败、信号退出与 20 轮清理 |
| M2 | 真实客户端窗口与输入语义 | `inox-compositor` | W01–W04：3 窗口/子表面/popup、configure、布局边界、焦点/按键状态、坏客户端隔离 |
| M2.5 | 真实输出、输入与设备生命周期（ADR 0012 已获批；修正 ADR 0010 的冲突条目） | `inox-compositor` | O01/O04、H01 建档：CI 后端覆盖 + 人类 E4，含可见指针、VT/设备恢复；建立性能基线，不以 headless 代真机 |
| M3 | Inox 会话中的 X11/数据互通 | `inox-xwayland`, `inox-compositor` | X01–X03：真实 X11 往返/输入、双向 MIME、拖放接受/取消与异常退出；第三方宿主通过不算本阶段完成 |
| M4 | Shell 用户任务、双屏与持久化 | L3 组件、`inox-compositor`、`inox-config` | W05、O02–O03、D01–D05、C01–C03、Q01–Q03：任务状态/焦点、设置回执/回退、混合 DPI/热插拔、真实语义与双语 |
| M5 | 安全会话与指定设备上日常可用 | `inox-session`, `inox-idle`, `inox-lock`, `inox-notifications` 等 | D06、S01–S05、H02；P05 开始：监督限流、认证/锁屏故障不解锁、挂起/恢复、权限负向测试 |
| M6 | 指定支持矩阵的发布候选 | `inox-integration` 与全部 crate | C04、P01–P05、H01–H03、L01–L03：迁移/回退、性能与长稳原始数据、两种渠道安装生命周期、无 P0/P1、人类批准 |

M2.5 的物理后端实现已接入 libseat、DRM/GBM、udev 与 libinput，使用与 headless 共用的
帧状态机处理首帧、空闲、提交失败及暂停/恢复；提供软件光标和启动期配置消费。
多输出拓扑、按输出的帧生命周期与多 GPU 设备模型见 [ADR 0019](docs/adr/0019-multi-output-topology-and-multi-gpu.md)（状态 `proposed`）：
`InoxState` 持有一张输出表，所有输出映射进同一个全局逻辑坐标空间（位置可为负），
每个输出有自己的工作区与帧状态；物理后端枚举 seat 上的每块 GPU，每块用自己的渲染器驱动自己的 connector。
分数缩放见 [ADR 0020](docs/adr/0020-fractional-scaling.md)（状态 `proposed`）。
O02/O03 的虚拟拓扑、几何与像素证据由 `crates/inox-compositor/tests/multi_output.rs` 与
`tests/fractional_scale.rs` 在 CI 取得（E1+E3）；**O01–O04 的 E4 真机步骤仍未执行，旋转、PRIME 跨 GPU
复制路径与显示试用也未交付**，不能将 CI 或实现完成读作 M2.5/M4 验收通过。
记录见 [硬件验收](docs/quality/l2-hardware-validation.md)。

> M3 的 X11↔Wayland 拖放腿**上游阻塞**：`xwayland-satellite` v0.8.3 未实现 XDND，因此上表 M3 行的 X03 只交付 **Wayland↔Wayland 拖放**与 **X11 内部拖放**，跨边界那条腿的 E3 用例延期——M3 不得被读作 X03 完整通过。X02 中「锁屏时读剪贴板」的断言因 `ext-session-lock-v1` 与 `inox-lock` 尚不存在而移至 M5 的 S02（延期，不是通过）。两条决策与依据见 [ADR 0017](docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md)。
>
> **M4 的合成器侧已交付多输出与分数缩放的一部分，且同样不得被读作通过。** 输出拓扑（ADR 0019）
> 与分数缩放（ADR 0020）已实现，并以两个进程内真实客户端测试取得虚拟拓扑与像素证据（O02 的双屏迁移与
> 20 轮拔插、O03 的 1.5x 物理落点）：这些是 **E1+E3**，不是真机证据。旋转、PRIME 跨 GPU 缓冲复制路径、
> 显示试用（15 s 未确认回退）与 O02/O03 的 E4 步骤仍未交付。
>
> **M4 只交付了 L3 的一部分，且同样不得被读作通过。** 七个 L3 组件已按 [ADR 0013](docs/adr/0013-shell-service-and-interaction-contracts.md)
> 的服务契约有了实现：`inox-shell` 的跨入口导航与服务契约（快照/代次/命令结果/loading-empty-error-retry）、
> `inox-panel` 的任务分组与托盘生命周期、`inox-launcher` 的 XDG 条目解析与带代次的搜索、`inox-notifications`
> 的协议契约与有界队列策略、`inox-settings` 的草稿/校验/原子保存/回读状态机、`inox-lock` 的锁状态机与凭据清理、
> `inox-wallpaper` 的缩放几何与原子应用；`inox-config` 的 `shell.toml`、`notifications.toml` 字段模型与
> `config.toml` 的 `[locale]` 随各自的消费者落地。证据等级为 **E0/E1**（编译、lint、文档、单元与属性测试）
> 加**离屏渲染**（真实控件树在固定尺寸下的像素与事件断言，见[测试策略](docs/book/src/development/testing.md)）。
> **E2/E3 尚未取得，未完成的 DoD 项包括 W05、O02–O03、C02–C03 的实测、D01–D05 与 Q01–Q03 的宿主侧证据**，
> 合成器的 `zwlr_layer_shell_v1` v4 宿主已补齐，真实协议客户端测试纳入合成器 CI 分片；
> L3 的 Iced 客户端接线与 GUI 驱动证据仍缺失。此前 `iced_layershell` 0.19.1 与 0.19.0
> **都无法编译**（其 `iced_exdevtools` 依赖对 `winit-core` 0.31.0-beta.3 的 `non_exhaustive` 枚举
> `NativeKeyCode` 做了穷尽匹配）。因此第 7 节与 ADR 0013 要求的「M4 前完成 Iced + layer-shell 原型」
> **仍未完成**，各组件以普通窗口运行并把 layer-shell 放置策略建模为纯数据；锁屏的协议腿另受 ADR 0015
> 的人类安全审查约束，通知的 D-Bus 服务按本表属 M5（D06）。决策与边界见
> [ADR 0018](docs/adr/0018-l3-component-architecture.md)（状态 `proposed`）。

---

L4 首版实现边界见 [ADR 0021](docs/adr/0021-l4-session-services.md)：会话监督、标准空闲通知客户端、登录状态模型与预览已经提供；完整服务 ready、真实认证、安全锁屏及 logind 挂起仍待审查/实现，不能视为 M5 验收通过。

## 14. 风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| AI 生成代码质量不稳定 | 高 | CI 严格检查，人类审查关键模块，AGENTS.md 约束 |
| 跨 crate 依赖管理复杂 | 中 | 已由 ADR 0016 从多仓库改为单一工作区：path 依赖 + 唯一 `Cargo.lock` 取代 tag 锁定与人工兼容矩阵；分层方向由 B04 机械强制。剩余成本是根配置的爆炸半径，由 CODEOWNERS 与 CI 路径/分片控制 |
| GitHub Actions 无 GPU | 高 | llvmpipe 验证逻辑/协议；真实输出/VT/挂起单独 E4；缺证据不声明支持 |
| Wayland 协议兼容性 | 中 | 优先支持核心协议，逐步扩展，参考 Smithay 示例 |
| INOX 商标冲突 | 低 | 开源项目暂不商业化，必要时更名 |
| 本地无法编译导致调试困难 | 中 | 完整 CI 环境/失败证据；人类使用 CI 发布物验收，不在本地构建 |
| 性能回退被忽视 | 中 | 第 8 节预算纳入 PR 检查，Tracy 证据要求 |
| 无障碍/i18n 后期返工 | 高 | M4 前验证 Iced/语义桥/驱动原型，核心键盘/语义/双语从 M4 实测 |
| 配置或协议演进破坏用户状态 | 高 | 版本、历史 fixture、服务端回执、原子保存与回退；跨仓库契约测试 |
| 锁屏/挂起/捕获竞态 | 高 | 合成器锁状态、可信认证、拒绝/撤销、锁 UI 崩溃和新增输出测试 |
| 验收门槛写在文档但未执行 | 高 | 场景 ID → 实际测试/job/硬件 Issue 映射；未执行/阻塞不算通过 |

---

## 15. 许可证

本项目采用 **BSD 3-Clause** 许可证。仓库根目录包含 `LICENSE` 文件，版权归 Inox Desktop 贡献者所有；工作区内各 crate 不再各自复制一份。

```
BSD 3-Clause License

Copyright (c) 2026, Inox Desktop Contributors
All rights reserved.
...
```

依赖许可证白名单与校验由 `cargo-deny`（§6、§10）强制执行。

---

## 16. 附录

### 16.1 参考项目

- **Plasma Desktop**：参考本地固定快照中的真实 GUI/状态/迁移测试，边界与取舍见[对照分析](docs/quality/plasma-reference.md)，不将其历史设计草案当成当前保证。

- **COSMIC**：System76 的 Rust 桌面环境，验证了 Smithay + Iced 的可行性。
- **Niri**：基于 Smithay 的滚动平铺合成器，展示了异步与全局状态管理。
- **xfwl4**：Xfce 的 Wayland 合成器，基于 Smithay，开发策略值得借鉴。
- **Smithay**：合成器库，提供底层抽象。
- **waydriver**：Wayland GUI 测试工具。

### 16.2 相关链接

- 组织：https://github.com/Inox-desktop
- Smithay：https://github.com/Smithay/smithay
- Iced：https://github.com/iced-rs/iced
- xwayland-satellite：https://github.com/Supreeeme/xwayland-satellite

### 16.3 术语表

| 术语 | 含义 |
|------|------|
| Compositor | Wayland 合成器，负责输入、渲染与窗口管理 |
| Shell | 桌面外壳，包含面板、启动器、通知等 UI |
| Session | 会话，负责启动合成器、Shell 与系统服务 |
| Layer | 架构分层（L1–L4），见 §2 |
| ADR | Architecture Decision Record，架构决策记录 |
| DoD | Definition of Done，验收标准 |
| Headless | 无真实显示设备的运行/测试模式 |
| MSRV | Minimum Supported Rust Version |
| Portal | xdg-desktop-portal，桌面集成标准接口 |

---

**文档结束**
*本 PRD 将随项目演进持续更新。*
