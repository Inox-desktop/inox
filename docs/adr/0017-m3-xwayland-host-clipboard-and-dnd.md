# ADR 0017: M3 的 XWayland 宿主协议、剪贴板与拖放

- 状态：proposed
- 日期：2026-10-16
- 决策者：架构组

## 背景

PRD §13 把 M3 定义为「Inox 会话中的 X11/数据互通」，DoD 为 X01–X03：在 Inox 会话中启动真实 X11 客户端并切焦点、resize、关闭（X01）；Wayland↔X11 与 Wayland↔Wayland 双向剪贴板（X02）；拖放文本与 `text/uri-list`（X03）。X01 明文要求「就绪需实际 X11 往返，socket 存在不足以通过」，并规定**第三方宿主通过不算本阶段完成**；验收规范 §5 要求 X02/X03 的证据为 E2+E3 / E3，而 §1 规定 **E3 必须使用 Inox 自己的合成器**。

[ADR 0007](0007-xwayland-satellite-integration.md) 已决定 `inox-xwayland` 以**监督外部 `xwayland-satellite` 进程**的方式集成无根 XWayland（不链接其 crate），并把当时的范围限定为进程监督、就绪探测与生命周期；**剪贴板双向同步与拖放被明确排除，推迟到 M3**。本 ADR 记录 M3 如何在这一进程边界之上补齐 X01–X03。

以下上游事实由直接阅读上游源码与 README 核实（`xwayland-satellite` v0.8.3）：

- **宿主前提与版本下限**：satellite 要求宿主合成器实现核心接口、`xdg_shell` 与 **`wp_viewporter`**；其中 `wl_compositor` 硬性要求 **≥ 4**（它调用 `wl_surface.damage_buffer`），`xdg_wm_base` 硬性要求 **≥ 2**。`xdg_wm_base`、`wl_compositor`、`wl_shm`、`wl_seat`、`wp_viewporter` 任一缺失都会让它在启动时 **panic**。Smithay 0.7 广告的是 `wl_compositor` v5 / `xdg_wm_base` v6，两处下限在 Inox 侧已经满足。
- **可选能力**：dmabuf、xdg-activation、xdg-decoration、xdg-foreign、pointer-constraints、**primary selection**、tablet input 与分数缩放按「宿主是否广告」来决定是否启用（上游 README「Wayland protocols」）。
- **剪贴板实现**：satellite 用**标准 `wl_data_device_manager`** 加 `zwp_primary_selection_device_manager_v1` 实现选区（上游 `src/server/selection.rs`，经 smithay-client-toolkit）；它**不使用 `wlr-data-control`**。
- **焦点语义**：satellite 只是普通 Wayland 客户端，只在**自己是焦点客户端时**才收到选区事件（Smithay 只把选区事件投递给焦点客户端）。X11↔Wayland 粘贴因此依赖自然的焦点交接：先把焦点交给 X11 窗口，再粘贴。
- **无 XDND**：v0.8.3 中不存在任何 `XdndEnter` / `XdndDrop` / `XdndAware` 处理。跨 X11/Wayland 边界的拖放在「监督外部 satellite」架构下**不可能工作**。X11 内部的拖放（X 客户端 → X 客户端）在 Xwayland 内部原生可用，不需要 Inox 提供任何东西。

约束与依据：

- PRD §5.4 与 [ADR 索引](README.md)：引入新的协议策略必须先写 ADR 并获批。本 ADR 涉及的协议面来自上游宿主前提，不是能力扩张。
- [ADR 0002](0002-layering-and-dependency-direction.md) 规则 4 与 [ADR 0006](0006-protocol-strategy.md)：Wayland 协议绑定只能从 `inox-protocols` 出口。本里程碑**不新增协议 XML、也不新增 `inox-protocols` 绑定**：宿主侧实现全部来自 Smithay（`wayland::viewporter`、`wayland::selection::*`），而 `inox-protocols` 已经为消费方再导出 `wp::viewporter`；在没有真实消费方时继续加再导出，与「只引入有真实消费方的模块」的取向相悖。
- **装饰：M3 有意不广告 `zxdg_decoration_manager_v1`。** 上游的判断是 `needs_satellite_decorations = wl_decoration.is_none() && ...`：宿主不广告装饰协议时，satellite **自行绘制 X11 窗口的标题栏**。因此 M3 不必实现服务端装饰协商就能得到有装饰的 X11 窗口；反过来说，一旦广告该协议，回答装饰协商的责任就转移到 Inox 合成器身上，那属于后续里程碑的协议面扩张。
- **缓冲：M3 有意不广告 `wl_drm` 与 `zwp_linux_dmabuf_v1`。** satellite 只把宿主**实际拥有**的全局再暴露给 Xwayland，因此 Xwayland 会回落到 `wl_shm` 缓冲。这足以支撑 M3 的全部 DoD（正确性、输入、剪贴板、拖放都不要求零拷贝），dmabuf 属于后续里程碑。
- **X11 屏幕几何来自宿主的 `wl_output` 事件**：satellite 以 `-force-xrandr-emulation` 启动 Xwayland，屏幕尺寸由宿主的输出事件推导；宿主若从未发送带 `Current` 标志的 `mode`，X11 侧会得到 **0×0 屏幕**。M3 的 E3 用例断言真实几何，因此这条依赖是显式的，不能靠「窗口能映射」掩盖。
- 根级 `AGENTS.md` §3.1：不在本地编译/测试，证据一律来自 CI；E3 用例必须在 CI 的 Inox headless 会话中运行，不得以本地运行或第三方宿主替代。
- 验收规范 §1：「E3 必须使用 Inox 自己的合成器」「第三方 headless 合成器仅适合 E2」；§3：「测试注入接口必须限制在测试环境，不能成为默认公开的全局注入能力」。
- 根级 `AGENTS.md` §2/§8 与 ADR 0002：合成器与 Shell、各服务之间不通过 Rust 依赖耦合；新增 Cargo 依赖边必须能说明必要性，同层边还需 ADR 与登记。
- 根级 `AGENTS.md` §3.2：`unsafe` 默认禁止；本 ADR 的全部实现走 Smithay 安全 API，不新增例外。
- 根级 `AGENTS.md` §6 与验收规范 §5：剪贴板涉及数据完整性，必须覆盖拒绝、取消、退出与并发路径，而不是只覆盖顺利路径。

考虑过但未被采纳的方案：

- **在 M3 自行实现 `wlr-data-control`**：它是**全局、无焦点限制**的读取路径，主要消费方是剪贴板管理器。在没有按客户端凭据授权的前提下开放它，等于给任意客户端一个绕过焦点读取剪贴板的后门，与 X02 的「未授权客户端不可读取」直接冲突 —— 见「决策」第 3 条。
- **在 M3 自行实现 X11↔Wayland 拖放（XWM 级 XDND 桥）**：这正是 ADR 0007 明确拒绝的「重写上游成熟组件」——把 satellite 不做的部分搬进 `inox-xwayland`，意味着 Inox 要承担 XDND 状态机、数据代理与跨协议生命周期管理；且上游一旦补上 XDND 即为重复实现 —— 见「决策」第 5 条。
- **在 M3 广告 `zxdg_decoration_manager_v1`（服务端装饰）**：satellite 在宿主未广告装饰协议时自行绘制 X11 窗口标题栏，M3 因此不需要任何装饰协商代码；提前广告等于主动承担一条尚未准备实现的协商责任，并让「谁画标题栏」这件事在两份实现之间漂移 —— 见「决策」第 1 条。
- **在 M3 引入 dmabuf 以避免 `wl_shm` 回落**：M3 的 DoD 没有零拷贝或性能要求（合成器仍是离屏软件渲染），提前广告 dmabuf 会把缓冲导入/导出、修饰符协商与失败回退一并带进来，风险远超收益。
- **把剪贴板授权建立在客户端自报身份上**（如按 `xdg-shell` 的 app_id 或客户端自报名称放行）：客户端自报名称不构成授权（验收规范 S04），且比 seat 的焦点状态更弱、更易伪造。
- **M3 继续只在第三方 headless 宿主（weston）上验证端到端**：X01 已明文规定第三方宿主通过不算本阶段完成 —— 不成立；该测试保留为宿主无关的 E2 检查。
- **让 E3 用例在进程内调用合成器 API，或让 `inox-integration` 以 path 依赖链接各组件**：M3 的证据等级是 E3，需要驱动**运行中**的 Inox 会话（切焦点、向 X11 窗口输入、指针移动与按键用于拖放），进程内调用覆盖不到这条链路；而为了让测试能调用组件 API 而增加 Cargo 边，只会把 `graphics` 依赖拖进 `inox-integration` 的测试分片并改动层级表，买不到任何额外覆盖 —— 见「决策」第 7、8 条。
- **不补 frame callback 也断言「X11 窗口可见」**：真实客户端在缺少 frame callback 时不会可靠重绘，M1/M2 的断言（合成器内部状态 + 背景像素）无法区分「窗口被映射」与「窗口被画出来」—— 见「决策」第 6 条。

## 决策

1. **合成器广告 `wp_viewporter`、`wl_data_device_manager`（v3）与 `zwp_primary_selection_device_manager_v1`，实现经 Smithay 的 `wayland::viewporter` 与 `wayland::selection` 模块**，不手写协议实现。`wp_viewporter` 与 `xdg_shell` 是 satellite 的硬前提，`wl_data_device_manager` 承载剪贴板与拖放，`zwp_primary_selection_device_manager_v1` 提供 X11 主选区（中键粘贴）所需的协议面。**M3 有意不广告 `zxdg_decoration_manager_v1`、`wl_drm` 与 `zwp_linux_dmabuf_v1`**：装饰由 satellite 自行绘制（宿主无装饰协议时的既定行为），缓冲回落 `wl_shm`；三条都记在「约束与依据」中，广告其中任何一条都意味着同时承担它带来的协商与回退责任。
2. **键盘焦点是剪贴板的授权依据**：每次焦点变更都调用 `set_data_device_focus`，只有焦点客户端会被**授予**选区，也只有焦点客户端能**设置**选区（后者由 Smithay 在 `wl_data_device` 处理函数内拒绝非焦点客户端）。这是 Inox 对 X02「未授权客户端不可读取」的 M3 回答，且**不需要任何特权接口**——授权由 seat 的焦点状态给出，而不是由一个可被绕过的服务端开关给出。X11↔Wayland 粘贴依赖自然的焦点交接：焦点在 X11 窗口上时粘贴。

   **读取面的边界（实现期核实，2026-10-17）**：Smithay 不撤销客户端**已经持有**的 offer——焦点变更时 `send_selection` 只面向新的焦点客户端，失去焦点的客户端什么都收不到（把焦点设为 `None` 也一样），而 `wl_data_offer.receive` 不做焦点检查。所以 M3 交付的是「未获焦点的客户端**不会被授予**选区」，不是「授权被主动收回」；丢弃过期 offer 是客户端自身的义务，协议对此有明文（`wl_data_device.selection`：「the data_offer is valid ... until the client loses keyboard focus」）。让曾持焦点的客户端**失去**读取能力，需要服务端撤销已发出的 offer：Smithay 的 `selection` 模块没有该调用，但经 `DisplayHandle::backend_handle()` 加 wayland-server 的对象枚举是可达的（wlroots 走的就是「焦点变更时让过期 offer 失效」这条路线）。M3 不做这件事——它改变本条的授权机制，且须与推迟到 M5 的 `wlr-data-control` 授权策略一并决定，因此**需要新的 ADR**。X02 的通过结论不得据此写成「未授权客户端无法读取」。
3. **M3 有意不实现 `wlr-data-control`。** 它是全局、无焦点的读取路径，加进来就必须同时给出**按客户端凭据的授权过滤**，否则第 2 条的授权模型整体作废。该协议与剪贴板管理器一并推迟到 **M5**——锁屏与授权要求（S02/S04）也在那里落地；届时需要的是「谁能读剪贴板」的显式策略，而不是一个默认打开的全局路径。本 ADR 记录该推迟的推理与前置条件，不在此预先设计该策略。
4. **X02 中「锁屏时读剪贴板」的断言在 M3 不可验证**：`ext-session-lock-v1` 与 `inox-lock` 都尚不存在（M5）。M3 交付的是它的**结构性前提**——焦点门控的选区授予（见第 2 条的边界说明），锁屏 UI 在 M5 接管焦点后即可复用；该断言**移到 M5 的 S02**。这是明确的延期，不是通过。
5. **X03 的跨 X11/Wayland 边界拖放被上游阻塞，M3 只交付两条腿**：(a) 合成器内的 **Wayland↔Wayland** 拖放完整可用（accept/drop、reject/cancel、目标销毁、至多一次完成、grab 清理）；(b) **X11 内部**（X 客户端 → X 客户端）拖放继续原生可用。跨 X11/Wayland 边界那条腿的端到端用例**延期**，取代条件是「上游 satellite 提供 XDND」或「Inox 更换宿主方案」——两者都需要新 ADR 决定。因此 **M3 的 X03 只是部分交付**，不得被读作 X03 完整通过。
6. **合成器在每次渲染后发送 frame callback（`Window::send_frame`）。** 没有它，真实客户端（satellite 托管的 X11 toplevel、GTK 应用）无法可靠重绘，「X11 窗口可见」无法被确定性地复现。这补上了 M1/M2 留下的一处缺口——它们的测试只断言合成器状态与背景像素。与此相关的一条显式依赖：**X11 屏幕几何由宿主的 `wl_output` 事件给出**（satellite 以 `-force-xrandr-emulation` 启动 Xwayland），E3 用例断言真实几何（X11 侧屏幕不是 0×0），因此合成器必须持续发送带 `Current` 标志的输出 mode，而不是把几何留给客户端推断。
7. **新增一个仅用于测试的输入注入套接字**，由**非默认** cargo feature `test-input` 与 `--test-input <PATH>` 开关共同启用。理由是 M3 的证据等级（E3）要求驱动**运行中**的 Inox 合成器：切焦点、向 X11 窗口输入键盘、指针移动与按键用于拖放。约束：默认关闭；默认 feature 构建中不存在；feature 未开启时该选项被**明确报错拒绝**；测试中套接字位于私有 `XDG_RUNTIME_DIR` 下；它**不是 Wayland 全局**；除「能写入该套接字」外不授予任何权限；只为测试而存在。这满足验收规范 §3 既有规则「测试注入接口必须限制在测试环境，不能成为默认公开的全局注入能力」。**诚实的保留**：CI 的 `--all-features` 构建会把该代码编译进来，因此它在**运行期**默认关闭、在**构建期**默认不包含，但不能被证明在 all-features 构建中不存在。
8. **M3 的端到端（E3）用例移入 `inox-integration`，但以进程方式驱动组件，不新增任何 Cargo 依赖。** `inox-integration` 从测试可执行文件自身的路径定位同一 `target/<profile>/` 下由工作区构建的 `inox-compositor` 与 `inox-xwayland` 二进制，并把它们当作子进程驱动（配合第 7 条的注入套接字）。理由：E3 只要求用真实客户端驱动**锁定组件组合**，进程级驱动已经满足；为调用组件 API 而增加 Cargo 边不会带来额外覆盖，却会把 `graphics` 依赖拖进该 crate 的测试分片。因此 `inox-integration` 的依赖面**保持现状**（不依赖任何 Inox crate），层级表与依赖方向检查（B04）**不受影响**；CI 在运行 M3 job 前需要以 `--features inox-compositor/test-input` 构建合成器二进制。原基于 weston 的 `inox-xwayland` 测试**保留**为宿主无关的 E2 检查——X01 明确要求 Inox 会话且第三方宿主不算，所以它无法单独满足 M3，但仍是有效的宿主无关回归。
9. **`inox-xwayland` 的就绪判定改为真实的 X11 连接建立往返**（仍不链接任何 X11 库、M3 不引入 `-listenfd` 按需激活），以对齐 X01「socket 存在不足以通过」。这是对 ADR 0007「就绪探测」措辞的收紧而非取代：该 ADR 的「连接套接字成功」在实现层被明确为一次完整的连接建立往返。`-listenfd` 按需激活仍不在范围内。

## 后果

正面：

- **M3 的 DoD 在 Inox 自己的合成器上被覆盖**：X01 的真实 X11 往返与输入、X02 的剪贴板（Wayland↔X11 与 Wayland↔Wayland）、X03 的 Wayland↔Wayland 拖放都有 E3 用例，不再依赖第三方宿主。
- **授权模型简单且不可绕过**：焦点即授权，没有需要维护的开关、名单或特权服务；剪贴板读取面与用户可见的焦点一致。
- **协议面被有意收窄**：M3 只广告 DoD 需要的那三个协议，装饰与 dmabuf 都不广告 —— 上游对「宿主没有的能力」有既定回退（satellite 自绘标题栏、Xwayland 回落 `wl_shm`），因此这个收窄不付出功能代价，却避免了提前承担装饰协商与缓冲导入的责任。
- **依赖图保持干净**：全部协议面来自 Smithay 的安全 API，不手写协议、不引入新依赖、不新增 `unsafe` 例外；E3 用例以进程方式驱动组件，因此**不新增任何 Cargo 依赖边**，层级表与 B04 不受影响。
- **上游边界被显式记录**：跨边界拖放不可用不是 Inox 的实现缺陷，而是上游 satellite 缺失 XDND 的直接后果；记录后可以被追踪与复审，而不是被误当成待修 bug。
- **frame callback 补齐了 M1/M2 的验证缺口**：真实客户端的重绘从此可被复现，「窗口可见」不再是合成器内部状态的同义反复。

负面：

- **X03 只交付部分**：跨 X11/Wayland 边界的拖放在可预见的将来不可用，用户若在两个边界之间拖动会得到明确的无响应（既无接受也无取消事件），体验上会被读作「拖放不完整」。
- **延期项需要被持续跟踪**：X02 的锁屏断言移到 M5 的 S02，`wlr-data-control` 与剪贴板管理器推到 M5，跨边界拖放等待上游。三者都有明确的责任阶段，但都不会因为 M3 变绿而自动解决。
- **测试专用代码进入发布构建**：`test-input` feature 在 `--all-features` 构建中会被编译进来。运行期默认关闭、且不是全局能力，但「不存在」这一更强的性质无法在 all-features 构建中被证明。
- **E3 用例依赖工作区构建产物**：它需要能在同一 `target/<profile>/` 中定位到已构建的 `inox-compositor`（含 `test-input` feature）与 `inox-xwayland`；因此 CI 必须在运行前显式构建这两个二进制，缺失时用例必须**失败**而不是静默跳过（不能以「跳过」冒充通过）。
- **焦点门控剪贴板的固有代价**：剪贴板管理器类工具（后台读取全局选区）在 M3 不可能存在，这是有意为之，但也意味着 M5 之前没有剪贴板历史。
- **对上游回退行为的依赖被写进验证**：X11 几何依赖宿主持续发送带 `Current` 的输出 mode、装饰依赖 satellite 的自绘回退。这些是上游的行为，升级 satellite 或改变协议面时必须复核，否则可能得到 0×0 屏或双份标题栏。

影响面：本决策直接作用于 `crates/inox-compositor/**`（`wp_viewporter` / `wl_data_device_manager` v3 / `zwp_primary_selection_device_manager_v1` 的广告与实现、`set_data_device_focus` 的焦点跟随、Wayland↔Wayland 拖放、每帧 `send_frame`、`test-input` feature 与 `--test-input`）、`crates/inox-xwayland/**`（就绪判定改为真实 X11 连接建立往返）、`crates/inox-integration/**`（M3 的 E3 用例；以进程方式驱动工作区构建产物，**不新增依赖边**）与 `.github/workflows/ci.yml`（新增 M3 的 job：先以 `--features inox-compositor/test-input` 构建合成器与 `inox-xwayland`，再在私有 `XDG_RUNTIME_DIR` 下运行该用例）。**本里程碑不新增协议 XML，也不新增 `inox-protocols` 绑定**——宿主侧实现由 Smithay 提供（`wayland::viewporter`、`wayland::selection::*`），`inox-protocols` 已为消费方再导出 `wp::viewporter`，因此在没有真实消费方之前不改动该 crate。同样地，**本 ADR 不新增任何 Cargo 依赖边，也不改动 `[workspace.metadata.inox.layers]` 或 `same_layer_exceptions`。** 文档侧本次同步更新：`docs/adr/0017-m3-xwayland-host-clipboard-and-dnd.md`（新增）、`docs/adr/README.md`（0017 索引行）、`PRD.md`（§2.3 协议矩阵、§13 的 M3 脚注）、`README.md`（§4 的 M3 状态）、`docs/book/src/**`（`architecture/protocols.md`、`architecture/components.md`、`roadmap.md`、`usage/building.md`），以及 `docs/quality/acceptance.md` 的 §3/§5 说明性注释（X02 延期到 M5、X03 跨边界腿上游阻塞、注入套接字的门控方式），不改动任何阈值、通过条件或证据等级。

取代条件：需要新 ADR 的情形有三——(1) 上游 `xwayland-satellite` 提供 XDND（此时跨边界拖放转为可实现，须重写第 5 条并更新 PRD §13 的脚注与验收规范 §5 的 X03 注释）；(2) Inox 更换 XWayland 宿主方案（如自建 XWM 或以 `-listenfd` 做进程内集成，届时 ADR 0007 与本文第 1、9 条一并复核）；(3) M5 落地锁屏与剪贴板管理器时，第 3、4 条的延期项必须由一份具名 ADR 明确授权面与授权方式，不能以「推迟项到期」为由直接开放全局读取。
