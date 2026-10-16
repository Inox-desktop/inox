# ADR 0005: M1 的 headless 渲染路径（自建 surfaceless EGL 与离屏渲染目标）

- 状态：accepted
- 日期：2026-10-16
- 决策者：架构组

## 背景

PRD §13 把 M1 的验收标准（DoD）定义为：「CI 中 headless 创建输出并渲染帧，收到终止信号后退出码 0，clippy 无警告」。ADR 0003 在影响面中把 M1 表述为「Smithay headless 输出」，隐含了一条「使用 Smithay `backend::headless` 后端创建输出并渲染」的路径。该假设需要在本 ADR 中被纠正。

ADR 0003 固定的 Smithay 版本为 **v0.7.0**。经核对 v0.7.0 源码树，**该版本已不存在 `backend::headless` 模块**（旧版本曾提供，上游已移除）。因此 PRD §13 M1 隐含的「使用 Smithay headless 后端」路径**不可用**：M1 无法直接依赖上游提供现成的 headless 输出与渲染后端，必须由 Inox 自行在 Smithay 现有抽象之上搭建。

约束与依据：

- 根级 `AGENTS.md` §4 要求「必须使用 Smithay 抽象实现合成器、必须使用 calloop 作为主事件循环」。自建 headless 路径不得绕过 Smithay 抽象，也不得替换 calloop。
- PRD §6.1 要求 CI 在无显示设备的环境中可复现；PRD §8 的性能预算在 M1 阶段尚无硬件覆盖义务。
- ADR 0004 §3 的排期表规定 Smithay + calloop 在 M1 引入，并规定跨仓库 Git 依赖到「第一个真正需要它的里程碑」才引入，同时需在 `deny.toml` 的 `[sources] allow-git` 登记。
- 根级 `AGENTS.md` §3.2 允许 `inox-compositor` / `inox-xwayland` 的 FFI 胶水代码作为 `unsafe` 的例外，并要求每个 `unsafe` 附 `// SAFETY:` 理由。
- 根级 `AGENTS.md` §8 要求升级 MSRV 时说明破坏性影响。

考虑过但未被采纳的方案：

- **降级 Smithay 到仍有 `backend::headless` 的旧版本**：会偏离 ADR 0003 固定的 v0.7.0，失去上游修复与新协议支持，且旧版本与 edition 2024 / 新版 calloop 的兼容性无保证 —— 不成立。
- **在 CI 中使用虚拟 DRM/KMS 设备（如 vkms）驱动真实输出链路**：需要在 CI 内核/权限层面配置，失败面与控制成本远超 M1 的 DoD 所需，推迟到后续里程碑更合适。
- **不创建 `wl_output`，仅渲染到离屏缓冲**：无法满足「创建输出」的 DoD，也无法验证 `wl_output` 广告链路 —— 不成立。

## 决策

M1 的 headless 渲染路径**由 Inox 自行搭建，全部建立在 Smithay 现有抽象之上**，具体如下：

1. **显示连接**：使用 EGL surfaceless 平台（`EGL_MESA_platform_surfaceless`，对应 Smithay 的 `EGLSurfacelessDisplay`）创建 `EGLDisplay` 与 `EGLContext`，不依赖任何窗口系统或 DRM 设备。
2. **渲染器与目标**：以 `GlesRenderer` 作为渲染器，配一块与输出 mode 同尺寸的**离屏 `GlesRenderbuffer`** 作为渲染目标。
3. **输出**：用 `Output::new` + mode 直接构造 `Output`（**不经 DRM/KMS**），并通过 `OutputManagerState` 向客户端广告 `wl_output`。
4. **帧与断言**：每帧渲染纯色，帧可通过 `ExportMem::copy_framebuffer` 读回像素，用于断言「确实渲染出了内容」。
5. **CI 与 DoD**：合成器不依赖 DRM/KMS 或输入设备即可创建输出并渲染，满足 PRD §13 M1 的 DoD；CI 中使用软件渲染（`LIBGL_ALWAYS_SOFTWARE=1`，llvmpipe），依据 PRD §6.1。渲染是否真的发生由离屏像素读回断言，而非依赖日志推断。
6. **依赖面**：Smithay 采用最小特性集 `default-features = false, features = ["wayland_frontend", "renderer_gl"]`，**不启用** `backend_drm` / `backend_libinput` / `backend_udev` / `backend_session_libseat` / `backend_x11` / `backend_winit` / `backend_vulkan` / `xwayland`。理由：M1 不需要这些后端，且不引入 DRM/libinput/libseat 等系统库依赖，符合 ADR 0004 对 CI 失败面最小化的取向。
7. **事件循环与套接字**：使用 calloop（ADR 0003 已固定）；帧由 calloop 定时器驱动，`SIGINT` / `SIGTERM` 经 calloop 的信号源转为事件循环退出（退出码 0），信号源在监听套接字绑定**之前**注册，以保证已绑定套接字的合成器必然能处理终止信号。Wayland 套接字绑定在 `XDG_RUNTIME_DIR` 下，该变量必须是已存在的绝对路径目录；CI 与集成测试需各自提供私有目录。
8. **`unsafe` 策略**：`inox-compositor` 的 `Cargo.toml` 将 `[lints.rust] unsafe_code = "forbid"` **放宽为 `deny`**（`forbid` 无法局部放宽，`deny` 允许以局部 `#[allow]` 表达「仅允许这几处」），并把 `#[allow(unsafe_code)]` 限定在两处：
   - `src/render.rs`：`EGLDisplay::new(EGLSurfacelessDisplay)` 与 `GlesRenderer::new(context)`（EGL/GL 胶水）；
   - `src/runtime.rs`：从 calloop 的 `Generic` 事件源取得 `Display` 以派发客户端请求（`NoIoDrop::get_mut`）。

   两个 `unsafe` 调用均被包裹进安全 API 并附带 `// SAFETY:` 理由。依据根级 `AGENTS.md` §3.2 对 `inox-compositor` / `inox-xwayland` 的例外。
9. **MSRV**：`inox-compositor` 的 `rust-version` 由 `1.85` 提升到 `1.86`，因为 `smithay` 0.7.0 依赖的 `calloop` 0.14.5 要求 rustc ≥ 1.86。**仅该仓库**提升；`inox-common` / `inox-config` / `inox-protocols` 保持 `1.85`。依据根级 `AGENTS.md` §8（升级需说明破坏性影响）。
10. **跨仓库依赖**：`inox-compositor` 通过锁定 `tag = "v0.1.0"` 的 git 依赖引入 `inox-common`（用于统一的 `tracing` 初始化），依据 ADR 0004 §3 的排期表，并在 `deny.toml` 的 `[sources] allow-git` 中登记来源。

## 后果

正面：

- 在 Smithay v0.7.0 不再提供 headless 后端的现实下，仍能达成 PRD §13 M1 的 DoD：创建 `wl_output`、渲染纯色帧、干净退出（退出码 0）。
- 渲染路径完全建立在 Smithay 既有抽象（`EGLSurfacelessDisplay`、`GlesRenderer`、`Output`、`ExportMem`）之上，不绕过 `AGENTS.md` §4 的「必须使用 Smithay 抽象」约束。
- 离屏 `GlesRenderbuffer` + `copy_framebuffer` 提供了可断言的像素证据，使「确实渲染出了内容」在 CI 中可被机器验证，而非仅凭日志推断。
- 最小特性集（仅 `wayland_frontend` + `renderer_gl`）不引入 DRM/libinput/libseat 系统库，CI 失败面被压到最小，延续 ADR 0004 的取向。
- `unsafe` 被收敛到两处命名函数（`src/render.rs`、`src/runtime.rs`）并附 `// SAFETY:` 理由，FFI 与事件源访问面可控、可审查。

负面：

- headless 渲染路径成为 **Inox 自维护代码**；上游若日后重新提供 headless 后端，需回访本 ADR 决定是否迁移或替换。
- surfaceless EGL **依赖运行环境存在可用的 GL 驱动**（CI 依赖 llvmpipe），**真实 GPU 路径要到后续里程碑才被覆盖**。
- `deny` + 局部 `allow` 比 `forbid` 弱，需要靠代码审查保证 `unsafe` 不扩散。
- 未使用 `backend_drm` 意味着 **M1 不校验真实输出链路**，DRM/KMS 风险推迟到后续里程碑。
- `inox-compositor` 的 MSRV 提升到 1.86，与其余 L1 仓库（1.85）暂时不一致，需要在后者的升级节奏上保持关注。

影响面：本决策直接作用于 PRD §13 的 M1 行（DoD 文本不变，仅实现路径由本 ADR 明确），以及 `inox-compositor` 的 `Cargo.toml`（Smithay 特性集、MSRV、`unsafe_code` lint、`inox-common` git 依赖）、`deny.toml`（`[sources] allow-git`）与 `AGENTS.md`（仓库特有约束），并影响 CI 配置（`LIBGL_ALWAYS_SOFTWARE=1`、llvmpipe）。M2 起将在既有 headless 路径上继续扩展（输入事件、多窗口、平铺布局）。

取代条件：若上游 Smithay 重新提供 headless 后端，或真实 GPU 渲染路径在后续里程碑被正式纳入（例如 CI 改用 vkms 或硬件设备），需撰写取代本 ADR 的新 ADR，并同步更新 PRD §13 的 M1 行、`inox-compositor` 的 `Cargo.toml` / `deny.toml` / `AGENTS.md` 与 CI 工作流。
