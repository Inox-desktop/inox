# 测试策略

测试要求以 PRD §7 与根级 `AGENTS.md` §6 为准。核心取向是：**核心逻辑必须能在 headless 环境被测试**，副作用集中在系统边界层，且**不得 mock 掉被测的核心逻辑** —— 只在 DRM、输入设备、D-Bus 这类边界打桩。

## 测试类型与适用范围

| 测试类型 | 工具 | 范围 | 适用范围 |
|----------|------|------|----------|
| 单元测试 | `cargo test` | 所有纯逻辑模块 | 每个 crate 都必须有 |
| 属性测试 | `proptest` | 窗口管理与布局算法 | 合成器、Shell 的布局与窗口簿记 |
| headless 集成测试 | `cargo test` + 软件渲染 | 合成器输出创建、渲染、输入、协议 | `inox-compositor` |
| GUI 投影断言 | Rust 单测（`view.rs` 的 JSON 值断言） | 令牌 → 颜色/尺寸、Fluent 文案、可达条目 | 七个 QML 组件（已落地，见下） |
| QML 加载测试 | `QQmlApplicationEngine` + `QT_QPA_PLATFORM=offscreen` | QML 模块能加载、根对象非空、QML 回报 ready | 七个 QML 组件（已落地） |
| GUI 离屏帧 | panel/lock 的离屏 QML 宿主 + `wl_shm` 帧读回 | 提交给 layer / 锁表面的像素尺寸与内容 | `inox-panel`、`inox-lock`（已落地） |
| GUI 测试（虚拟合成器） | `waydriver` | 组件在真实合成器中的表面、焦点与截图 | L3 组件（**尚未接线**，见下） |
| 性能测试 | Tracy | 帧时间、输入延迟 | 性能预算验证（PRD §8） |
| 依赖安全 | `cargo-deny` | 许可证、漏洞、重复版本 | 工作区 CI 的 `deny` job（完整依赖图） |
| 静态检查 | clippy、fmt | 全部代码 | 工作区 CI 的 `clippy` / `fmt` job |

覆盖率的默认要求是核心 crate 行覆盖 ≥ 70%；每个 crate 可以在自己的 `AGENTS.md` 中收紧，不得放宽到 0。

## 为什么是这套组合

- **纯逻辑与副作用分离**：合成器的输入判定、窗口簿记与平铺几何都是不依赖 Smithay 运行时状态的纯函数，因此可以用普通单元测试与 `proptest` 覆盖，不需要真实设备。副作用（协议、渲染、事件循环）留在边界层，由集成测试覆盖。
- **不用 mock**：`tests/window_management.rs` 在**同一进程内**用 `UnixStream::pair()` 连接一个真实的 `wayland-client`，创建 ≥2 个 xdg toplevel，断言合成器可观测状态（托管窗口数、`Space` 中的平铺落点、焦点）。被验证的是真实协议处理路径，而不是桩。`wayland-client` / `wayland-protocols` 只作为 dev-dependency，不进入发布产物。
- **断言要看得见**：渲染是否真的发生由**离屏像素读回**断言，不能只依赖日志。像素比较留 ±1 个 8 位步长的容差，因为不同驱动的 float → unorm 舍入不同。
- **CI 无设备**：PRD §6.1 要求 CI 在没有显示设备与输入设备的环境中可复现，因此 M2 的输入通过进程内注入 API 驱动，真实输入设备链路（libinput/udev）推迟到具备后端能力的里程碑。

## L3 组件的 GUI 测试：投影、QML 加载与离屏帧

界面从 Iced 迁到 QML + Qt Quick 后，测试装置也换了一套（ §决策 5）：Iced 的 `UserInterface` / `tiny_skia` 像素装置删除，换成下面四类。共同前提是**不需要合成器**：CI 里没有支持 `wlr-layer-shell-unstable-v1` 的宿主，Qt 用 `offscreen` 平台。

1. **纯逻辑单测**：`app.rs` / `view_model.rs` / 状态机的原有测试全部保留，不因为换 UI 栈而放宽。
2. **投影断言**：`view.rs` 把「状态 + 令牌 + 文案」投影成 JSON，测试直接断言这个值 —— 颜色等于令牌的转换结果（`theme::qml_color(Rgba)`），字号与间距等于令牌推导值，文案来自 Fluent（`en-US` 与 `zh-CN` 各断言一次），可达条目数与会话一致。
3. **QML 加载测试**：在 `QT_QPA_PLATFORM=offscreen` 下用 `QQmlApplicationEngine` 加载本 crate 的 QML 模块，断言根对象非空、QML 通过 `reportReady` 回报 ready。环境变量必须在**子进程**里设置：edition 2024 下改写本进程环境是 `unsafe`，而这些 crate 手写的 Rust 不写 `unsafe`，所以父测试用 `std::env::current_exe()` 自 spawn 一个子进程并带上 `QT_QPA_PLATFORM` / `QT_QUICK_BACKEND`。
4. **panel/lock 的离屏帧**：这两处的界面就是像素 —— 面板把帧提交给 layer 表面、锁屏提交给锁表面。`headless_tests/protocol.rs` 用真实 `wayland-server` 在 socket pair 上扮演合成器（锁屏那一侧是手写的 `ext-session-lock-v1` peer），驱动编译出的二进制，并把它提交的 `wl_shm` 帧**读回来**断言尺寸、不透明度与像素内容。

### 每个 crate 怎么跑

```bash
QT_QPA_PLATFORM=offscreen cargo test -p inox-panel --all-features
```

CI 的 `test` 分片对每个 crate 跑的就是 `cargo test -p <crate> --all-features`（作业级环境里设 `QT_QPA_PLATFORM=offscreen`），与本地命令逐字一致；QML 分片额外安装 Qt 6 开发包与字体（见[持续集成](./ci.md)与[依赖管理](./dependencies.md)）。

`QT_QPA_PLATFORM=offscreen` 是**必须**的，不是可选优化：QML 加载测试与面板/锁屏的离屏宿主都要在这个平台上建 `QQuickWindow`，没有它 Qt 会去找 X11/Wayland 显示并失败。

### 三条边界，不能当作已通过的证据

1. 它证明控制树、投影、QML 模块加载与像素输出正确，**不**证明组件在真实合成器中的表面、焦点与放置正确 —— 那需要 waydriver 在真实宿主上启动、截图、断言，而该 job **尚未接线**（ 的 M02、的 M10；这条证据缺口最早记录在 ）。
2. 离屏的面板与锁屏**没有 QPA 无障碍桥**，AT-SPI 语义导出与真实屏幕阅读器（Q01 的人工部分）都没有覆盖；五个窗口组件走标准 QPA 路径，真实屏幕阅读器验收同样未取得。
3. 运行测试的机器必须装有至少一种字体：文本由 Qt 在出帧时布局，没有字体时标签渲染为空，像素断言会变成在断言一张空帧（CI 的 QML 分片为此安装 `fonts-dejavu-core` 与 `fonts-noto-cjk`）。

**没有截图产物这件事要说清**：Iced 时代各组件有 `tests/render.rs`，在设置 `INOX_SCREENSHOT_DIR` 时把每个场景写成 PNG，CI 作为 artifact 收集。这套装置随 Iced 一起删除，迁移后的测试**不写 PNG**，CI 的上传步骤目前取不到文件。真实截图证据要等真实宿主 job 接线后才有，本页不声称它存在。

## 合成器测试的注意事项

`inox-compositor` 的 `AGENTS.md` 记录了几条踩坑约束：

- 一个测试进程只能持有一个 EGL display，因此**每个测试二进制中只保留一个创建 EGL 的测试**。
- 进程级测试必须自备私有 `XDG_RUNTIME_DIR`（Wayland 套接字绑定要求绝对路径的运行时目录），并给子进程设定超时，避免 CI 挂死。
- 键盘解析依赖 XKB rules 文件，CI 必须安装 `xkb-data`。

## 各 crate 现状

| crate | 现有测试 |
|------|----------|
| `inox-compositor` | 命令行解析、按键映射、平铺布局、窗口簿记、合成器状态的 `#[cfg(test)]` 单元测试；布局与窗口簿记的 `proptest` 属性测试；`tests/headless_render.rs`（创建输出并渲染一帧）、`tests/signal_shutdown.rs`（终止信号与帧预算都以退出码 0 结束）、`tests/window_management.rs`（真实客户端多窗口、平铺与输入） |
| `inox-common` | `logging` 与 `paths` 的单元测试。`XdgEnv::from_process` 刻意不做单元测试：edition 2024 下改写进程环境是 `unsafe`，而该 crate `forbid(unsafe_code)`，因此解析逻辑全部经注入的环境快照验证 |
| `inox-config` | 默认值、解析、字段路径与行列定位、范围校验、主题解析与 `ConfigStore` 热重载的单元测试（含「重载失败保留旧值」） |
| `inox-protocols` | `tests/standard_bindings.rs` 断言再导出的接口名与版本；`tests/ipc_roundtrip.rs` 在真实连接上跑私有协议的往返 |
| L3 组件（七个） | 纯逻辑的单元测试 + `proptest` 属性测试（导航栈、快照/请求代次、任务分组是划分、缩放几何、搜索排序、通知队列的终态、锁状态机的安全不变量）；这些 crate 是 **bin-only**（含 cxx-qt bridge，不能有 `lib.rs`），原来的 `tests/*.rs` 迁进了 bin 内的 `#[cfg(test)]` 模块（`src/headless_tests/` 或 `src/headless_tests.rs`）：投影断言（令牌 → 颜色/尺寸、Fluent 文案、可达条目）、`QT_QPA_PLATFORM=offscreen` 下的 QML 加载测试（子进程自 spawn）、目录双向一致性（`catalog.rs`）。`inox-panel` / `inox-lock` 另有 `headless_tests/protocol.rs`：真实协议 peer 驱动编译出的二进制，面板一侧断言 layer 表面参数、glass 请求顺序与提交的预乘 `Argb8888` 帧，锁屏一侧断言 `ext-session-lock-v1` 的取锁与帧；`inox-lock` 还有默认拒绝路径的 CLI 测试 |
| L4 组件（`inox-session`、`inox-idle`、`inox-greet`） | 重启预算、进程组/注销/目录清理、Wayland 线协议 idle 通知、锁前挂起与登录生命周期；greeter 的登录界面按 `state::Greeter` 的生命周期阶段做投影断言，`QT_QPA_PLATFORM=offscreen` 下加载 QML 并等待 `reportReady`，目录双向检查在 `headless_tests/catalog.rs`；独立 CI job 验证真实 Inox headless 启动/注销。认证、greeter 宿主与挂起 E3/E4 尚未完成 |
| 本站点（`docs/book/`） | 无单元测试要求。文档正确性由 CI 的 `book` job（`mdbook build`）保证，站点结构由 `SUMMARY.md` 与实际章节文件的一致性约束 |

## 新增行为时的要求

新增或修改行为必须同时新增/更新对应测试。具体地说：

- 新的纯逻辑 → 单元测试；属于核心算法（窗口管理、布局）的 → 补 `proptest` 不变量。
- 新的合成器能力 → 在 headless 路径上补集成测试，断言可观测状态而非日志。
- 新的 Shell 界面 → 投影断言 + `QT_QPA_PLATFORM=offscreen` 下的 QML 加载测试；断言落在 JSON 值与（面板/锁屏的）离屏帧像素上，而不是日志。真实宿主截图要等 waydriver job 接线（当前**未接线**）。
- 性能相关的改动若引入可测量的回退，需要在 PR 中给出 Tracy 证据（PRD §8）。
