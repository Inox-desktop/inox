# ADR 0003: 技术栈（Rust 2024 + Smithay + calloop + Iced）

- 状态：accepted
- 日期：2026-10-16
- 决策者：架构组

## 背景

Inox 需要一次性确定语言、合成器框架、事件循环、GUI 工具包、配置格式与可观测性方案。这些选择互相关联（例如合成器框架决定事件循环模型，GUI 工具包决定 Shell 的整体架构），且都属于 PRD §5.4 与根级 `AGENTS.md` §4 中「更换核心依赖必须先写 ADR」的范畴 —— 因此必须在 M0 阶段以 ADR 固定，避免后续里程碑反复摇摆。

PRD §4 已给出完整技术栈表；PRD §16.1 列出参考项目（COSMIC、Niri、xfwl4 均基于 Smithay），用于证明所选组合在真实项目中的可行性。PRD §8 给出性能预算（输入到屏幕延迟 < 20 ms P95、合成帧 CPU < 8 ms），PRD §9、§11 分别要求 TOML 配置与 Fluent 国际化。根级 `AGENTS.md` §4 则给出架构级硬性要求：必须以 Smithay 抽象实现合成器、必须使用 calloop 作为主事件循环、Shell 组件必须基于 Iced 并遵循 Elm 架构、禁止在组件中硬编码颜色/尺寸。

考虑过但未被采纳的方案：

- **自研合成器（直接基于 `libwayland` / `wayland-rs` 手写后端）**：可控性最高，但 DRM/KMS、libinput、渲染器、协议实现全部需要自建，与 PRD §1.1「最大化利用 Rust 生态中成熟的合成器库」直接冲突，且远超 AI-first 模式下的合理工作量。
- **wlroots（C 库）绑定**：生态成熟，但引入大量 FFI 与 GPL-adjacent 依赖风险，与根级 `AGENTS.md` §3.2（默认禁止 unsafe、例外最小化）及 §3.6 的许可证白名单方向不符。
- **GTK4 / Qt 作为 Shell 工具包**：与合成器（Rust）语言栈割裂，难以复用 L1 类型，且 Elm 式单向数据流不成立，违反根级 `AGENTS.md` §4。
- **JSON / YAML 配置**：PRD §9 要求「人类可读、AI 友好、可携带注释」，TOML 更贴合；且 serde + `#[serde(default)]` 的默认值语义已由根级 `AGENTS.md` §7.1 固定。

## 决策

采用 PRD §4 规定的技术栈，具体如下：

| 类别 | 技术 | 约束 |
|------|------|------|
| 语言 | **Rust stable，edition 2024** | 每个仓库用 `rust-toolchain.toml` 固定 `channel = "stable"`，包含 `rustfmt`、`clippy` |
| MSRV | 与 edition 2024 对齐（`rust-version = "1.85"`，M0 骨架统一） | 升 MSRV 需说明破坏性影响（根级 `AGENTS.md` §8） |
| 合成器框架 | **Smithay** | 必须使用其抽象实现合成器；直接操作 DRM/KMS 之外的底层接口需 ADR 批准 |
| 事件循环 | **calloop** | 作为唯一主事件循环；禁止自建线程轮询循环处理核心事件；耗时任务放入 calloop 的异步/线程机制 |
| 显示后端 | Smithay `backend_drm` | DRM/KMS 控制 |
| 输入后端 | Smithay `backend_libinput` | 输入设备抽象 |
| 会话/设备访问 | **libseat** | 合成器不以 root 运行（PRD §10） |
| XWayland | **xwayland-satellite** | 无根 XWayland 集成 |
| GUI 工具包 | **Iced**（Elm 架构） | Shell 组件遵循 `Message` → `update` → `view` 单向数据流；只消费主题令牌，禁止硬编码颜色/尺寸 |
| 配置格式 | **TOML + serde** | 所有字段 `#[serde(default)]`，提供完整默认值；解析错误包含文件、行号、字段路径 |
| 日志 | **tracing + tracing-subscriber** | 结构化日志，默认 `info`；禁止 `println!` / `eprintln!`；默认无遥测上传 |
| 错误处理 | **thiserror（库）+ anyhow（应用）** | 禁止吞掉错误（`let _ = ...` 需有明确理由） |
| 测试 | `cargo test` + **proptest** + **waydriver** | 合成器提供 headless 集成测试；Shell 用 waydriver 在虚拟合成器中截图断言 |
| 性能分析 | **Tracy（tracy-client）** | 帧时间与输入延迟证据，用于 PR 中的性能回退说明 |
| 文档站点 | **mdBook**（`inox-docs`，无 Rust crate） | `language = "zh-CN"` |
| 国际化 | **Project Fluent（`fluent-rs`）** | 用户可见字符串禁止硬编码；默认语言 `en-US`，提供 `zh-CN` |
| 代码质量 | **rustfmt + clippy + cargo-deny** | clippy 零警告（CI `-D warnings`）；cargo-deny 强制许可证白名单（BSD/MIT/Apache-2.0/ISC/Zlib，禁止 GPL/AGPL/SSPL） |
| CI/CD | **GitHub Actions** | 唯一的构建与测试验证途径（PRD §5.1、根级 `AGENTS.md` §3.1） |

补充约束：

- 每个仓库的许可证固定为 **BSD 3-Clause**，根目录必须有 `LICENSE`。
- 依赖升级不得随意跨 major；升级需跑通 CI 并在 PR 中说明破坏性影响（根级 `AGENTS.md` §8）。
- 跨组件通信优先使用标准 Wayland 协议或 D-Bus；Inox 私有通信走定义于 `inox-protocols` 的 `inox-ipc`，且为 unstable，破坏性变更需版本化并更新 PRD §2.3 的协议矩阵。
- 更换上述任一**核心依赖**（Smithay、calloop、Iced、wayland-rs、serde/TOML、tracing、Iced 的 Elm 架构约定）必须先写 ADR 并获得人类批准。

## 后果

正面：

- Smithay + calloop + Iced 的组合已在 COSMIC、Niri、xfwl4 等真实项目中被验证（PRD §16.1），降低了「选型不可行」的风险。
- 全 Rust 栈使 L1 类型、错误与配置模型可被所有层直接复用，无需跨语言桥接。
- calloop 的事件循环模型与 Smithay 后端天然对齐，避免自建线程/轮询带来的竞态与延迟。
- TOML + serde default 语义使配置缺失不报错，配合 SIGHUP / `inoxctl reload` 热重载满足 PRD §9.1。
- tracing 结构化日志与 Tracy 性能证据使 PRD §8 的预算可被测量而非猜测。
- 单一许可证白名单由 cargo-deny 强制，PRD §10 的依赖安全要求可自动化。

负面：

- Smithay 与 Iced 的 API 尚在演进，edition 2024 + MSRV 1.85 会限制部分较老生态 crate 的使用；升级成本集中在合成器与 Shell 两个仓库。
- Iced 的 Elm 架构要求全量状态提升到单一 `update` 循环，复杂 UI（设置应用）容易出现巨型消息枚举，需要靠模块拆分控制。
- calloop 的回调模型使「跨阶段共享可变状态」需要额外设计（`Rc<RefCell<..>>` / 显式状态机），对新贡献者的学习曲线较陡。
- 强制 `tracing` + 禁止 `println!` 意味着示例与调试代码也必须遵守（测试除外），初期会有摩擦。
- 选择 Smithay 意味着合成器的后端能力受上游进度约束；需要自行补齐的部分（如 `ext-session-lock-v1`、`wlr-output-management`，PRD §2.3 中标记为「计划」）必须自行实现。

影响面：本决策是所有仓库的 `Cargo.toml`、`deny.toml`、`rust-toolchain.toml` 与 `AGENTS.md` 的直接来源；M1（Smithay headless 输出）与 M4（Iced Shell）分别是本决策在合成器侧与 UI 侧的首次实证（PRD §13）。

取代条件：若 Smithay 或 Iced 出现无法绕过的阻塞（例如 upstream 停滞导致核心协议无法实现、或性能预算无法达成），需撰写取代本 ADR 的新 ADR，并同步更新 PRD §4、根级 `AGENTS.md` §4 与全部仓库的 `AGENTS.md`。
