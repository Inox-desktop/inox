# ADR 0025: 锁屏界面绘制到会话锁表面与登录界面首版

- 状态：accepted
- 日期：2026-10-17
- 决策者：架构组
- 批准记录：2026-10-17 本仓库维护者在开发会话中明确批准本次实施——授权把 `inox-lock` 的界面真正画到会话锁表面上、把 `inox-greet` 的登录界面补全，并把这里记录的决策与边界作为项目约束。批准的是**这次实施与本文记录的决策**，不是 ADR 0015 仍然点名的那项认证机制；PR 链接与评审记录待补。**不**批准任何认证机制，也不把「界面已存在」读作「锁屏已可用」。
- 关系：实施 ADR 0022 点名的复审触发条件「把锁屏 UI 换成 Iced 表面」；**部分取代 ADR 0022 决策 6** 中「按 `configure` 提交不透明锁定帧」这句所描述的帧内容——帧仍然逐像素不透明，但它现在承载界面，而不是一块纯色（该决策的取锁、遮蔽、授权、不读键盘、无解锁路径其余部分**一律不变**）；并取代 `crates/inox-lock/AGENTS.md` 旧文中「帧只是一层不透明底色」与「提示与掩码字段都不在这条模式里」两句话（它们是 crate 规则，不是 ADR 0022 的表述）。实施 ADR 0015 关于锁屏界面清理与遮蔽的条目；细化 ADR 0021 的 `inox-greet` 界面条目；复用 ADR 0018 记录的离屏渲染路径。**不改变** ADR 0002 的分层与依赖方向（不新增同层边、不新增 L3→合成器边），不改变 ADR 0013 的服务契约。

## 背景

PRD §13 的 M5 要求交付 S02–S05，并且把 M5 的「仍未交付」清单写成「完整服务 ready、**真实认证与锁屏界面**、logind 挂起、锁屏 UI 崩溃后的接管、通知的 D-Bus 服务（D06），以及 H02 的全部真机步骤」。ADR 0022 落地了其中一半：合成器广告 `ext_session_lock_manager_v1`，以会话投递的令牌默认拒绝授权，`inox-lock --take-lock` 真实取锁并遮蔽每个输出，`inox_session_lock` 的会话级用例把它接到一次完整的注销里。另一半没动，而它是**两个不同的东西**：认证边界按 ADR 0015 是人类评审项，界面按 ADR 0022 的复审条件是本文要处理的。

缺口的确切形状，读代码比读文档准：

- `inox-lock` 的界面是完整的，也是不可达的。`view_model::project` 把状态投影成 D05 点名的每一个画面（提示、验证中、失败与原因、限流等待、无可信 UI、不可用 + 重试），`view::view` 把它画成控件树，`theme` 把令牌映射成 iced 样式，`app::update` 是完整的 Elm 循环——而 `src/session.rs` 的协议腿在 `commit_cover` 里用 `LOCK_FRAME_RGB` 填一块纯色。`crates/inox-lock/src/lib.rs` 的「What is verified, and what is not」把这件事写成三条：协议腿不存在、表面不存在、界面只被测试夹具绘制。前两条已由 ADR 0022 推翻，第三条仍然成立，本文处理它。
- 协议腿自己维护了第二份覆盖簿记。`src/session.rs` 的 `Client` 有 `outputs`、`committed` 和 `locked`，`state::LockMachine` 也有整套输出覆盖簿记与 `reports_locked()`；`app::State::take_outgoing` 的文档写着「这是那两条传输要排空的接口」，而从来没有东西排空过它。两份簿记可以互相不同意，而它们同意的那个状态就是「会话被遮蔽了」。
- 界面无法在真实表面上出现的原因是可核验的 API 事实，不是排期：锁表面不是窗口。`iced` 的 `wayland` feature 经 `iced_winit` 创建 `xdg-shell` 表面，没有任何入口让 `iced` 把控件树画到 `ext_session_lock_surface_v1` 上；把它当普通窗口打开，得到的是一块**普通窗口**，正是 ADR 0015 明确拒绝的「看起来像锁屏的东西」。
- `inox-greet` 更早。ADR 0021 只要求「只有显式 `--preview` 打开 Iced 预览，登录按钮禁用，不采集凭据」，而 `src/app.rs` 的预览确实只做到这一步：`Message::Activate` 与 `Message::Cancel` 都被映射成 `Event::Cancel`，`state::Phase` 的六个取值里只有 `Unavailable` 有视觉表示，`greet-usage`/`greet-blocked` 之外的大部分 id 没有被任何控件使用。`state::Greeter` 的代次关联、30 秒期限、滚动 60 秒 3 次尝试这些规则有测试，但它们在界面上看不出来。

已核验的素材（按锁定版本读源码，不用历史提案代替可行性证据）：

- `iced_runtime` 0.14.0 `src/user_interface.rs`：`UserInterface::{build(root, bounds, cache, renderer), update(events, cursor, renderer, clipboard, messages), draw(renderer, theme, style, cursor), into_cache()}`；`Cache` 可跨帧复用。
- `iced_core` 0.14.0 `src/renderer.rs`：`Headless::screenshot(size, scale_factor, background_color) -> Vec<u8>`，RGBA 顺序、`background_color` 为合成底色；`iced_tiny_skia` 0.14.1 的 `Renderer` 实现它，并在 `window/compositor.rs` 里经 `tiny_skia` 落盘。
- `smithay` 0.7.0 `src/wayland/compositor/handlers.rs:127-137`：提交时 `merge_into` 对被**替换**的旧 buffer 调用 `release()`，且仅当新旧不同；`tree.rs:151-154` 在表面销毁时释放当前与待定 buffer。这决定了本文的重绘策略（决策 4）。
- `crates/inox-lock/tests/protocol.rs` 与 `crates/inox-compositor/tests/session_lock.rs`：前者以一个手写的 `ext-session-lock-v1`/`wl_shm`/`inox-ipc` 服务端 peer 驱动真实二进制，后者以进程内真实客户端驱动合成器。本文的证据落在这两个文件与 `crates/inox-lock/tests/render.rs` 上。
- `crates/inox-compositor/src/state.rs` 的锁表面渲染路径：锁定期间只合成锁表面，没有锁表面的输出显示合成器自己的不透明锁定底色。本文依赖这条已实现的语义作为「绘制失败不等于失守」的兜底。

未验证、也不由本文声称的：真实显示器上的外观（H02 的 E4）；键盘输入路径；锁屏 UI 崩溃后的接管（ADR 0022 未决项）；AT-SPI 语义导出（iced 0.14 无桥，ADR 0018 已记录）。

## 决策驱动因素与备选

| 方案 | 收益 | 成本 / 风险 | 选择或否决依据 |
|------|------|-------------|----------------|
| 保持现状（协议腿填纯色，界面只被夹具绘制） | 零改动、零新依赖 | Q01/Q02/D05 在真实表面上永远取不到证据；协议与状态各有一份覆盖簿记；`take_outgoing` 的接口没人实现 | 否决：M5 点名的缺口就是它 |
| 用 `iced::application` 开一个窗口当锁屏 | 最省事，iced 自己的输入与焦点全都有 | 它创建 `xdg-shell` 表面，不是锁表面：普通客户端仍能被操作，键盘焦点归属无法证明 | 否决：ADR 0015 的原话拒绝该方案 |
| 自建 iced 的 session-lock 平台层（仿 `iced_layershell`） | 与 iced 运行时完全一体，动画/焦点/IME 全部继承 | 要自己实现平台层、窗口生命周期与帧调度；`iced_layershell` 在本工作区**两个版本都无法编译**（ADR 0018 记录），同源风险已知 | 否决（本轮）：收益是未来的，成本是当下的，且它不解决「谁来画第一帧」这个当务之急 |
| **离屏渲染进 `wl_shm` 缓冲**：`UserInterface` + `iced_tiny_skia::Renderer` 画 `app::view(&state)`，按 configure 尺寸渲染后提交；协议腿由 `app::State` 驱动 | 复用已经存在并被测试覆盖的离屏路径；界面与协议共用一份状态，`take_outgoing` 第一次真有人排空；没有新增第三方 crate（三者已在依赖图中） | 锁表面不是窗口：没有 iced 运行时的输入、焦点与动画；三个 iced crate 从 dev-dependencies 变为 dependencies | **采用** |
| 顺带接上键盘输入路径 | 界面变得可操作，Q01 的「仅键盘完成锁定/解锁」向前一步 | 没有经过评审的认证边界时，屏幕上没有任何可操作的东西，输入路径只可能是不可能执行的代码；同时违反 ADR 0022 决策 6 的「不读键盘」 | 否决（本轮）：并入未决项，与认证边界一起落地 |

## 决策

### 1. 锁状态仍归合成器，本 crate 仍是可信客户端

取锁、遮蔽每个输出、`locked` 事件在锁定帧渲染完成后才发出、输入与选区隔离、客户端死亡不解除锁定——ADR 0022 决策 2 与 3 的全部内容**不变**，本文不改动合成器一侧的任何代码。本文改变的只有「这条客户端往锁表面提交什么像素」以及「这些像素由谁的状态决定」。

### 2. 每个输出一个 `render::Interface`，按 configure 的逻辑尺寸绘制

`inox-lock` 新增 `src/render.rs`：

- `Interface::new(width, height)` 为一个输出建立渲染器、控件树缓存与尺寸；`resize` 处理第二次 configure；`render(&state)` 走完 `UserInterface::build → update → draw → Headless::screenshot` 并返回 RGBA；`render_argb(&state, &mut Vec<u8>)` 直接写成 `wl_shm` 的字节序。
- 每帧先投递一次 `RedrawRequested`：iced 的控件在处理它时确定交互状态，没见过的会被画成禁用（`iced_widget::button`）。少了它，画面会声称所有控件都不可用，而投影并没有这么说。
- 尺寸是协议要求的确切尺寸，取自 configure，不取自任何本地推断。**不被绘制的尺寸**（0，或整帧超过 `MAX_FRAME_BYTES` = 256 MiB）返回 `None`，该输出保留合成器自己的锁定底色。绝不提交空缓冲或尺寸不符的缓冲——那是协议错误，会断开连接并把会话留在锁定状态。
- 帧永远是 `Argb8888`、逐像素不透明。屏幕容器用 `background` 令牌铺满，`screenshot` 的合成底色是白色，两者叠加保证 alpha 为 255；`tests/render.rs` 与 `tests/protocol.rs` 都断言这一点，因为「不透明的遮蔽帧」是 ADR 0015 的原文要求，不是渲染细节。

### 3. 协议腿由 `app::State` 驱动，不再有第二份覆盖簿记

- 协议事件变成 `app::Message`：注册表发现 `wl_output` → `OutputAdded`，全局移除 → `OutputRemoved`，`lock_surface.configure` 提交成功后 → `OutputCovered`。
- `app::State::take_outgoing()` 是唯一的对外出口，由 `pump` 排空：`Lock` 变成 `ext_session_lock_manager_v1.lock`，`CoverOutput` 变成 `get_lock_surface`，`Authenticate` 与挂起两条在本构建里不可能出现，出现就记日志并丢弃（决策 5）。
- 合成器发来 `locked` 时，如果 `LockMachine::reports_locked()` 还不成立，记一条 warn。两个来源对「哪些输出已被遮蔽」的判断不一致，是这个组件最不该静静接受的事情。
- 锁定状态的权威仍是合成器。本地状态机决定**什么时候**可以请求覆盖与报告，不决定会话是否安全。

### 4. 重绘策略：没有帧时钟，只在有新的东西要说时提交

锁表面没有窗口，也就没有 iced 运行时的帧调度。帧在两种情况下提交：某个表面刚被 configure（它还没有任何内容），或投影与上一次绘制时不同。`MAX_FRAMES_IN_FLIGHT`（4）是每输出未释放帧数的上限；达到上限时保留已在屏幕上的覆盖帧——**落后一个状态的覆盖帧仍然是覆盖帧**，而为此分配无限内存不是。

上限是 4 而不是 2，因为合成器在提交时释放被替换的那个 buffer（`smithay` 0.7.0 `handlers.rs:136`），所以稳态是「一个在显示、一个在交接」；多出的两个位置让一个还在路上的 `release` 不会卡住一次重绘。

### 5. 本构建对状态说真话：`ServiceFailure::NotProvided`

没有经过评审的认证边界，就报告「本次构建没有认证边界」，而不是留一个空白或假装它能用：

- `contract::ServiceFailure` 新增 `NotProvided`，`message_id()` 给出 `lock-boundary-not-provided`，`disabled_message_id()` 给出 `lock-disabled-boundary-missing`（与「服务不应答」的 `lock-disabled-boundary` 区分），`is_retryable()` 对它是 `false`。
- `view_model::project` 因此画出 `Face::Unavailable`：原因、字段与提交控件禁用并各带可读原因，`retry` 为 `None`——**不给一个按了也没有用的重试控件**。
- **标题跟着画面走，提示只在键盘有事可做时出现。** 画不出密码输入的画面（`Unavailable`、`HeldWithoutUi`）把标题换成那一状态本身（「会话锁定无法运行」/「会话已锁定」），并且没有「按回车解锁」这一行。说得更直白一些：一句「请输入密码」配一个禁用字段、下面还写着没有任何东西会校验它，正是本 ADR 要消除的那种邀请——`Face` 的两个变体因此不再带「提示下方的一行」，那一行就是标题。
- `app::update` 的 `Message::BoundaryRetried` 只在 `is_retryable()` 为真时把边界复位成 `Idle`。否则一次重试会让凭据字段变得可输入，把口令送给一条不存在的传输，正是投影的注释里写着「a credential typed for nothing」的那件事。
- `main.rs` 的 `--take-lock` 在请求取锁**之前**就把这两条事实喂给状态（`ProtocolPresent`、`AuthUnavailable(NotProvided)`），所以第一帧起就是诚实的：屏幕说的是它没法校验任何东西，而不是请人输密码。
- **不读键盘**（ADR 0022 决策 6 不变）。屏幕上没有任何可操作的东西，输入路径会是不可能执行的代码；`session.rs` 不绑定 `wl_seat`，渲染也不消费 `Message`。这条与认证边界一起在未决项里。

### 6. 输出消失时显式销毁锁表面

`wl_output` 的全局移除到来时，按角色后表面的顺序调用 `ext_session_lock_surface_v1.destroy` 与 `wl_surface.destroy`。丢弃 `wayland-client` 的代理**不发送任何请求**（只释放本地对象 id），协议要求客户端主动告知，所以必须显式调用。

### 7. `inox-greet`：预览变成完整界面，仍然不认证

- 新增 `view_model`（状态 → 画面，纯函数）、`view`（控件树）、`theme`（令牌 → iced 样式）三个模块，与 L3 组件同形（ADR 0018 接受每个组件各留一份的重复）。
- 六个 `Phase` 各有视觉表示：不可用（说明边界未获批准）、就绪、验证中、会话启动中、失败、已启动。每个禁用的控件带可读原因（D05）。
- **`--preview` 只显示本构建真实所处的那个阶段。** 认证传输的可用性在构造时固定为「没有」——那是本构建的事实——所以运行的二进制显示的是*不可用*那一幅：身份字段不可输入、登录按钮禁用、键盘只剩 Escape。其余五个阶段由测试与截图工件覆盖，没有任何开关能让运行的二进制假装自己有认证边界。这不是预览做得不够完整，而是预览唯一诚实的形态；一个能演出其余阶段的开关会是一台没有人会校验的认证机器。
- 身份字段是真正的 `text_input`：**用户名是公开数据**，不是凭据，它想要光标、选区、输入法与粘贴。凭据永远不进 iced 的编辑器——这个 crate 根本不收集口令，`state::Greeter` 没有存它的字段。
- 键盘由 **crate 自己的焦点模型**驱动：Tab/Shift+Tab 在可操作控件间移动，Enter 操作焦点所在的控件，Escape 清空。映射是纯函数 `app::key_binding` 并有单测。**Space 刻意不是快捷键**：它是用户名里可能出现的字符。
- **提示只说键盘真的能做的事。** `hint` 是 `Option`，由投影按「登录控件是否可用」选出：可用时是完整的一行，不可用但还能清空时只说 Escape，什么都不做时没有提示。一行「按回车登录」配一个禁用的登录控件，和一句「请输入密码」配一个禁用字段是同一类错误——它描述的是一个不存在的界面。
- 「可见的焦点」与「键盘作用的控件」由同一份投影决定：`view_model` 只把可操作的控件放进 `focus_order`，身份字段只在焦点落在它上面时才画成可编辑框，否则画成值（值保存在模型里，因此交回键盘不丢东西）。这不是画法偏好，而是 iced 0.14 的约束：`iced::widget::button` 没有 `id`、不可聚焦，`operation` 也没有 `unfocus`，一个留在树里的文本框会在焦点环说「键盘在按钮上」时继续吃掉按键。
- 默认路径不变：不带 `--preview` 时仍然拒绝真实登录并以非零码退出，文案里仍然写着 ADR 0015。仍然不接 PAM、不启动会话、不是安全登录表面。
- 与锁屏的措辞差异是**刻意的**：锁屏区分「本次构建没有这个服务」与「服务不应答」，因为它的传输能报告两种不同的失败；greeter 的 `available` 只在构造时确定一次，没有第二个事实可命名，因此两者共用 `greet-disabled-unavailable`。

### 8. 会话边界不变

`inox-session` 只在 `INOX_SESSION_TAKE_LOCK=1` 时启动锁屏客户端（ADR 0022 决策 6 与 ADR 0021 的边界），`inox-greet` 只在 `--preview` 下打开界面。本文不新增任何启动路径，也不把锁屏界面变成默认开启的东西。

## 失败、恢复与安全

- **取锁被拒**：`finished` 且没有 `locked`，会话保持未锁定，退出非零（ADR 0022 不变）。
- **合成器长时间不确认**：`CONFIRMATION_TIMEOUT`（10 秒）到期后客户端停止等待并报告 `StoppedBeforeConfirmation`。合成器保留它已接受的东西；本进程不再持有没人应答的请求。这个界是宽而不是紧：确认意味着合成器已经画出锁定帧，在软件渲染的宿主上是工作量而不是一次往返。
- **绘制失败或尺寸不可绘制**：记一条 warn，该输出保留合成器的锁定底色。锁不因此失守，这也是 ADR 0022 已实现的语义，本文只是依赖它。
- **未释放的帧达到上限**：保留已有覆盖帧，停止提交新帧。这是有界退化，不是失守。
- **`OutgoingCommand::Authenticate` 抵达**：构造上不可达（字段被禁用，什么都不会被提交）。仍然显式处理：记下代次、丢弃凭据、不做别的。这条臂存在的意义是让「凭据悄悄流向不存在的地方」在类型之外也不成立。
- **旧事件与代次**：`BoundaryState::is_current` 的规则不变；新增的 `NotProvided` 不可重试，因此 `BoundaryRetried` 不能把屏幕推回可输入状态。
- **敏感内容**：令牌只经 `INOX_LOCK_TOKEN` 进入本进程，不进日志、不进文案；凭据掩码仍由长度推出，明文不进控件树，因此锁屏截图没有可脱敏的内容；greeter 收集的是用户名，不是凭据。
- **本模型不保护的**：同 UID 的非沙箱进程可以读会话进程的环境（ADR 0022 决策 3）；合成器不撤销已经发出的剪贴板 offer（ADR 0017 的既有边界）。界面在软件渲染下的外观与真机外观的差异属 H02。
- **范围理由**：本文不涉及认证、不新增授权判定、不改协议版本、不新增特权路径。它对「谁能解锁会话」的回答是「本构建没有任何人」。

## 兼容、迁移与回退

- **依赖**：`inox-lock` 把 `iced_core` 0.14、`iced_runtime` 0.14、`iced_tiny_skia` 0.14 从 dev-dependencies 提升为 dependencies。三者在 `[workspace.dependencies]` 中已声明、版本不变、已在工作区依赖图内（由 `iced` 0.14 引入），许可证为 MIT/Apache-2.0，均在 `deny.toml` 的允许列表内；`xkbcommon` 等系统库的引入属未决项，本文不引入。**CI 无需改动**：`plan` job 以「manifest 是否声明 `iced`」决定 inox-lock 分片安装字体与收集截图，而 `iced` 一直是它的正式依赖。根 `Cargo.toml` 中「三者仅作 dev-dependency」的注释随之修正。
- **协议**：没有新协议条目。`ext-session-lock-v1` 的用法不变；锁表面内容由单色变为界面是同协议的同一用法，消费者无需改动。`inox_ipc` 仍为 v2。
- **配置**：无新字段，无迁移。
- **用户可见文案**：`inox-lock` 新增两个 id（`lock-boundary-not-provided`、`lock-disabled-boundary-missing`），`inox-greet` 重排为 `greet-` 前缀的一组 id；两个语言同步，`tests/catalog.rs` 双向断言（Q03）。
- **回退**：删除 `render` 模块并让协议腿恢复填一块纯色即可回到 ADR 0022 决策 6 的行为；`--take-lock` 不被默认路径调用，greeter 的默认拒绝路径也不变。已锁定的会话在没有锁客户端时保持锁定，回退不会把它变成可用的桌面。

## 验收与证据

| 验收 ID / 场景 | 责任 crate | E0–E4 等级 | 测试与失败注入 | 通过条件 / 证据位置 |
|----------------|-----------|------------|----------------|----------------------|
| S02 表面内容（锁表面承载界面而非纯色，尺寸与不透明性） | `inox-lock` | E1+E2 | 手写服务端 peer 读回客户端交出的共享内存帧：字节数等于 configure 尺寸的 4 倍、每个 alpha 为 `0xff`、像素不止一种颜色、背景占多数 | `crates/inox-lock/tests/protocol.rs::the_binary_claims_covers_and_never_unlocks_on_a_signal`（扩充） |
| 帧的生命周期（二次 configure 与已释放帧的回收） | `inox-lock` | E1+E2 | 服务端像真实合成器一样在替换缓冲时发 `wl_buffer.release`，并连续三次重新 configure：断言每次都提交了**新尺寸**的帧、被替换的帧收到 `wl_buffer.destroy`、且未释放帧数不超过 `MAX_FRAMES_IN_FLIGHT` | `crates/inox-lock/tests/protocol.rs::a_reconfigure_is_drawn_at_the_new_size_and_the_frame_it_replaced_is_destroyed`（把销毁改成直接丢弃会让它变红） |
| S02 协议腿不回退（取锁、认领、遮蔽、信号不解锁） | `inox-lock`、`inox-compositor` | E1+E2 | 现有断言全部保留：认领带令牌、每个输出一块锁表面、ack 后提交同尺寸帧、SIGTERM 后不发 `unlock_and_destroy`、不销毁锁对象 | 同上；合成器侧 `crates/inox-compositor/tests/session_lock.rs` 不变 |
| 本构建的界面是「不可用」而非「请输密码」 | `inox-lock` | E1 | `view_model` 单测（`NotProvided` 无重试、禁用原因可读）、`app::update` 的不可重试复位、渲染像素（画面里没有激活态按钮）、`render` 单测（两个面画出的帧不同） | `crates/inox-lock/src/{view_model,app,render}.rs` 的 `#[cfg(test)]`；`tests/render.rs` 新增场景 |
| 默认路径仍然拒绝（不假装已锁定） | `inox-lock` | E1 | 真实子进程不带参数运行：非零退出、文案含 ADR 0015 与 `--take-lock`、不含密码提示；`--help` 不需要显示；未知参数失败 | `crates/inox-lock/tests/cli.rs`（新增） |
| D05 与 Q01（禁用原因可读、键盘可达且焦点唯一） | `inox-lock`、`inox-greet` | E1+E2 | 投影单测断言「禁用必有原因」与「焦点只落在可操作控件上」；greet 的 `app` 与 `render` 测试走完 键入→Tab→Enter→清空 整条路径（阶段由测试构造，因为 `--preview` 只能到达不可用那一幅）；焦点环由像素断言 | `crates/inox-greet/src/{view_model,app}.rs`、`tests/render.rs` |
| Q02（截图覆盖浅/深/错误态/200% 字号） | `inox-lock`、`inox-greet` | E2 | 两个 crate 的真实控件树在 1280×720 下渲染，`INOX_SCREENSHOT_DIR` 落盘为 CI 工件；lock 新增「本构建没有边界」场景 | `crates/inox-*/tests/render.rs` + CI 的 `l3-screenshots-<crate>` 工件 |
| Q03（双语目录一致且都在用） | `inox-lock`、`inox-greet` | E1 | `compare_catalogs` 双向断言；源码扫描断言「代码提到的 id 都在目录里、目录里的 id 都被用到」 | 两个 crate 的 `tests/catalog.rs` |
| S02 的 E3（完整会话仍能注销） | `inox-session` | E3 | 现有 `INOX_SESSION_TAKE_LOCK=1` 的 headless 会话用例不变；首帧布局现在落在它的 20 秒启动预算内 | `crates/inox-session/tests/session_lock.rs` |

未取得、也不由本 ADR 声明取得的证据：**真实认证**（ADR 0015 的人类安全评审）；**H02 的真机步骤**（锁屏在两个真实配置上的外观与行为，属 E4）；**Q01 的锁屏侧完整结论**——键盘路径与认证边界一起才能让「仅键盘完成解锁」成立，本 ADR 交付的是其余部分；**Q02 在真实显示器上的外观**。

## 实施顺序与未决项

1. `inox-lock`：`contract::ServiceFailure::NotProvided` 与不可重试规则 → `view_model`/`app` 的对应分支 → `src/render.rs` → 协议腿改由 `app::State` 驱动并提交渲染帧 → 协议测试读回帧内容 → 新增默认路径的 CLI 测试。
2. `inox-greet`：`view_model` → `theme` → `view` → `app` 的 Elm 循环与焦点模型 → 文案目录 → 目录一致性与离屏渲染测试。
3. 文档：两个 crate 的 README 与 `AGENTS.md`、`src/lib.rs` 的「已实现/未实现」清单、PRD §2.3/§13/§10、根 README、`docs/book` 的 roadmap 与组件/协议页、`docs/quality/acceptance.md` 的相关行。
4. CI 与依赖：只在 `Cargo.lock` 记录 `inox-lock` 的依赖清单变化，`plan`/`system-deps` 与 `deny.toml` 不需要改动。

未决项（阻塞或待人类决定，不能以「后续支持」掩盖）：

- **认证边界与键盘输入路径**：PAM 或其它机制、helper 权限与沙箱、凭据传递与销毁、可信连接的建立与撤销——责任角色：安全审查者；最迟解决阶段：启用默认锁屏之前（ADR 0015 的原文要求）。本文不批准其中任何一项，也不单独交付键盘输入：没有可用边界时屏幕上没有可操作的东西，输入路径会是不可能执行的代码。
- **锁屏 UI 崩溃后的接管**：ADR 0022 的未决项不变。现在的界面进程就是锁定进程，它死掉之后新实例仍会被合成器拒绝，会话保持锁定到注销为止。
- **重绘触发面**：本构建的状态在取锁之后不再变化（无边界、无键盘、无挂起），因此「投影变化 → 重绘」这条路径在真实会话里只走一次，只有单测覆盖。接入边界或挂起时必须为它补一条 E3 用例。
- **`inox-lock` 的运行时输入**：`wl_seat`/`wl_keyboard` 的绑定、键位映射（需要 `xkbcommon`）与 iced 事件构造——责任角色：架构组；最迟阶段：与认证边界同一批。
- **greeter 在传输失败后不重新探测。** `Greeter::available` 只在构造时确定；一次 `AuthReply::Unavailable` 会把它置为假并把阶段留在 `Unavailable`，此后的进程不再尝试认证。恢复依靠 greeter 宿主重启该进程（与 ADR 0021 的宿主边界一致），本 crate 不自己重连——责任角色：架构组；最迟阶段：接入真实 greeter 宿主时。
- **AT-SPI 语义导出**：iced 0.14 无桥，Q01 的语义树部分仍未取得，与其余组件相同。

## 后果

正面：M5 点名的「锁屏界面」第一次真的存在于锁表面上——同一个控件树既被 CI 断言像素，也被提交给合成器；协议腿第一次由状态机驱动，`take_outgoing` 这个接口从「没人实现」变成唯一出口，两份覆盖簿记合为一份；「本构建没有认证边界」成为状态里的事实而不是文档里的一句话，因此屏幕上不会出现请人输密码却没人校验的画面；`inox-greet` 从一个骨架预览变成有完整生命周期、可读禁用原因与键盘可达路径的登录界面，而它的默认路径仍然拒绝登录。

负面：锁表面不再是「一块纯色」，因此多了一条渲染路径要维护，以及首帧的字体枚举与文本布局成本（它落在会话启动预算里）；三个 iced crate 进入 `inox-lock` 的运行时依赖面；界面在真实表面上只呈现「不可用」一种状态，键盘路径未交付，所以 M5 的 Q01 与 S02 仍**不能**被读作完整通过；greeter 仍然不是安全登录表面，也不是 M5 的验收对象。

影响面：`crates/inox-lock`（新增 `src/render.rs`，改动 `session.rs`、`app.rs`、`contract.rs`、`view_model.rs`、`main.rs`、`lib.rs`、`Cargo.toml`、两个目录、`tests/*`）、`crates/inox-greet`（新增 `view_model.rs`/`view.rs`/`theme.rs`/`tests/{catalog.rs,support/}`，重写 `app.rs`、两个目录、`tests/render.rs`）、根 `Cargo.toml`（注释）、PRD §2.3/§10/§13 与修订记录、根 `README.md`、`docs/book/src/{roadmap.md,architecture/*,usage/building.md,development/*}`、`docs/quality/acceptance.md`、两个 crate 的 `README.md`/`AGENTS.md`、`docs/adr/README.md`。根级硬性规则（unsafe、unwrap、日志、许可证、i18n、令牌）一律不放宽。

复审条件：引入真实认证或键盘输入路径；把锁屏界面改成 iced 的 session-lock 平台层；支持多 seat、动画或输入法；把锁屏 UI 崩溃后的接管列为通过条件；上游 `ext-session-lock-v1` 离开 staging；或 `inox-greet` 获得真实 greeter 宿主时，重新评估本 ADR 的界面归属与生命周期条目。
