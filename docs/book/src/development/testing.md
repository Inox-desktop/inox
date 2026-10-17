# 测试策略

测试要求以 PRD §7 与根级 `AGENTS.md` §6 为准。核心取向是：**核心逻辑必须能在 headless 环境被测试**，副作用集中在系统边界层，且**不得 mock 掉被测的核心逻辑** —— 只在 DRM、输入设备、D-Bus 这类边界打桩。

## 测试类型与适用范围

| 测试类型 | 工具 | 范围 | 适用范围 |
|----------|------|------|----------|
| 单元测试 | `cargo test` | 所有纯逻辑模块 | 每个 crate 都必须有 |
| 属性测试 | `proptest` | 窗口管理与布局算法 | 合成器、Shell 的布局与窗口簿记 |
| headless 集成测试 | `cargo test` + 软件渲染 | 合成器输出创建、渲染、输入、协议 | `inox-compositor` |
| GUI 测试（离屏） | `iced_tiny_skia` + `iced_runtime::UserInterface` | 真实控件树的布局、绘制像素与事件处理 | L3 组件（已落地，见下） |
| GUI 测试（虚拟合成器） | `waydriver` | 组件在真实合成器中的表面、焦点与截图 | L3 组件（待 layer-shell 宿主可用） |
| 性能测试 | Tracy | 帧时间、输入延迟 | 性能预算验证（PRD §8） |
| 依赖安全 | `cargo-deny` | 许可证、漏洞、重复版本 | 工作区 CI 的 `deny` job（完整依赖图） |
| 静态检查 | clippy、fmt | 全部代码 | 工作区 CI 的 `clippy` / `fmt` job |

覆盖率的默认要求是核心 crate 行覆盖 ≥ 70%；每个 crate 可以在自己的 `AGENTS.md` 中收紧，不得放宽到 0。

## 为什么是这套组合

- **纯逻辑与副作用分离**：合成器的输入判定、窗口簿记与平铺几何都是不依赖 Smithay 运行时状态的纯函数，因此可以用普通单元测试与 `proptest` 覆盖，不需要真实设备。副作用（协议、渲染、事件循环）留在边界层，由集成测试覆盖。
- **不用 mock**：`tests/window_management.rs` 在**同一进程内**用 `UnixStream::pair()` 连接一个真实的 `wayland-client`，创建 ≥2 个 xdg toplevel，断言合成器可观测状态（托管窗口数、`Space` 中的平铺落点、焦点）。被验证的是真实协议处理路径，而不是桩。`wayland-client` / `wayland-protocols` 只作为 dev-dependency，不进入发布产物。
- **断言要看得见**：渲染是否真的发生由**离屏像素读回**断言，不能只依赖日志。像素比较留 ±1 个 8 位步长的容差，因为不同驱动的 float → unorm 舍入不同。
- **CI 无设备**：PRD §6.1 要求 CI 在没有显示设备与输入设备的环境中可复现，因此 M2 的输入通过进程内注入 API 驱动，真实输入设备链路（libinput/udev）推迟到具备后端能力的里程碑。

## L3 组件的 GUI 测试：离屏渲染

L3 组件的界面测试**不需要合成器，也不需要 GPU**。这是刻意的设计，因为 CI 里没有支持 `wlr-layer-shell-unstable-v1`
的宿主，而面板、启动器、通知与壁纸在 PRD §2.3 中都是 layer-shell 客户端。

做法是直接驱动 iced 自己的运行时：用 `iced_runtime::UserInterface` 在固定尺寸上布局**真实的控件树**，
用 `iced_tiny_skia::Renderer`（与二进制使用同一个渲染器）绘制，再用 `Headless::screenshot` 取回 RGBA 像素：

```rust
let mut renderer = iced_tiny_skia::Renderer::new(Font::default(), Pixels(16.0));
let mut ui = UserInterface::<Message, iced::Theme, iced_tiny_skia::Renderer>::build(
    view(state), Size::new(1280.0, 32.0), Cache::default(), &mut renderer,
);
ui.draw(&mut renderer, &theme, &Style::default(), mouse::Cursor::AVAILABLE);
let pixels = Headless::screenshot(&mut renderer, Size::new(1280, 32), 1.0, Color::WHITE);
```

由此得到三类断言，覆盖验收 Q02 的一部分：

- **像素**：背景区域取到主题的 background/surface 令牌；禁用项与可用项渲染结果不同；浅色与深色两种主题各断言一次。
- **交互**：`UserInterface::update(&[Event::...], cursor, renderer, clipboard, &mut messages)`
  注入真实的鼠标与键盘事件，把产生的 `Message` 喂回组件自己的 `update()`，再断言状态与再次渲染的结果
  —— 点击、键入、Esc、方向键这些验收 D01/D02/D04 的行为因此可以在 CI 中验证。
- **产物**：设置 `INOX_SCREENSHOT_DIR` 时测试把像素写成 PNG，供 CI 作为 artifact 收集，用于人工比对基线；
  基线变更仍需人类审查（Q02 要求截图差异有理由）。

这套装置有三条边界，不能当作已通过的证据：

1. 它证明**控件树与事件处理**正确，不证明组件在真实合成器中的表面、焦点与放置正确 —— 那部分需要 layer-shell 宿主，
   目前被上游阻塞（`iced_layershell` 0.19.1/0.19.0 无法编译，见[依赖管理](./dependencies.md)与
   [ADR 0018](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0018-l3-component-architecture.md)）。
2. 它不覆盖 AT-SPI 语义导出与真实屏幕阅读器（Q01 的人工部分）。
3. 运行测试的机器必须装有至少一种字体：文本经 cosmic-text 布局，没有字体时标签会渲染为空，
   像素断言会变成在断言一张空帧（CI 的 `test` 分片为此安装 `fonts-dejavu-core`）。

`waydriver` 仍在计划中，用于在虚拟合成器中做端到端 GUI 验证；它需要宿主支持组件所需的协议，
因此排在 layer-shell 宿主之后。

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
| L3 组件（七个） | 纯逻辑的单元测试 + `proptest` 属性测试（导航栈、快照/请求代次、任务分组是划分、缩放几何、搜索排序、通知队列的终态、锁状态机的安全不变量）；每个组件的 `tests/catalog.rs` 用 `compare_catalogs` 钉住 `en-US`/`zh-CN` 的 id 与占位符；六个有界面的组件另有 `tests/render.rs`（离屏渲染的像素与事件断言，见上一节） |
| L4 组件（`inox-session`、`inox-idle`、`inox-greet`） | 重启预算、进程组/注销/目录清理、Wayland 线协议 idle 通知、锁前挂起与登录状态；greeter 双主题/语言/字号离屏渲染；独立 CI job 验证真实 Inox headless 启动/注销。认证、锁屏与挂起 E3/E4 尚未完成 |
| 本站点（`docs/book/`） | 无单元测试要求。文档正确性由 CI 的 `book` job（`mdbook build`）保证，站点结构由 `SUMMARY.md` 与实际章节文件的一致性约束 |

## 新增行为时的要求

新增或修改行为必须同时新增/更新对应测试。具体地说：

- 新的纯逻辑 → 单元测试；属于核心算法（窗口管理、布局）的 → 补 `proptest` 不变量。
- 新的合成器能力 → 在 headless 路径上补集成测试，断言可观测状态而非日志。
- 新的 Shell 界面 → M4 起用 `waydriver` 在虚拟合成器（`weston --backend=headless`）中启动、截图并断言。
- 性能相关的改动若引入可测量的回退，需要在 PR 中给出 Tracy 证据（PRD §8）。
