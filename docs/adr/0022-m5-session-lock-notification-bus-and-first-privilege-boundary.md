# ADR 0022: M5 的会话锁协议腿、通知总线服务与首个特权边界

- 状态：proposed
- 日期：2026-10-17
- 决策者：架构组与安全审查者（待人类评审）
- 批准记录：待补（人类评审 PR / 评论与日期；合并草案不等于接受）
- 关系：实施 ADR 0015 第「锁定与认证」「特权接口」两节的协议腿与本提案新增的依赖/协议条目；细化 ADR 0006（协议策略）、ADR 0013（服务契约）与 ADR 0021（L4 首版边界）。**不改变** ADR 0002 的分层与依赖方向，不批准认证机制、不死锁的安全锁屏 UI、也不把 H02/P05 的人工证据算作已取得。

## 背景

PRD §13 的 M5 要求交付 D06（`org.freedesktop.Notifications` 服务端）与 S01–S05（会话监督、锁屏、空闲/挂起、特权接口、故障语义）。ADR 0015 已经决定了安全模型 —— 锁状态由合成器所有、采用 `ext-session-lock-v1`、锁屏 UI 崩溃或认证失败都不得解锁、认证是必须的人类评审项 —— 但把三件实现选择留给了「实施 ADR」：

1. **协议腿在谁手里。** `inox-lock` 今天只有状态机、凭据缓冲与界面模型；`main.rs` 载入配置与目录后以非零码退出，明说无法开始锁定。合成器不广告 `ext_session_lock_manager_v1`。因此 X02 中「锁屏时读剪贴板」的断言、S02 的全部断言都无处可测。
2. **通知服务需要什么依赖与线程模型。** `inox-notifications` 已经具备协议契约、有界队列、脱敏与动作至多一次，且明确把 `State::take_outgoing` 留作「M5 总线要拉的接缝」；但工作区里没有任何 D-Bus 依赖，`crates/inox-notifications/AGENTS.md` 也写明引入 `zbus`/`dbus` 需要先有排期与独立的 PR。
3. **谁有权锁定会话。** S04 要求「首次特权接口引入，最迟 M5」；ADR 0015 要求「普通客户端不能夺取可信锁屏角色或伪造成功」，并且「客户端自报 app_id、进程名或 UID 检查不足以表达能力」。

已核验的素材：`wayland-protocols` 0.32.13 的 `protocols/staging/ext-session-lock/ext-session-lock-v1.xml`（本 ADR 引用的 locked/finished 语义、错误码与生命周期均引自该文件，接口名与版本与 `ext.rs` 的 `#[cfg(feature = "staging")]` 门一致）；本仓库根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]` 与 `rules`；`crates/inox-protocols/protocol/inox-ipc.xml`（version 1，请求 `destroy`/`get_version`/`reload_config`，事件 `version`/`config_reloaded`）；`crates/inox-compositor/src/state.rs` 的全局清单与 `scene_elements`/`sync_focus`/`surface_under`；`crates/inox-lock/src/{lib,contract,state}.rs`；`crates/inox-notifications/src/{lib,contract,app}.rs`。CRATES 版本以 `Cargo.lock` 为准。

## 决策驱动因素与备选

| 方案 | 收益 | 成本 / 风险 | 选择或否决依据 |
|------|------|-------------|----------------|
| 保持现状（继续只有状态机与契约） | 零新增依赖与协议 | D06、S01–S05 全部无证据；M5 无法推进 | 否决：M5 的 DoD 就要求这两项 |
| layer-shell 顶层遮罩充当锁屏 | 实现最快 | 无法证明输入与其它层的隔离，UI 崩溃即失守 | 否决：ADR 0015 已明确拒绝 |
| 合成器自己认证、自己画锁屏 | 少一个进程 | 把认证塞进持有输入/输出的进程，且违反 ADR 0015 的「认证是评审项、UI 与锁状态解耦」 | 否决 |
| **采用 `ext-session-lock-v1`：合成器服务端 + `inox-lock` 客户端，授权用会话投递的能力令牌** | 锁状态与 UI 进程解耦；崩溃保持锁定；协议本身已定义遮蔽与生命周期 | 需要 L3→L2 新依赖、一条私有协议扩展与一次人类安全评审 | **采用** |
| 通知服务用 C 绑定的 `dbus` crate | 依赖树小 | 需要 `libdbus` 系统库与 unsafe FFI 绑定；工作区目前没有任何 `unsafe` 例外可用于 L3 | 否决 |
| 自研 D-Bus 编解码 | 无新依赖 | 与「不自研安全与系统协议」的一贯取舍冲突，且要自己维护 SASL/类型系统 | 否决 |
| **通知服务用 `zbus`（纯 Rust，MIT）** | 纯 Rust、无系统库、阻塞 API 可直接跑在普通线程上 | 依赖树较大（async-io / zvariant），需要许可证与重复版本检查 | **采用** |

## 决策

### 1. 协议绑定：再导出上游 `ext-session-lock-v1`，不写自有 XML

`inox-protocols::standard::ext::session_lock` 再导出 `wayland_protocols::ext::session_lock::v1` 的 `client` 与 `server` 两半（上游处于 *staging*，`staging` feature 已在 `inox-protocols` 的 `Cargo.toml` 中启用）。管理者与两个子接口都固定在版本 1。这与 ADR 0006「协议绑定集中在 `inox-protocols`、协议以 XML 为唯一事实来源」一致：本项目不复制、不修改上游 XML。

### 2. 合成器实现服务端：锁状态归合成器，且跨客户端存活

- 广告 `ext_session_lock_manager_v1`（v1）。`lock` 请求通过授权检查后创建锁对象；**不**接受第二个在世的锁对象（协议允许以 `finished` 拒绝）。
- **锁状态是合成器状态，不是客户端状态。** 锁定期间合成器的渲染路径只合成锁表面；某个输出没有锁表面时，该输出显示不透明的锁定底色。普通客户端的窗口与 layer 表面不参与锁定帧的合成 —— 这是「所有输出先遮蔽」的可断言形式，也是本 ADR 的主要安全断言。
- `locked` 事件只在一次完整的锁定帧渲染通过之后发送：`lock` 请求置位重绘，事件循环随即渲染一帧（此时每个输出渲染的都是锁定场景），渲染完成后才发送 `locked`。没有输出时该条件真空成立，仍走同一条路径。合成器**不**等待客户端先交出锁表面（协议允许设定时限；不等待消除了「客户端慢」与「普通帧被看见」之间的竞态）。
- **输入隔离**：锁定期间键盘焦点只给锁表面（没有锁表面时为无焦点）、`wl_data_device`/主选区焦点清空、compositor 快捷键不再拦截按键（`Forward` 而不是 `Intercept`）、指针命中只在锁表面之间选取。因此普通客户端拿不到按键、拿不到新的选区授予、也无法被点击激活。
- **新增输出**：锁定期间到达的输出第一次渲染就是锁定帧，不需要单独的遮蔽步骤；锁客户端为新输出创建锁表面时合成器照常发 `configure`。
- **客户端死亡不解除锁定，且本实现不把锁交给第二个客户端**：锁对象随连接销毁，但合成器保持锁定并继续渲染锁定场景（没有锁表面时就是锁定底色）。协议允许合成器让新客户端「take responsibility for unlocking the session」，但本实现**不做接管**：Smithay 0.7 的 `SessionLockManagerState` 用它自己的 `locked_outputs` 去重，而该字段没有公开的重置入口，接管者会为同一输出拿到 `duplicate_output` 错误而被断开——一个半坏的锁客户端比一个明确的拒绝更糟。因此：会话在锁屏 UI 崩溃后保持锁定，**新的锁客户端也会被拒绝**，只有注销能结束这个会话。这是协议明文允许的（"It is acceptable for the session to be permanently locked if this happens"），也是 ADR 0015 要求的失效方向；恢复路径登记在「未决项」。
- 协议错误按 XML 实现：已发 `locked` 后再 `destroy` → `invalid_destroy`；未发 `locked` 就 `unlock_and_destroy` → `invalid_unlock`；同一输出第二个锁表面 → `duplicate_output`；`wl_surface` 已有角色或已提交缓冲 → `role` / `already_constructed`。
- 剪贴板：Smithay 不会撤销客户端已持有的 offer（`sync_focus` 的文档已记录这条边界）。本 ADR 交付的是「锁定后不再授予任何普通客户端选区」；**撤销已发出的 offer 仍不在范围内**，与 ADR 0017 的既有边界一致，D06/S02 的断言因此表述为负向授予而不是撤销。

### 3. 授权：会话投递的一次性令牌 + `inox-ipc` 认领，默认拒绝

- `inox-session` 为每个会话生成一次性令牌，投递给合成器（`--lock-token <token>`）与可信锁屏客户端（环境变量 `INOX_LOCK_TOKEN`）。两者都由会话启动，令牌不落盘、不进日志。
- 客户端在调用 `lock` 之前，必须在同一条连接上经 `inox-ipc` 的 `claim_lock_role(token)` 认领锁屏角色。合成器按**客户端连接**记录认领结果，并在 `lock` 时校验：未认领、令牌不符或已过期 → 立即对新对象发 `finished`（协议允许的拒绝），并记录一次拒绝事件。
- **默认无特权**：没有配置令牌的合成器（例如开发者手工启动、或测试未指定）拒绝一切锁请求。这保持 ADR 0015「回退遇到不支持的锁定/认证组合时拒绝启动该安全功能」的取向：宁可不锁，也不接受一个说不清来源的锁。
- **边界如实记录**：令牌是能力，不是身份 —— 客户端自报的 app_id/PID/名称不参与判断。但它是经环境变量投递的，因此在 Linux 上**同 UID 的非沙箱进程**可以读 `/proc/<pid>/environ` 取走它；本模型不声称抵御「已控制用户全部进程」的攻击者（ADR 0015 的原话）。它对沙箱应用有效：沙箱通常看不到会话进程的 `/proc`，也猜不出令牌。升级到继承 fd / 能力句柄只对「能访问同一 PID 命名空间」这一前提有意义，因此登记为复审项而不是本 ADR 的实现义务。

### 4. `inox-ipc` 升到版本 2：认领请求与锁状态广播

`crates/inox-protocols/protocol/inox-ipc.xml` 的接口版本由 1 升到 2，新增：

- 请求 `claim_lock_role(string token)` —— 认领可信锁屏角色，供第 3 条授权使用；
- 事件 `session_lock_state(uint state)` —— 合成器把「当前是否处于已锁定状态」广播给绑定者（`0` = 未锁定，`1` = 已锁定）。

锁状态事件的方向是**合成器 → 客户端**，是权威方对观察者的通知，因此不构成授权：任何绑定者都只能观察，不能据此获得能力。它有两个用途：`inox-idle` 用它把「准备挂起 → 收到锁完成确认」接上真实来源（README 与 ADR 0021 记录的 5 秒代次确认模型不变）；`inox-notifications` 用它决定是否对正文脱敏（D06 的「锁屏隐藏敏感正文」）。

`inox_ipc` 是 unstable 私有协议，版本协商由既有的 `version` 事件承担；v1 客户端仍可绑定并只使用 v1 的请求。破坏性变更的登记要求在 PRD 协议矩阵中同步完成。

### 5. 通知服务：`zbus` + 传输线程，`contract` 保持唯一状态机

- 新增第三方依赖 `zbus`（MIT，无系统库；只用阻塞 API 与 `p2p` 测试能力），**只**由 `inox-notifications` 使用。
- 传输层在**独立线程**上持有连接与 `zbus::blocking::Connection`，把方法调用翻译成 `contract::Request` 送进 iced 应用，并从应用取回 `Outgoing`。同步方法（`Notify`、`CloseNotification`、`GetCapabilities`、`GetServerInformation`）在传输线程上等待带**上限**的答复；超时返回 D-Bus 错误，迟到的答复由 `contract::Calls` 的去重记忆吸收。
- 应用侧的 `update` 仍然不做 I/O，`State::take_outgoing` 仍是唯一出口：新增的只是「把通道接到 `Message::Request`」与「把 `Outgoing` 送回传输层」两段接线。
- 服务端实现 `org.freedesktop.Notifications` 的四个方法与两个信号，能力集维持现状（不含 `icon-static`/`body-images`/`persistence`，理由见 crate 文档）。总线上已有其它所有者时，以可读错误退出，不静默抢名。
- 锁屏脱敏由第 4 条的锁状态事件驱动，不新增配置文件字段。

### 6. `inox-lock` 的协议腿只在显式开关下运行

`inox-lock` 默认行为不变：认证边界未获批，因此仍以非零码退出并说明原因。新增 `--take-lock` 显式开关，作用是**真实地**取锁：连接合成器、经 `inox-ipc` 认领角色、请求锁、为每个输出创建锁表面、按 `configure` 提交不透明锁定帧，然后一直保持。该模式**没有**解锁路径：不读键盘、不校验任何凭据、收到 `SIGTERM`/`SIGINT` 就退出且**不**调用 `unlock_and_destroy`（合成器保持锁定）。它存在的理由是让 S02 的协议腿与失败注入可测，且不能把它读作「锁屏已可用」。

### 7. 依赖与分层

`inox-lock` 新增 `wayland-client`、`calloop`、`inox-protocols`；`inox-notifications` 新增 `wayland-client`、`inox-protocols` 与 `zbus`。两条都是 **L3 → L2**（向下），ADR 0002 规则 1 允许，`l3_dependencies_on_compositor` 例外表不变（两者都不依赖 `inox-compositor`）。两个 crate 的 `AGENTS.md` 中「只依赖 L1」的表述随之修正，并必须说明新增边的用途与方向。

## 失败、恢复与安全

- **锁定失败**：合成器拒绝（未授权、令牌不符、已有在世锁对象）→ 客户端立即收到 `finished`，会话保持未锁定，`inox-idle` 的策略把这次准备判为 `RefuseSuspend`（既有 5 秒代次模型）。
- **锁定中 UI 崩溃**：合成器保持锁定并继续渲染锁定场景；`inox-session` 按既有重启预算有界重启 `inox-lock`，但新实例会被合成器拒绝（第 2 条：本实现不支持接管），因此每次重启都消耗预算并留下一条可见的拒绝日志，预算耗尽后进入可诊断退化：**锁定保持**，会话不再尝试重启。这条链路的结局是「会话锁定到注销为止」，必须如实说明，不得写成「UI 崩溃可自动恢复」。
- **合成器崩溃**：会话结束（既有语义），**不**宣称恢复原窗口；锁状态随之消失是会话级故障的一部分，如实记录而不是当作解锁。
- **注销**：`SIGTERM` 到 `inox-session` 仍按既有逆序停止并升级 KILL；锁定不是注销的阻碍，`inox-lock` 退出时不发 `unlock_and_destroy`，因此不会出现「注销期间短暂解锁」。
- **敏感内容**：令牌与凭据都不进日志；`inox-notifications` 在锁定时只保留 `policy::PopupContent::Withheld` 这个类型（没有正文、应用名与图标的变体），脱敏不是布尔开关。
- **不受本模型保护**：同 UID 非沙箱进程可读会话进程的环境（见第 3 条）；合成器不撤销已发出的剪贴板 offer（见第 2 条）；不提供截屏/输入注入协议，因此相关特权请求按「未提供即可用性拒绝」处理（S04）。

## 兼容、迁移与回退

- **协议**：`ext-session-lock-v1` 是新广告的全局，旧客户端不受影响；`inox_ipc` 由 v1 升到 v2，新增一项请求与一项事件，v1 客户端继续工作。PRD §2.3 协议矩阵随之更新（`ext-session-lock-v1` 从 M5 目标记为已广告；`inox-ipc` 标 v2）。无磁盘格式变更，无配置迁移。
- **依赖**：`zbus` 的用途、锁定版本、许可证（MIT）与运行依赖（无系统库；运行期需要一条会话总线，测试用 p2p 连接不依赖 `dbus-daemon`）在 PR 记录，并由 `cargo-deny` 对完整依赖图核验。
- **回退**：删除合成器的全局与 `inox_ipc` v2 条目即回到今天的行为；`inox-lock --take-lock` 不被默认路径调用，删除开关即回到「拒绝启动安全功能」。已锁定的会话在没有锁客户端时保持锁定，回退不会把它变成可用的桌面。

## 验收与证据

| 验收 ID / 场景 | 责任 crate | E0–E4 等级 | 测试与失败注入 | 通过条件 / 证据位置 |
|----------------|-----------|-----------|----------------|----------------------|
| S02 协议腿（遮蔽、`locked` 时序、输入隔离、协议错误） | `inox-compositor` | E1+E2 | 进程内真实客户端：锁定后读回输出像素、普通客户端收不到按键/选区、`destroy`/`unlock_and_destroy`/重复输出的错误码 | `crates/inox-compositor/tests/session_lock.rs`（新增） |
| S02 客户端腿（取锁、覆盖全部输出、认领授权、信号不解锁） | `inox-lock`、`inox-compositor` | E1+E2 | 真实 `ext-session-lock-v1` 服务端 peer + 真实 `inox-lock --take-lock` 子进程：断言认领令牌、每个输出一块锁表面、ack 后提交同尺寸不透明帧、SIGTERM 后**不**发 `unlock_and_destroy` | `crates/inox-lock/tests/protocol.rs`（新增） |
| S02 合成器腿（遮蔽、`locked` 时序、输入隔离、协议错误、UI 死亡与拒绝接管） | `inox-compositor` | E1+E2 | 进程内真实客户端：锁定后读回像素、无输入与选区授予、非持有者 `unlock_and_destroy` 得到 `invalid_unlock` 且会话保持锁定、UI 死亡后新客户端被拒 | `crates/inox-compositor/tests/session_lock.rs`（新增） |
| S02 E3（会话启动真实锁屏并仍能注销） | `inox-session`、`inox-compositor`、`inox-lock` | E3 | `INOX_SESSION_TAKE_LOCK=1` 下的完整 headless 会话：断言锁定被确认，且 SIGTERM 仍在既有预算内完成注销 | `crates/inox-session/tests/headless.rs`（新增用例） |
| S04 授权（默认拒绝 / 认领后允许 / 令牌不符拒绝） | `inox-compositor`、`inox-protocols` | E1+E2 | 真实客户端经 `inox_ipc` 认领：三种结果分别断言，且拒绝不影响其它客户端 | 同上 `tests/session_lock.rs`、`crates/inox-protocols/tests/ipc_roundtrip.rs` |
| D06 总线服务（Notify/替换 ID/关闭/动作/超时/勿扰/锁屏脱敏/突发） | `inox-notifications` | E1+E2 | p2p 连接上的真实 D-Bus 连接 + 真实 `app::State`；注入每秒 100 条持续 10 s；两条信号读线上字面内容 | `crates/inox-notifications/tests/bus.rs`（新增，18 例）。**未覆盖**：会话总线上的名字所有权（需要一个总线守护进程，本 crate 分片没有），只有 `NameTaken` 映射的单测 |
| X02 锁屏剪贴板断言（自 M3 延期） | `inox-compositor` | E2+E3 | 锁定后原持焦客户端不再获得新的选区授予 | 同上 `tests/session_lock.rs` |
| 锁状态事件本身（v2 请求/事件、枚举取值、绑定即广播） | `inox-protocols`、`inox-compositor` | E0+E1+E2 | 真实 socket 往返（`ipc_roundtrip`）+ 进程级 `ipc_global`；合成器侧由 `tests/session_lock.rs` 的锁定/解锁断言驱动 | `crates/inox-protocols/tests/ipc_roundtrip.rs`、`crates/inox-compositor/tests/ipc_global.rs` |
| 锁状态消费者 | `inox-notifications`（已）、`inox-idle`（未接线） | E1 | 通知侧：事件→`SessionLockChanged` 的映射、交接/重试、`Disabled` 状态单测；**没有**真实 `inox_ipc` 全局驱动的用例（该 crate 不含 `wayland-server`），idle 侧尚未订阅 | `crates/inox-notifications/src/lock_state.rs` 的单测 |

未取得、也不由本 ADR 声明取得的证据：**H02 的真机步骤**（O01–O04、S02–S03 的 E4 部分）需要人类在登记硬件上执行；**P05** 的 8 h/24 h 长稳需要完整跑完；**真实认证**（PAM/helper、凭据传递与销毁、锁完成确认的信任来源）按 ADR 0015 仍须人类安全评审，本 ADR 不批准任何认证机制，也不把「`--take-lock` 能锁住」读作认证完成。

## 实施顺序与未决项

1. `inox-protocols`：`ext_session_lock` 再导出与其绑定测试；`inox_ipc` v2 的 XML、绑定与往返测试。
2. `inox-compositor`：锁状态簿记（纯逻辑 + 属性测试）→ 服务端 dispatch → 渲染与输入隔离 → 授权 → 集成测试。
3. `inox-lock`：协议客户端与 `--take-lock`，合成器侧协议的错误路径由 E3 用例覆盖。
4. `inox-notifications`：`zbus` 传输层与总线测试，接上锁状态事件。
5. `inox-session`：生成并投递令牌，把 `inox-lock` 从「不启动」改成「按策略启动」——策略是 `INOX_SESSION_TAKE_LOCK=1`，因为 `--take-lock` 没有解锁路径，无条件启动会锁住一个没人要求锁住的会话。
6. **`inox-idle` 的锁完成确认尚未接线**：`session_lock_state` 已经存在并有往返测试，但 idle 还没有订阅它，因此 S03 的「准备挂起 → 收到锁完成确认」在真实会话里仍然不可达（挂起本身也还没有传输层）。这是本 ADR 交付范围之外的一步，登记在未决项里，不得读作已完成。
6. 文档：README §4、`docs/book/src/roadmap.md`、PRD §2.3/§13、两个 crate 的 `AGENTS.md`、验收映射。

未决项（阻塞或待人类决定，不能以「后续支持」掩盖）：

- **认证边界**：PAM 或其它机制、helper 权限与沙箱、凭据传递与销毁、可信连接的建立与撤销 —— 责任角色：安全审查者；最迟解决阶段：启用默认锁屏之前（ADR 0015 的原文要求）。本 ADR 不批准。
- **挂起**：`logind` 的 prepare-sleep/delay inhibitor 时序与失败策略 —— 责任角色：架构组；最迟阶段：S03 的 E3 用例落地前。挂起完全未实现，本 ADR 也不执行它。
- **`inox-idle` 消费锁状态**：`session_lock_state` 事件已经存在，但 idle 尚未订阅它，因此 `IdlePolicy` 的锁完成确认在真实会话里没有来源（第 5 条之后的第 6 条）。责任角色：架构组；最迟阶段：S03 的 E3 用例落地前。
- **完整会话 ready**：S01 要求的「必需服务按就绪依赖启动 + 完整 ready」需要 L3 的 ready 回执（另需协议条目）—— 责任角色：架构组；最迟阶段：M5 结论声明前。本 ADR 不新增该条目。
- **令牌的能力句柄升级**：若将来把「同 UID 非沙箱进程」纳入威胁模型，令牌须换成继承 fd/能力句柄 —— 责任角色：安全审查者；触发条件见第 3 条。
- **剪贴板 offer 撤销**：锁定后撤销已发出的 offer 仍未实现，与 ADR 0017 的边界一致；本 ADR 交付的是「锁定后不再授予任何普通客户端选区」，并已由 `crates/inox-compositor/tests/session_lock.rs` 断言（锁定期间同一 `set_selection` 被拒、解锁后同一调用被授予）。
- **锁屏 UI 崩溃后的恢复**：本实现不支持接管（见「失败、恢复与安全」第一条）。恢复需要 Smithay 暴露其逐输出簿记的重置，或 Inox 在自己的协议条目里承担该角色——责任角色：架构组；触发条件：把「UI 崩溃后能重新锁定」列为 M5/M6 的通过条件之前。

## 后果

正面：M5 第一次有了可测的锁协议腿 —— 锁状态与 UI 进程解耦、崩溃保持锁定、遮蔽与输入隔离都有了可断言的实现与证据位置；通知守护进程第一次真的能被应用调用；首个特权接口以「默认拒绝 + 会话投递能力」落地，而不是靠自报身份。X02 自 M3 延期的锁屏剪贴板断言随之可测。

负面：工作区新增一个较大的第三方依赖树（`zbus`）与两条 L3→L2 边；`inox_ipc` 进入 v2，消费者必须各自核对版本协商；`--take-lock` 是一个没有解锁路径的模式，误用会把会话留在锁定状态（文档与启动日志必须写明，且不得成为默认路径）；锁屏在认证获批前仍不是可用功能，M5 的「日常可用」结论仍取决于人工评审与 H02。

影响面：`crates/inox-protocols`（standard.rs、protocol/inox-ipc.xml）、`inox-compositor`（state.rs、config.rs、runtime）、`inox-lock`、`inox-notifications`、`inox-session`、`inox-idle`、`inox-integration`、根 `Cargo.toml`（依赖）、`.github/workflows/ci.yml`（新用例所需的系统包与 job）、PRD §2.3/§13、README §4、`docs/book/src/roadmap.md`、相关 crate 的 `AGENTS.md`。根级硬性规则（unsafe、unwrap、日志、许可证、i18n、令牌）一律不放宽。

复审条件：引入真实认证、把锁屏 UI 换成 Iced 表面、支持多 seat、新增截屏/输入注入协议、把同 UID 非沙箱进程纳入威胁模型，或上游 `ext-session-lock-v1` 离开 staging 时，重新评估本 ADR 的授权与生命周期条目。
