# AGENTS.md — inox-compositor

本 crate 遵循根仓库的**全部硬性规则**（[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)），本文件只补充本 crate 特有内容，不放宽任何硬性规则。

## 职责

基于 Smithay 的 Wayland 合成器。所属层级：**L2**。

广告的全局以 `src/state.rs` 的模块文档为唯一清单（`wl_compositor` v6/`wl_subcompositor`、`wl_shm`、`wl_seat`、每个输出一个 `wl_output`（v4）+`zxdg_output_manager_v1`、`xdg_wm_base`、`zwlr_layer_shell_v1`（v4）、私有 `inox_ipc` 与私有 `inox_glass_manager_v1`（v1），M3 起的 `wp_viewporter`、`wl_data_device_manager`、`zwp_primary_selection_device_manager_v1`，以及分数缩放起的 `wp_fractional_scale_manager_v1`，以及会话锁与通知总线起的 `ext_session_lock_manager_v1`（v1））。M3 起剪贴板与主选区的可见性跟随键盘焦点，`sync_focus` 是唯一交接点。

输出拓扑：`InoxState` 持有一张输出表，所有输出映射进同一个 `Space`，位置是全局逻辑坐标（可为负）。分数缩放：每个输出有自己的缩放因子，渲染、光标与 layer 放置按该因子折算为物理像素，命中测试、平铺与工作区留在逻辑坐标。

## crate 特有约束

- **`unsafe` 例外（根级 §3.2）**：本 crate 的 `Cargo.toml` 使用 `[lints.rust] unsafe_code = "deny"`（而非 `forbid`）。注意它**不能**写 `lints.workspace = true`：cargo 拒绝在继承工作区 lint 的同时覆盖其中一项，因此本 crate 逐字重述了 `[workspace.lints]` 再改写 `unsafe_code`；**改根 `Cargo.toml` 的 lint 集时必须同步这个副本**（没有机械检查）。`#[allow(unsafe_code)]` 的既有两处为：
  - `src/render.rs`：`EGLDisplay::new` / `GlesRenderer::new` 的 EGL/GL 胶水；
  - `src/runtime.rs`：从 calloop `Generic` 事件源取得 `Display` 以派发客户端请求（`NoIoDrop::get_mut`）。

  `src/backend/drm.rs::create_renderer` 新增物理 GPU 的 `EGLDisplay::new` / `GlesRenderer::new` 最小胶水，仍只在命名函数局部 `allow` 并有 SAFETY 注释。`DrmDevice::new`、`GbmDevice::new`、libinput udev 构造与 fd 转换均使用安全 API。帧生命周期已在 2026-10-17 当前开发会话获人类批准，取代真实后端的预授权数量/API 假设；不得把文件白名单读作任意 unsafe 的授权。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`；面向 CLI 的输出（如 `--help`）用显式 stdout 写入。
- MSRV：工作区统一为 `[workspace.package] rust-version = "1.89"`，本 crate 通过 `rust-version.workspace = true` 继承。工作区内不再存在两个 MSRV。
- 工作区内依赖：`inox-common`、`inox-config` 与 `inox-protocols` 都是**path 依赖**（在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明，本 crate 用 `{ workspace = true }` 引用），不再是 git 依赖，因此不涉及 `[sources] allow-git` 与 tag 升级（根级 §8）。`inox-protocols` 是 Wayland 协议（核心 `wl_*` 绑定与私有 `inox-ipc`）的唯一来源。
  - **注意 `inox-compositor -> inox-protocols` 是 L2 → L2 的同层边**：分层规则 1 禁止同层依赖，而协议集中到 `inox-protocols`（规则 4）又要求它。该边已登记在根 `Cargo.toml` 的 `[workspace.metadata.inox.rules] same_layer_exceptions`，规则 1 的裁决（层内顺序）尚未完成。
- **`test-input` feature（默认关闭）**：`--test-input <PATH>` 只在启用该 feature 的构建里存在，用于让端到端测试驱动**正在运行的合成器进程**（`key`/`motion`/`button`/`dump-frame`，见 `src/test_input.rs`）。约束：默认构建必须直接以 `ConfigError` 拒绝该选项；该通道**不得**成为 Wayland 全局、不得自动启用、不得暴露除「向调用者指定 socket 写命令」以外的能力。修改它时必须同步 `README.md`、本文件与模块文档。
- **后端开关**：`--backend` 选择 `auto`（默认）/ `headless` / `drm`；`--headless` 保留为 `headless` 的别名。`auto` 与 `drm` 都必须得到可用物理输出，失败返回 `RunError::BackendUnavailable`，不得静默回落 headless。CI 进程测试显式 `--headless`；进程内 `InoxState::new` 始终是确定性的 headless 状态。
- **多输出与多 GPU**：`InoxState::apply_outputs` 是拓扑变更的**唯一入口**，接受期望的输出集合、按 connector 名与 `Output` 实例复用未变的输出（保留其全局、layer 表面与窗口），并让消失输出的窗口迁移到主存活输出。禁止再新增「全量替换」的输出入口：`replace_output` 只是 `apply_outputs` 的单输出包装，仅为兼容既有调用点。物理后端枚举 seat 上的**每一块**能驱动输出的 GPU，每块用**自己的** libseat 租约、`DrmDevice`、`GbmDevice` 与 EGL/GLES 渲染器驱动自己的 connector；跨 GPU 没有复制路径，这一点在广告 `linux-dmabuf` 之前必须先解决。
- **缩放**：`scale` 取值域为 `0.25..=8.0`，物理后端**不再拒绝**非整数缩放。渲染路径中不得出现硬编码的 `1.0` 缩放：元素位置按输出 scale 折算为物理像素，`draw_render_elements` 也必须收到该 scale（元素的**位置**是物理像素、**尺寸**由逻辑尺寸乘 scale 得出）。

- **安全锁屏**：`ext_session_lock_manager_v1` v1 由 Smithay 的 `SessionLockManagerState` 实现，本 crate 加上的是策略与锁定场景，四处强制点是**唯一**可能出错的地方，改动时必须一并核对：`scene_elements`（锁定期间只合成锁表面，没有锁表面时为不透明底色）、`sync_focus`（键盘交给锁表面、选区焦点清空）、`surface_under`（只命中锁表面）、`inject_key`（不再拦截合成器快捷键）。`locked` 只在**每个输出都渲染过一帧锁定帧**之后发送（`crate::session_lock::LockCoverage`，含锁定期间新增输出的覆盖）；客户端死亡**不**解锁，且**不**允许第二个客户端接管。授权是默认拒绝：只有先在 `inox_ipc` 上用 `claim_lock_role` 认领过角色、且令牌与本 crate 启动参数 `--lock-token` 一致的连接才能取锁，其余立即收到 `finished`；未配置令牌时一切锁请求被拒绝，客户端自报名称/PID 不参与判断。
- **`Dispatch<ExtSessionLockV1>` 是手工转发，不是 `delegate_session_lock!`**：Smithay 的 `unlock_and_destroy` 分支在发出 `invalid_unlock` 之后**仍然**会清空逐输出簿记并调用 `SessionLockHandler::unlock`（缺少 `return`），因此一个被拒绝或从未收到 `locked` 的连接本可以解开不属于它的锁。本 crate 在转发前先校验「就是当前持有的那个锁对象、且已经发过 `locked`」，这是「只有所有者、且只在确认之后才能解锁」在本合成器成立的原因。改动这条路径前先读 `src/state.rs` 里那段说明。

- **输入法**：文本事务与输入法生命周期位于 `src/state/ime.rs`，公开文本全局为 `zwp_text_input_manager_v3` v1；`zwp_input_method_manager_v2` 与 `zwp_virtual_keyboard_manager_v1` 只对 `ClientState::input_method()` 的专用连接可见，公共监听套接字不得使用该状态。`sync_focus` 同步文本焦点与锁屏隔离；序号失配的编辑整批丢弃。候选窗按输出缩放绘制、鼠标交互不改变应用键盘焦点。新增权限路径必须同时检查锁定期间已绑定对象。`runtime/input_method.rs` 使用 calloop 监督显式选择的引擎，不用 shell 解析 argv。真实客户端、进程与 Fcitx5 拼音 CI 测试均须保留。

- **显示试用**：`org.inox.Configuration1.Reload` 立即应用并**拒绝**输出改动；输出模式/缩放/位置只能经同一对象路径上的 `org.inox.Configuration2` 的 `Trial`/`Confirm`/`Rollback` 应用。四条不可放宽的规则：**一个会话同一时刻至多一个试用**（第二个 `Trial` 失败）；**期限 30 秒由合成器持有**（`src/trial.rs::TRIAL_DEADLINE` 与 calloop 定时器），到期即恢复试用前的偏好，设置窗口消失也一样；**成功回执必须核对设备状态**——`rescan` 对不支持的 mode 只是 `warn!` 并保留旧 scanout，所以 `physical::apply_output_preferences` 必须在 `refresh_topology` 之后用 `GpuBackend::preferences_in_effect` 逐 connector 读回 mode/scale/position，核对不通过就先回滚再返回失败；**回退只退输出偏好**，主题/语言/输入/平铺保持试用后的值，且不写任何文件（持久化属于 `inox-settings`）。代次规则只由 `src/trial.rs::decide` 决定并有单元测试：迟到或陌生代次的 `Confirm` 失败且不固定模式，未知代次的 `Rollback` 幂等成功。headless 路径（`data.physical.is_none()`）继续拒绝输出改动。真实模式切换与 30 秒回退是 E4 真机证据，不得由 headless/离屏测试推断。

- **毛玻璃（私有 `inox-glass-v1`）**：协议规范在 `inox-protocols/protocol/inox-glass-v1.xml`，纯逻辑（参数、clamp、capabilities 判定、pane 收集与坐标/clip 折算）在 `src/glass.rs`，模糊合成 pass 在 `src/render.rs`（两个 region 大小的可分离 scratch pass + `//_DEFINES_` 标记的 GLSL ES 1.00 shader，缓存于 `HeadlessRenderer`），协议接线在 `src/state.rs`。以下规则不可放宽：
  - **`capabilities` 绝不广告不会兑现的模糊**：只有「表面此刻有 layer 角色」∧「会话未锁定」∧「该表面的合成路径真的会施加效果」三者同时成立才报 `MAX_BLUR_RADIUS`（64，冻结），否则报 0。普通 `xdg_toplevel` 由平铺路径合成，不实现毛玻璃，因此报 0；客户端据此选择降级为纯半透明。
  - 参数以 **`WlSurface` 为键**存放在 `InoxState::frost`；表面、glass 对象或客户端任一消失都必须清除（`CompositorHandler::destroyed`、glass 对象的 `Dispatch::destroyed`、`detach_layers_of`、`prune_frost` 四处），死表面不得再被磨砂，也不得把参数留给复用的对象 id。
  - 同一表面第二个 `get_glass` 是**致命协议错误** `already_created`，不得静默替换成第二个控制器。
  - **锁定期间没有任何 pane**：`scene_elements` 在锁定分支提前返回（此时还没有考虑任何 layer 表面），`glass::collect_panes(locked = true)` 另有单元测试固定该行为；毛玻璃因此不可能出现在锁定场景里。`tests/session_lock.rs` 用同一场景补上端到端证据：锁定前背景图案可见、pane 内的边真的被抹开，锁定帧只有锁表面（图案与 pane 一个像素都不剩），锁定期间新建的 glass 对象收到 `capabilities(max_radius = 0)`，且毛玻璃请求不打扰已持有的锁；解锁后同一场景回到屏幕。
  - **物理（DRM）路径不实现毛玻璃（已知限制）**：`OutputDamageTracker` 的 damage 来自渲染元素自身的 damage snapshot，磨砂 passthrough 在该路径还没有诚实的 damage 描述。因此 `backend::drm` 只在场景里出现 pane 时 `warn!` 一次并保持场景清晰，且该路径 `frosting_supported()` 为假使 `capabilities` 报 0。**不得**把这写成「真机已支持毛玻璃」；真机验收属 E4，不得由 headless 测试推断。

## 主要依赖

`smithay`（`default-features = false`，启用 `desktop` + `wayland_frontend` + `renderer_gl` + `backend_drm` + `backend_gbm` + `backend_libinput` + `backend_udev` + `backend_session` + `backend_session_libseat`，**不启用** `backend_x11` / `backend_winit` / `backend_vulkan` / `xwayland`）、`calloop`（`signals`）、`anyhow`、`thiserror`、`tracing`、`inox-common` 与 `inox-protocols`（均为工作区内 path 依赖）。

**系统库依赖**：`libseat` / `libdrm` / `gbm` / `libinput` / `libudev`（编译时需 `-dev` 包，已在 CI 安装）；`libxkbcommon` + `xkb-data` 从 M1 就在。`libudev` 为 LGPL-2.1+（glibc 动态链接例外），属 Smithay 生态常规依赖。

**测试依赖**（不进入发布产物）：`proptest`（平铺布局与窗口簿记的算法验证）、`wayland-client` / `wayland-protocols`（集成测试驱动真实客户端）、`tempfile`（拖放测试给客户端一个真实 `wl_shm` 缓冲：Smithay 只有在表面提交过缓冲后才有可供命中测试的几何）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

本 crate 是工作区成员，命令应尽量用 `-p` 限定范围（CI 的测试矩阵跑的就是同一批命令）。在 `crates/<name>/` 目录内或仓库根执行均可。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-compositor --all-features` |
| 本 crate 测试（默认 feature） | `cargo test -p inox-compositor` |
| 本 crate 构建 | `cargo build -p inox-compositor --release --all-features` |
| 本 crate 文档 | `cargo doc -p inox-compositor --no-deps --all-features` |
| 全工作区测试 | `cargo test --workspace --all-features` |
| 静态检查 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt --all` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`）；测试代码不受「禁止 unwrap/expect/panic」约束。
- **核心算法用 `proptest`**：`layout`（master/stack 平铺几何）与 `workspace`（窗口顺序/焦点）的属性测试必须覆盖「每窗口一块、块不越界、块互不重叠、master 列在 stack 左侧」等不变量。
- **headless 集成测试**（`tests/`）必须可在无显示设备、无 DRM 设备的环境中通过：CI 使用软件渲染（`LIBGL_ALWAYS_SOFTWARE=1`）。
- 「渲染确实发生」必须由**离屏像素读回**断言，不能只依赖日志；颜色比较需留 ±1 个 8 位步长的容差（不同驱动的 float→unorm 舍入不同）。
- 窗口/输入集成测试（`tests/window_management.rs`）在**同一进程内**用 `UnixStream::pair()` 连接真实 `wayland-client`，创建 ≥2 个 xdg toplevel，断言合成器可观测状态（托管窗口数、`Space` 中的平铺落点、焦点），**不得 mock 被测核心逻辑**。
- 键盘解析依赖 XKB rules 文件（`xkb-data`），CI 必须安装该系统包。
- 一个测试进程只能持有一个 EGL display，**每个测试二进制中只保留一个创建 EGL 的测试**。
- 进程级测试必须自备私有 `XDG_RUNTIME_DIR`（Wayland 套接字绑定需要绝对路径的运行时目录），并给子进程设定超时，避免 CI 挂死。
- **协议面**（`tests/protocol_surface.rs`）：进程级——广告的全局必须与 `src/state.rs` 的清单一致，M3 全局的版本（`wp_viewporter` v1、`wl_data_device_manager` v3、`zwp_primary_selection_device_manager_v1` v1）必须可绑定，`wl_compositor` 必须 ≥ 6 且 `wp_fractional_scale_manager_v1` 可绑定。`--test-input` 在**未启用 feature 的构建**里必须被拒绝，该断言位于 `#[cfg(not(feature = "test-input"))]`（`--all-features` 的构建反而启用它，断言在那里没有意义）；由于其余 CI job 一律用 `--all-features`，这条门禁由 CI 的 `m3-e2e` job 用**默认 feature** 构建二进制并断言它非零退出且错误信息点名该选项来验证。
- **layer-shell**（`tests/layer_shell.rs`）：真实客户端覆盖四层像素顺序、独占区与平铺位置、显式/空输出、初次空提交后的 configure/ack、None/Exclusive/OnDemand 焦点、指针命中、frame callback、卸载/重映射、销毁和崩溃释放资源，以及未确认 configure 的客户端隔离。放置/独占区算法的属性测试覆盖极端整数参数；安全锁屏不属于该协议。
- **物理后端边界**：`backend::frame` 覆盖首帧、空闲、提交中 damage、失败限流、pause/resume、移除与陈旧完成；`backend::input` 覆盖多设备按键引用与移除清理；DRM 模式排序使用 proptest。真实客户端测试补充能力变更、按键清理、软件光标像素和输出移除/恢复；`signal_shutdown` 强制无效 libseat 后端，验证 auto/drm 不回落、不伪报 ready。E4 真机结果不得由这些测试推断。
- **多输出**（`tests/multi_output.rs`）：进程内、真实客户端——两个不同分辨率/缩放的 headless 输出各有自己的逻辑工作区且互不重叠；窗口落在主输出的工作区内；主输出消失后窗口迁移到存活输出；全部输出消失时窗口仍被托管、恢复后被主输出收养；O02 的 20 轮拔插往返后客户端仍连着、窗口始终在存活工作区内；重复应用同一拓扑集合不改变窗口位置。
- **分数缩放**（`tests/fractional_scale.rs`）：进程内、真实客户端——1.5x 输出的物理 mode、`zxdg_output_v1` 的逻辑尺寸与 `wl_output.scale` 的上取整分别断言；真实客户端提交 64x64 缓冲后**读回物理像素**，断言红块覆盖物理 `[12, 108)`：这是区分「真的按 1.5 合成」与「只广告了协议」的判别性断言。没有这条像素证据不得声明分数缩放已支持。
- **剪贴板**（`tests/clipboard.rs`）：进程内、真实客户端——持焦点的客户端设置选择，随后获得焦点的客户端拿到 offer 并经 fd 读回字节（逐字节相等）；未获焦点的客户端**不得**被投递选择（这是 M3 的访问控制属性）。
- **拖放**（`tests/drag_and_drop.rs`）：进程内、真实客户端——注入指针按下取得隐式抓取序号，客户端 A 发起拖放（两种 mime），指针移到 B 上后 B 收到 enter 与 mime，drop 后传输字节与源一致；另覆盖「指针移出所有表面后取消」（目标恰好收到一次 leave、不再收到 drop）与「拖放途中目标窗口被销毁」（合成器不崩、会话仍可用）。
- **毛玻璃**（`tests/glass_blur.rs`）：进程内、真实客户端——背景层表面画一条锐利的黑白竖直边，半透明的 top 层「pane」覆盖它并创建真实 `inox_glass_manager_v1` 对象设置区域与半径。判别性断言：pane 外的边保持锐利（严格处于两端色之间的列 ≤ 2），pane 内的过渡至少 `radius/2` 列（同一场景、同一客户端、**未创建** glass 对象的对照渲染为 0 列），且 pane 区域与对照渲染有明显像素差异（≥ radius 列），因此「删掉模糊」必然使测试失败。另断言 layer 表面 capabilities == `MAX_BLUR_RADIUS`、普通 `xdg_toplevel` 为 0，以及同一表面第二个 `get_glass` 收到 `already_created` 致命协议错误。纯逻辑（clamp 幂等/单调、区域归一化上界、锁定无 pane、pane 排序与 clip）在 `src/glass.rs` 用单元测试与 proptest 固定。
- 单线程驱动 + 截止时间：合成器与全部客户端在同一线程交替「收发」与「服务 display」，客户端套接字设为非阻塞，因此**不需要 sleep 排序、也不会有死锁线程**；每个等待都有超时。
- `test-input` 的解析器与 PPM 编码器是纯函数，必须有单元测试（合法/非法/未知动词/越界数字/尾随空白/CRLF）。

## 目录结构

```
src/
  lib.rs          库入口与模块说明
  main.rs         可执行入口（CLI、日志初始化）
  config.rs       命令行解析、后端选择与配置数据（不依赖 Smithay）
  input.rs        按键 → 合成器动作的纯逻辑（Alt+Tab / Super+Q 等）
  layout.rs       master/stack 平铺几何（纯逻辑 + proptest）
  layers.rs       layer-shell 放置与独占区计算（纯逻辑 + proptest）
  workspace.rs    窗口簿记、每个窗口所属输出与布局顺序（纯逻辑 + proptest）
  topology.rs     输出放置、指针跨输出钳制与逻辑尺寸折算（纯逻辑 + proptest）
  trial.rs        显示试用的纯状态机：代次、30 秒期限、确认/回退判定
  glass.rs        私有 inox-glass-v1 的纯逻辑：半径 clamp、区域归一化、capabilities 判定、pane 收集（单元测试 + proptest）
  runtime.rs      calloop 事件循环：后端探测、套接字、信号、帧定时器（headless）
  runtime/reload.rs  会话 D-Bus 的 Configuration1/Configuration2 端点、离循环候选读取与试用期限
  runtime/physical.rs 物理后端的设备事件源、逐输出帧调度与输出偏好应用
  render.rs       EGL surfaceless + GLES 离屏渲染（unsafe 例外之一）
  state.rs        Smithay 合成器状态、seat/xdg-shell/shm/viewporter/selection handler、窗口管理与输入注入
  test_input.rs   仅测试的控制通道（`--test-input`，`test-input` feature 默认关闭）
  backend/
    mod.rs        后端入口与说明
    drm.rs        单个 DRM 设备：libseat 租约、GBM/EGL 渲染器、每 connector 一个 scanout、模式/缩放选择与呈现
    input.rs      libinput 事件、设备能力与多设备按键清理
    frame.rs      与 headless 共用的帧状态机、代次与有限重试
tests/
  headless_render.rs     创建输出并渲染一帧（进程内）
  multi_output.rs        进程内真实客户端：双输出平铺、迁移、20 轮拔插往返（O02）
  fractional_scale.rs    进程内真实客户端：1.5x 输出的逻辑/物理几何与像素落点（O03）
  ipc_global.rs          进程级：广告 inox_ipc 全局并应答 get_version
  signal_shutdown.rs     进程级：终止信号与帧预算都须以退出码 0 结束
  window_management.rs   真实客户端创建 ≥2 窗口，断言平铺与输入（进程内）
  protocol_surface.rs    进程级：M3 全局的接口与版本可绑定；未启用 feature 时拒绝 --test-input
  session_lock.rs        进程内真实客户端：锁定遮蔽、`locked` 时序、输入/选区隔离、授权拒绝、协议错误与 UI 死亡后拒绝接管；以及毛玻璃场景在锁定前/锁定中的判别性像素证据
  clipboard.rs           进程内真实客户端：只有焦点客户端能读到剪贴板
  drag_and_drop.rs       进程内真实客户端：拖放传输、取消、目标中途销毁
  glass_blur.rs          进程内真实客户端：毛玻璃的判别性像素证据（pane 内模糊、pane 外锐利、与无 glass 对象的对照渲染不同）＋ capabilities 角色前置条件与重复 get_glass 的致命错误
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `compositor`。
