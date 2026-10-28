# 0045 — 多窗口管理与任务栏显示：`inox-window-manager-v1`、窗口模式与工作区

- **状态**：proposed
- **日期**：2026-10-28
- **决策者**：YG（人类）/ Inox contributors
- **相关**：ADR 0008（M2 的输入事件、多窗口与平铺布局；当前只有索引条目与实现，正文 `docs/adr/0008-*.md` 在本仓库中不存在）、
  ADR 0002（分层规则 1：禁止同层依赖；规则 3：合成器与 Shell 只通过协议/D-Bus 通信）、
  ADR 0018（L3 组件的独立性与 layer-shell 宿主）、ADR 0017（M3 的 XWayland 宿主）、
  `docs/quality/acceptance.md` W05 / D03 / W01–W03、
  `crates/inox-protocols/protocol/inox-window-manager-v1.xml`、
  `crates/inox-compositor/src/{layout,workspace,state}.rs`、
  `crates/inox-panel/src/{tasks,contract,app}.rs`

## 背景

W05 要求「窗口在平铺/浮动、最大化、全屏、最小化与至少 2 个工作区之间切换；从任务栏恢复」，
D03 要求面板的任务栏「任务数量/分组/激活态与合成器一致」。今天这两条都不成立，原因不同：

**合成器侧**：`Workspace` 只有「窗口 + 它属于哪个输出 + 布局顺序」三件事
（`crates/inox-compositor/src/workspace.rs`）。没有浮动、没有最大化、没有全屏、没有最小化，
也没有工作区——`Space` 里一个窗口一个 tile，平铺是唯一形态。`xdg-shell` 的
`maximize_request` / `fullscreen_request` / `minimize_request` / `move_request` / `resize_request`
走的是 Smithay `XdgShellHandler` 的**默认空实现**，因此客户端点最大化按钮什么都不会发生，
而且它会一直以为自己没有最大化。`sync_window_states` 只推 `Activated` 一个位。

**面板侧**：`inox-panel` 的任务模型（`tasks.rs`）与状态机（`app.rs`）**已经完整实现且有测试**——
分组、激活态、稳定顺序、代次、每窗口至多一个在途请求、`OutgoingCommand::{Snapshot, Activate, Toggle}`。
缺的是**传输**：`app.rs` 的模块文档写着「The transport does not exist yet」，`layer.rs` 把
`take_outgoing()` 取出的命令记录一条 `info!` 后丢弃。面板因此永远停在 `Connecting`，
任务栏上是空的。D03 的坑不在模型，在模型与合成器之间那段没人写的线。

这中间没有协议可用：`xdg-shell` 不允许一个客户端查询另一个客户端的 toplevel，
`inox-ipc` 是「每个连接绑一次」的会话级控制全局（版本查询、配置重载、锁角色、锁状态广播），
不是逐观察者的订阅通道。面板要的不是合成器的全部状态，而是四个事实与几个动词：

- **事实**：哪些窗口存在、它们叫什么（`app_id` + `title`）、在哪个工作区/输出上、哪个有焦点；
- **动词**：激活、最小化、关闭、切换模式（平铺/浮动/最大化/全屏）、移到工作区、切换工作区。

这些正好是任务栏已经建模、`app::Message` 已经定义、而 `layer.rs` 无处可送的那批消息。

## 决策

### 1. 新增私有协议 `inox-window-manager-v1`（`inox-protocols`，L2）

定义在 `crates/inox-protocols/protocol/inox-window-manager-v1.xml`，绑定由 `wayland-scanner`
在编译期生成（`src/window_manager.rs`），与 `inox-ipc`、`inox-glass-v1` 同一套形态。
两个接口：

- `inox_window_manager_v1`（v1）只有一个请求 `get_window_list`，每个调用创建一个观察者；
- `inox_window_list_v1`（v1）承载快照与增量事件（`session_generation`、
  `workspace_state`、`window`、`window_removed`、`snapshot_done`、`request_failed`），
  以及六个动词（`activate`、`minimize`、`close`、`set_mode`、`move_to_workspace`、
  `switch_workspace`）。

三条不可放宽的协议性质：

- **一条请求由状态事件确认，不由应答确认。** `window` / `window_removed` *就是*确认，
  `request_failed` 是唯一的否定结果。合成器在改动真正生效前发「已完成」是在撒谎，
  而客户端只应显示它回读到的效果（交互契约规则 2）。
- **句柄只在它的 `session_generation` 内有意义**，代次在任何句柄之前发送。合成器重启会
  复用同一个整数表示不同的窗口，先发代次是让客户端能丢弃陈旧视图的唯一办法。
- **不授予任何像素。** 事件只有句柄、字符串与枚举；请求只是用户通过快捷键已经能做的动作。
  未来要加特权动词，必须同时加授权，不得拓宽这个接口。

协议 XML 是唯一事实来源；`src/window_manager.rs` 的元数据测试冻结接口名/版本/请求集/事件集/
枚举取值/`window` 事件的参数签名，`tests/window_manager_roundtrip.rs` 在真实 socket pair 上
断言快照顺序、窗口记录的每个字段与六个动词的参数逐字到达。

### 2. 合成器（L2）：窗口模式状态机 + 工作区

`crates/inox-compositor/src/workspace.rs` 从「窗口 → 输出」扩成三层：

- **模式**：每个窗口有 `WindowMode::{Tiled, Floating, Maximized, Fullscreen}` 之一。
  模式不是标志位：最大化一个平铺窗口会取代平铺，回到 `Tiled` 时重新进入平铺布局。
  `minimized` 与模式**正交**——最小化保留模式与几何，激活时原样恢复。
- **工作区**：至少一个、编号从 0 开始；每个窗口属于一个工作区。窗口只在**当前**工作区
  被合成，其他工作区的窗口保留状态但不可见（`visible = 0`）。
- **几何与焦点的单一来源**：`arrange_windows` 仍然是唯一摆放窗口的地方，但它按模式分支——
  `Tiled` 走既有 master/stack 布局；`Maximized` 填满该输出的**工作区**（即 `work_area`，
  因此独占区面板不会被盖住，这与 O02 的面板保留区规则一致）；`Fullscreen` 填满输出的**整个**
  area（面板被盖住，这是全屏的定义）；`Floating` 用自己的几何，不参与平铺。
  `sync_focus` 仍然是 `xdg_toplevel.activated` 的唯一来源，并新增推送
  `Maximized` / `Fullscreen` / `Minimized` / `Tiled`（左/右/上/下）四个状态位。

`XdgShellHandler` 的默认空实现换成真实语义：`maximize_request` / `unmaximize_request` /
`fullscreen_request` / `unfullscreen_request` / `minimize_request` 都改窗口模式并重排；
`move_request` / `resize_request` 把窗口转为 `Floating` 并（在 headless/注入路径上）
由合成器自己完成拖动与缩放。**客户端请求与任务栏请求走同一条路径**：两处都只调用
`InoxState` 上的同一个方法，因此「点最大化按钮」与「面板切换模式」不会各自演化出第二套语义。

快捷键（`src/input.rs` 的纯逻辑）新增：`Super+F` 浮动/平铺切换、`Super+M` 最大化、
`Super+Shift+F` 全屏、`Super+H` 最小化、`Super+1..=9` 切工作区、
`Super+Shift+1..=9` 把焦点窗口移到该工作区。录制进 `Action` 枚举并有单元测试。

### 3. 合成器：`inox_window_manager_v1` 服务端

新模块 `crates/inox-compositor/src/window_manager.rs` 持有观察者列表
（每个 `inox_window_list_v1` 一个），并把「窗口集合发生了变化」变成一个可测试的纯函数：

```rust
// 纯逻辑，不依赖 Wayland，可在单元测试里穷举
pub fn diff(previous: &[WindowRecord], current: &[WindowRecord]) -> Vec<Change>;
```

`Change::{Window(WindowRecord), Removed(u32)}`。完整状态而非增量是刻意的：漏掉一条事件的
观察者会在下一条上收敛。事件的发送时机只有两个入口——快照（`get_window_list` 之后）与
`InoxState` 中窗口/模式/工作区/焦点发生变化之后的一次 `announce_windows()` 调用，
它负责算 diff 并逐个观察者发送。**不允许**在别处零散地发窗口事件。

`window` 事件只描述**已经 map 的窗口**（第一次带缓冲的 commit 之后）：创建一个 toplevel
却从不绘制的客户端不应在任务栏里留下一个可能永远不出现的按钮，这与既有
`promote_awaiting_toplevel` 的门槛是同一条规则。

授权：与 `inox_ipc` 相同，任何客户端可绑定。这里泄露的是任务栏本来就画在屏幕上的标题与
app id，请求只是用户级窗口动作。会话锁定时 `activate` / `set_mode` 等改动窗口可见性的请求
以 `not_permitted` 拒绝——锁定期间不得把普通窗口带到前台。

### 4. 面板（L3）：把既有模型接到协议上

`inox-panel` 新增 `src/window_service.rs`（协议腿），职责是把
`inox_window_list_v1` 的事件翻译成**既有**的 `app::Message`，并把既有的
`OutgoingCommand` 队列翻译成协议请求：

| 事件 | 既有消息 |
|------|----------|
| `session_generation` | `Message::ServiceReady { generation }` |
| `window` | `Message::WindowUpdated { key, window }`（快照期间同一条路径） |
| `window_removed` | `Message::WindowRemoved { key }` |
| `snapshot_done` | `Message::Snapshot { generation, windows }` |
| `request_failed` | `Message::CommandSettled { request, outcome }` |

面板**不新增任何窗口模型**：`tasks.rs` 的 `WindowState` 已经有 `minimized` / `maximized` /
`fullscreen` / `workspace` / `output` / `focused` 全部字段——它们今天没有数据来源，正是这条线
要填的东西。`contract.rs` 的 `RejectionReason::{NotPermitted, UnknownWindow, ServiceGone, Protocol}`
与协议枚举一一对应映射，不新增错误类型。

任务按钮的**点击语义保持既有定义**（`TaskActivated` / `TaskToggled`）：点未聚焦的窗口 = 激活，
点已聚焦的窗口 = 最小化。新增两个右键/中键动作：中键 = 关闭（`OutgoingCommand::Close`），
右键 = 在模式间循环（`OutgoingCommand::SetMode`）。工作区指示器画在任务区左侧，
点击切换工作区（`OutgoingCommand::SwitchWorkspace`）。文案全部走 Fluent（`en-US` + `zh-CN`）。

**服务不可用是设计状态，不是崩溃**：合成器不提供该全局时，面板以一次 `info!` 说明并停在
既有 `ServiceState::Unavailable`，任务栏画出既有的「窗口服务不可用」句子与重试按钮（D05），
**不**非零退出——面板没有窗口服务仍是一个可用的时钟与托盘。

### 5. 文档、验收与登记

- 本文登记进 `docs/book/src/architecture/decisions.md` 索引（状态 `proposed`）。
- `docs/book/src/architecture/protocols.md` 的私有协议一节加 `inox-window-manager-v1`。
- `docs/book/src/configuration/compositor.md` 的 `[layout]` 一节补工作区数量与快捷键。
- `docs/quality/acceptance.md` 的 W05 与 D03 行**不改判**：本次交付的是实现与 E1/E2 证据，
  E3/E4 仍未取得，行内的通过条件一个字不动。
- 根 `Cargo.toml` 的分层表不需要改：没有新增 crate，没有新增依赖边
  （`inox-panel` 已依赖 `inox-protocols`；`inox-compositor` 亦然）。

## 后果

### 正面

- W05 的窗口模式与工作区、D03 的任务栏显示第一次有了闭环：合成器真的会最大化/全屏/最小化/
  切工作区，面板真的会画出这些窗口并驱动它们。
- 客户端请求（`xdg-shell`）与任务栏请求（本协议）走**同一条**状态迁移路径，不会各自演化。
- 面板侧不需要新模型：既有的 `tasks` / `contract` / `app` 三分法与它们的属性测试原样复用，
  新增的只有一个协议腿。
- 协议是加法式的：不绑定该全局的客户端完全不受影响；`inox-ipc` 的版本不变。
- 合成器侧的 diff 是纯函数，可单元测试；「事件只从两个入口发出」使漏发成为可审查的属性。

### 负面 / 限制

- **E3/E4 仍未取得**：本次证据是 E1（合成器 `workspace`/`layout` 的单元与属性测试、
  面板映射的单元测试）与 E2（真实 Wayland 客户端驱动的合成器集成测试、
  真实 socket pair 上的协议往返）。W05 要求的「状态/几何与截图」中的截图证据仍缺，
  waydriver 宿主 job 尚未接线，因此 W05 与 D03 **不得**被读作通过。
- **浮动窗口的几何没有持久化**：会话重启后浮动窗口回到默认位置，不写任何文件（持久化属于
  `inox-settings` 的后续工作）。
- **工作区数量固定为配置给出的值**，运行中不动态增删；`move_to_workspace` 越界会被拒绝而不是
  创建工作区（一个没有任何入口的工作区是不可达的）。
- 合成器新增一个全局与一个模块，`state.rs` 的 `announce_windows` 成为新的「必须一起看」的地方，
  与 `sync_focus` / `arrange_windows` 同级。丢一次调用就是任务栏不同步，因此有测试钉住它。
- 任务栏的中键关闭是**请求**：客户端可以保存、弹窗或拒绝，面板只在收到 `window_removed` 后
  才移除按钮。

## 考虑过但未被采纳

- **扩展 `inox-ipc` 加窗口事件**：该全局是「每连接绑一次」的会话控制对象，把一个持续订阅的
  观察者塞进去会让每个消费者都收到它不需要的窗口流量，也会把「绑定即获得订阅」混进权限模型。
  独立全局的代价只是多一次 `wl_registry` 绑定，语义却干净得多。
- **用 D-Bus 传窗口列表**：合成器没有会话总线连接，引入它会把 L2 与会话的启动顺序耦合起来；
  而且窗口事件与 surface 生命周期同源，走 Wayland 连接天然有序。
- **面板直接用 `wlr-foreign-toplevel-management`**：那是 wlroots 私有协议，Inox 合成器不是
  wlroots。抄它的形状可以，绑定它不行——本协议的事件与它同构，但字段按 Inox 的需求定
  （工作区、输出、模式是 wlroots 那个协议没有或不完整的）。
- **让客户端自己上报窗口元数据给面板**：任务栏要显示的是合成器的事实（哪种模式、哪个工作区、
  是否可见、谁有焦点），这些只有合成器知道。客户端自报的模式与真实状态分叉时，
  任务按钮就会撒谎。
- **在面板里复制一份窗口簿记**：`tasks.rs` 已经是那份簿记，且有不变量测试。再写一份
  「从协议事件重建的窗口表」会让两处顺序/分组规则慢慢分叉。

## 取代条件

出现以下任一情况时，本决策应被重新评估或取代：

- 上游出现可用的窗口管理标准协议（例如 `ext-foreign-toplevel-list-v1` 成熟且覆盖工作区与模式），
  则应迁移并让本协议退役；
- Wayland 之外的会话总线（D-Bus）成为合成器与 Shell 的既定通道，且窗口事件被证明需要跨会话
  持久观察（届时考虑把订阅搬到总线，本协议只留动词）；
- 1.0 发布时协议需要冻结：若 v1 被接受，应移除 unstable 标记并重新编号。
