# ADR 0007: XWayland 集成采用监督外部 xwayland-satellite 进程（不链接其 crate）

- 状态：proposed
- 日期：2026-10-16
- 决策者：架构组

## 背景

PRD §13 把 M3 的验收标准（DoD）定义为「headless 中启动并映射一个 X11 应用；剪贴板双向同步；拖放端到端通过」，涉及仓库为 `inox-xwayland` 与 `inox-compositor`。PRD §3.1 把 `inox-xwayland` 的职责表述为「XWayland 集成（基于 xwayland-satellite）」，主要依赖列写作 `inox-compositor`；PRD §4 的技术栈把 XWayland 一行固定为 `xwayland-satellite`（无根 XWayland 集成）。ADR 0004 的排期表规定 `xwayland-satellite` 在 M3 引入。

`inox-xwayland` 被定义为 L2 服务，其第一件事是把无根 XWayland 接入一个 Inox 会话。上游 `xwayland-satellite`（Supreeeme/xwayland-satellite）是一个**独立可执行程序**：它作为 Wayland 客户端连接宿主合成器，对宿主要求实现 `xdg_wm_base` 与 `wp_viewporter`（上游 README 明示的宿主前提），随后由它自行 spawn `Xwayland` 进程并对外提供 X11 显示（默认 `:N`，套接字位于 `/tmp/.X11-unix/X<N>`）。

约束与依据：

- 根级 `AGENTS.md` §2：合成器与 Shell（以及各服务）**不通过 Rust 依赖耦合，只通过 Wayland 协议 / D-Bus / IPC 通信**。`inox-xwayland` 作为 L2 服务同样应与被集成的组件保持进程边界，避免把「服务」塌缩成「合成器的一个模块」。
- 根级 `AGENTS.md` §3.6：依赖许可证白名单**仅允许 BSD / MIT / Apache-2.0 / ISC / Zlib 等宽松许可证，禁止 GPL/AGPL/SSPL** 进入核心依赖，由 `cargo-deny` 强制。`xwayland-satellite` 采用 **MPL-2.0**（文件级弱 copyleft），**不在白名单内**；把它作为构建期依赖会让 `cargo deny check` 失败，除非为其开特例。
- 根级 `AGENTS.md` §4：必须使用 calloop 作为主事件循环，禁止自建线程轮询循环。
- 根级 `AGENTS.md` §3.1：不在本地编译/测试，验收以 CI 为准；因此引入任何新依赖都必须让 CI 的 `cargo-deny` 与构建链路保持可通过。

`xwayland-satellite` 的工程形态使得「把它作为库链接」不可行：

- **许可证**：MPL-2.0 属弱 copyleft，列在根级 `AGENTS.md` §3.6 的白名单之外；作为 crate 依赖会污染依赖图并触发 `cargo-deny` 拒绝。
- **形态**：它是**应用程序**（含 `main`、进程生命周期与状态机），并非为嵌入而设计的库；其对外暴露的 `lib` API 面向内部使用，是**会 panic 的内部实现**，不构成稳定的库接口。
- **分发**：它**未发布到 crates.io**（二进制通过源码构建或发行版打包获得）。作为依赖只能使用指向第三方 GitHub 的 `git` 依赖，引入一个不受本项目控制的来源，与「跨仓库依赖锁定 `tag`/`rev`、来源受控」的取向相悖。
- **耦合风险**：嵌入其内部 API 意味着把 `inox-xwayland` 的命运绑定到上游未承诺稳定的内部实现，任何上游重构都会直接破坏本仓库。

考虑过但未被采纳的方案：

- **链接 `xwayland-satellite` crate（或 vendored 源码）**：触发 MPL-2.0 许可证问题、引入第三方 git 源、并耦合其不稳定的内部 API —— 不成立。
- **把 XWayland 集成直接做进 `inox-compositor`**：会让合成器同时承担 X11 集成职责，违反 PRD §3.1 的仓库职责划分与根级 `AGENTS.md` §2 的进程/IPC 边界 —— 不成立。
- **在 M1 直接对 `inox-compositor` 做端到端验证**：`inox-compositor` 当前只广告 headless 渲染所需的协议，**尚未广告 `xdg_wm_base` 与 `wp_viewporter`**，无法满足 `xwayland-satellite` 的宿主前提 —— 见「决策」第 4 条。
- **自行实现一个无根 XWayland 集成层（不依赖 xwayland-satellite）**：等于重写上游成熟组件，工作量与风险远超收益，且 PRD §4 已固定技术选型 —— 不成立。

## 决策

1. **`inox-xwayland` 以「监督外部 `xwayland-satellite` 进程」的方式集成无根 XWayland，而不是链接其 crate。** `xwayland-satellite`（连同它自行 spawn 的 `Xwayland`）被当作**运行时系统依赖**对待，地位与 `Xwayland` 本身相同：由会话/发行版在运行时提供，`inox-xwayland` 只负责在会话中启动、观测与终止它。这与 niri 等合成器消费 `xwayland-satellite` 的方式一致。
2. **服务形态为独立的 L2 二进制/服务**：`inox-xwayland` 与 `inox-compositor` **没有构建期耦合**，二者只通过 Wayland 协议（宿主合成器侧）与进程边界协作。仓库职责与 PRD §3.1 的划分一致，依赖图保持干净，`cargo-deny` 无需为 MPL-2.0 或第三方 git 源开特例即可通过。
3. **M1 范围**：启动指定显示（默认 `:1`）的 satellite 并让它 spawn `Xwayland`；通过探测 `/tmp/.X11-unix/X<N>` 套接字判断 Xwayland 是否已就绪（就绪后记日志并可写可选的就绪文件）；监督子进程（检测意外退出、转发终止信号）；收到 `SIGINT`/`SIGTERM` 后终止子进程并以**退出码 0** 干净退出。事件循环使用 calloop（依据根级 `AGENTS.md` §4）。
4. **M1 的端到端验证针对第三方 headless 合成器**：由于 `xwayland-satellite` 要求宿主实现 `xdg_wm_base` 与 `wp_viewporter`，而 `inox-compositor` 在 M1 阶段尚未广告这两个协议，M1 的端到端测试在 **weston `--backend=headless`**（禁用 weston 自带的 Xwayland）上运行 `inox-xwayland`，并映射一个真实的 X11 应用（如 `xeyes`）以验证链路。**对 `inox-compositor` 的完整集成推迟到其广告上述协议之后。**
5. **新增依赖**：`calloop`（主事件循环，features `["signals"]`，依据根级 `AGENTS.md` §4）、`rustix`（发送 `SIGTERM` 等进程信号，features `["process"]`）、`inox-common`（统一 `tracing` 日志初始化，跨仓库 git 依赖**锁定 `tag = "v0.1.0"`**，依据根级 `AGENTS.md` §8），外加既有的 `anyhow`/`thiserror`/`tracing`。跨仓库 git 源在 `deny.toml` 的 `[sources] allow-git` 登记。**不引入** `xwayland-satellite` 的任何 crate 或 git 依赖。
6. **MSRV 提升为 `1.86`**：`inox-xwayland` 的 `rust-version` 设为 `1.86`（与 `inox-compositor` 在 ADR 0005 中的取值对齐），因为 calloop 0.14 要求 rustc ≥ 1.86。依据根级 `AGENTS.md` §8（升级需说明破坏性影响）。
7. **`unsafe` 策略**：`inox-xwayland` 的 `Cargo.toml` 保持 `[lints.rust] unsafe_code = "forbid"`。M1 的全部实现（`std::process` + `rustix` 安全 API）**不需要 `unsafe`**；根级 `AGENTS.md` §3.2 对 `inox-xwayland` 的 FFI 例外在本里程碑**不被使用**。

## 后果

正面：

- **依赖图保持干净**：`inox-xwayland` 不引入 MPL-2.0 依赖或第三方 git 源，`cargo-deny` 的许可证白名单**无需修改**即可通过，延续 ADR 0004 对依赖治理风险的收敛取向。
- **边界清晰**：集成以进程 + 协议为界，符合根级 `AGENTS.md` §2 与 PRD §3.1 的仓库职责划分；satellite 崩溃或升级与 `inox-xwayland` 的构建解耦。
- **紧跟上游**：直接消费上游发布的 `xwayland-satellite`（版本由发行版或其自身发布流程决定），可独立升级，无需等待本项目同步 vendored 源码。
- **M1 可在 CI 复现**：进程级生命周期测试用一个 stand-in satellite（shell 脚本）确定性地覆盖「就绪 / 超时 / 子进程早退 / 信号终止 → 退出码 0」；端到端测试在 headless weston 上映射真实 X11 应用，二者都不依赖 GPU。
- 就绪判定基于**实际探测 X11 套接字**，而非仅凭日志推断，使「Xwayland 可用」在 CI 中可机器验证。

负面：

- **运行时依赖**：`xwayland-satellite` 必须由会话/发行版在运行时提供；本仓库的测试与部署需显式满足这一前置条件（CI 通过源码构建并以 `--satellite <path>` 指定）。
- **与 `inox-compositor` 的完整集成被推迟**：M1 只在第三方 headless 合成器上验证；在 `inox-compositor` 广告 `xdg_wm_base` 与 `wp_viewporter` 之前，真实 Inox 会话上的 XWayland 链路尚未被覆盖。
- **进程边界带来额外职责**：就绪探测、超时、子进程监督与信号转发需由 `inox-xwayland` 自行实现并测试，而非由库 API 内部完成。
- **功能不完整**：PRD M3 的其余部分 —— `-listenfd` 按需激活、剪贴板双向同步、拖放 —— **不在 M1 范围内**，推迟到后续里程碑。
- PRD §3.1 表中 `inox-xwayland` 的「主要依赖」列仍写作 `inox-compositor`，与本 ADR 的「无构建期耦合」表述不一致；该列为目标形态描述，按本决策应理解为运行时/协议协作关系，而非 Cargo 依赖。

影响面：本决策直接作用于 PRD §13 的 M3 行（DoD 文本不变，仅明确 `inox-xwayland` 侧的实现与验证路径）与 §3.1 / §4 中 `inox-xwayland` 的依赖描述，并决定 `inox-xwayland` 的 `Cargo.toml`（`calloop`/`rustix`/`inox-common` git 依赖、MSRV `1.86`、`unsafe_code = "forbid"`）、`deny.toml`（`[sources] allow-git` 登记 `inox-common` 来源）与仓库级 `AGENTS.md`（仓库特有约束），以及其 CI（新增一个 headless 端到端 job：安装 weston / xwayland / x11-apps 并构建上游 satellite）。M2 仍聚焦合成器输入与窗口管理；M3 将在本 ADR 建立的进程边界上补齐剪贴板与拖放。

取代条件：若 `inox-compositor` 广告 `xdg_wm_base` 与 `wp_viewporter` 并采用 `-listenfd` 做**进程内按需激活**，使「外部监督」不再必要；或上游出现一个**可作库使用且许可证宽松**的 satellite API —— 需撰写取代本 ADR 的新 ADR，并同步更新 PRD §13 的 M3 行、`inox-xwayland` 的 `Cargo.toml` / `deny.toml` / `AGENTS.md` 与 CI 工作流。
