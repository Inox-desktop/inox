# inox-auth

Inox 的**认证边界**：唯一允许校验口令、唯一允许启动别人的会话的组件。

- 层级：**L2**（见根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）
- 二进制：`inox-auth`（特权 helper，服务系统套接字时需要 root）
- 契约：`crates/inox-protocols/protocol/auth-v1.md`（规范文本）
- 决策：[ADR 0029](../../docs/adr/0029-inox-authentication-boundary.md)（认证边界）、
  [ADR 0037](../../docs/adr/0037-greeter-host-vt-handoff-and-the-boot-login-chain.md)（VT/seat 交接与会话负责人）

## 它做什么

```
登录界面（inox-greet，greeter 用户）
        │  auth-v1：Hello / Authenticate / StartSession / Cancel
        ▼
/run/inox/auth.sock（root:greeter，0660）
        │
        ▼
inox-auth（root）
  Authenticate：SO_PEERCRED → 账号策略 → 滚动限额 → PAM → Accepted/Rejected
  StartSession：记录当前活动会话 → logind 选空闲 VT → 启动会话负责人
        │                                      │（管道："started" / "failed"）
        │                                      ▼
        │                        inox-auth --session-leader --user … --vt …
        │                          pam_open_session（root，会话栈）
        │                          logind.ActivateSession(自己的会话)
        │                          set_thread_groups → exec 会话程序（目标账号）
        ▼
  会话结束 → reap → ActivateSession(记录的会话，通常是 greeter)
```

- **PAM 认证**：经安全封装 `pam` 0.8 使用系统 PAM 栈（认证服务默认 `inox-auth`），不自研口令校验。
- **凭据**：全程 `inox_common::secret::Secret`（清零、不可打印、唯一出口）；不使用 `pam::PasswordConv`（它把口令存进 `String`）；PAM 的消息文本不写日志。**会话负责人从不接触口令**。
- **对端**：`SO_PEERCRED` 的 uid/gid，在读取任何字节之前校验。
- **会话**：`pam_open_session` 只在会话负责人进程里运行（`pam_systemd` 为调用进程注册 logind 会话），随后同一进程 `set_thread_groups` + `exec` 固定程序（默认 `inox-session`）。程序来自配置、永不上线路。
- **附加组**：`nix::unistd::getgrouplist` + 过滤（主组在首位、去重、gid 0 永不进入）。**不使用** `users` 的组列表 API：它会把 root 的组写进列表（RUSTSEC-2025-0040）。
- **VT/seat**：只问 `org.freedesktop.login1`（`GetSessionByVT`、`GetSessionByPID`、`Seat.ActiveSession`、`ActivateSession`）。不执行 VT ioctl，不打开或转发 DRM fd。VT 默认范围 **2..=6**（跳过 greeter 当前所在 VT）。
- **单会话**：v1 同时只允许一个用户会话；第二个 `StartSession` 答复 `Unavailable(Busy)`。
- **交回**：会话进程退出后 `ActivateSession(启动前记录的会话)`，画面回到登录界面；`inox-auth` 重启时若 seat 上没有活动会话，会保守地把 greeter 自己的会话放回。
- **不泄漏账号是否存在**：不存在的账号、root、系统账号一律按「拒绝」答复。

## 两种构建

| feature | 内容 | 谁用 |
|---------|------|------|
| `wire`（`default-features = false`） | 帧与类型，**不链接 `libpam`**、不含 `zbus` | `inox-greet`：登录界面不得包含能替别人启动会话的代码 |
| `server`（默认） | PAM、logind、会话负责人、套接字服务、`inox-auth` 二进制 | 部署与测试 |

```sh
cargo check -p inox-auth --no-default-features   # 只检查契约
cargo test  -p inox-auth --all-features          # 完整边界
```

## 运行

```sh
# 系统实例（需要 root；默认 /run/inox/auth.sock，自动识别 greeter 账号）
sudo inox-auth

# 开发实例（显式 socket 与允许的对端；非 root 会被记为开发实例）
inox-auth --socket /tmp/inox-auth.sock --allow-uid "$(id -u)" --session-program /bin/true
```

### 参数

| 参数 | 默认 | 说明 |
|------|------|------|
| `--socket PATH` | `/run/inox/auth.sock` | 套接字 |
| `--socket-group GID` / `--allow-uid UID` / `--allow-gid GID` | greeter 账号 | 允许连接的对端 |
| `--service NAME` | `inox-auth` | **认证**用的 PAM 栈 |
| `--session-service NAME` | `inox-auth-session` | **会话负责人**用的 PAM 栈（必须是 permit-auth，见下） |
| `--minimum-uid UID` | `1000` | 系统账号下限（root 永远拒绝） |
| `--session-program PATH` | `inox-session` | 会话程序；永不上线路 |
| `--session-arg ARG` | 无 | 传给会话程序的字面参数（可重复） |
| `--session-vt-min N` / `--session-vt-max N` | `2` / `6` | 用户会话可用的 VT 范围 |
| `--seat NAME` | `seat0` | logind seat |
| `--attempts N` / `--window-seconds N` | `3` / `60` | 滚动限额 |
| `--exit-after-session` | 关闭 | 会话结束后退出整个 helper（部署状态复位；ADR 0037 之后不再是环境泄漏的必要缓解） |

内部模式（**不是给人用的**，只由 helper 自己以 root 启动）：

```sh
inox-auth --session-leader --user <name> --vt <n> \
          --session-service inox-auth-session \
          --session-program /usr/bin/inox-session --seat seat0 \
          --session-vt-min 2 --session-vt-max 6
```

它校验 root、账号策略与 VT 范围，`pam_open_session` 成功后在标准输出回报一行 `started`，随后 `exec` 会话程序。

## 部署需要什么

三份 PAM 栈，各管一件事，**不要合并**：

| 文件 | 谁加载 | 内容 |
|------|--------|------|
| `/etc/pam.d/inox-auth` | `inox-auth` 守护进程（`--service inox-auth`） | **口令校验**：`auth`/`account` 走 `system-login` |
| `/etc/pam.d/inox-auth-session` | 会话负责人（`--session-service inox-auth-session`） | `auth required pam_permit.so`；`account`/`session` 走 `system-login` |
| `/etc/pam.d/inox-greeter` | systemd（`PAMName=inox-greeter`），**不由本 crate 加载** | greeter 自己的 logind 会话 |

负责人没有口令，而 `pam` 0.8 的安全封装只允许在已认证的 client 上 `open_session`，所以 `inox-auth-session` 的这一层必须是 permit-auth；用 `required` 而不是 `optional`——模块缺失时 `optional` 会被 PAM 静默跳过，故障会推迟到「会话打不开」并指向 logind。**不要**把 `/etc/pam.d/inox-auth` 的 `auth` 行改成 permit：那是密码校验栈。

此外还需要：一个把 `inox-auth` 以 root 拉起的单元（`packaging/systemd/inox-auth.service`，显式传 `--service inox-auth --session-service inox-auth-session --session-program /usr/bin/inox-session`，与本 crate 的默认值一致），以及 `inox-host` 拉起的登录界面（`INOX_AUTH_SOCKET`）。

真机登录验收的步骤见 ADR 0037 §验收；真机 PAM/VT 行为属 E4，**必须由人完成**。

## 已知限制

- 真机 PAM/VT 行为是 E4。CI 只能证明「能调用、能映射、不 panic」，不能证明任何一台机器能登录。
- 没有 `inox-auth-session`（或等价的 permit-auth 会话栈）时负责人打不开会话，`StartSession` 答复 `started:false`。
- logind 不可达（CI、开发实例、非 systemd 机器）时：VT 不仲裁、不切换、不交回；会话仍会启动。
- 启动自愈只在 seat 上没有活动会话、且能按 greeter uid 唯一识别出 greeter 会话时生效。
- 会话没有 `setsid`；libpam 内部会构造一份不受本 crate 控制的 `CString`；`Secret` 未 `mlock`。
- 负责人的确认行在 `pam_open_session` 之后、`exec` 之前发出；`exec` 若在此之后失败，界面先看到 `started:true`，随后会话立即结束并交回显示。
- 多用户同时登录、会话选择菜单、挂起/恢复矩阵未实现（ADR 0037 §未决项）。

逐条记录在 ADR 0029 与 ADR 0037。
