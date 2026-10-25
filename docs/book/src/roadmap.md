# 里程碑与路线图

里程碑划分与验收标准（DoD）以 PRD §13 为准；当前状态以本仓库 [README 第 4 节](https://github.com/Inox-desktop/inox/blob/main/README.md)为准，组件实现现状以 `crates/<name>/` 中的代码为准。不一致时以它们为准。

## 里程碑表

| 里程碑 | 目标 | 涉及 crate | 验收标准（DoD） | 当前状态 |
|--------|------|------------|-----------------|----------|
| **M0** | 工程基线、CI 模板、`AGENTS.md`、`LICENSE` | 工作区骨架、`.github` | 骨架、约束、许可证与首次真实 CI 全绿 | **已完成** |
| **M1** | Headless 合成器：创建输出、渲染纯色、干净退出 | `inox-compositor` | CI 中 headless 创建输出并渲染帧，收到终止信号后退出码 0，clippy 无警告 | **已完成** |
| **M2** | 输入事件、多窗口、平铺布局 | `inox-compositor` | 处理键盘/指针事件；管理 ≥2 窗口；平铺布局通过 `proptest` | **已实现**（ADR 0008 仍为 `proposed`） |
| **M2.5** | 真实输出、输入与设备生命周期 | `inox-compositor` | CI 后端覆盖 + 人类真机（O01/O04、H01 建档），不以 headless 代真机 | 单输出物理后端已实现，CI 验证逻辑边界；E4 真机验收未执行 |
| **M3** | XWayland 集成、剪贴板、拖放 | `inox-xwayland`、`inox-compositor` | 真实 X11 往返/输入、双向 MIME、拖放接受/取消与异常退出（X01–X03） | **已实现**（ADR 0017 仍为 `proposed`）：Inox 会话内的 X11 客户端、焦点跟随剪贴板、Wayland↔Wayland 拖放与真实 X11 往返就绪判定均已落地；跨 X11/Wayland 边界的拖放被上游阻塞，X03 仅部分交付 |
| **M4** | Shell（面板、启动器、设置）：QML + Qt Quick | L3 组件、`inox-compositor`、`inox-config` | W05、O02–O03、D01–D05、C01–C03、Q01–Q03 | **部分交付**：七个 L3 组件的界面已迁到 QML + Qt Quick（[ADR 0033](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0033-qml-qt-quick-shell-evaluation.md)/[0035](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0035-cxx-qt-binding.md)/[0036](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0036-component-migration-plan.md)/[0038](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0038-offscreen-qml-for-layer-and-lock-surfaces.md)），面板与锁屏保留真实 layer / 会话锁表面；证据仍只到 E0/E1（投影断言、QML 加载、离屏帧读回）。E2/E3 仍缺：真实合成器宿主上的 waydriver 截图 job **未接线**，离屏的面板与锁屏没有 AT-SPI 语义桥。见 ADR 0039 的 M10 |
| **M5** | 会话管理、锁屏、通知、日常可用 | `inox-session`、`inox-idle`、`inox-lock`、`inox-notifications` | D06、S01–S05、H02 | 进行中：会话锁协议腿两侧已实现（ADR 0022）、锁屏界面已绘制到锁表面（[ADR 0025](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0025-lock-screen-surface-ui-and-login-interface.md)），L4 监督与空闲/登录界面见 ADR 0021；真实认证已由 [ADR 0029](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0029-inox-authentication-boundary.md) 落地（`inox-auth` + `auth-v1` + 登录界面口令字段）；**greeter 宿主、锁屏的键鼠输入路径与解锁接入、挂起、完整会话 ready 与 H02 真机未完成 |
| **M6** | 发布候选：集成测试、性能、打包 | `inox-integration` 与全部 crate | C04、P01–P05、H01–H03、L01–L03 | 未开始 |

验收标准的完整前置、断言与证据要求见[验收规范](https://github.com/Inox-desktop/inox/blob/main/docs/quality/acceptance.md)；本表只概括，不维护第二份标准。

## 当前状态说明

M0 与 M1 已完成，M2 已实现，M3 已实现（X03 仅部分交付），M4 部分交付：

- **M1**（[ADR 0005](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0005-headless-compositor-runtime.md)，状态 `accepted`）：自建 surfaceless EGL + `GlesRenderer` + 离屏渲染目标的 headless 渲染路径；创建 `wl_output`、渲染纯色帧、收到终止信号后退出码 0。
- **M2**（[ADR 0008](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0008-m2-input-windows-tiling.md)，状态 **`proposed`**）：在 M1 的 headless 路径上扩展，经 `wl_seat` 处理键盘与指针事件，以 `xdg-shell` toplevel 为窗口并由 Smithay `desktop::Space` 管理 ≥2 个窗口，按 master/stack 平铺（`proptest` 验证）。注意 ADR 本身尚未转为 `accepted` —— 实现已合入，决策记录仍是提案状态。
- **M3**（[ADR 0017](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md)，状态 **`proposed`**）：在 Inox 自己的合成器上完成 X11/数据互通。合成器广告 `wp_viewporter`、`wl_data_device_manager`（v3）与 `zwp_primary_selection_device_manager_v1`（经 Smithay 的 `wayland::viewporter` 与 `wayland::selection`），剪贴板的读取权**跟随键盘焦点**（`set_data_device_focus`），Wayland↔Wayland 拖放与 X11 内部拖放可用，每次渲染后发送 frame callback，`inox-xwayland` 的就绪判定改为真实的 X11 连接建立往返。E3 用例移入 `inox-integration`，以子进程方式驱动工作区构建的 `inox-compositor` 与 `inox-xwayland`（不新增 Cargo 依赖边）。两处边界必须说明：跨 X11/Wayland 边界的拖放**被上游阻塞**（`xwayland-satellite` v0.8.3 未实现 XDND），X02 的「锁屏时读剪贴板」断言与 `wlr-data-control` 推迟到 M5 —— 因此 **X03 只是部分交付**，M3 不得被读作 X03 完整通过。

- **M4**（[ADR 0018](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0018-l3-component-architecture.md)，状态 **`proposed`**，其 UI 栈部分已由 ADR 0033/0035/0036/0038/0039 取代）：L3 的七个组件——`inox-shell` 的跨入口导航与服务契约、`inox-panel` 的任务/菜单栏/Dock/托盘/时钟、`inox-launcher` 的 XDG 条目解析与搜索、`inox-notifications` 的协议契约与队列策略、`inox-settings` 的草稿/保存/回读、`inox-lock` 的锁状态机与凭据清理、`inox-wallpaper` 的缩放与原子应用；`inox-config` 的 `shell.toml`、`notifications.toml` 与 `config.toml` 的 `[locale]` 随各自消费者落地。界面已全部从 Iced 迁到 **QML + Qt Quick**（经 cxx-qt 绑定）：Rust 保留领域模型、Fluent 文案与令牌，QML 渲染 JSON 投影；面板与锁屏是离屏渲染，仍持有真实 layer / 会话锁表面。**证据只到 E0/E1**：投影断言、`QT_QPA_PLATFORM=offscreen` 下的 QML 加载测试，以及面板/锁屏读回自己提交的 `wl_shm` 帧。E2/E3 的缺口是硬前提而非排期：真实合成器宿主上的 waydriver 截图 job 尚未接线（ADR 0036 的 M02、ADR 0039 的 M10），离屏的面板与锁屏也没有 AT-SPI 语义桥。因此 W05、O02–O03、D01–D05 与 Q01–Q03 的宿主侧证据**均未取得**，M4 的 DoD 不成立。历史备注：迁移前 E2/E3 被两层阻塞，一是合成器尚未广告 layer-shell，二是第三方 `iced_layershell` 0.19.1/0.19.0 无法编译——第一条已补齐，第二条随 Iced 退出依赖图而失效。合成器侧的 O02/O03 只取得了虚拟拓扑与像素的 E1+E3 证据，真机部分见下一节。锁屏的协议腿另受 ADR 0015 的安全审查约束，通知的 D-Bus 服务属 M5。

此外，工程组织形态在 2026-10-16 发生变化：原多仓库策略（每组件一个 GitHub 仓库）被 [ADR 0016](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0016-single-repository-cargo-workspace.md) 取代，全部组件合并为单一 Cargo workspace。这不改变任何组件的功能完成度，但改变了「在哪里核对状态」——从「在对应仓库中核对」变为「在 `crates/<name>/` 中核对」。

## 仍未完成的部分

以下能力在已完成的里程碑范围内**没有**被覆盖，属于后续里程碑或尚未排期：

- **真实后端的 E4 验收**：libseat、DRM/GBM、udev/libinput 路径已实现，启动期读取输入、平铺和主题配置，提供软件光标与明确的帧生命周期。ADR 0012 已于 2026-10-17 获维护者批准；CI 验证状态机及共享逻辑，无法证明真实翻页/VT/设备恢复。O01/O04 的 E4 未执行，M2.5 仍不能宣称通过。多输出拓扑与多 GPU 设备模型（[ADR 0019](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0019-multi-output-topology-and-multi-gpu.md)）及分数缩放（[ADR 0020](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0020-fractional-scaling.md)）已实现并在 CI 取得虚拟拓扑与像素证据（O02 的双屏迁移与 20 轮拔插、O03 的 1.5x 物理落点，E1+E3）；二者的 E4 真机步骤、旋转、PRIME 跨 GPU 缓冲复制路径与显示试用仍未交付，两份 ADR 状态均为 `proposed`。
- **跨 X11/Wayland 边界的拖放**：`xwayland-satellite` v0.8.3 未实现 XDND，M3 只交付 Wayland↔Wayland 拖放与 X11 内部拖放；跨边界那条腿的 E3 用例延期，取代条件是「上游提供 XDND」或「Inox 更换宿主方案」（[ADR 0017](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md)）。
- **锁屏剪贴板与剪贴板管理器**：X02 的「锁屏时读剪贴板」断言已随 ADR 0022 的协议腿落地——`crates/inox-compositor/tests/session_lock.rs` 断言锁定期间普通客户端的选区获取被拒、解锁后同一调用被授予；撤销已发出的 offer 与全局 `wlr-data-control` 仍未实现。全局、无焦点限制的 `wlr-data-control` 需要按客户端凭据做授权过滤，同样推迟到 M5（[ADR 0017](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md)）。
- **`-listenfd` 按需激活**：M3 未纳入，`inox-xwayland` 仍是「在会话中启动并监督外部 satellite」的形态（[ADR 0007](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0007-xwayland-satellite-integration.md)）——就绪判定已改为真实的 X11 连接建立往返。
- **真实宿主上的 L3 证据（仍阻塞 L3 的 E2/E3）**：`inox-compositor` 已接入 `zwlr_layer_shell_v1` v4 并提供真实客户端的协议与像素测试；`inox-panel` 也确实创建真实 layer 表面、`inox-lock` 创建真实会话锁表面（界面都是离屏 QML 渲染，[ADR 0038](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0038-offscreen-qml-for-layer-and-lock-surfaces.md)）。缺的不是协议腿，而是**真实合成器宿主上的 waydriver job**：它要启动这些 L3 表面、截图并断言，目前**尚未接线**（[ADR 0036](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0036-component-migration-plan.md) 的 M02、[ADR 0039](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0039-qml-migration-completion-and-private-abi.md) 的 M10）。启动器、通知、壁纸与设置按 ADR 0038 是普通 Qt 窗口，它们的放置策略虽已建模测试，但没有宿主会应用（这是选定的形态，不再算阻塞）。
- **锁屏**：协议腿两侧都已实现（[ADR 0022](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0022-m5-session-lock-notification-bus-and-first-privilege-boundary.md)，状态 `proposed`）——合成器广告 `ext_session_lock_manager_v1`，并以「会话令牌 + `inox_ipc` 认领」默认拒绝授权；`inox-lock --take-lock` 会真实取锁、遮蔽每个输出，并把锁屏界面画上去（[ADR 0025](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0025-lock-screen-surface-ui-and-login-interface.md)，`accepted`）。界面由 crate 自己的状态机驱动，一个输出一个离屏 QML 渲染器，提交的帧就是 `headless_tests/protocol.rs` 读回并断言的那一帧。**仍未完成的**是认证边界（PAM/helper 权限、凭据传递与销毁），它同时决定键鼠输入路径——没有可用边界时屏幕上没有可操作的东西，输入路径会是不可能执行的代码；这两件事都要等 ADR 0015 要求的人类安全评审。此外**锁屏 UI 崩溃后不支持接管**，会话会保持锁定到注销为止（理由与恢复条件见 ADR 0022）。
- **通知的 D-Bus 服务**：协议契约、队列与脱敏策略已实现并测试，`org.freedesktop.Notifications` 的实际服务端按 PRD §13 属 M5（D06）。
- **配置接入消费方**：L3 组件已通过 `InoxConfig` 读取 `config.toml`、`shell.toml`、`notifications.toml` 与主题；**合成器真实会话已消费启动配置，运行期重载与设备应用回执仍未完成**。配置格式本身仍是 ADR 0009/0014 的提案（`proposed`）。
- **M5 的一部分已落地**：会话锁的**协议腿两侧**（[ADR 0022](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0022-m5-session-lock-notification-bus-and-first-privilege-boundary.md)，`proposed`）——合成器广告 `ext_session_lock_manager_v1` 并以会话令牌默认拒绝授权，`inox-lock --take-lock` 真实取锁并覆盖每个输出，锁屏 UI 崩溃保持锁定且**不支持接管**；`inox-ipc` 升到 v2，新增锁屏角色认领与锁状态广播；`inox-session` 生成并只向可信锁屏客户端投递令牌。证据是 E1+E2（两侧各自的真实客户端/真实服务端 peer 测试）与一条 E3 会话用例。通知的 D-Bus 服务（D06）也已落地：`org.freedesktop.Notifications` 由 `zbus` 传输线程提供，18 个 p2p 真实 D-Bus 用例覆盖 Notify/替换/关闭/动作/超时/勿扰/锁屏脱敏与每秒 100 条持续 10 s；**仍未交付**：真实认证、锁屏的键鼠输入路径、logind 挂起、完整会话 ready 握手、会话总线上的名字所有权测试、idle 侧的锁状态订阅，以及 H02 的全部真机步骤。锁屏界面本身已由 [ADR 0025](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0025-lock-screen-surface-ui-and-login-interface.md) 交付，表面承载方式见 [ADR 0038](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0038-offscreen-qml-for-layer-and-lock-surfaces.md)：`inox-lock` 用离屏 QML 把界面渲染成 configure 尺寸的 `wl_shm` 帧并提交给合成器，`headless_tests/protocol.rs` 读回该帧断言它是不透明的多色画面，默认路径则另有一个非零退出的拒绝用例。
- **L4 目前已不是骨架**：`inox-session`、`inox-idle`、`inox-greet` 均有实现，边界见 [ADR 0021](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0021-l4-session-services.md)（`proposed`）；`inox-greet` 的登录界面已补全为六个阶段各有画面、键盘可达、禁用原因可读（测试覆盖全部六个阶段；`--preview` 只显示本构建真实处于的那一个——没有认证边界，所以是不可用、不可输入的那一幅，默认仍拒绝真实登录），身份按系统账户库选择、界面不提供手工输入账户名（[ADR 0032](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0032-greeter-account-selection-and-login-interface.md)，`proposed`）；完整服务 ready 握手仍未实现。
- **`inox-integration` 的完整会话端到端测试**：目前有分层方向检查（验收项 B04）与 M3 的 XWayland/剪贴板/拖放 E3 用例；该用例以子进程方式驱动工作区构建的二进制，因此该 crate 仍不依赖任何 Inox crate，驱动完整会话的 E2E 按计划在 M6 落地。

## 组件版本

工作区统一版本为 `0.1.1`（根 `Cargo.toml` 的 `[workspace.package] version`），全部 crate 共享它。兼容性记录是工作区唯一的 [`Cargo.lock`](https://github.com/Inox-desktop/inox/blob/main/Cargo.lock)，不再有需要人工登记的兼容矩阵。

组件仓库时期打过的 `v0.1.0`（`inox-common`、`inox-compositor`、`inox-protocols`）与 `v0.1.1`（`inox-common`）标签保留在待归档的旧仓库中，不重写历史；`v0.2.0` 已被重切为 `v0.1.1` 并退役。
