# AGENTS.md — inox-auth

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

**认证边界**。所属层级：**L2**（见根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

它拥有的是：`auth-v1` 私有契约的实现（帧、类型、边界检查）、PAM 认证会话、套接字服务与其对端凭据校验、滚动限额、**VT 分配与显示交回（经 logind）**、**一次登录一个会话负责人进程**，以及运行这一切的特权 helper 二进制 `inox-auth`。它**不**拥有登录界面（`inox-greet`）、不拥有会话监督（`inox-session`）、不拥有宿主编排（`inox-host`）、也不拥有锁屏的解锁路径（未接）。

规范文本是 `crates/inox-protocols/protocol/auth-v1.md`；决策与已知限制见 [ADR 0029](../../docs/adr/0029-inox-authentication-boundary.md) 与 [ADR 0037](../../docs/adr/0037-greeter-host-vt-handoff-and-the-boot-login-chain.md)。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外。**PAM 的 FFI 在 `pam`/`pam-sys` 内部**，本 crate 只用安全封装；需要 FFI 的新功能先问「有没有安全封装」，再考虑写 ADR 请求例外。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`；面向人的文本（用法行与启动前的拒绝）走 Fluent。
- **默认 feature 的边界就是权限边界。** `server`（默认）才含有 PAM、会话启动、logind 与套接字服务；`wire` 永远可编译且**不链接 `libpam`**，也不含 `zbus`。`inox-greet` 以 `default-features = false` 依赖本 crate，这是一条必须保持的性质：**登录界面不得包含能替别人启动会话的代码**。
- **凭据只经 `inox_common::secret::Secret`。** 不得把口令存进 `String`、`Vec<u8>`（解码用的 scratch 除外，且必须在一段作用域内被 `zeroize::Zeroizing` 清零）、环境变量、文件或命令行。`Debug`/`Display` 不得输出口令、口令长度或 PAM 消息文本。给 `Secret` 加 `Clone`/`PartialEq`/`Deref`/`AsRef` 之前先读 ADR 0015。
- **不使用 `pam::PasswordConv`。** 它把口令存进两个 `String` 字段，本 crate 无法清零；认证必须由 `LoginConversation`（`src/pam.rs`）驱动，并在 `pam_authenticate` 返回后立刻清零它持有的那份拷贝。
- **`pam_open_session` 只在会话负责人进程里调用（`pam::open_session`），绝不在守护进程里。** 理由见 ADR 0037 §决策 5：`pam_systemd` 为**调用进程**注册 logind 会话，共享的守护进程调用它会把用户会话的 cgroup 与 VT 绑到错误的进程上。守护进程只做 `pam_authenticate` + `pam_acct_mgmt`，随后立刻 `pam_end`。
- **会话负责人的 PAM 服务必须与认证服务分开。** 负责人没有口令（ADR 0037 §失败、恢复与安全），而 `pam` 0.8 的安全封装只允许在已认证的 client 上 `open_session`，因此负责人的栈必须是 `auth required pam_permit.so` + `account`/`session` 走 `system-login`（默认服务名 `inox-auth-session`，见 `server::DEFAULT_SESSION_SERVICE`，文件由 `packaging/pam.d/inox-auth-session` 提供）。用 `required` 而不是 `optional`：模块缺失时 `optional` 会被 PAM 静默跳过，故障会推迟到「会话打不开」并指向 logind；`required` 在打包问题发生处失败。**绝不可**把认证栈（默认 `inox-auth`）的 `auth` 行改成 permit：那等于让所有人免密登录。
- **负责人不接收口令、不接收线路上的任何东西。** 它只拿到 root 父进程给的账号名与 VT（`--user` / `--vt`），非 root 一律拒绝，VT 必须落在 `--session-vt-min..=--session-vt-max` 内。参数校验在 `leader::validate`（由 `leader::run` 在 root 检查之后调用），单测覆盖。
- **答复不得泄漏账号是否存在。** 不存在的账号、root 与系统账号一律 `Rejected`；`wire::UnavailableReason` 里没有「用户不存在」这一项，新增它等于给屏幕加一个账号枚举探针。
- **不给报文任何权威。** 会话程序、参数、允许的 uid/gid、PAM 服务名、VT 范围、seat 全部来自启动配置；`StartSession` 只带关联号。任何「让线路决定启动什么」的改动都需要 ADR。
- **对端凭据来自 `SO_PEERCRED`**，且必须在读取任何字节之前校验。`std` 的 `UnixStream::peer_cred` 仍是 nightly，因此用 `rustix::net::sockopt::socket_peercred`。
- **续接只能成功一次**，且必须校验 `(uid, attempt, 期限)` 三者；被接受的尝试在转发 `StartSession` 之前就从服务端状态里取走。
- **绝不用 `users` 的组列表 API。** `users` 0.10 的 `get_user_groups` 忽略 `getgrouplist` 返回的计数、把整个 1024 项缓冲映射成组列表，于是列表里会出现 gid 0（root 的组）——[RUSTSEC-2025-0040]，把它交给 `set_thread_groups` 就是提权。附加组必须走 `nix::unistd::getgrouplist` + `session::session_groups`（主组在首位、去重、gid 0 永不进入），并保持「主组为 0 的账号直接拒绝」。
- **降权只能用标准库与 `rustix` 的安全 API**：`CommandExt::{uid, gid}` + `rustix::thread::set_thread_groups`（Linux 上附加组是每线程凭据，必须在 spawn/exec 之前于同一条线程上设置）。**不得**用 `nix::unistd::fork`（`unsafe fn`）、`pre_exec`（`unsafe`）或 `setsid` 绕过这条。**不得**在设组失败时继续启动会话。
- **VT 与 seat 归 logind + libseat。** 本 crate 只通过 `logind::Logind` 发 D-Bus 请求（`GetSessionByVT`、`GetSessionByPID`、`ListSessions`、`Seat.ActiveSession`、`ActivateSession`），**不得**执行任何 VT ioctl、**不得**打开或转发 DRM/输入设备 fd。VT 选择与交回必须是纯函数 + 可注入的 `Logind`，以便无 D-Bus 单测。
- **一次只允许一个用户会话**（v1，ADR 0037 §决策 2）：会话存活期间的第二次 `StartSession`/`Authenticate` 一律 `Unavailable(Busy)`。
- **logind 不可达时降级，不阻断登录**：`warn!` 后按范围首个空闲 VT 继续；没有可记录的会话就不做交回（`debug!`）。CI 与开发实例没有 logind，登录链不得因此失败。
- **`auth-v1` 帧/字段/答复字节不变。** 任何改动先改 `crates/inox-protocols/protocol/auth-v1.md` 并走版本化流程。
- **不写 `unsafe`、不绕 CI、不自行合并**（根级 §3、§9）。
- **已知限制必须留在代码、`README.md` 与本文件里**（见下节），**不得**在文档里把这些写成已实现。

## 已知限制（必须如实保留）

- 真机 PAM/VT 行为是 **E4**：本 crate 的 CI 只能证明「能调用、能映射、不 panic」，登录成功与否必须由人在真机验证；文档不得写成已验证。
- 未安装 `inox-auth-session`（或等价的 permit-auth 会话栈）时，会话负责人无法打开会话，`StartSession` 会答复 `started:false`（原因在日志里）。这是当前 `packaging/` 与本 crate 的接口约定。
- logind 不可达（CI、开发实例、非 systemd 机器）时：VT 不经过仲裁、不切换、也不交回；`xdg` 变量由本进程按配置给出。会话本身仍会启动。
- 启动自愈是保守的：只在 seat 上**没有活动会话**、且**能按 greeter uid 唯一识别**出 greeter 会话时才 `ActivateSession`；识别不出（0 个或多个）就什么都不做（`debug!`）。
- `--exit-after-session` 在 ADR 0037 §决策 5 之后不再是环境泄漏的必要缓解，只作为部署状态复位保留（默认关闭）。
- 会话没有 `setsid`（`Command` 无法在不使用 `pre_exec` 的情况下建立新 session；会话领导权来自 `pam_systemd`/logind）。
- libpam 内部会构造一份不受本 crate 控制的 `CString`；`Secret` 未 `mlock`；`pam` 0.8 的 `Client` 内部会调用 `env::set_var`（只对 USER/HOME/PWD/SHELL），本 crate 不依赖也不读取该进程环境。
- `pam` 0.8 的 `Client::get_user` 在 `PAM_USER` 未被解析时会命中它内部的 `assert`（库内 panic）。负责人在 `open_session` 之前显式调用 `get_user()`，让这种情况尽早发生、并在父进程侧表现为 `started:false`；`packaging/pam.d/inox-auth-session` 的 `account` 走 `system-login`，其 `pam_unix` 会解析用户。
- 负责人的确认行写在 `pam_open_session` 成功之后、`exec` 之前；`exec` 若在此之后失败，登录界面会先看到 `started:true`，随后会话立刻结束并交回显示（日志里有原因）。
- 多用户同时登录、会话选择菜单、挂起/恢复矩阵未实现（ADR 0037 §未决项）。

## 主要依赖

- 上游 Inox crate：`inox-common`（L1：`secret::Secret`、i18n 装载）。
- 第三方（`server` feature）：`pam` 0.8（MIT OR Apache-2.0，安全封装，传递 `pam-sys 1.0.0-alpha5`）、`nix` 0.31（MIT，`unistd::getgrouplist`）、`users` 0.10（MIT，**只**用于 `get_user_by_name` 的 uid/gid/home/shell）、`rustix` 1（`net`/`process`/`fs`/`thread`）、`zbus` 6（MIT，logind 客户端；工作区已声明，ADR 0027/0037）、`anyhow`（仅二进制）、`tracing-subscriber`。
- 第三方（始终）：`thiserror`、`zeroize`、`tracing`、`inox-common`。
- 依赖安全：本 crate 的引入让 `users` 进入依赖图，三条 RUSTSEC 记录在 `deny.toml` 中带理由忽略（ADR 0029 §3.5）。**新增对 `users` 的调用前先读那段注释**：组相关 API 一律不得使用。
- 测试：`tempfile`、`rustix`（dev，取自身 euid）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-auth --all-features` |
| 本 crate 构建 | `cargo build -p inox-auth --release --all-features` |
| 仅契约（不链接 PAM、不含 logind） | `cargo check -p inox-auth --no-default-features` |
| 本 crate 文档 | `cargo doc -p inox-auth --no-deps --all-features` |
| 静态检查 | `cargo clippy -p inox-auth --all-targets --all-features -- -D warnings` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |

## 测试要求

- 纯逻辑必须有单元测试，**含拒绝与错误路径**：帧的每种畸形、滚动限额的窗口边界、PAM 返回码到答复的映射、账号策略（root/系统账号/主组为 root 的账号、gid 0 永不进入会话组列表、重复组折叠、附加组读取失败的降级）、套接字模式、续接的消费与过期。
- ADR 0037 新增的规则必须各有对应用例：VT 范围校验、空闲 VT 选择（跳过活动会话的 VT、跳过被占用的 VT、范围耗尽、上限边界、无 logind 的降级）、负责人的参数校验（非 root/越界 VT/非登录账号）、负责人确认管道（成功/失败/无输出/超时）、一次只允许一个会话、交回序列（记录了哪个会话就交回哪个、没有记录就不交回、启动时自愈）。VT 选择与交回用可注入的 `Logind` 断言**调用序列**。
- `tests/boundary.rs` 用**真实套接字**驱动真实 `server::run_with`，只把 PAM、logind 与会话负责人换成脚本化实现（系统边界，根级 §6 允许打桩，而**服务自己的规则一个都不许 mock**）。新增任何服务端规则必须同时新增对应用例。
- 真实 PAM 路径在 CI 里只能验证「能调用、能映射、不 panic」（`tests/pam.rs`）；**真机登录属于 E4，必须由人完成**，不得在文档里写成已验证。
- 测试代码不受「禁止 unwrap/expect/panic」约束。

## 目录结构

```
crates/inox-auth/
  Cargo.toml    工作区继承：字段一律 *.workspace = true；feature 见上
  src/          lib.rs（边界与 feature 门）、wire.rs（契约实现）、pam.rs（认证 + 负责人会话）、
                boundary.rs（桩接缝）、session.rs（账号与 exec/spawn）、server.rs（套接字服务与
                一次登录的生命周期）、logind.rs（VT 选择/交回，D-Bus）、leader.rs（会话负责人与
                启动它的 launcher）、limit.rs（滚动限额）、i18n.rs（Fluent）
  i18n/         en-US / zh-CN 的 Fluent 目录
  tests/        boundary.rs（真实套接字 + 注入的 logind/launcher）、pam.rs（真实 PAM 的接缝）、
                catalog.rs、cli.rs
  README.md     本 crate 说明
  AGENTS.md     本文件（CLAUDE.md 是其符号链接）
```

## 提交信息格式

Conventional Commits，scope 使用 `auth`；纯文档改动 scope 使用 `docs`。
