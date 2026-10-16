# ADR 0010: M2.5 的真实后端（libseat session + DRM/KMS 输出 + libinput 输入）

- 状态：proposed
- 日期：2026-10-16
- 决策者：架构组

## 背景

ADR 0005 与 ADR 0008 分别交付了 M1（headless 输出与纯色帧）与 M2（headless 路径上的输入注入、多窗口、平铺）。二者都建立在**无 DRM/KMS、无 libinput** 的 Smithay 最小特性集上（仅 `desktop` + `renderer_gl` + `wayland_frontend`）。PRD §4 早已把「显示后端 Smithay `backend_drm` / 输入后端 Smithay `backend_libinput` / 会话管理 libseat」列入技术栈，但直到 M2 结束，这条真实设备路径都仍是纸面承诺；[state.rs](../../repos/inox-compositor/src/state.rs) 与 [lib.rs](../../repos/inox-compositor/src/lib.rs) 的注释里也显式留着「no DRM/KMS backend and no libinput backend」的坑位。

现在必须把这个决策落地的原因：

- **M3 XWayland 之前必须完成输出与输入路径的真实化**。XWayland、剪贴板、拖放都要求合成器至少有一个可翻页的输出与一个可派发键盘/指针事件的 seat；继续留在 headless + 注入路径会把风险攒到 M3。
- **PRD §14 的「GitHub Actions 无 GPU」风险条目**要求显式说明真实硬件路径与 CI 的分工，当前没有一份 ADR 说清这件事。
- **依赖方向与 `unsafe` 面积发生变化**：Smithay `backend_drm` / `backend_gbm` / `backend_libinput` / `backend_udev` / `backend_session_libseat` 会把 `libseat` / `gbm` / `drm` / `input` / `udev` 等 crate 拉进依赖树，并引入若干 fd 归属相关的 `unsafe`。按根级 `AGENTS.md` §3.2 与 §4，这类变更**必须先写 ADR**。

约束与依据：

- 根级 `AGENTS.md` §4：「必须使用 Smithay 抽象实现合成器」——本 ADR 只启用 Smithay 提供的 backend feature，不自建 DRM/libinput 绑定。
- 根级 `AGENTS.md` §4：「必须使用 calloop 作为主事件循环」——vblank 与 libinput fd 都通过 calloop 的 `Generic` 源接入，禁止自建轮询线程。
- 根级 `AGENTS.md` §3.1：CI 是唯一验收途径——真实 DRM/libinput 路径无法在 GitHub runner 上跑（无 DRM master、无 `/dev/input` 权限），CI 只保证**编译通过 + headless 集成回归**，硬件验收由人类在 PR 完成后单独执行。
- ADR 0005 §8：`unsafe` 允许局限在 `inox-compositor` 的 FFI 胶水，模式为「局部 `#[allow(unsafe_code)]` + `// SAFETY:`」。本 ADR 沿用该模式，不改动 `Cargo.toml` 的 `unsafe_code = "deny"`。
- ADR 0003：Smithay 锁定 v0.7.0，calloop 锁定 0.14。本 ADR 不升级 major。

考虑过但未被采纳的方案：

- **自研 logind D-Bus 客户端**：`inox-compositor` 会新增一个 D-Bus 依赖（`zbus` 或 `smithay-dbus-*`），把 session 管理的实现负担留在项目内部；libseat 已经覆盖 logind / seatd / 直接开设备三条路径，成本与失败面显著更低 —— 不成立。
- **只用 udev + uaccess 自管 session（不引入 libseat）**：能跑通裸机但没有 logind/seatd 集成，VT 切换、多 session、无 root 权限设备访问都要重新发明；且这条路径要求合成器自己维护 `Session` trait 的实现，`unsafe` 面积更大 —— 不成立。
- **只上 DRM 输出，输入继续用注入到 M3 之后**：可以拆得更细但会让 `runtime.rs` 长期同时挂两套帧驱动（Timer + vblank）却没有真输入，M3 XWayland 需要真键盘事件才能端到端跑通 —— 拆两个 PR 更合适：M2.5a 输出、M2.5b 输入，都在同一 ADR 下。
- **放弃 headless，改在 CI 用 vkms 或 software DRM 驱动**：GitHub runner 无内核模块权限，装 vkms 需要 privileged job，代价远超收益；headless 路径继续作为 CI 与 `waydriver` 测试的唯一入口 —— 不成立。

## 决策

M2.5 引入**真实后端**（DRM/KMS 输出 + libinput 输入），通过 libseat 拿设备访问权；headless 路径完整保留。具体：

1. **Session 后端**：使用 Smithay 的 `backend::session` 抽象，启用 Smithay feature `backend_session` + `backend_session_libseat`，由 `libseat` crate 提供 logind / seatd / 直接开设备三选一。启动顺序为 `Session::auto()`：优先 logind，回落 seatd，再回落到 `Session::Direct`（依赖 udev + uaccess ACL）。**禁止**在 `inox-compositor` 内自研 logind D-Bus 客户端。

2. **Smithay 特性集**：从 `["desktop", "renderer_gl", "wayland_frontend"]` 扩到增加 `backend_drm`、`backend_gbm`、`backend_libinput`、`backend_udev`、`backend_session`、`backend_session_libseat`。**不启用** `backend_x11` / `backend_winit` / `backend_vulkan` / `xwayland`（XWayland 由 ADR 0007 走独立进程，避免把 winit 或 x11 拉进合成器）。

3. **输出模型**：
   - 每个 DRM 设备一个 `DrmDevice`，其下每个 connector 一个 Smithay `Output`；GBM 设备由 DRM FD 派生（`GbmDevice::new`）。
   - 渲染目标抽象为 `RenderTarget` trait：headless 用现有的 `HeadlessRenderer`（`GlesRenderbuffer`），真实路径用 `GbmSurface`。二者通过同一 `Space::render_elements_for_region` 产出的 `RenderElement` 消费。
   - 首选模式选择顺序：connector 的 `preferred` → 分辨率最大者 → mode 列表首项。M2.5a 支持多输出但不做跨输出布局（`Space::map_output` 默认坐标堆叠），跨输出布局与用户配置留到 M4。
   - 翻页时通过 primary plane 提交 GBM BO；`DrmEvent::VBlank` 是**真实分支**唯一的帧驱动源。

4. **输入模型**：
   - `Libinput` context 通过 `libinput_udev_assign_seat(seat_name)` 绑定 libseat 报告的 seat 名；fd 以 `Generic` 源接入 calloop，回调内 `dispatch()` 直到 `WouldBlock`。
   - 事件翻译到 Smithay `Seat`：`Event::Keyboard` → `keyboard.input`（keycode 偏移 +8 由 evdev → XKB），`Event::PointerMotion{Abs}` → `pointer.motion`，`Event::PointerButton` → `pointer.button`，`Event::Device{added|removed}` → 在合成器的 `Seat` 上增删 capability。
   - 触屏、平板、手势、高 DPI 缩放、软件光标绘制不在本里程碑；`SeatHandler::cursor_image` 保留现有占位。M2.5b 只处理 `seat0`。

5. **帧循环迁移**：现有的 `Timer::from_duration(interval)` 帧源**仅在 headless 分支**注册；DRM 分支改由 `DrmDevice` 的 vblank fd 驱动。`exit_after_frames` 语义保持不变（每 `present` 一次即视为一帧）。

6. **CLI 与运行时回退**：`inox-compositor` 新增 `--backend <headless|drm|auto>`（默认 `auto`）。`auto` 依次尝试 libseat → DRM，失败时回落 headless 并 `warn!` 记录原因。`--headless` 保留为 `--backend=headless` 的别名。`--output-size` / `--refresh-rate` 只对 headless 生效；DRM 分支忽略并 `debug!`。

7. **`unsafe` 边界**：`Cargo.toml` 保持 `unsafe_code = "deny"`。新增 `#[allow(unsafe_code)]` **只允许**出现在三个新文件的最小 FFI 胶水处，每处附 `// SAFETY:` 注释：
   - `src/backend/drm.rs`：Smithay `DrmDevice::open` / `DrmDevice::from_file` 的 fd 归属；
   - `src/backend/gbm.rs`：`GbmDevice::new` 与 DRM FD 的借用生命周期；
   - `src/backend/input.rs`：`Libinput::init` 与 udev-seat 绑定；
   沿用 ADR 0005 §8 的模式。现有的 `render.rs` 与 `runtime.rs` 中的两处 `allow` 不变。仓库级 `AGENTS.md` 允许列表同步扩到六处。

8. **依赖与许可证**：全部通过 Smithay re-export 引入，不新增直接的 Rust 依赖。传递依赖的许可证覆盖 MIT / Apache-2.0 / BSD-3-Clause / ISC，符合根级 §3.6。系统库 `libseat` / `libdrm` / `gbm` / `libinput` / `libudev` 是运行时依赖；其中 `libudev` 为 LGPL-2.1+（glibc 动态链接例外），属 Smithay 生态常规；`cargo-deny` 只检查 Rust 依赖，不受影响。CI apt 包新增：`libseat-dev libdrm-dev libgbm-dev libinput-dev libudev-dev`。

9. **CI 与验收分工**：
   - `cargo build --no-default-features --features "backend_drm,backend_gbm,backend_session_libseat,backend_libinput,backend_udev"` 作为**编译面覆盖**步骤，保证真实后端至少被类型检查；runner 无 DRM master，不跑相关集成测试。
   - `cargo test --all-features` 依然只跑 headless 集成回归（`tests/headless_render.rs` / `signal_shutdown.rs` / `window_management.rs`），行为不变。
   - 纯逻辑单元测试覆盖：connector/mode 排名（用 `proptest`）、vblank→render 状态机、libinput event → `Action` 翻译表、`--backend` CLI 解析。
   - **真机验收由人类完成**（PRD §5.5）：功能清单在 PR 描述中给出——开机看到终端画面、鼠标移动、Alt+Tab 切窗、Super+Q 干净退出（退出码 0）、HDMI 热插拔、lid 关闭 → 请求退出。

10. **分层与依赖方向不变**：`inox-compositor` 仍在 L2，不新增跨仓库 Rust 依赖。`inox-common` / `inox-config` / `inox-protocols` 无改动。

## 后果

正面：

- 合成器可以在真实硬件上创建输出、翻页、派发键盘与指针事件，M3 XWayland 的前置条件具备。
- Session 归属通过 libseat 抽象，覆盖 logind / seatd / uaccess 三条主流路径，避免自造轮子。
- Headless 路径完整保留，CI 与 `waydriver` 集成测试继续在无 GPU、无输入设备的环境下运行，符合 PRD §6.1。
- `unsafe` 边界从两处扩到六处，但都集中在命名良好的 FFI 胶水函数，可审查、可回滚。
- Smithay feature 集合的扩展是**声明性的**，不引入与 XWayland/winit/vulkan 的耦合，未来 ADR 可以按需再打开。

负面：

- 真实硬件路径无法在 CI 上跑通，只能靠**编译面覆盖 + 人类硬件验收**；某些 GBM modifier / atomic 提交相关的问题要等到实机才能暴露，这是 GitHub Actions 无 GPU 风险的直接体现（PRD §14）。
- libseat 引入 seatd 或 logind 之一作为运行时软依赖；在没有二者的裸环境（例如某些容器）中必须回落到 `Session::Direct` 并要求 uaccess ACL 已配置。
- `runtime.rs` 的分支变复杂：headless 与 DRM 走两套帧源、两套输出枚举；抽象到 `Backend` trait 后仍需保留 `Backend::Headless` 与 `Backend::Drm` 两个变体，`Backend::Auto` 探测有失败回落路径。
- 单 seat、单 pointer、无 touch 的模型对**多显卡 / 混合显卡**（prime）与**多 seat**（例如平板 / 服务器场景）留了债；`Session::paused` / `resumed` 只登记接口，VT 切换行为留到 M4。
- vblank 驱动意味着**没有输出时不会渲染**：`Space` 上没有 connector 时事件循环只剩 socket / display / signal 三个源，这与 M2 的「只要 timer 在跑就一直渲染纯色」有可观察差别，集成测试要显式选 `--headless`。
- 新增系统依赖（libseat / libdrm / gbm / libinput / libudev）抬高发行版打包门槛；PRD §12 打包章节需要在 M6 前跟进。

影响面：

- **元仓库**：本 ADR；[PRD.md](../../PRD.md) §13 里程碑表在 M2 与 M3 之间插入 M2.5 行；[docs/adr/README.md](./README.md) 索引新增 0010 行。
- **`inox-compositor`**：`Cargo.toml`（Smithay features、`deny.toml` `[sources] allow-wildcard` 若被触发）、`src/lib.rs`、`src/config.rs`（`--backend`）、`src/runtime.rs`（Backend 选择与源注册）、`src/state.rs`（输出集合与 `RenderTarget` 抽象）、`src/render.rs`（`RenderTarget` trait）、`src/backend/`（新目录：`mod.rs`、`headless.rs`、`drm.rs`、`gbm.rs`、`input.rs`）、`.github/workflows/ci.yml`（新系统依赖 + 编译覆盖步骤）、`AGENTS.md`（`unsafe` 允许列表扩到六处）。
- **不影响**：`inox-common` / `inox-config` / `inox-protocols` / `inox-xwayland` / L3-L4 仓库。

取代条件：

- 若 Smithay 上游重新提供 headless 后端并支持通过统一 `Backend` trait 表达（不再需要自维护 surfaceless EGL），需撰写取代 ADR 0005 的文档，本 ADR 只影响 §5、§9 的分支细节，不直接被取代。
- 若 libseat 上游停止维护或出现覆盖更广的 session 抽象，需取代本 ADR 第 1、2、7 条。
- 若 M4 引入多 seat / prime / 跨输出布局，需扩展（而非取代）本 ADR 第 3、4、6 条。
