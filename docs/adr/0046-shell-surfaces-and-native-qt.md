# 0046 — Shell 表面与原生 Qt：`inox-shell-v1`，以及面板/锁屏离开离屏渲染

- **状态**：proposed
- **日期**：2026-10-29
- **决策者**：YG（人类）/ Inox contributors
- **相关**：ADR 0002（分层规则；合成器与 Shell 只通过协议通信）、
  ADR 0018（L3 组件的独立性与 layer-shell 宿主）、ADR 0040（液态玻璃材质与组件）、
  ADR 0045（`inox-window-manager-v1` 与任务栏显示）、
  `docs/qml-migration-contract.md`（QML 迁移执行契约 §1 表面模式、§11 已知缺口）、
  `docs/quality/acceptance.md` D01 / D05 / Q01 / Q02 / S02 / S03 / S04、
  `crates/inox-protocols/protocol/inox-shell-v1.xml`、
  `crates/inox-panel/src/{offscreen.cpp,layer.rs}`、`crates/inox-lock/src/{offscreen.cpp,session.rs}`、
  `crates/inox-compositor/src/state.rs`

## 背景

`inox-panel` 与 `inox-lock` 走的是「**真实的协议表面 + 离屏的 QML 渲染**」：协议腿由 Rust 自己建一条
`wayland-client` 连接、自己用 `wl_shm` 提交 `Argb8888` 帧（`inox-panel/src/layer.rs`、
`inox-lock/src/session.rs`），而画面由 `QT_QPA_PLATFORM=offscreen` + software 场景图的
`QQuickWindow::grabWindow()` 渲染出来（`inox-panel/src/offscreen.cpp`）。这条路径当初是对的：它让
QML 迁移不必同时重写输入、放置与安全边界，也让像素断言能在 CI 里跑。

它现在到了上限，而且上限不是「再优化一下」能推开的——限制在架构上：

1. **没有表面，就没有表面之间的动画。** 离屏帧只在「状态变了 / configure / 指针命中 / buffer release」
   时提交一帧，帧与帧之间没有 `wl_surface.frame` 回调驱动的时间线，也没有合成器在绘制时插手的余地。
   面板内部能用 QML 动画，但面板**长成**菜单、菜单从按钮里**钻出来**、Dock 图标弹起后菜单接住——
   这些是跨表面的过渡，只有合成器能在中间插帧，而离屏路径连「我现在真的被显示了」都不知道。
2. **没有 popup，就没有子菜单。** 今天 panel 的点击是 Rust 按 `entries` 命中表算出来的，QML 不接输入；
   离屏路径没有第二个表面，所以菜单只能画在面板自己的矩形里并被裁掉。真正的子菜单需要独立表面、
   需要合成器在点击外部时先收到那次点击（grab）并吞掉它。
3. **帧的代价是双份的。** 每帧要渲染、`grabWindow()` 读回、必要时**再渲染两遍**（黑底、白底）反推
   alpha（Qt 6.8 之前 offscreen 后端的 backing store 硬编码 `RGB32`，见 `offscreen.cpp` 的
   `reconstruct_alpha`），然后走 `wl_shm` 上传。玻璃窗格要的是真实 alpha，这份代价付了整整一个里程碑。
4. **可访问性没有出口。** `docs/qml-migration-contract.md` §11 把「panel/lock 的离屏表面没有 AT-SPI
   语义桥」列为已知缺口：离屏 `QQuickWindow` 不在任何窗口系统里，Qt 的无障碍桥接不到它，Q01 因此
   不得声称通过。
5. **输入只有 Rust 一条路。** hover、按下、拖拽中的中间态都要靠 Rust 命中表 + 一条指针消息通道模拟，
   任何 QML 侧的 `MouseArea`/`HoverHandler` 都拿不到事件。

同时，**标准协议也不够高**，这不是 Qt 的错：

- `zwlr-layer-shell-unstable-v1` 能放置、能留独占区、能给键盘三种模式，但它**从不告诉客户端工作区
  是什么**（没有 `workarea` 事件，面板只能从配置推算，一旦别的东西占了空间就漂移）；
- `xdg_popup` 是「菜单」的模型：它在 `xdg_surface.get_popup` 时定死 positioner、抓取必须带一个来自
  真实输入事件的 serial、且必须在父表面映射之后才合法。面板的菜单**经常是键盘打开的、是状态变化
  打开的、需要在工作区变化时重排**——没有 serial 可交；
- layer 表面的 `keyboard_interactivity` 会被 popup **继承**，而 D01 要求面板普通展示不偷焦点，
  于是一个 `none` 的面板**永远无法**给它的菜单键盘焦点——键盘导航的菜单在这个组合下不可能实现；
- 玻璃（`inox-glass-v1`）的资格是「有 layer 角色」，因为合成器需要「已知桌面层」才敢模糊；
  popup 没有 layer 角色，于是菜单不能磨砂，或者和面板把同一片像素模糊两遍。

还有一条今天就已经成立的事实，解释为什么「layer-shell + xdg_popup」这条路在面板上走不通：
合成器实现了 `XdgShellHandler::new_popup`（窗口的 popup 会进 `PopupManager`，由 Smithay 的
`AsRenderElements for Window` 连同窗口一起画出来），但 `WlrLayerShellHandler` 只实现了
`new_layer_surface` / `ack_configure` / `layer_destroyed`——**`new_popup` 是 Smithay 的默认空实现**。
于是一个以 layer 表面为父的 `xdg_popup` 既不会被 `PopupManager` 追踪，也永远收不到 initial configure，
**永远不会映射**。也就是说：普通窗口的菜单是好的，layer 表面（今天的面板）的菜单从来没能出现过。
这条缺口**可以**补（实现 `new_popup` 即可，Smithay 的渲染路径本来就遍历 popup），但补上之后仍然只剩
一个没有键盘、没有工作区事件、没有合成器侧关闭与过渡的 popup——这正是下面决策 2 要解决的部分。

## 决策

### 1. 面板与锁屏变成**原生 Qt Wayland 客户端**

`inox-panel` 与 `inox-lock` 不再使用 `QT_QPA_PLATFORM=offscreen`，也不再自己提交 `wl_shm` 帧。
它们用 Qt 的 wayland 平台插件、EGL/GLES 场景图、真实的 `QQuickWindow`（每个表面一个进程，与今天
「一个进程一种表面」一致），由 `wl_surface.frame` 驱动绘制，硬件合成，玻璃窗格由合成器模糊。

这一条是其余所有决策的前提：只有真实表面才有 `frame` 回调、才有 popup、才有输入、才有 AT-SPI。

### 2. 新增私有协议 `inox-shell-v1`（`inox-protocols`，L2）

定义在 `crates/inox-protocols/protocol/inox-shell-v1.xml`，绑定由 `wayland-scanner` 在编译期生成
（`src/shell.rs`），与 `inox-ipc`、`inox-glass-v1`、`inox-window-manager-v1` 同一套形态。
三个接口：

- `inox_shell_v1`：`get_surface(surface, role)` 分配 shell 角色，`capabilities` 报告实现了哪些可选能力
  与最长过渡时长；
- `inox_shell_surface_v1`：`set_placement`（layer / anchor / 独占区 / 四个 margin / 尺寸，**一个原子
  请求**）、`set_keyboard`、`set_transition`、`get_popup`；事件 `configure`、`workarea`、
  `keyboard_state`、`closed`；
- `inox_shell_popup_v1`：`set_position`（父锚点 + 自身锚点 + 偏移）、`set_size`、`set_keyboard`、
  `set_dismissable`、`dismiss`、`set_transition`、`get_popup`（**嵌套即子菜单**）；事件 `configure`、
  `dismissed(reason)`、`keyboard_state`。

四条不可放宽的协议性质：

- **放置是一个请求，状态在 commit 时生效。** 六个 setter 会让合成器看到半个放置：先按旧 margin
  预留、再按新 margin 纠正，中间那一帧的独占区是错的。
- **关闭权在合成器，不在客户端。** popup 只声明「我可被关闭」，指针 grab、Escape、父表面消失之后
  的连锁关闭都是合成器的；客户端从 `dismissed(reason)` 得知发生了什么，并据此选退场动画。
  没有 serial 要交——shell 菜单本来就是从键盘、定时器或协议事件打开的。
- **动效是提示，没有「动画完成」事件。** 合成器可以因为减少动态偏好、没有渲染器、正在退出而
  整段丢掉过渡，shell 必须在「所有过渡都是瞬时」的前提下也好看。等待动画结束才提交下一状态的
  客户端，会在从不做动画的合成器上挂死。
- **协议不携带任何回读。** 没有 fd、没有图像、没有内容，所以它不是 S04 意义上的特权接口；
  `src/shell.rs` 的元数据测试钉住这一条。

### 3. 合成器实现 shell 角色、shell popup 的合成与过渡

三个阶段（见下「实施阶段」）。两点必须说清楚，免得把工作量记到错的地方：

- **标准 popup 的渲染不需要补。** Smithay 的 `AsRenderElements for Window` 与 `for LayerSurface`
  已经把 `PopupManager::popups_for_surface` 的元素连同父表面一起画出来，所以窗口的菜单今天是好的。
  缺的是 `WlrLayerShellHandler::new_popup`（layer 表面为父的 popup 不被追踪、不收 configure）。
  这条**不在 P1 的必经路径上**：Inox 自己的壳换用 `inox_shell_popup_v1` 之后不再依赖它；如果仍要支持
  「第三方面板弹出自己的 xdg_popup」，再单独补，并且要有自己的测试。
- **`inox_shell_popup_v1` 的合成是我们自己的责任。** 它不是一个 `PopupKind::Xdg`，`PopupManager`
  不认识它，所以层级、偏移、裁剪都得在 `scene_elements` 里显式做：popup 必须紧跟在它的父表面
  之后画（不能画在所有窗口之上，否则浮动窗口互相遮挡时菜单会挂到错的层级），偏移取
  `set_position` 定下的锚点关系而不是重新推算。

### 4. 锁屏**不**新增协议：`ext_session_lock_v1` 保持不动，只换渲染器

锁屏需要的是「原生表面 + 动画 + 语义树」，这些在换掉渲染器之后就有了；它**不需要**新的角色。
`ext-session-lock-v1` 承载的是安全语义（锁定事件必须在所有输出上先呈现一帧锁定画面才可发出、
锁屏期间普通客户端不得被合成、客户端崩溃不得自动解锁），把这套东西在一条 unstable 私有协议里
重新实现一遍，是拿 S02 去换动画——不划算，也不该由一次 UI 迁移顺手做。

因此：`inox-lock` 保留 `session.rs` 的 `ext_session_lock_v1` 客户端腿（认领、逐输出锁表面、
`locked`/`finished` 状态机不变），把 `offscreen.cpp` + `render.rs` 的帧管线换成 Qt 原生表面。
它的锁表面由 Qt 的 wayland 平台插件创建，角色由 crate 内的 C++ 集成按 `ext_session_lock_surface_v1`
的 configure 尺寸驱动（configure 的宽高是**硬要求**，不匹配即协议错误，这条在锁屏上不可放宽）。

一条安全约束同时写死：**锁屏不使用 IME**。Qt 会顺手接上 `text-input-v3`，而输入法进程是普通客户端；
密码框接受输入法组合串等于把口令经第三方进程过一遍。锁屏的按键继续走 `app::key_binding` 的原始
键路，QML 只上报掩码长度（迁移契约 §6 的规则不变）。

### 5. 输入与语义的归属：QML 收事件，Rust 仍然决定一切「是什么」

原生表面意味着指针/键盘事件由 Qt 送到 QML。这**推翻**了迁移契约里「QML 不接输入」那条，
但不推翻它的目的：

- **文案、颜色、尺寸、圆角、材质、焦点顺序、动作语义仍由 Rust 投影。** QML 里仍然禁止颜色与像素
  字面量、禁止拼用户可见句子（契约 §4 不变）。
- **动作仍是一个窄通道。** QML 侧 `dispatch('{"type":"activate","id":7}')`，Rust 解析成既有的
  `Message`，`app::update` 仍是唯一改状态的地方。
- **`entries` 命中表保留，但改变身份**：它从「Rust 用它算点击」变成「Rust 用它导出语义树与
  导航顺序」，并继续作为 QML 命中区域的**输入**（投影里给出每个条目的矩形，QML 的 `TapHandler`
  只负责把那块区域映射回 `id`）。这条是 Q01 的关键：Qt 的无障碍桥需要真实的控件树，而
  `Accessible.role/name` 的值来自 Rust 的投影。

### 6. 玻璃资格从「layer 角色」扩为「合成器已知桌面层的角色」

`inox-glass-v1` 的资格判断从 `layer_role` 改为「该表面有 `zwlr_layer_surface_v1` 或
`inox_shell_surface_v1` / `inox_shell_popup_v1` 角色」。理由与今天完全相同：合成器需要知道它画在
哪一层，才能确定模糊的是「它背后已经合成好的像素」。popup 的玻璃区域裁剪到 popup 自己，
不会与父表面重复模糊同一片像素——**同一片像素被两个玻璃对象各模糊一遍，是 v1 就有的 bug 面**，
本次不为它发明 group 语义，只把「一个表面一个对象、区域裁剪到该表面」这条原样应用到 popup 上。

### 7. 指针输入不再经 Rust 命中

`src/layer.rs` 的 `Input`/`Pointer`/`scene` 命中逻辑在面板切到原生表面后删除；点击由 QML 的
`TapHandler` 产生 `dispatch`，语义（左键激活/已聚焦最小化、中键关闭、右键循环模式）仍在 Rust 的
`Message` 与 `app::update` 里，与今天完全同一份代码。这条不是「把逻辑挪进 QML」，而是「把**坐标**
挪进 Qt」：坐标是 Qt 唯一比 Rust 更懂的东西。

### 8. 第二条连接保留，表面连接归 Qt

Qt 拥有表面所在的 `wl_display`；Rust 继续用自己的一条 `wayland-client` 连接做**不绑定表面**的事：

- `inox-window-manager-v1`（ADR 0045）的观察者与动词——它按连接发快照，第二条连接完全合法；
- `SIGHUP` / `org.inox.Panel` 重载（`reload.rs`，与会话 D-Bus 的既有边界）；
- 时钟定时器与 `calloop` 循环（`app::Message::Tick` 仍只有一处）。

**玻璃对象必须挂在 Qt 的那条连接上**，因为它绑定到具体的 `wl_surface`：
`inox_glass_manager_v1.get_glass(surface)` 的 surface 只能是同一连接创建的。因此玻璃的**决策**
留在 Rust（`frost.rs` + `inox-glass::decide`，含 clamp 与 v1/v2 协商），**发送**由 crate 内的 C++
集成经 Qt 连接完成，能力事件回传给 Rust 再决策一轮。这条往返只在启动与能力变化时发生，不在每帧。

### 9. `wlr-layer-shell` 继续广告，但不再是 Inox 自己的壳的路径

第三方面板/状态栏（waybar 之类）继续用 layer-shell，合成器支持不变。Inox 自己的 panel/menubar/dock
走 `inox-shell-v1`，**不**保留「拿不到 inox-shell 就退回 layer-shell」的路径：那会让两条放置代码
在半数机器上分叉，而这是本 ADR 明确要付代价避免的事。拿不到 `inox_shell_v1` 全局就是可读错误 +
非零退出（与今天「没有 layer 宿主就是失败，不是降级」同一条纪律）。

### 10. 私有 ABI 的授权边界（本 ADR 授权的全部内容）

面板/锁屏的 Qt 表面角色不可能由标准 Qt API 完成：`QWindow` 只会变成 `xdg_toplevel`。因此授权在
**crate 内、单文件、带版本闸门**地使用 QtWaylandClient 私有 ABI：

- 允许包含的私有头：`QtWaylandClient/private/qwaylandwindow_p.h`、`qwaylanddisplay_p.h`、
  `qwaylandinputdevice_p.h`、`qwaylandshellsurface_p.h`；
- 允许继承的私有类：`QWaylandShellSurface`（**唯一一个**）；
- 允许的私有操作只有三类：给窗口安装自己的 shell integration、取该窗口的 `wl_surface`、把
  configure 得到的尺寸交给该窗口。**具体符号在 P3 的 PR 里逐个列出**，并由 CI 的真实编译验证——
  本 ADR 不预先写死一份可能与实际 Qt 版本不符的清单，写死的清单会变成一种「文档说可以」的假保证；
- **禁止**：改写 `QWaylandIntegration` 之外的全局状态、任何 GPL-only 模块（Qt Wayland Compositor、
  Qt Virtual Keyboard、Qt QML Compiler）、以及让私有类型出现在任何 `.h` 或 Rust 边界上——所有私有
  调用集中在 `crates/inox-<crate>/src/native/qt_shell_surface.cpp` 一个翻译单元里。

三种闸门必须同时存在，缺一不可：

1. **编译期**：`#if QT_VERSION < QT_VERSION_CHECK(6, 5, 0) #error` —— 低于该版本 `QWaylandShellSurface`
   的可继承形态没有保证；
2. **运行期**：`qVersion()` 与编译期版本比较，主版本一致、次版本不低于编译期才继续，否则是可读错误；
3. **失败即退出**：初始化失败不退回 `offscreen`、不退回 `xdg_toplevel`，非零退出并在日志里点名
   缺失的那一步（与「没有 layer 宿主就是失败」同一条纪律）。

**CI 前置（这是一条真实的阻塞项，不是待办清单里的一行）**：`license-audit` 与其他 Qt 作业今天装在
Ubuntu noble 上，那里的 Qt 是 **6.4.2**，低于上面的闸门；私有头来自 `qt6-wayland-private-dev`
（Debian/Ubuntu 有该包，但要用它就得同时把 Qt 版本抬上去）。因此 **P3 阻塞在「CI 的 Qt 基线 ≥ 6.5」**
这件事上：要么换更新的基础镜像，要么用 `aqtinstall` 装一份固定版本的 Qt 并同步改 `license-audit`
的装机与 `ldd` 白名单。这一步必须单独评审，不能夹在面板 PR 里顺手改。`cargo-deny` 不受影响：私有头
不是 Cargo 依赖。

## 实施阶段

每一阶段都要能独立回退；在 P5 之前，离屏路径与原生路径在不同 crate 内共存，**不许**用运行时开关
在一个二进制里共存（那样两条路径都会缺测试）。

| 阶段 | 内容 | 通过条件 | 证据等级 |
|------|------|----------|----------|
| **P0（本次）** | `inox-shell-v1` 的 XML 与绑定、元数据测试与 socket 往返测试；本 ADR；迁移契约与 crate 文档更新 | `cargo test -p inox-protocols` 全绿；`rg -n "inox.shell" crates/inox-protocols` 命中 XML、`src/shell.rs` 与 `tests/shell_roundtrip.rs`；合成器与面板**行为不变** | E0+E1 |
| **P1** | 合成器广告 `inox_shell_v1`：角色分配、放置（复用 `layers::geometry`/`reserve`）、`configure`/ack、`workarea`、`closed`；**shell popup 的合成**（紧跟父表面之后的层级与锚点偏移）与合成器侧关闭（外部点击、Escape、父消失）、键盘授予 | `crates/inox-compositor/tests/shell_surface.rs`（真实 `wayland-client` 替身）：放置与独占区数值、popup 画在父之上且**不**盖住更上层表面、外部点击产生 `dismissed(outside)` 且被吞掉、`keyboard_state` 与授予一致；`protocol_surface.rs` 断言全局可绑定 | E1+E2 |
| **P2** | 过渡：`set_transition` 在 map/unmap 上由合成器插帧（淡入淡出、滑入、缩放，命名曲线），并遵守减少动态偏好 | 帧序列断言：中间帧的 alpha/偏移严格介于首末帧之间；减少动态时只有一帧 | E1+E2 |
| **P3** | 面板切原生 Qt：C++ 壳集成（`inox_shell_surface_v1` 角色 + `inox_shell_popup_v1` 子菜单）、玻璃经 Qt 连接、QML 接输入并 `dispatch`、删除 `offscreen.cpp`/`offscreen.rs`/`wl_shm` 帧管线/alpha 重建/指针命中。**前置：CI 的 Qt 基线 ≥ 6.5（§10），否则私有头闸门直接 `#error`** | `QT_QPA_PLATFORM=offscreen` 的 QML 加载测试仍跑；**真实合成器宿主**上：面板出图、点击落到面板、菜单开着时点击外部关闭、子菜单两级、玻璃窗格像素与 `glass.tint` 一致 | E2，E3 需真实宿主截图 |
| **P4** | 锁屏切原生 Qt：`ext_session_lock_v1` 不变，Qt 原生锁表面（逐输出），删除 `inox-lock` 的离屏帧管线；锁屏**不接 IME** | 多输出锁表面尺寸等于 configure、`locked` 前已有锁定帧、锁屏期间普通客户端像素不存在、崩溃仍锁定（既有断言全保留） | E2；S02 的认证腿仍缺 |
| **P5** | 清理：删除两条离屏路径的残留、`INOX_PANEL_OFFSCREEN_NO_ALPHA_GRAB` 钩子、契约 §1 的离屏条目；QML 投影从「整份 JSON 每变化重发」改为按区块更新 | `rg -n "offscreen|reconstruct_alpha|grabWindow" crates/` 零命中；投影测试仍绿 | E0+E1 |

**P3/P4 之前不得宣称的能力**（写在这里，防止后续 PR 顺手把它们说成已解决）：无缝跨表面 morph、
AT-SPI 语义树、IME 文本输入（锁屏永不）、真机截图证据（wayland driver 作业仍未接线）。

## 后果

**正面**

- 面板/锁屏拿到真实表面：`wl_surface.frame` 驱动的绘制、硬件合成、popup、输入、AT-SPI、IME（面板）
  全部变成「默认就有」，而不是逐项自造。
- 子菜单、点击外部关闭、键盘导航的菜单**第一次**在协议层面可行——今天的组合在 layer-shell 上是
  不可表达的。
- 每帧的双份渲染、`grabWindow` 读回、黑/白两次 alpha 反推整条管线删除，`wl_shm` 上传消失。
- 玻璃窗格不再需要「面板自己铺色带」的妥协：真实 alpha + 合成器模糊/折射。
- 工作区成为一条事件，而不是配置推算：`workarea` 让菜单与 launcher 的翻转/夹取有真实输入。

**负面 / 限制**

- **Qt 私有 ABI 会漂移。** 私有类布局每个 Qt 次版本都可能变，这是本 ADR 主动接受的维护成本；
  缓解手段只有版本闸门与单点隔离，没有「一劳永逸」。
- **合成器要多实现一套放置。** 复用 `layers.rs` 的纯函数，但 surface 状态机（configure/ack、
  独占区、unmap 重映射）是第二份，必须与 layer-shell 的那份保持同一套不变量，否则两个壳在同一块
  屏幕上行为不一致。
- **锁屏换渲染器是安全敏感改动。** `ext-session-lock` 的时序（锁定帧先于 `locked`）不能被 Qt 的
  首帧延迟破坏；P4 必须保留 `session.rs` 现有的逐输出确认逻辑。
- **两条连接。** 面板进程同时是 Qt 的一个 Wayland 客户端和 Rust 的另一个；窗口服务、重载、时钟在
  后者，表面与玻璃在前者。调试时要看两条连接，不能假设它们同步。
- **契约要改。** 迁移契约 §1「panel/lock 是离屏」、§3「QML 不接输入」与两个 crate 的 `AGENTS.md`
  都由本 ADR 改写；在 P3/P4 落地前，文档描述的是**目标**，实现仍是离屏——这中间的每个 PR 都要
  在文档里说清楚自己处在哪一段。

## 考虑过但未被采纳

- **继续离屏，但改成 GPU 离屏**（`QQuickRenderControl` + EGL/GBM，把 dmabuf 交给合成器）。省掉了
  `grabWindow`，却仍然是「一个表面」，仍然没有 popup、没有输入、没有 AT-SPI、没有表面间过渡。
  它优化的是本 ADR 要废弃的那条路。
- **直接用 `LayerShellQt`**。它解决的正是「给 QWindow 一个 layer 角色」，但止步于此：没有 popup 的
  合成器侧关闭、没有工作区事件、没有过渡、没有玻璃资格扩展。用一个 KDE 的库换来一个仍然要写
  私有协议的状态，不如把那一层自己写清楚。
- **fork / vendored 一份 QtWaylandClient**。可控性最高，代价是把 Qt 的版本升级变成一份长期维护的
  patch 集，且在 LGPL 下还要维护修改声明。私有头 + 版本闸门是同一目标下更小的承诺。
- **让面板继续用 `xdg_popup`**（合成器补上 layer 表面的 popup 支持，Qt popup 窗口自然可用）。
  这条路能出子菜单，但菜单**没有键盘**（父表面 `none` 时 popup 继承 `none`），也无法在工作区变化
  时由合成器重排；而这两条是「像样的菜单」的全部难点。
- **把锁屏也搬进私有协议**。见决策 4：安全语义不该为了动画重写一遍。
- **把玻璃并进 `inox-shell-v1`**。两个协议各自只有一个职责，`inox-glass-v1` 已被面板、未来的通知栈
  与测试共用；合并会让「一个表面一个对象、区域裁剪、上限钳制」这些安全性质与角色状态机纠缠。
- **用 D-Bus 传工作区/动效提示**。这些事件与表面生命周期同源，走 Wayland 连接天然有序；跨到总线
  会引入「表面已销毁而消息仍在途」的竞态。

## 取代条件

出现以下任一情况时，本决策应被重新评估或取代：

- 上游出现能表达「合成器侧 shell 过渡 + popup 键盘路由 + 工作区事件」的标准协议
  （例如 layer-shell 的继任者或 `ext-` 系列补齐），则应迁移并让 `inox-shell-v1` 退役；
- Qt 私有 ABI 的维护成本超过收益（例如 Qt 7 移除 `QWaylandShellSurface` 的可继承性），则应评估
  「自带一个最小 wayland 表面层 + QML 渲染」的替代，而不是继续加私有钩子；
- Inox 决定把面板做成第三方可替换组件：那时第三方不该被要求实现私有协议，layer-shell 的支持必须
  重新成为一等公民，本 ADR 的「不退回 layer-shell」需要改写。
