# Inox Desktop 产品需求文档（PRD）

**版本**：1.1
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

---

## 1. 项目概述

**Inox** 是一个用 Rust 编写的现代 Wayland 桌面环境。名称取自“不锈钢”（Inox），与 Rust 的“锈”形成反差，寓意坚固、耐腐蚀、长期可维护。项目采用 **AI-first 开发模式**：AI 生成绝大部分代码，人类负责产品决策、架构验收与硬件测试。所有编译、测试、lint 均在 GitHub Actions 中完成，不在本地进行编译验证。

### 1.1 目标

- 构建一个模块化、可维护、高性能的 Wayland 桌面环境。
- 最大化利用 Rust 生态中成熟的合成器库与 GUI 框架。
- 通过多仓库拆分实现清晰的模块边界，便于 AI 独立开发与测试。
- 建立以 GitHub Actions 为核心的持续集成流水线，作为唯一的构建验证途径。
- 采用 BSD 3-Clause 许可证，保持开源友好。

### 1.2 非目标

- 不在项目初期支持所有 Wayland 协议或 X11 全部特性。
- 不追求与 GNOME/KDE 功能对等，优先保证核心体验稳定。
- 不进行本地编译验证；开发者与 AI 均依赖 CI 反馈。
- 不商业化，暂不进行商标注册（但需注意 INOX 商标风险）。
- 1.0 之前不提供完整的无障碍（AT-SPI 全量）、多语言与主题市场能力，仅保留可扩展接口。

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
│  inox-common, inox-config, inox-docs         │
└─────────────────────────────────────────────┘
```

- **Layer 1**：提供配置解析、通用类型、日志、工具函数，被所有上层依赖。
- **Layer 2**：核心 Wayland 合成器，基于 Smithay，处理 DRM/KMS、输入、渲染、窗口管理、XWayland 集成。
- **Layer 3**：桌面 Shell，使用 Iced 构建面板、启动器、通知、设置等用户界面组件。
- **Layer 4**：会话管理、显示管理器、空闲管理等系统级服务。

### 2.1 依赖方向约束

- 依赖只能向下：`L4 → L3 → L2 → L1`，禁止反向或跨层横向依赖。
- `inox-common` 不得依赖任何其他 Inox crate（叶子节点）。
- `inox-compositor` 与 `inox-shell` 之间**不通过 Rust 依赖**耦合，而是通过 Wayland 协议 / IPC 通信，保证可独立运行与测试。
- 循环依赖被 CI 与 `cargo-deny` / `cargo tree` 检查拦截。

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
| `wl_data_device` | 必须 | 剪贴板与拖放 |
| `xdg-shell` | 必须 | 标准窗口 |
| `xdg-decoration` | 必须 | 服务端/客户端装饰协商 |
| `wlr-layer-shell-unstable-v1` | 必须 | 面板/壁纸/锁屏层 |
| `viewporter` | 必须 | 缩放与裁剪 |
| `linux-dmabuf` | 必须 | GPU 缓冲零拷贝 |
| `presentation-time` | 必须 | 帧同步反馈 |
| `idle-inhibit` / `ext-idle-notify-v1` | 必须 | 空闲管理 |
| `pointer-constraints` / `relative-pointer` | 计划 | 游戏与 3D 应用 |
| `xfractional-scale-v1` | 计划 | 分数缩放 |
| `wlr-output-management` | 计划 | 显示器管理 |
| `wlr-screencopy` / `ext-image-copy-capture` | 计划 | 截图与录屏 |
| `ext-session-lock-v1` | 计划 | 锁屏 |
| `gtk-primary-selection` | 计划 | X11 主选区兼容 |
| `xwayland-shell` | 计划 | XWayland 集成 |
| `wlr-virtual-keyboard` / `input-method-v2` | 仅规划 | 输入法（1.0 之后） |

> 新增协议必须先在 `inox-protocols` 定义或引用，并说明安全与生命周期约束；破坏性变更需 ADR。

---

## 3. 仓库架构

所有仓库位于 GitHub 组织 `Inox-desktop` 下，采用多仓库策略，每个仓库独立版本、独立 CI。跨仓库依赖通过 Git 依赖（指定 tag 或 rev）管理。

### 3.1 仓库列表

| 仓库名 | 层级 | 职责 | 主要依赖 | 发布物 |
|--------|------|------|----------|--------|
| `.github` | 组织 | 组织级工作流模板、ISSUE/PR 模板、CONTRIBUTING | — | — |
| `inox` | 元仓库 | 项目总览、路线图、PRD、ADR、集成测试入口、文档索引 | 所有子仓库 | 文档 |
| `inox-common` | L1 | 共享类型、错误处理、日志、工具函数 | — | crate |
| `inox-config` | L1 | 配置模型、TOML 解析、默认配置 | `inox-common`, serde | crate |
| `inox-docs` | L1 | 用户与开发者文档 | — | 文档站点 |
| `inox-compositor` | L2 | Wayland 合成器，基于 Smithay | `inox-common`, `inox-config`, `inox-protocols` | 二进制 `inox-comp` |
| `inox-protocols` | L2 | 标准协议绑定与自定义 Wayland 协议扩展 | `wayland-rs` | crate |
| `inox-xwayland` | L2 | XWayland 集成（基于 xwayland-satellite） | `inox-compositor` | 二进制/服务 |
| `inox-shell` | L3 | Shell 主框架，插件系统 | `inox-common`, `inox-config`, Iced | 二进制 `inox-shell` |
| `inox-panel` | L3 | 面板（任务栏、系统托盘、时钟） | `inox-shell` | 二进制 `inox-panel` |
| `inox-launcher` | L3 | 应用启动器 | `inox-shell` | 二进制 `inox-launcher` |
| `inox-notifications` | L3 | 通知守护进程 | `inox-shell` | 二进制 `inox-notifications` |
| `inox-settings` | L3 | 系统设置应用 | `inox-shell` | 二进制 `inox-settings` |
| `inox-lock` | L3 | 锁屏 | `inox-shell` | 二进制 `inox-lock` |
| `inox-wallpaper` | L3 | 壁纸设置与渲染 | `inox-compositor` | 二进制 `inox-wallpaper` |
| `inox-session` | L4 | 会话管理器，启动合成器与 Shell | `inox-compositor`, `inox-shell` | 二进制 `inox-session` |
| `inox-idle` | L4 | 空闲管理 | `inox-session` | 二进制 `inox-idle` |
| `inox-greet` | L4 | 显示管理器（可选） | `inox-session` | 二进制 `inox-greet` |
| `inox-integration` | 测试 | 端到端集成测试，聚合所有组件 | 所有仓库 | 测试报告 |

**规划中的仓库**（1.0 之后或按需提前）：

| 仓库名 | 层级 | 职责 |
|--------|------|------|
| `inox-portal` | L3/L4 | `xdg-desktop-portal` 后端（截图、文件选择、屏幕共享） |
| `inox-screenshot` | L3 | 截图/录屏前端工具 |
| `inox-osd` | L3 | 音量/亮度等屏幕提示 |
| `inox-cli` | L1 | 统一的 `inoxctl` 命令行控制工具（IPC 客户端） |

### 3.2 依赖管理

- 每个仓库独立 `Cargo.toml`，版本遵循 SemVer。
- 跨仓库依赖使用 Git 依赖，锁定 `tag` 或 `rev`，例如：
  ```toml
  [dependencies]
  inox-config = { git = "https://github.com/Inox-desktop/inox-config", tag = "v0.1.0" }
  ```
- `inox-integration` 仓库通过 Git 依赖拉取所有组件，构建完整系统并运行端到端测试。
- AI 代理可自动更新依赖版本并创建 PR。

### 3.3 分支与发布策略

- 主分支：`main`，保护分支，要求 CI 通过。
- 功能分支：`feat/xxx`、`fix/xxx`、`chore/xxx`、`docs/xxx`。
- 发布：使用 `cargo-release` 打 tag，格式 `vX.Y.Z`。
- 每个仓库独立发布，集成仓库定期发布元版本。
- 版本兼容：集成仓库维护一份组件兼容矩阵（`inox` 元仓库 `compat.toml`）。

### 3.4 典型仓库目录结构

```
<repo>/
├── AGENTS.md            # 仓库级 AI 约束（继承并覆盖根级模板）
├── Cargo.toml
├── LICENSE              # BSD 3-Clause
├── README.md
├── rust-toolchain.toml  # 固定 stable
├── deny.toml            # cargo-deny 配置
├── .github/
│   └── workflows/ci.yml
├── src/
│   ├── lib.rs / main.rs
│   └── ...
└── tests/               # 集成测试
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
| 许可证 | BSD 3-Clause | 每个仓库包含 LICENSE |

---

## 5. 开发流程与 AI 协作

### 5.1 核心原则

- **AI 生成代码，人类验收**：AI 负责实现、测试、文档、重构；人类负责架构决策、PR 审查、硬件验收。
- **CI 是唯一真相**：不在本地编译或运行测试。所有验证通过 GitHub Actions 完成。
- **小步提交**：每个 PR 聚焦单一模块，便于 AI 理解和 CI 快速反馈。

### 5.2 AI 代理工作流

1. 人类编写任务描述（Issue 或 PRD 片段）。
2. AI 代理读取 `AGENTS.md`（每个仓库根目录），了解架构、命令、风格、禁止事项。
3. AI 生成代码，推送到功能分支，创建 PR。
4. GitHub Actions 自动运行：fmt、clippy、test、build、doc、deny。
5. AI 读取 CI 日志，自动修复错误，更新 PR。
6. 人类审查架构合理性与关键逻辑，批准合并。
7. 合并到 `main` 后，触发发布流程（可选）。

### 5.3 AGENTS.md 规范

每个仓库必须包含 `AGENTS.md`，内容至少包括：

- 项目简介与职责
- 架构约束（如“禁止 unsafe”“必须使用 Smithay 抽象”）
- 本地命令（但强调不在本地运行，仅作参考）
- CI 命令与预期
- 代码风格
- 测试要求
- 依赖管理规则
- 提交信息格式（Conventional Commits）

根级 `AGENTS.md` 见仓库根目录；各子仓库的 `AGENTS.md` 以根级为基础，仅补充仓库特有的约束，不得放宽根级的硬性规则。

### 5.4 变更管理与 ADR

- 架构决策记录（ADR）存放于元仓库 `docs/adr/NNNN-title.md`，采用 MADR 精简格式。
- 以下情况**必须**先写 ADR 并获得批准：新增/删除仓库、改变分层或依赖方向、引入新协议策略、更换核心依赖（Smithay/Iced/calloop）、改变 IPC 或配置格式的破坏性变更。
- ADR 状态：`proposed` → `accepted` → `superseded`。

### 5.5 人类验收职责

人类负责且仅负责以下不可委派的工作：

- 架构合理性与边界划分的最终判断。
- 真实硬件上的功能验收（GPU 兼容、输入延迟、多显示器、XWayland 应用）。
- 许可证/商标/合规决策。
- 合并批准。

---

## 6. CI/CD 策略

### 6.1 每个仓库的 CI 工作流

触发条件：`push` 到 `main`、`pull_request`。

步骤：

1. **Checkout**：`actions/checkout@v4`
2. **Rust 工具链**：`dtolnay/rust-toolchain@stable`，组件 `rustfmt`, `clippy`
3. **缓存**：`Swatinem/rust-cache@v2`
4. **格式检查**：`cargo fmt --all -- --check`
5. **Clippy**：`cargo clippy --all-targets --all-features -- -D warnings`
6. **测试**：`cargo test --all-features`
7. **构建**：`cargo build --release --all-features`
8. **文档**：`cargo doc --no-deps --all-features`
9. **依赖检查**：`cargo deny check`
10. **模块特定测试**：
    - 合成器：运行 headless 集成测试，使用软件渲染（`LIBGL_ALWAYS_SOFTWARE=1`），验证输出创建、渲染、输入事件。
    - Shell：使用 `waydriver` 在虚拟 Wayland 合成器（如 `weston --backend=headless`）中启动应用，截图并验证。
    - 其他：按需。

### 6.2 CI 门禁规则

- 任一检查失败即阻止合并；不提供 `--no-verify` 等绕过手段。
- PR 必须通过分支保护规则：至少 1 名人类审查、CI 全绿、无未解决评论。
- 覆盖率阈值由各仓库在 `AGENTS.md` 中声明（默认对核心 crate 行覆盖 ≥ 70%）。

### 6.3 集成测试

`inox-integration` 仓库负责端到端测试：

- 通过 Git 依赖拉取所有组件。
- 在 GitHub Actions 中启动 `inox-session`（headless 后端 + 软件渲染）。
- 运行 `waydriver` 测试套件，模拟用户操作，验证窗口管理、面板、启动器等功能。
- 生成测试报告与截图作为 artifact。

### 6.4 硬件验收

- CI 无法替代真实硬件测试。
- 人类定期在真实设备上运行最新构建，验证 GPU 兼容性、输入延迟、多显示器、XWayland 应用。
- 验收结果以 Issue 形式记录。

### 6.5 发布流程

- 每个仓库通过 `cargo-release` 打 tag，触发 GitHub Release。
- 集成仓库发布元版本，包含所有组件的兼容版本矩阵。

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

---

## 8. 非功能需求与性能预算

| 指标 | 目标 | 测量方式 |
|------|------|----------|
| 输入到屏幕延迟 | < 20 ms（P95） | Tracy / 高帧率录制 |
| 合成帧 CPU 预算 | < 8 ms（60 Hz，16.6 ms 帧预算的 50%） | Tracy |
| 空闲 CPU 占用 | < 1%（单核） | `top`/`pidstat` 采样 |
| 完整会话空闲内存 | < 300 MB RSS | 硬件测试记录 |
| 冷启动到可用会话 | < 3 s | 会话日志时间戳 |
| 崩溃恢复 | 合成器崩溃后重新拉起并恢复窗口 | 会话管理器测试 |
| 日志开销 | 默认 `info` 级，无高频刷屏 | 代码审查 |

- 任何 PR 若引入可测量的性能回退，需在 PR 中说明并提供 Tracy 证据。
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

### 9.2 主题系统

- 主题由 `theme.toml` 定义设计令牌（design tokens）：颜色、排版、间距、圆角、动效时长。
- Iced 组件只消费令牌，不得硬编码颜色/尺寸。
- 支持浅色/深色与跟随系统；默认提供 `inox-light`、`inox-dark`。

---

## 10. 安全、权限与 Portal

- **权限模型**：合成器不以 root 运行，设备访问通过 `libseat`。所有客户端按 Wayland 安全模型隔离，无全局输入注入给未授权客户端。
- **剪贴板/拖放**：仅在获得焦点的客户端间传递，遵守 `wl_data_device` 语义。
- **锁屏**：使用 `ext-session-lock-v1`，锁屏期间隔离普通客户端。
- **Portal**：通过 `inox-portal` 提供 `xdg-desktop-portal` 后端，覆盖截图、屏幕共享、文件选择、设置；前端应用通过标准 Portal 接口访问，避免合成器直接暴露特权接口。
- **依赖安全**：`cargo-deny` 强制许可证白名单（BSD/MIT/Apache-2.0/ISC/Zlib 等），禁止 GPL/AGPL 依赖进入核心；定期扫描 RUSTSEC 漏洞。
- **禁止项**：不得引入远程代码执行、不得默认开启网络监听、不得上传遥测。

---

## 11. 无障碍与国际化

### 11.1 无障碍（Accessibility）

- 目标：键盘可完成主要操作（启动器、面板、设置、锁屏）。
- Iced 组件暴露语义树，经由 `inox-shell` 桥接到 AT-SPI（D-Bus）供屏幕阅读器读取（1.0 之后完整支持，接口先行）。
- 尊重系统字体缩放与高对比度主题令牌。
- 支持屏幕放大所需的 `wlr-screencopy`/`ext-image-copy-capture`（计划）。

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

| 里程碑 | 目标 | 涉及仓库 | 验收标准（DoD） |
|--------|------|----------|-----------------|
| M0 | 组织与仓库初始化，CI 模板，AGENTS.md，LICENSE | `.github`, 所有仓库骨架 | 所有仓库含骨架、`AGENTS.md`、`LICENSE`、CI；空提交 CI 全绿 |
| M1 | Headless 合成器：创建输出、渲染纯色、干净退出 | `inox-compositor` | CI 中 headless 创建输出并渲染帧，收到终止信号后退出码 0，clippy 无警告 |
| M2 | 输入事件、多窗口、平铺布局 | `inox-compositor` | 处理键盘/指针事件；管理 ≥2 窗口；平铺布局通过 proptest |
| M3 | XWayland 集成、剪贴板、拖放 | `inox-xwayland`, `inox-compositor` | headless 中启动并映射一个 X11 应用；剪贴板双向同步；拖放端到端通过 |
| M4 | Iced Shell：面板、启动器、设置 | `inox-shell`, `inox-panel`, `inox-launcher`, `inox-settings` | 面板显示时钟/托盘；启动器可搜索并启动应用；设置读写并热重载；waydriver 截图验证通过 |
| M5 | 会话管理、锁屏、通知、日常可用 | `inox-session`, `inox-lock`, `inox-notifications` | `inox-session` 拉起完整会话；锁屏可解锁；通知弹出与超时正确 |
| M6 | 集成测试、性能优化、打包 | `inox-integration`, 所有 | 端到端测试全绿；满足第 8 节性能预算；产出至少 2 种分发物 |

---

## 14. 风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| AI 生成代码质量不稳定 | 高 | CI 严格检查，人类审查关键模块，AGENTS.md 约束 |
| 跨仓库依赖管理复杂 | 中 | 使用 Git tag 锁定版本，集成仓库统一测试，维护兼容矩阵 |
| GitHub Actions 无 GPU | 中 | 使用软件渲染（llvmpipe），headless 后端 |
| Wayland 协议兼容性 | 中 | 优先支持核心协议，逐步扩展，参考 Smithay 示例 |
| INOX 商标冲突 | 低 | 开源项目暂不商业化，必要时更名 |
| 本地无法编译导致调试困难 | 中 | 依赖 CI 日志，AI 自动修复，人类仅在硬件验收时本地构建 |
| 性能回退被忽视 | 中 | 第 8 节预算纳入 PR 检查，Tracy 证据要求 |
| 无障碍/i18n 后期返工 | 中 | 接口先行，文案/语义树从 M4 起强制规范 |

---

## 15. 许可证

所有仓库采用 **BSD 3-Clause** 许可证。每个仓库根目录包含 `LICENSE` 文件，版权归 Inox Desktop 贡献者所有。

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
