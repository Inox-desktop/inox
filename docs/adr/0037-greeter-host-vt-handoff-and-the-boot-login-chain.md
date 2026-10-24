# ADR 0037: greeter 宿主、VT/seat 交接与开机登录链

- 状态：accepted
- 日期：2026-10-23
- 决策者：架构组与安全审查者
- 批准记录：2026-10-23 本仓库维护者在开发会话中明确指示「目前需要可以真实登录」，并在给出的三个范围中选定「直接做完整 DM 宿主，替换 greetd」。该指令同时是 ADR 0015「真实认证 / 锁屏 / 挂起接线必须先完成人类安全审查」与 ADR 0021「greeter 宿主」未决项所要求的批准：批准把登录界面放到裸 VT 上、由本项目自己的认证边界启动用户会话，并在开发机上以本项目替换现有显示管理器。**本 ADR 不把 M5 标为完成**，也不声称真机登录（E4）已取得证据——那需要人在这台机器上按 §验收 的步骤完成。
- 关系：实施 ADR 0015 的「会话启动与监督」与「特权接口」条目中关于登录边界与 VT 归属的部分；**关闭 ADR 0021 的 «greeter 宿主» 未决项**（ADR 0029 只细化未关闭的那一条）与 ADR 0029 §6.6；细化 ADR 0029 §6.3（PAM 写会话环境进 helper 的缓解）并在 §决策 5 用会话负责人进程消除它；新增 L4 crate `inox-host`（新 crate 属破坏性架构变更，本 ADR 即其批准依据）；给 `inox-auth` 增加 `zbus`（已在工作区依赖表）与 logind 交互；**不改变** `auth-v1` 的任何帧、字段或答复语义（ADR 0029 / `crates/inox-protocols/protocol/auth-v1.md` 保持不变），不新增 Wayland 协议，不改变分层规则。

## 背景

ADR 0029 把认证边界做完了：`inox-auth` 验 PAM、`auth-v1` 是契约、`inox-greet` 是唯一的客户端，且 `inox-greet` 的真实路径要求边界套接字先存在。做完的部分到此为止——**没有任何东西把这两半放到裸 VT 上**：

- ADR 0021 记录的 greeter 宿主未决项（谁在裸 VT 上拉起登录界面、以哪个用户、登录后如何交接、注销后如何回来）至今没有实现，ADR 0029 §6.6 与 ADR 0032 §未决项都把它写成未验证。
- ADR 0029 §6.3 的已知限制「`pam_open_session` 在 helper 进程里写会话环境」当前靠 `--exit-after-session` 缓解，而这个开关默认关闭，理由正是「宿主还不存在」。
- 开发机的现状是 greetd + 第三方 greeter（`dms-greeter` 起 niri），`/etc/pam.d/inox-greet` 不存在，仓库里也没有任何 packaging。也就是说：**能验证口令，不能登录。**

本项目不使用 greetd、SDDM、GDM 或任何第三方登录管理器：它们自带一套认证（各自 PAM 栈与 daemon 权限模型），与本项目「唯一认证边界是 `inox-auth`」（ADR 0029）冲突，两套边界同时存在就没有单一权威点。因此宿主必须自己做，只是范围要小：合成器、输入、认证、界面都已经在别处，宿主只做**编排**。

已核验的现状（本仓库源码，非推测）：

- `inox-compositor` 的物理路径已经是 libseat + DRM/KMS + libinput，并且实现了 `SessionEvent::PauseSession` / `ActivateSession` 的处理（`crates/inox-compositor/src/runtime/physical.rs`）。也就是说：VT 被切走时它自己会让出设备，切回来自己恢复，不需要宿主转发设备句柄。
- `inox-auth` 的 `SessionPlan::start` 用 `CommandExt::{uid,gid}` + `rustix::thread::set_thread_groups` 降权，`pam_open_session` 在同一 helper 线程的 PAM 句柄上（`crates/inox-auth/src/session.rs`、`crates/inox-auth/src/server.rs`）。
- `inox-greet` 在 `Phase::SessionStarted` 时关闭自己的窗口（`crates/inox-greet/src/app.rs`），因为 ADR 0029 假设「会话启动后登录界面就结束了」。
- 开发机（Arch，systemd 262）上 `system-login` 已包含 `-session optional pam_systemd.so`，greeter 账户（uid 967）由 greetd 包创建，`greetd.service` 以 `Alias=display-manager.service` 占着 `tty1`。
- `pam` 0.8 的安全封装**不暴露 PAM 句柄**，本 crate 因此无法用 `pam_set_item(PAM_TTY)` 指定 VT；`pam_systemd` 只从进程环境读 `XDG_VTNR`（`$DISPLAY` 亦可），而在 edition 2024 + `unsafe_code = "forbid"` 下 `std::env::set_var` 不可用。这是下面「会话负责人」这个决策的直接来源。

## 决策驱动因素与备选

| 方案 | 收益 | 成本 / 风险 | 选择或否决依据 |
|------|------|-------------|----------------|
| 保持现状：人在已有图形会话里手跑 `inox-auth` + `inox-greet` | 零新代码 | 永远不是登录管理器；重新登录、开机、注销全都没有；`--exit-after-session` 不能开 | 否决：这就是要解决的问题 |
| 复用 greetd 当宿主，只替换 greeter 程序 | 少写编排代码，VT/seat 由 greetd 管 | greetd 有自己的 PAM 与 daemon 权限模型，与本项目唯一认证边界冲突；用户会话由 greetd 启动，`inox-auth` 就只能是摆设 | 否决：两套认证边界，ADR 0029 失效 |
| 自建 root 守护进程直接用 VT ioctl、向合成器传 DRM fd | 行为完全可控 | 需要 `unsafe` ioctl（ADR 0015 §特权接口 要求能力绑定到可信句柄）与逐设备 fd 转发；libseat 已经做完这件事，重写等于自造设备访问层 | 否决：把设备权柄搬进自家代码，收益为零 |
| **每个阶段一个 logind 会话，`inox-host` 只做编排，VT 由 logind 仲裁**（libseat 的 logind 后端提供设备访问与暂停/恢复） | 设备访问、VT 切换、暂停/恢复都由 logind + libseat 负责；Inox 不碰 ioctl、不传 fd；重启粒度等于阶段粒度 | 需要 greeter 单元带 PAM 会话、需要 auth 认领用户 VT；多一层 systemd 单元与 PAM 栈 | **选择**。它把「谁能拿设备」交给已审查的系统服务，宿主只做进程编排 |

## 决策

### 1. 进程拓扑与各自的权威

```
systemd
 ├─ inox-auth.service          root，Type=simple，Restart=always
 │    └─ 监听 /run/inox/auth.sock（root:greeter 0660）
 │       ├─ Authenticate（PAM，ADR 0029，不变）
 │       └─ StartSession → 会话负责人（见决策 5）→ inox-session（目标用户）
 └─ inox-greeter@<VT>.service  User=greeter，PAMName=inox-greeter，TTYPath=/dev/tty%i
      └─ inox-host（L4，本 ADR 新增）
           ├─ inox-compositor --socket wayland-inox-greet（后端交给合成器的默认 auto；有 seat 时即 DRM）
           └─ inox-greet（INOX_AUTH_SOCKET=/run/inox/auth.sock）
```

- **登录生命周期归 `inox-host`**：它拥有 greeter 阶段的进程组（合成器与登录界面）、监督与退出语义。它**不**认证、不碰凭据、不启动用户会话。
- **认证与用户会话归 `inox-auth`**：它仍是唯一能验口令、唯一能以别人身份启动进程的组件。宿主没有 `libpam`，`inox-host` 也不依赖 `inox-auth` 的 `server` feature（保持 ADR 0029 的 L4 → L2 单向、`default-features = false` 的既有边界；宿主只通过套接字说话，甚至不 `use` 契约类型——它只需要知道「套接字能不能连上」）。
- **设备与 VT 归 logind + libseat**：没有任何 Inox 组件执行 VT ioctl 或转发 DRM fd。greeter 阶段的设备访问来自 `inox-greeter@.service` 这个 logind 会话；用户阶段的来自会话负责人建立的那个 logind 会话。

### 2. VT 归属

- greeter 在模板单元 `inox-greeter@<VT>.service`，**实例名就是 VT 号**（`inox-greeter@1.service`、试运行用 `inox-greeter@4.service`）：`TTYPath=/dev/tty%i`，并且单元显式给 `Environment=XDG_VTNR=%i`。VT 号不依赖 systemd 是否替我们推导环境（本机实测 `getty@tty3.service` 的 `Environment=` 是空的），`pam_systemd` 只认进程环境里的 `XDG_VTNR`。
- 用户会话的 VT 由 `inox-auth` 在 `StartSession` 时分配：**从 2 开始的第一个空闲 VT**（`logind.Manager.GetSessionByVT` 为空即空闲），跳过 greeter 当前的 VT，上限 6。默认 `--session-vt-min 2 --session-vt-max 6`。v1 **同时只允许一个用户会话**，第二个 `StartSession` 在第一个存活期间答复 `Unavailable(Busy)`。
- 切换由 `inox-auth` 调 `logind.Manager.ActivateSession`：会话负责人建立用户会话后激活它（greeter 的 VT 因此变成非活动，合成器收到 `PauseSession` 并让出设备）；用户会话结束时 `inox-auth` 激活**它启动前记录的那个活动会话**（通常就是 greeter）。记录的是会话 id，不是硬编码的 VT 号，因此试运行（`inox-greeter@4`）与正式部署（`tty1`）走同一条代码路径。
- **救援通道**：安装脚本启用 `getty@tty3.service`。用户会话从 `2..=6` 里取第一个**没有会话**的 VT，而运行中的 getty 在 logind 里就是一个占用 VT3 的会话，所以正常配置下 VT3 不会被分配给用户会话；它始终是一个可用的 getty，任何登录链故障都能 `Ctrl+Alt+F3` 进去修（getty 没跑时选到 VT3 只是切走一个空终端，不致命）。

### 3. 宿主的行为（`inox-host`）

1. 启动时先确认边界套接字可连（`connect(2)` 成功）；不可连则**在开窗之前**以非零码退出（沿用 `inox-greet` 真实路径的规则，ADR 0032）。失败文案同时出现 `authentication boundary` 与 `inox-auth`。
2. 起合成器：`<bin-dir>/inox-compositor --socket wayland-inox-greet`（默认 backend `auto`；`--compositor-arg` 可追加）。等待 `$XDG_RUNTIME_DIR/wayland-inox-greet` 出现**且**合成器进程仍存活，上限 10 s；超时则停掉合成器并以非零码退出。
3. 起登录界面：`<bin-dir>/inox-greet`，环境为 `WAYLAND_DISPLAY=wayland-inox-greet`、`INOX_AUTH_SOCKET=<路径>`，其余继承（`XDG_RUNTIME_DIR`、`XDG_STATE_HOME` 由单元给）。
4. 监督：登录界面退出且合成器存活 → 按 ADR 0015 的有界预算重启（60 s 内最多 3 次，有界退避），超限则整体退出让 systemd 记录；合成器退出 → 宿主退出（systemd `Restart=always`）。收到 SIGTERM/SIGINT → 逆序停子进程并把它们清理掉（与 `inox-session` 的清理口径一致）。
   子进程经宿主内建的 `--exec-child <program>` 启动：calloop 的 signalfd 会把 SIGINT/SIGTERM 屏蔽在宿主线程上，子进程会继承该屏蔽字，不重置的话 `SIGTERM` 永远到不了它们（`inox-session` 用同一手法）。这个模式不是用户选项、不进 `--help`、不承载任何能力（同用户、无特权）。宿主在启动前对两个程序做一次可执行性 `stat` 预检，让「打包漏了二进制」表现为「这个程序起不来」而不是「合成器立刻退出」。
5. 宿主**不**判断「登录成功」：用户的会话在别的 VT 上，宿主在自己的 VT 被暂停与恢复之间什么也看不到，也不需要看到（决策 4）。

### 4. 登录界面在会话启动后驻留（`inox-greet` 行为变更）

`Phase::SessionStarted` 不再关闭窗口。理由：VT 被切到用户会话后登录界面的帧不可见，关掉它只会让「注销回来」需要宿主重新拉起一个全新的客户端（并带来「谁在什么时候重启它」这个没有答案的问题）。改为：`SessionStarted` 显示启动态并在 **3 秒后**回到 `Ready`（选中的账户保留，口令清空——被拒绝的登录是重新输入，启动成功的登录也是）。VT 恢复时屏幕上已经是登录画面，不需要任何跨 VT 通知。

这条变更**不改变**安全语义：界面只画「边界真的答复过」的状态；`SessionStarted` 只有在 `Reply::SessionStarted{started:true}` 之后才进入。

### 5. 会话负责人：`inox-auth` 的子进程，一次登录一个进程

现状（ADR 0029）在 helper 的 PAM 线程里 `pam_open_session`，再把子进程 fork 出去。这有两个问题，本 ADR 一并解决：

1. PAM 只能给 helper 自己的进程环境写 `XDG_SESSION_ID` 等变量；helper 长期存活时上一个账号的会话变量会留给下一个账号（ADR 0029 §6.3 的已知限制，靠 `--exit-after-session` 缓解）。
2. `pam_systemd` 只能在**调用 `pam_open_session` 的那个进程**的环境里读到 `XDG_VTNR`，而助手进程不能被安全地改写自己的环境（见 §背景 的 `set_var` 论证）。没有 VT 的会话拿不到 seat，合成器也就拿不到 DRM。

因此 `StartSession` 改为：

1. `inox-auth`（root）选出空闲 VT，然后以 `Command::env("XDG_VTNR", vt)` 启动自己的一个子进程：`inox-auth --session-leader --user <name> --vt <n>`（环境由父进程在 spawn 时给出，是安全 API）。
2. 会话负责人（root）打开会话：`pam_start(inox-auth-session)` → `pam_authenticate` → `pam_acct_mgmt` → `pam_open_session`。**这个栈的 `auth` 行是 `pam_permit`，`account`/`session` 走真正的 `system-login`**，原因见下面「实现时必须偏离本 ADR 字面的两处」。口令不进入这个进程，它本来就一个字节都没有。
3. 会话 id 与 VT 由负责人自己落到会话程序的环境里：PAM 把环境写进 PAM 句柄而不是进程环境（`pam` 0.8 又不暴露句柄），所以负责人用 `logind.Manager.GetSessionByPID(getpid())` 取回 `XDG_SESSION_ID`，再 `logind.ActivateSession`，然后用 `Command::{uid,gid,envs}.exec()` 变成会话程序（`--session-program`，默认 `inox-session`），显式注入 `XDG_SESSION_ID`、`XDG_VTNR`、`XDG_SEAT`（`XDG_RUNTIME_DIR` 由 pam_systemd 在 logind 侧创建，logind 的会话属性就是权威）。`XDG_VTNR` 由父进程在 spawn 时用 `Command::env` 给出，因此 `pam_systemd` 在 `pam_open_session` 时就读得到它。
4. 会话进程的存活就是这个 logind 会话的存活；会话进程退出后 logind 会话结束，`inox-auth` 观察到子进程退出（新：它现在会 reap），再 `ActivateSession(记录的会话)` 把画面交回登录界面。启动自愈是保守的：只有在 seat 上**没有活动会话**、且能按 greeter 的 uid 从 `ListSessions` 里**唯一**认出 greeter 会话时才激活它，认不出就只记 `debug!` 什么都不做。

后果：`pam_open_session` 从「helper 的全局污染」变成「每次登录一个一次性进程」，ADR 0029 §6.3 的限制因此**不再需要 `--exit-after-session` 缓解**；该开关保留（默认关闭，部署单元显式开启，作为状态复位手段）。

**实现时必须偏离本 ADR 字面的两处**（2026-10-23 实现时按 `pam` 0.8 的真实 API 校正，正文保留原意图以便追溯）：

- 第 2 步不能对认证栈 `pam_start` 后直接 `open_session`：`pam` 0.8 的安全封装在未 `authenticate()` 的 client 上直接拒绝 `open_session`，而 `pam::start(service, Some(user), …)` 在该版本里有**use-after-free**（`try_str_option_to_ptr` 返回已 drop 的 `CString` 指针）、`set_item(PAM_USER)` 的参数类型安全代码构造不出来、裸 `pam_sys` 需要 `unsafe`（本 crate 禁止）。因此负责人必须先对一个 **permit-auth 的会话专用栈**认证一次，再 `open_session`。这份栈是 `packaging/pam.d/inox-auth-session`（`auth required pam_permit.so`、`account`/`session` 走 `system-login`），**绝不能**把 permit 行写进认证栈 `inox-auth`——那等于免密登录。
- 第 3 步不能「从自己环境里读回 PAM 写入的变量」：PAM 的环境在 PAM 句柄里，`pam` 0.8 不暴露 `getenvlist`，而在 edition 2024 + `forbid(unsafe_code)` 下写进程环境不可用（§背景 已有的论证）。改用 logind 作为权威：`GetSessionByPID` 取会话 id，`Command::envs` 注入三个 `XDG_*` 变量。
- 上游缺陷需要在 `pam`/`pam-sys` 修好后再评估：届时负责人可以退回同进程实现，这会重开本决策。

### 6. PAM 栈与 packaging

仓库新增 `packaging/`（不属于任何 crate，不参与依赖图）：

- `packaging/pam.d/inox-auth`：登录账号用。`auth` 与 `account` 走 `system-login`（`pam_unix` + `pam_shells` + `pam_nologin`），`session` 走 `system-login`（含 `pam_systemd`、`pam_env`、`pam_limits`）。**不得**包含 `pam_securetty`（登录界面不在 tty 上产生 tty 归属）与本项目自己的模块。
- `packaging/pam.d/inox-greeter`：greeter 阶段自己的 logind 会话用。`auth optional pam_permit.so`（greeter 不认证）、`account`/`session` 走 `system-login`（给 greeter 一个 `XDG_RUNTIME_DIR`、cgroup 与 VT）。
- `packaging/pam.d/inox-auth-session`：**会话负责人**打开会话用的第三个栈。`auth required pam_permit.so`、`account`/`session` 走 `system-login`。它必须与 `inox-auth` 分开：负责人没有口令，而安全封装的 `pam` 0.8 只允许在「已认证」的 client 上 `open_session`，所以负责人只能对一个 permit 栈认证后再开会话；把 permit 行写进 `inox-auth` 等于让所有人免密登录。`session` 走 `system-login` 这一点是用户会话能拿到 logind 会话、VT 与 `/run/user/<uid>` 的唯一来源。
- `packaging/systemd/inox-auth.service`：`Type=simple`、`User=root`、`RuntimeDirectory=inox`、`ExecStart=/usr/bin/inox-auth --exit-after-session --service inox-auth --session-service inox-auth-session --session-program /usr/bin/inox-session`（三个名字都显式给出，让装上去的文件与被加载的文件不可能不同名）、`Restart=always`、`Before=inox-greeter@.service`、不要 `CapabilityBoundingSet` 放宽（默认 root 即可，最小化留给后续 ADR）。
- `packaging/systemd/inox-greeter@.service`：`User=greeter`、`PAMName=inox-greeter`、`TTYPath=/dev/tty%i`、`StandardInput=tty`、`TTYReset=yes`、`TTYVHangup=yes`、`StateDirectory=inox-greeter`、`Environment=XDG_VTNR=%i XDG_STATE_HOME=/var/lib/inox-greeter`、`Restart=always` 配 `RestartSec=2` 与 `StartLimitIntervalSec=60`/`StartLimitBurst=20`（开机头一秒宿主会因为边界套接字还没绑定而按设计拒绝一两次；systemd 默认的 5 次/10 s 会在八秒内用光并把单元标成 failed，那等于把机器留在黑屏上，所以这条限制要放宽到足以覆盖启动顺序竞争、又仍然有界）、`Conflicts=getty@tty%i.service`、`After=inox-auth.service`。模板单元不能带 `Alias=display-manager.service`，别名由安装脚本对 `inox-greeter@1.service` 建立。
- `packaging/install.sh`：幂等；装 PAM 栈与单元、创建 greeter 账户（已存在则跳过）、`systemctl daemon-reload`，**默认不改开机链**。`--enable` 才把 `display-manager.service` 指向 Inox 并 `systemctl disable greetd`。
- `packaging/systemd/inox-login-fallback.service`：`Type=oneshot`，`ExecCondition` 检查 `greetd` 是否安装，`ExecStart=systemctl start greetd.service`；只由 greeter 单元的 `OnFailure=` 触发，不 enable、不参与关机/开机顺序。
- `packaging/uninstall.sh`、`packaging/rollback-to-greetd.sh`：把 `display-manager.service` 指回 greetd 并清理 Inox 的单元/栈；**先切换再停服务**，任何一步失败都保持当前可登录状态。
- `packaging/try-inox-greeter.sh`：在**不动开机链**的前提下起 `inox-greeter@4.service`（即 `/dev/tty4`）并 `chvt 4`，用于真机试运行；`packaging/try-rollback.sh` 停掉它。
- 卸载/回退**不删除** `getty@tty3`。

### 7. 明确不做（v1）

会话选择菜单（inox-session 与 niri 二选一）、多用户同时登录、greeter 的 AT-SPI 语义导出、锁屏解锁路径、Suspend/VT 恢复矩阵。它们各自需要独立评审，列在 §未决项。

## 失败、恢复与安全

- **边界不可达**：宿主在起合成器之前拒绝，屏幕上是 systemd 的失败而不是一个骗人的登录框。边界在登录界面运行中消失 → 登录界面已是 `Unavailable` 语义（ADR 0032），尝试被拒绝，宿主不会伪造答复。
- **合成器崩溃**：宿主退出，systemd 重启 greeter 单元；用户会话在别的 VT 上，不受影响（ADR 0015「合成器崩溃属会话级故障」指的是用户会话的合成器）。
- **登录界面崩溃**：宿主在有界预算内重启它；超限则退出让 systemd 记一次失败重启，不会无限循环。
- **整条登录链起不来**（合成器拿不到 DRM、PAM 栈缺失、任何让单元最终 failed 的原因）：greeter 单元在重试预算（20 次 / 60 s）用尽后进入 failed，`OnFailure=inox-login-fallback.service` 会拉起 `greetd.service`——**只在 `/usr/bin/greetd` 真的存在时**（`ExecCondition`），只 `systemctl start`、不 `enable`。于是这台机器最坏也只是回到切换之前的登录界面，而不是一块黑屏 + 一个需要想起来的 VT3。下次开机仍然先试 Inox。
- **用户会话启动失败**：`SessionStarted{started:false}` → 登录界面停在 `Failed` 并说明；VT 没有切换，屏幕上仍是登录界面。
- **注销后回不来**：`inox-auth` 若在用户会话期间崩溃，systemd 重启它，新进程启动时若发现没有活动用户会话，则 `ActivateSession` 当前 seat 上的 greeter 会话（自愈）。这是 `--exit-after-session` 之外的兜底。
- **凭据**：本 ADR 不改变凭据路径。会话负责人不接收口令——它只拿到父进程已经从 PAM 验过的**账号名与 VT**，口令在 `Authenticate` 结束时就已经清零。
- **permit 栈不能变成认证栈**：`inox-auth-session` 与 `inox-greeter` 的 `auth` 行是 permit，这是它们存在的目的；`inox-auth` 的 `auth` 行永远走 `system-login`。三个文件的名字、用途与「谁加载它」都写在各文件头注释里，混淆其中两个就是密码less登录或全体登录失败。
- **权限**：会话负责人以 root 启动、`pam_open_session` 后立刻降权并 exec，进程不再持有任何特权；它接受的参数只来自 root 父进程的命令行，永不来自线路。
- **VT/seat 越权**：Inox 组件不执行任何 VT ioctl；`ActivateSession` 由 root 调用，作用于 logind 自己的会话对象。greeter 会话以 `greeter` 账户运行，登录界面无法启动用户会话（它没有 `server` feature 的代码）。
- **恶化的可见性**：`getty@tty3` 常驻，安装脚本不碰 login 链直到 `--enable`。

## 兼容、迁移与回退

- `auth-v1` 不变：没有新请求、没有新字段、没有版本号变化。`inox-greet` 的客户端代码（`boundary.rs`）不变。
- `inox-greet` 的行为变化只有一条：`SessionStarted` 后驻留并复位。`--preview` 路径与文案不变。
- 新依赖：`inox-auth` 增加 `zbus`（工作区已声明，版本 6，MIT，ADR 0027 已记录）。`inox-host` 依赖 `calloop`（信号与事件循环，已在工作区）、`anyhow`、`tracing`、`rustix`（`process`，等待/发信号）、`inox-common`（日志与 i18n）。`inox-host` **不**依赖 `inox-auth`、`inox-compositor`、`inox-greet` 中任何一个（L4 同层不横向依赖），它只按路径启动它们。
- `inox-host` 进入 L4：根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`、根 `AGENTS.md` §2、PRD §3.1 三处必须同时登记，否则 `layering.rs` 失败。
- 回退：`rollback-to-greetd.sh` 把 `display-manager.service` 指回 greetd，停 `inox-greeter@*`。PAM 栈与单元可以留着（不影响 greetd）。

## 验收与证据

| 验收 ID / 场景 | 责任仓库 | E0–E4 等级 | 测试与失败注入 | 通过条件 / 证据位置 |
|----------------|----------|------------|----------------|----------------------|
| 宿主编排：边界缺失拒绝启动 | `inox-host` | E1+E2 | 单元测试：`Supervisor` 状态机覆盖边界不可达、合成器早退、合成器超时、登录界面连续崩溃超预算、SIGTERM 清理；集成测试用假合成器/假界面脚本（不 mock 宿主自己的规则） | 4 条拒绝路径都以非零码退出且没有留下子进程；`crates/inox-host/tests/` |
| 宿主 CLI 与文案 | `inox-host` | E1 | `--help`、未知参数、`en-US`/`zh-CN` 目录双向一致 | 与 `inox-greet`/`inox-auth` 同口径 |
| VT 选择与交接 | `inox-auth` | E1（纯逻辑）+ E4（真机） | 单元测试：空闲 VT 选择（跳过 greeter VT、跳过不可用范围、范围耗尽）、会话 id 记录与回交、第二个会话被拒 | 真机：登录后 `loginctl list-sessions` 出现目标账号的会话且 `VT` 为新 tty；注销后回登录界面 |
| 会话负责人 | `inox-auth` | E1+E4 | 单元测试：参数校验（未知账号、VT 越界、非 root 拒绝）、`XDG_VTNR` 由父进程给出而非线路；真机验证 `XDG_SESSION_ID`/`XDG_RUNTIME_DIR` 到达会话进程 | 真机：会话进程环境含 `XDG_SESSION_ID`，`/run/user/<uid>` 存在 |
| 登录界面驻留与复位 | `inox-greet` | E1 | 单测：`SessionStarted` → 3 s 后 `Ready`、口令清空、账户保留；渲染测试补该画面 | `crates/inox-greet/src/app.rs` 单测 + `tests/render.rs` |
| 认证边界的真实二进制 + 真实 `auth-v1` 客户端（E2，2026-10-23 已取得一次） | `inox-auth` | E2 | 非 root 开发实例（`--socket <tmp> --allow-uid <self> --session-program /bin/true --service login`）+ 手写客户端：Hello(v1) → `Welcome`；未接受过的 `StartSession` → `Error(Malformed)`；不存在的账号 → `Rejected`（不泄漏存在性）；客户端声明 v99 → `Error` | 实测回帧：`16/00000001`、`18/0000000000000001`、`21/00`、`21/01`，日志为 `the attempt names an account this boundary will not log in ... no such account`。**不含**真实口令校验与会话启动（那需要人在真机上做） |
| greeter 阶段的**无 VT 预演**（E2，2026-10-23 已取得一次） | `inox-host` + 真实合成器/界面 | E2 | 在私有 `XDG_RUNTIME_DIR` 里用替身边界套接字（只 accept，不答复）跑 `inox-host --bin-dir <target/debug> --auth-socket <sock> --compositor-arg --headless`：宿主起了合成器、合成器绑定 `wayland-inox-greet`、登录界面连上并读账户（`accounts=2`）、合成器 `mapped toplevel windows=1`、10 s 后仍在运行；SIGTERM 后三个进程全部消失且宿主退出 0 | 证据：本机 2026-10-23 的实测日志（`LibGL` llvmpipe，1280×720 输出）。这条路径不碰 DRM/VT/logind，因此**不能**替代 E4，但它把「宿主编排 + Wayland 握手 + 界面真能画」与「设备/session 交接」分开定位 |
| 开机链替换与回退 | `packaging` | E4 | 人在开发机按 `docs/book/src/usage/login.md` 操作：装、试运行（`inox-greeter@4`）、切换、重启、注销、回退到 greetd | 每一步的实际输出与 `loginctl`/`journalctl -u inox-greeter@1` 记录写进验收报告；**未取得前不得声称真机登录已通过** |

新增验收 ID（`docs/quality/acceptance.md` §8）由 packaging 任务同步登记：登录链的安装/试运行/切换/回退/救援 console 各一条。

## 实施顺序与未决项

1. ADR 0037（本文）与工作区登记（层级表、PRD、根 AGENTS）。
2. `inox-host`（新 crate）+ `inox-greet` 的驻留复位 + `inox-auth` 的 VT/负责人/reap，三者可并行，接口见 §决策。
3. `packaging/`（PAM 栈、单元、安装/回退/试运行脚本）与文档 `docs/book/src/usage/login.md`。
4. 真机试运行（`inox-greeter@4`，不动开机链），由维护者执行；失败则回到步骤 2/3。
5. 切换开机链（`install.sh --enable`），重启验证；取得 E4 证据后再更新 PRD §13 的 M5 交付清单。

未决项（本 ADR 不关闭）：会话选择菜单（需要 `auth-v1` 或宿主层面的会话列表接口）、多用户同时登录（需要多会话与限额策略）、greeter 的 AT-SPI 语义导出（ADR 0018）、锁屏解锁路径（ADR 0025 决策 5）、Suspend/VT 恢复矩阵（ADR 0015 E4）、`inox-host` 是否最终并入 `inox-session`（若两者编排逻辑趋同，另提 ADR）。

## 后果

- 正面：开机即登录成为可能；设备访问与 VT 仲裁留在已审查的 logind/libseat；认证边界仍然是唯一一处验口令与启动别人的会话；`--exit-after-session` 不再是掩盖 PAM 环境泄漏的补丁；宿主小到可以完整测试其编排规则。
- 负面：多一个 L4 crate 与一份 packaging（两条新的部署面需要人维护）；多一层 systemd 单元与一份 greeter PAM 栈；v1 只支持单用户单会话，切换用户仍是未决项；会话选择要到后续 ADR 才可用。
- 复审触发条件：出现第二个并发用户会话、需要会话选择、`inox-host` 与 `inox-session` 编排逻辑合并、或 `pam` 安全封装开始暴露 `PAM_TTY`（届时会话负责人可以退化成同进程实现，需要重评）。
