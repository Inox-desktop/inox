# 里程碑与路线图

里程碑划分与验收标准（DoD）以 PRD §13 为准；当前状态以本仓库 [README 第 4 节](https://github.com/Inox-desktop/inox/blob/main/README.md)为准，组件实现现状以 `crates/<name>/` 中的代码为准。不一致时以它们为准。

## 里程碑表

| 里程碑 | 目标 | 涉及 crate | 验收标准（DoD） | 当前状态 |
|--------|------|------------|-----------------|----------|
| **M0** | 工程基线、CI 模板、`AGENTS.md`、`LICENSE` | 工作区骨架、`.github` | 骨架、约束、许可证与首次真实 CI 全绿 | **已完成** |
| **M1** | Headless 合成器：创建输出、渲染纯色、干净退出 | `inox-compositor` | CI 中 headless 创建输出并渲染帧，收到终止信号后退出码 0，clippy 无警告 | **已完成** |
| **M2** | 输入事件、多窗口、平铺布局 | `inox-compositor` | 处理键盘/指针事件；管理 ≥2 窗口；平铺布局通过 `proptest` | **已实现**（ADR 0008 仍为 `proposed`） |
| **M2.5** | 真实输出、输入与设备生命周期 | `inox-compositor` | CI 后端覆盖 + 人类真机（O01/O04、H01 建档），不以 headless 代真机 | 未开始 |
| **M3** | XWayland 集成、剪贴板、拖放 | `inox-xwayland`、`inox-compositor` | 真实 X11 往返/输入、双向 MIME、拖放接受/取消与异常退出（X01–X03） | **已实现**（ADR 0017 仍为 `proposed`）：Inox 会话内的 X11 客户端、焦点跟随剪贴板、Wayland↔Wayland 拖放与真实 X11 往返就绪判定均已落地；跨 X11/Wayland 边界的拖放被上游阻塞，X03 仅部分交付 |
| **M4** | Iced Shell：面板、启动器、设置 | L3 组件、`inox-compositor`、`inox-config` | W05、O02–O03、D01–D05、C01–C03、Q01–Q03 | 未开始 |
| **M5** | 会话管理、锁屏、通知、日常可用 | `inox-session`、`inox-idle`、`inox-lock`、`inox-notifications` | D06、S01–S05、H02 | 未开始 |
| **M6** | 发布候选：集成测试、性能、打包 | `inox-integration` 与全部 crate | C04、P01–P05、H01–H03、L01–L03 | 未开始 |

验收标准的完整前置、断言与证据要求见[验收规范](https://github.com/Inox-desktop/inox/blob/main/docs/quality/acceptance.md)；本表只概括，不维护第二份标准。

## 当前状态说明

M0 与 M1 已完成，M2 已实现，M3 已实现（X03 仅部分交付）：

- **M1**（[ADR 0005](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0005-headless-compositor-runtime.md)，状态 `accepted`）：自建 surfaceless EGL + `GlesRenderer` + 离屏渲染目标的 headless 渲染路径；创建 `wl_output`、渲染纯色帧、收到终止信号后退出码 0。
- **M2**（[ADR 0008](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0008-m2-input-windows-tiling.md)，状态 **`proposed`**）：在 M1 的 headless 路径上扩展，经 `wl_seat` 处理键盘与指针事件，以 `xdg-shell` toplevel 为窗口并由 Smithay `desktop::Space` 管理 ≥2 个窗口，按 master/stack 平铺（`proptest` 验证）。注意 ADR 本身尚未转为 `accepted` —— 实现已合入，决策记录仍是提案状态。
- **M3**（[ADR 0017](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md)，状态 **`proposed`**）：在 Inox 自己的合成器上完成 X11/数据互通。合成器广告 `wp_viewporter`、`wl_data_device_manager`（v3）与 `zwp_primary_selection_device_manager_v1`（经 Smithay 的 `wayland::viewporter` 与 `wayland::selection`），剪贴板的读取权**跟随键盘焦点**（`set_data_device_focus`），Wayland↔Wayland 拖放与 X11 内部拖放可用，每次渲染后发送 frame callback，`inox-xwayland` 的就绪判定改为真实的 X11 连接建立往返。E3 用例移入 `inox-integration`，以子进程方式驱动工作区构建的 `inox-compositor` 与 `inox-xwayland`（不新增 Cargo 依赖边）。两处边界必须说明：跨 X11/Wayland 边界的拖放**被上游阻塞**（`xwayland-satellite` v0.8.3 未实现 XDND），X02 的「锁屏时读剪贴板」断言与 `wlr-data-control` 推迟到 M5 —— 因此 **X03 只是部分交付**，M3 不得被读作 X03 完整通过。

此外，工程组织形态在 2026-10-16 发生变化：原多仓库策略（每组件一个 GitHub 仓库）被 [ADR 0016](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0016-single-repository-cargo-workspace.md) 取代，全部组件合并为单一 Cargo workspace。这不改变任何组件的功能完成度，但改变了「在哪里核对状态」——从「在对应仓库中核对」变为「在 `crates/<name>/` 中核对」。

## 仍未完成的部分

以下能力在已完成的里程碑范围内**没有**被覆盖，属于后续里程碑或尚未排期：

- **真实后端（M2.5 仍未完成）**：真实输入设备链路（libinput / udev / seat）与真实输出链路（DRM/KMS）都没有在运行中打通 —— M2 的输入事件经进程内注入验证，`crates/inox-compositor/src/backend/drm.rs` 是脚手架，后端特性已编译检查但无真机路径（ADR 0010、ADR 0012 均为 `proposed`）。
- **跨 X11/Wayland 边界的拖放**：`xwayland-satellite` v0.8.3 未实现 XDND，M3 只交付 Wayland↔Wayland 拖放与 X11 内部拖放；跨边界那条腿的 E3 用例延期，取代条件是「上游提供 XDND」或「Inox 更换宿主方案」（[ADR 0017](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md)）。
- **锁屏剪贴板与剪贴板管理器**：X02 的「锁屏时读剪贴板」断言移到 M5 的 S02（`ext-session-lock-v1` 与 `inox-lock` 尚不存在）；全局、无焦点限制的 `wlr-data-control` 需要按客户端凭据做授权过滤，同样推迟到 M5（[ADR 0017](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md)）。
- **`-listenfd` 按需激活**：M3 未纳入，`inox-xwayland` 仍是「在会话中启动并监督外部 satellite」的形态（[ADR 0007](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0007-xwayland-satellite-integration.md)）——就绪判定已改为真实的 X11 连接建立往返。
- **L3、L4 的全部组件**：`inox-shell`、`inox-panel`、`inox-launcher`、`inox-settings`、`inox-lock`、`inox-notifications`、`inox-session`、`inox-idle`、`inox-greet` 均为骨架（`src/main.rs` 只有日志初始化）。
- **配置接入消费方**：`inox-config` 已实现配置模型、解析与错误定位，但合成器目前只解析命令行参数，尚未读取配置文件；配置格式本身仍是 ADR 0009 的提案（`proposed`）。
- **`inox-integration` 的完整会话端到端测试**：目前有分层方向检查（验收项 B04）与 M3 的 XWayland/剪贴板/拖放 E3 用例；该用例以子进程方式驱动工作区构建的二进制，因此该 crate 仍不依赖任何 Inox crate，驱动完整会话的 E2E 按计划在 M6 落地。

## 组件版本

工作区统一版本为 `0.1.1`（根 `Cargo.toml` 的 `[workspace.package] version`），全部 crate 共享它。兼容性记录是工作区唯一的 [`Cargo.lock`](https://github.com/Inox-desktop/inox/blob/main/Cargo.lock)，不再有需要人工登记的兼容矩阵。

组件仓库时期打过的 `v0.1.0`（`inox-common`、`inox-compositor`、`inox-protocols`）与 `v0.1.1`（`inox-common`）标签保留在待归档的旧仓库中，不重写历史；`v0.2.0` 已被重切为 `v0.1.1` 并退役。
