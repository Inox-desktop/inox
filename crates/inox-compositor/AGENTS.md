# AGENTS.md — inox-compositor

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

基于 Smithay 的 Wayland 合成器。所属层级：**L2**。M1 广告 `wl_output`、`wl_compositor` 与私有 `inox_ipc` 全局（后者来自 `inox-protocols`）。

## crate 特有约束

- **`unsafe` 例外（根级 §3.2）**：本 crate 的 `Cargo.toml` 使用 `[lints.rust] unsafe_code = "deny"`（而非 `forbid`）。注意它**不能**写 `lints.workspace = true`：cargo 拒绝在继承工作区 lint 的同时覆盖其中一项，因此本 crate 逐字重述了 `[workspace.lints]` 再改写 `unsafe_code`；**改根 `Cargo.toml` 的 lint 集时必须同步这个副本**（没有机械检查）。`#[allow(unsafe_code)]` **当前**只出现在下面两处（与 M2 一致）：
  - `src/render.rs`：`EGLDisplay::new` / `GlesRenderer::new` 的 EGL/GL 胶水（ADR 0005）；
  - `src/runtime.rs`：从 calloop `Generic` 事件源取得 `Display` 以派发客户端请求（`NoIoDrop::get_mut`）（ADR 0005）。

  ADR 0010 §7 **预先授权** `src/backend/drm.rs`、`src/backend/gbm.rs`、`src/backend/input.rs` 在 M3a / M3b 实现落地时引入局部 `unsafe`（封装 libseat `OwnedFd`、`DrmDevice` fd 归属、`GbmDevice::new`、`Libinput::init` 等）；目前 `src/backend/drm.rs` 只是脚手架，本身不包含 `unsafe` 块。每一处新落的 `unsafe` 必须是命名函数、包裹在安全 API 内、附 `// SAFETY:` 理由；**上述白名单以外新增 `allow` 必须先在 `docs/adr/` 提交 ADR**。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`；面向 CLI 的输出（如 `--help`）用显式 stdout 写入。
- MSRV：工作区统一为 `[workspace.package] rust-version = "1.86"`（`smithay` 0.7 依赖的 `calloop` 0.14 要求 rustc ≥ 1.86），本 crate 通过 `rust-version.workspace = true` 继承。工作区内不再存在两个 MSRV。
- 工作区内依赖：`inox-common` 与 `inox-protocols` 都是**path 依赖**（在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明，本 crate 用 `{ workspace = true }` 引用），不再是 git 依赖，因此不涉及 `[sources] allow-git` 与 tag 升级（根级 §8）。`inox-protocols` 是 Wayland 协议（核心 `wl_*` 绑定与私有 `inox-ipc`）的唯一来源（ADR 0002 规则 4）。
  - **注意 `inox-compositor -> inox-protocols` 是 L2 → L2 的同层边**：ADR 0002 规则 1 禁止同层依赖，而协议集中到 `inox-protocols`（规则 4）又要求它。该边已登记在根 `Cargo.toml` 的 `[workspace.metadata.inox.rules] same_layer_exceptions`，规则 1 的裁决（层内顺序）尚未完成。
- **后端开关（ADR 0010）**：`--backend` 选择 `auto`（默认）/ `headless` / `drm`；`--headless` 保留为 `headless` 的别名。`auto` 失败时回落 headless 并 `warn!`；`drm` 失败直接 `RunError::BackendUnavailable`。CI 的集成测试必须显式或默认地走 headless 分支（runner 无 DRM master）。

## 主要依赖

`smithay`（`default-features = false`，启用 `desktop` + `wayland_frontend` + `renderer_gl` + `backend_drm` + `backend_gbm` + `backend_libinput` + `backend_udev` + `backend_session` + `backend_session_libseat`，**不启用** `backend_x11` / `backend_winit` / `backend_vulkan` / `xwayland`，见 ADR 0010 §2）、`calloop`（`signals`）、`anyhow`、`thiserror`、`tracing`、`inox-common` 与 `inox-protocols`（均为工作区内 path 依赖）。

**系统库依赖**：`libseat` / `libdrm` / `gbm` / `libinput` / `libudev`（编译时需 `-dev` 包，已在 CI 安装）；`libxkbcommon` + `xkb-data` 从 M1 就在。`libudev` 为 LGPL-2.1+（glibc 动态链接例外），属 Smithay 生态常规依赖。

**测试依赖**（不进入发布产物）：`proptest`（平铺布局与窗口簿记的算法验证）、`wayland-client` / `wayland-protocols`（集成测试驱动真实客户端）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

本 crate 是工作区成员，命令应尽量用 `-p` 限定范围（CI 的测试矩阵跑的就是同一批命令）。在 `crates/<name>/` 目录内或仓库根执行均可。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-compositor --all-features` |
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

## 目录结构

```
src/
  lib.rs          库入口与模块说明
  main.rs         可执行入口（CLI、日志初始化）
  config.rs       命令行解析、后端选择与配置数据（不依赖 Smithay）
  input.rs        按键 → 合成器动作的纯逻辑（Alt+Tab / Super+Q 等）
  layout.rs       master/stack 平铺几何（纯逻辑 + proptest）
  workspace.rs    窗口簿记与布局顺序（纯逻辑 + proptest）
  runtime.rs      calloop 事件循环：后端探测、套接字、信号、帧定时器（headless）
  render.rs       EGL surfaceless + GLES 离屏渲染（unsafe 例外之一）
  state.rs        Smithay 合成器状态、seat/xdg-shell/shm handler、窗口管理与输入注入
  backend/
    mod.rs        后端入口与说明
    drm.rs        M2.5 真实 DRM/KMS 脚手架（libseat 会话 + card 探测，present 路径待 M3a）
tests/
  headless_render.rs     创建输出并渲染一帧（进程内）
  ipc_global.rs          进程级：广告 inox_ipc 全局并应答 get_version
  signal_shutdown.rs     进程级：终止信号与帧预算都须以退出码 0 结束
  window_management.rs   真实客户端创建 ≥2 窗口，断言平铺与输入（进程内）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `compositor`。
