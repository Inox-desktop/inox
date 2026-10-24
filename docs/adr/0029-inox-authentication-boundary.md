# ADR 0029: 认证边界、`auth-v1` 私有契约与登录会话启动

- 状态：accepted
- 日期：2026-10-17
- 决策者：架构组与安全审查者
- 批准记录：2026-10-17 本仓库维护者在开发会话中明确批准本次实施——批准 ADR 0015 点名的认证边界（系统认证机制、helper 权限、凭据传递与清理），授权把口令输入加入 `inox-greet` 并在本 ADR 与本文件的 crate 规则中记录，并授权按本文实现。**本 ADR 是 ADR 0015「实现前必须先完成人类安全审查，实际认证依赖、许可证与最小 helper 权限在实现 ADR/PR 记录」那一条所要求的那份记录**；它批准的是这次实施与本文记录的决策，不批准把 M5 标为完成（见 §6、§8）。
- 关系：实施 ADR 0015 的「锁定与认证」条目中关于认证边界与凭据卫生的部分，并给出其要求的依赖/许可证/最小权限记录；细化 ADR 0021 的 «greeter 宿主» 未决项——**只细化，不关闭**，宿主仍不存在；复用 ADR 0026 的「契约文档在 `inox-protocols`、类型在消费者」的做法（`protocol/auth-v1.md`）；为 `inox-greet` 增加一条向下的 L4 → L2 依赖边（`inox-auth`，`default-features = false`），方向合法、不新增同层边、不违反 ADR 0002 规则 1；不改变 ADR 0013、0018、0022、0025 的任何决策。- 关系：实施 ADR 0015 的「锁定与认证」条目中关于认证边界与凭据卫生的部分，并给出其要求的依赖/许可证/最小权限记录；细化 ADR 0021 的 «greeter 宿主» 未决项——**只细化，不关闭**，宿主仍不存在；复用 ADR 0026 的「契约文档在 `inox-protocols`、类型在消费者」的做法（`protocol/auth-v1.md`）；为 `inox-greet` 增加一条向下的 L4 → L2 依赖边（`inox-auth`，`default-features = false`），方向合法、不新增同层边、不违反 ADR 0002 规则 1；不改变 ADR 0013、0018、0022、0025 的任何决策。
- 后继：[ADR 0037](./0037-greeter-host-vt-handoff-and-the-boot-login-chain.md)（2026-10-23，accepted）关闭本文 §6.6 的 «greeter 宿主仍不存在»，并把 §6.3 的会话生命周期改成**一次登录一个会话负责人进程**（`pam_open_session` 在负责人里，`--exit-after-session` 由此不再必要、只作状态复位）；`auth-v1` 的帧、字段与答复语义不变。本文正文作为历史记录保持原样。

## 背景

ADR 0015 把真实认证定为**人类安全评审项**：PAM 或其他机制、helper 权限与沙箱、凭据传递与清理，未确定即为阻塞。ADR 0021 与 ADR 0025 因此把登录界面做成「本构建真实所处的阶段」——界面完整但不可用，二进制默认拒绝真实登录，`--preview` 只是界面预览。2026-10-17 维护者批准了该评审项，于是缺口的形状变成工程问题：

- `inox-greet` 的状态机、控件树、焦点模型、限流与期限都已实现且被测试，但没有任何东西能回答一次认证：`Command::Authenticate` 排队却无人排空（`crates/inox-greet/src/app.rs` 原文如此），`App::can_submit` 要求 `available`，而所有构造点都传 `false`。
- 工作区没有任何认证后端：全仓库没有 PAM 依赖，也没有任何 crate 依赖 `inox-greet`。
- 凭据类型已经存在且经过审查：`inox-lock` 的 `credential::Secret`（清零、不可打印、唯一出口），但它是 L3 组件私有的一份。

## 备选方案与取舍

| 方案 | 收益 | 成本 / 风险 | 选择或否决依据 |
|------|------|-------------|----------------|
| 继续不做认证 | 零风险、零代码 | ADR 0015 的阻塞项已被批准；再拖等于把「显示管理器」永远留在预览状态 | 否决 |
| 自己写 PAM FFI | 不引入第三方 crate | 需要在 Inox crate 内写 `unsafe`（根级 §3.2 只给合成器/XWayland 例外），FFI 生命周期与 `pam_end` 语义要自己扛 | 否决：用**安全封装**就能不碰硬规则 |
| `pam` 0.7 | 依赖稳定（`pam-sys` 0.5.6 正式版） | `PamError` 的返回码不公开：**无法区分「口令错」与「PAM 坏了」**，界面只能二选一地撒谎 | 否决（见下） |
| **`pam` 0.8** | 唯一的 `PamReturnCode` 公开版本，能把拒绝与边界故障分开（本 ADR §2.2） | 其 `pam-sys` 依赖是 `1.0.0-alpha5`（预发布）；`users ^0.10` | **采用**：区分这两种答复是界面的诚实性问题，alpha pin 由 `Cargo.lock` 固定并在 §7 记录 |
| 复用系统 greetd | 认证/权限/VT 全部交给成熟守护进程，代码量最小 | 把 Inox 的显示管理器降级为别人的 greeter 客户端；引入一个外部 GPL-3.0 守护进程作为运行前提；PRD §3.1 的「显示管理器」不再是本仓库的能力 | 否决：与产品定位冲突，且让认证边界落在本仓库之外 |
| 每条连接一个尝试（被接受的尝试绑定连接） | 「一条连接=一次尝试」最简单 | 界面无法在「密码已对」与「用户确认启动会话」之间做决定：要么提前启动会话，要么把套接字挂在 UI 决定上 | 否决：见 §2.4 |
| 在 helper 内 `fork` + `setuid` | 一次调用完成降权 | `nix::unistd::fork` 是 `unsafe fn`，`std` 的 `CommandExt::groups` 仍是 nightly：无 `unsafe` 就无法安全降权 | 否决：改用 `Command::uid/gid` + `rustix::thread::set_thread_groups`（§2.5） |

## 决策

### 1. 分层与所有权

1. 新增 **L2 crate `inox-auth`**（`crates/inox-auth`），拥有：`auth-v1` 的实现（帧、类型、边界检查）、PAM 会话、套接字服务、滚动限额、以目标账号身份启动会话，以及特权 helper 二进制 `inox-auth`。它**不**拥有登录界面，也不拥有会话监督。
2. 该 crate 分两个 feature：`wire`（恒有：帧与类型，**不含** PAM、不链接 `libpam`）与 `server`（默认开启：PAM、会话启动、套接字服务、二进制）。`inox-greet` 以 `default-features = false` 依赖它：**登录界面不链接 `libpam`，也不包含能替别人启动会话的代码**。这是本 crate 默认 feature 的意义所在，不是细节。
3. 账号名语法、凭据上限与帧边界只有一份实现（`wire`），界面与服务端调用同一个函数；`inox-greet::state::valid_username` 是它的再导出，不再有第二份语法。
4. **凭据类型下沉到 L1**：`inox_common::secret::Secret`（清零、不可打印、唯一出口）成为唯一的凭据缓冲实现，被 `inox-auth`、`inox-protocols` 的契约实现路径与 `inox-greet` 消费。它放在 L1 是因为 L2 的协议层与 L4 的界面都必须持有同一组不变量，而**两份安全不变量就是两次漂移的机会**；这也满足 `inox-common`「新增模块只在出现真实消费者时引入」的规则（消费者是这三个 crate）。
   - `inox-lock` **本次不动**：它的解锁路径是另一个里程碑（ADR 0022 未决项），改动它不在本次授权范围内。两份实现必须保持同一组规则；**当解锁路径落地时，第一件事是把 `inox-lock::credential` 换成 `inox_common::secret`**，届时删除第二份。
5. `inox-protocols` 只增加**契约文档** `protocol/auth-v1.md`，不增加代码，也不新增任何 Inox 依赖：沿用 ADR 0026 对配置契约的做法（文档是规范文本，类型留在说它的 crate 里，因此不产生层级边）。

### 2. 边界本身

1. **系统认证机制是 PAM**，经**安全封装** `pam` 0.8 使用（`crates/inox-auth` 保持 `#![forbid(unsafe_code)]`），服务名默认 `inox-greet`，部署在 `/etc/pam.d/inox-greet`。**不自研口令哈希/校验，不由合成器或普通 Shell 接触口令**（ADR 0015 原文）。
2. **返回码映射必须区分「拒绝」与「边界故障」**：`Auth_Err`/`User_Unknown`/`Cred_Insufficient`/`Authinfo_Unavail`/`Perm_Denied`/`MaxTries`/`Acct_Expired`/`Cred_Expired`/`Cred_Err`/`New_Authtok_Reqd`/`Try_Again`/`AuthTok_*` 记为 `Rejected`；其余（`System_Err`/`Service_Err`/`Module_Unknown`/`Abort`/`Conv_Err` …）记为 `Unavailable(Failed)`。**不存在的账号按 `Rejected` 答复**：屏幕上没有任何「该用户不存在」的文案，否则登录界面就是账号枚举探针。
3. **helper 政策**（最小权限的落点）：
   - 服务**系统套接字必须 root**：非 root 且 `--socket` 指向默认路径时在启动前拒绝。显式指定其他路径的实例按开发实例处理并在日志里说明。
   - **root、uid < `minimum_uid`（默认 1000）的账号一律不启动会话**，与账号是否存在无关。
   - helper 自身不保留任何会话私有状态；会话程序、服务名、允许的 uid/gid 全部来自启动参数与账号数据库，**报文中只有账号名、口令与关联号**。
   - 套接字：默认 `/run/inox/auth.sock`，目录 `0755`、文件 `0660` 且属组为 greeter 的组（存在 `greeter` 账号时自动识别其 uid/gid，否则由 `--allow-uid`/`--socket-group` 指定）；路径存在但不是套接字即拒绝，是套接字但有人监听即报「已在运行」。
   - **对端凭据只来自 `SO_PEERCRED`**（`rustix::net::sockopt::socket_peercred`，`std` 的 `peer_cred` 仍是 nightly），在任何读取之前校验；不在允许集合内的对端得到 `Error{UnauthorizedPeer}`。
4. **被接受的尝试绑定到 `(对端 uid, attempt, 期限)`，不绑定连接**：认证成功即记住该尝试并结束连接；`StartSession` 由**同 uid 的后续连接**发起，且只能成功一次，未被续接则在 30 s 后过期撤销。理由：界面必须能在认证成功后仍决定「不启动」（取消、关窗），而连接绑定的两个出路都不可接受（提前启动，或把套接字挂在一个 UI 决定上）。同 uid 的另一个进程本来就有权发起登录，因此这不扩大任何权限；续接只在服务端校验 uid 与期限，且不产生新的会话。
5. **会话启动不写 `unsafe`**：以目标账号的 uid/gid 用 `std::os::unix::process::CommandExt::{uid, gid}`（标准库在 `fork` 与 `exec` 之间完成降权），附加组用 `rustix::thread::set_thread_groups` —— Linux 上附加组是**每线程**凭据，而在 fork 出子进程的那条线程上设置，子进程继承的正是这份列表；`std` 的 `CommandExt::groups` 至今仍是 nightly，而只要 `setgid` 而不设组，会话就会带着 **root 的附加组**运行。二者顺序（先设组，再 spawn）写在 `session::start` 里。
6. **未打开 PAM 会话就不算会话**：`pam_authenticate` 成功只是「可以启动」，`pam_open_session` 只在真的启动会话时调用，失败即答复 `SessionStarted{started:false}` 并且什么都不启动。会话运行期间认证器线程保持 PAM 会话打开并在子进程退出后 `pam_close_session`，因此同一时间只有一个尝试（新的尝试得到 `Unavailable{Busy}`）。
7. **限额**：服务端自身做滚动限额（默认 60 s 内 3 次，与锁屏同一条规则），**在账号查询之前**执行：它约束的是特权工作的频率，而不是名字的拼写；客户端（登录界面）另有一份同样的限额，那是给人的礼貌，不是边界。
8. **凭据卫生**（ADR 0015 三条要求的落点）：
   - 口令从解码起就在 `Secret` 里，解码用的 scratch 缓冲在一段作用域内自清零，`Debug`/`Display` 只输出 `<redacted>`。
   - PAM 会话由自定义 `Conversation` 实现驱动（**不使用 `PasswordConv`**，它会把口令存进 `String`），`pam_authenticate` 一返回就清零会话持有的那份拷贝；`open_session` 不需要凭据。
   - **PAM 的消息文本不写日志**：只记录「有消息到达」，不记录内容（第三方模块的消息可能包含账号名或对输入的猜测）。
   - 界面侧：口令字段是**掩码**（`MASK_GLYPH` 按长度重复），按键经 `app::key_binding` 以消息到达，**永不进 `iced::widget::text_input`**（iced 的编辑器持有本 crate 无法清零的 `String`，还提供复制粘贴）；提交时 `Event::Submit` 把 `Secret` **移动**进命令，模型不再留副本，因此被拒绝的登录是重新输入而不是重放。
   - 投影（可被测试打印、可进日志）只拿到**字符数**，从不拿到值。
9. **`inox-greet` 的默认路径语义随之改变**：不带参数时是真实路径，必须能找到边界套接字，否则在开窗之前以非零码退出并说明（不再是「ADR 0015 未评审」的理由）；`--preview` 仍是「没有边界时的界面」。屏幕上「仅供预览」的横幅只在 `--preview` 出现，真实构建的横幅是「登录后将启动 Inox 会话」，因为**在真的会收集凭据时说不收集凭据，是最坏的一种撒谎**。会话启动后界面关闭自己的窗口，把显示留给会话。

### 3. 依赖、许可证与 CI

1. 新增第三方依赖（均在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明一次）：`pam = "0.8"`（MIT OR Apache-2.0）、传递依赖 `pam-sys 1.0.0-alpha5`（MIT OR Apache-2.0，由 `Cargo.lock` 固定，预发布状态见 §7）、`users = "0.10"`（MIT，供 `pam` 与账号查询使用）、`libc`（已在图中）。全部落在 `deny.toml` 的白名单内。
2. **没有新增 `unsafe`**：`inox-auth` 与 `inox-common` 都 `#![forbid(unsafe_code)]`，FFI 在 `pam`/`pam-sys`/`users`/`rustix` 内部。根级 §3.2 的例外清单不变。
3. CI：`pam-sys` 的构建脚本链接系统 `libpam`/`libpam_misc`，因此 `libpam0g-dev` 加入 `.github/actions/system-deps` 的公共列表（clippy/doc 不链接最终二进制，但构建脚本仍会运行）。测试分片由层级表推导，`inox-auth` 登记在 `l2` 后自动获得自己的分片。
4. `inox-greet` 新增一条依赖边 `inox-greet → inox-auth`（L4 → L2，向下、合法、带 `default-features = false`）；`layering.rs` 无需新增例外。
5. **`users` 1.0 之前不再被本工作区直接用于组列表**。`users` 0.10 有三条 RUSTSEC 记录，其中一条（[RUSTSEC-2025-0040]）正是**我们原本要用的那个函数**：它的 `get_user_groups` 分配固定 1024 项缓冲、忽略 `getgrouplist` 写回的计数、再把整个缓冲映射成组列表，于是真实条目之后的每一项都读成 gid 0（root 的组）；把它交给 `set_thread_groups`，会话就会带着 root 组运行——即提权。因此：
   - 会话的附加组改由 `nix::unistd::getgrouplist`（安全封装，按返回计数 `set_len`，缓冲区按需增长）取得，`nix` 已在依赖图中（calloop 传递依赖）；
   - 结果再经 `session_groups` 过滤：主组在首位、去重、**gid 0 永不进入列表**（纵深防御，且对「库返回什么」不设信任）；
   - 主组为 gid 0 的账号直接拒绝（`eligible`），因为它无论如何都会带 root 组；
   - `users` 仅因 `pam` 0.8 的 `open_session` 需要它而留在图中，唯一调用点是 `get_user_by_name`（只读 `pw_name`/`pw_uid`/`pw_gid`/`pw_dir`/`pw_shell`），**不触碰**上述三条记录涉及的代码路径（组查询、`gr_mem` 的非对齐读取、组列表）。三条记录因此在 `deny.toml` 中带理由忽略，并在注释里写明同样的推理；
   - 移除条件是 `pam` 不再依赖 `users`（或出现维护中的等价封装）——届时删掉 `deny.toml` 的三条忽略。

[RUSTSEC-2025-0040]: https://rustsec.org/advisories/RUSTSEC-2025-0040

## 验收与证据

**本次 CI 能给出的（E0/E1）**：

- `inox-auth` 单元测试：帧的往返与拒绝路径（未知 tag、超长帧在分配前拒绝、截断、多余字节、版本不匹配、超长账号名/口令）、滚动限额的边界、PAM 返回码到答复的映射、账号策略（root 与系统账号被拒、附加组读取失败只降级为单组）、套接字模式与对端判定、会话计划（程序来自配置、工作目录与 `XDG_RUNTIME_DIR` 的推导、`is_executable`）。
- `inox-auth` 真实套接字集成测试（19 项）：真实 `server::run` + 真实 `auth-v1` 帧 + 脚本化认证器，覆盖「接受→另一条连接续接→会话启动」「拒绝不启动」「边界故障≠口令错」「未知账号不花认证器的时间」「root 无论配置如何都不能登录」「跨连接的滚动限额」「不被服务的对端在任何读取前被拒」「领先于握手的请求被拒」「未知 tag」「超长帧」「版本不匹配」「取消不启动」「未接受的尝试不能续接」「续接只能用一次」「过期尝试不启动」「会话运行中为忙」「会话启动失败如实答复 `started:false`」「`--exit-after-session` 在会话结束后退出并删除套接字」「答复里的关联号原样回显」「答复不回显客户端发过的东西」「客户端中途消失不卡死服务」。
- `inox-greet` 单元测试与集成测试：状态机（口令只在焦点落在口令字段时进入模型、提交即移交且不留副本、取消与拒绝后重新输入、超长口令拒绝而非截断、空口令可提交）、`auth-v1` 客户端（真实套接字 + 手写假服务端：请求内容、四种答复的映射、边界不在时为 `Unreachable`、版本不匹配为 `Unreachable`）、离屏渲染（双主题 × 双语言 × 200% 字号、口令字段画出的是计数而不是值、超长口令以 danger 色给出理由）、目录双向检查、CLI（`--help`、未知参数、没有边界时的拒绝）。
- 分层与依赖检查：`cargo test -p inox-integration --test layering` 覆盖新 crate 与新边。

**本次 CI 不能给出、必须由人在真机上完成的（E4）**：

1. 在真实 `/etc/pam.d/inox-greet` 上以真实账号登录一次，并确认会话真的起来（`pam_systemd` 是否注册了 logind 会话、`XDG_RUNTIME_DIR` 是否就绪、libseat 是否拿到 seat）。
2. VT 归属与切换：helper/界面/会话之间的 VT 交接没有实现（见 §6）。
3. 真实显示器上的外观与输入路径（登录界面仍是 xdg-shell 窗口，见 §6）。
4. 多用户切换、失败注入（PAM 模块崩溃、套接字被替换）等 ADR 0015 点名的 E3 场景。

## 后果

**正面**：认证边界有了真实实现与可复核的契约；登录界面能真正登录，同时保留了「拒绝与故障可区分、答复可关联、限额有限、凭据不落地」这些已经过审查的性质；登录界面不链接 `libpam`；没有新增 `unsafe`；依赖全部在白名单内。

**负面 / 已知限制（逐条记录，不粉饰）**：

1. `pam-sys` 是预发布版本（`1.0.0-alpha5`）。缓解：唯一消费者是本 crate 的安全封装，版本由 `Cargo.lock` 固定；`pam-sys 1.0` 正式发布时应评估升级。
2. libpam 的会话回调会为主机命令构造一个 `CString`，其内存在 `libpam` 与模块内部不受本 crate 控制；`Secret` 也未 `mlock`，可能落到 swap。这是**进程与库的属性**，不是本 crate 能声明的保证（ADR 0015 原文亦如此）。
3. `pam_open_session` 会在 **helper 进程**里写入会话环境变量（`env::set_var`，在 `pam` crate 内以 edition 2018 编译）。若 helper 长期存活，上一个账号的会话变量可能影响下一个。缓解：`--exit-after-session` 让每次登录都是新进程（会话结束后 helper 退出并删除套接字，由宿主重新拉起）；默认**不**开启，因为宿主尚未存在。**多用户切换之前必须开启它。**
4. 会话不是新的会话首进程（没有 `setsid`）：`Command` 无法在无 `pre_exec` 的情况下调用它。现状是它有自己的进程组（`process_group(0)`）；真正的会话领导权预期来自 `pam_systemd`/logind。**未在真机验证。**
5. helper 必须 root 才能读账号数据库并启动别人的会话；非 root 实例只可能是开发实例（日志中说明）。会话的附加组在非 root 实例下不设置（`setgroups` 需要 `CAP_SETGID`），此时进程本来就只能启动自己的会话。
6. **greeter 宿主仍不存在**：谁把登录界面在裸 VT 上拉起来、登录界面在会话启动后如何退出显示、两个阶段之间如何交接，都不在本次范围内（ADR 0021 的未决项）。因此「真机登录」目前需要人在一个已有显示环境里手动运行 `inox-auth` 与 `inox-greet`。
7. `inox-lock` 的解锁路径**未接**本边界：锁屏的键盘输入与解锁仍是 ADR 0022/0025 的未决项，本 ADR 只把边界做出来。
8. AT-SPI 语义导出与 waydriver 证据仍不存在（ADR 0018/0021 未决项）。

## 回退

`inox-auth` 是新增 crate，`inox-greet` 的改动集中在状态机、传输与文案。回退方式：

- 想让登录界面回到「只预览」的状态：把 `inox-greet` 的默认路径改回拒绝（并同步 `tests/cli.rs` 与 `greet-banner`）——但**不得**让二进制在没有边界时表现出可以登录的样子。
- 想完全移除认证：删掉 `crates/inox-auth`、`protocol/auth-v1.md`、层级表条目与 `inox-greet` 的依赖边，并恢复 `inox-greet/AGENTS.md` 的凭据条款。此时 `inox_common::secret` 变为无消费者，可一并删除或保留给锁屏（后者更合适：解锁路径落地时会用到）。
- 没有磁盘 schema 或配置格式变更，不需要迁移。

## 复审条件

新增或改变以下任何一项时必须复审本文并另提实施 ADR：

- 认证机制（改用非 PAM 的后端、加入 2FA/生物识别、多因素会话）
- helper 的权限模型（去除 root、加入沙箱/namespace、加入 systemd 单元与 polkit 策略）
- helper 的存活模型（默认开启 `--exit-after-session`、引入会话级 helper、多用户同时登录）
- 契约的破坏性变更（新增请求/答复、变更续接语义、把会话程序搬上线路）
- greeter 宿主与 VT/seat 交接（ADR 0021 未决项）
- 锁屏解锁路径接入本边界（同时合并 §1.4 的两份凭据实现）
- 依赖大版本（`pam`/`pam-sys` 转正、`users` 升级）
