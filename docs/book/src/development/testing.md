# 测试策略

测试要求以 PRD §7 与根级 `AGENTS.md` §6 为准。核心取向是：**核心逻辑必须能在 headless 环境被测试**，副作用集中在系统边界层，且**不得 mock 掉被测的核心逻辑** —— 只在 DRM、输入设备、D-Bus 这类边界打桩。

## 测试类型与适用范围

| 测试类型 | 工具 | 范围 | 适用范围 |
|----------|------|------|----------|
| 单元测试 | `cargo test` | 所有纯逻辑模块 | 每个 crate 都必须有 |
| 属性测试 | `proptest` | 窗口管理与布局算法 | 合成器、Shell 的布局与窗口簿记 |
| headless 集成测试 | `cargo test` + 软件渲染 | 合成器输出创建、渲染、输入、协议 | `inox-compositor` |
| GUI 测试 | `waydriver` | Shell 组件的交互与外观 | L3 组件（M4 起） |
| 性能测试 | Tracy | 帧时间、输入延迟 | 性能预算验证（PRD §8） |
| 依赖安全 | `cargo-deny` | 许可证、漏洞、重复版本 | 工作区 CI 的 `deny` job（完整依赖图） |
| 静态检查 | clippy、fmt | 全部代码 | 工作区 CI 的 `clippy` / `fmt` job |

覆盖率的默认要求是核心 crate 行覆盖 ≥ 70%；每个 crate 可以在自己的 `AGENTS.md` 中收紧，不得放宽到 0。

## 为什么是这套组合

- **纯逻辑与副作用分离**：合成器的输入判定、窗口簿记与平铺几何都是不依赖 Smithay 运行时状态的纯函数，因此可以用普通单元测试与 `proptest` 覆盖，不需要真实设备。副作用（协议、渲染、事件循环）留在边界层，由集成测试覆盖。
- **不用 mock**：`tests/window_management.rs` 在**同一进程内**用 `UnixStream::pair()` 连接一个真实的 `wayland-client`，创建 ≥2 个 xdg toplevel，断言合成器可观测状态（托管窗口数、`Space` 中的平铺落点、焦点）。被验证的是真实协议处理路径，而不是桩。`wayland-client` / `wayland-protocols` 只作为 dev-dependency，不进入发布产物。
- **断言要看得见**：渲染是否真的发生由**离屏像素读回**断言，不能只依赖日志。像素比较留 ±1 个 8 位步长的容差，因为不同驱动的 float → unorm 舍入不同。
- **CI 无设备**：PRD §6.1 要求 CI 在没有显示设备与输入设备的环境中可复现，因此 M2 的输入通过进程内注入 API 驱动，真实输入设备链路（libinput/udev）推迟到具备后端能力的里程碑。

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
| 其余 crate | 仍是骨架，尚无业务逻辑可测 |
| 本站点（`docs/book/`） | 无单元测试要求。文档正确性由 CI 的 `book` job（`mdbook build`）保证，站点结构由 `SUMMARY.md` 与实际章节文件的一致性约束 |

## 新增行为时的要求

新增或修改行为必须同时新增/更新对应测试。具体地说：

- 新的纯逻辑 → 单元测试；属于核心算法（窗口管理、布局）的 → 补 `proptest` 不变量。
- 新的合成器能力 → 在 headless 路径上补集成测试，断言可观测状态而非日志。
- 新的 Shell 界面 → M4 起用 `waydriver` 在虚拟合成器（`weston --backend=headless`）中启动、截图并断言。
- 性能相关的改动若引入可测量的回退，需要在 PR 中给出 Tracy 证据（PRD §8）。
