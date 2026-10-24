# 登录与开机链

> **真机证据尚未取得。** 本页描述的是**可以**在这台机器上执行的步骤，不是已经通过的验收。
> 安装链的表现、VT 交接、注销回登录画面、回退与救援 console 的实际结果，必须由人在登记的机器上
> 逐条跑一遍并留证（验收 S06–S11，见[验收规范 §8](https://github.com/Inox-desktop/inox/blob/main/docs/quality/acceptance.md)）。
> 在拿到那些记录之前，任何文档、PR 或发布说明都不得声称「Inox 能真机登录」。
> 决策依据：[ADR 0037](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0037-greeter-host-vt-handoff-and-the-boot-login-chain.md)。

Inox 不使用 greetd、SDDM 或 GDM：登录界面、认证边界和会话启动都由本项目自己提供，
口令只在一个地方校验（`inox-auth`，ADR 0029）。本页说明那套东西怎么装、怎么在不碰开机链的前提下
先试一遍、怎么切换、怎么退回 greetd，以及出问题时从哪个 VT 进去修。

## 组成

```text
systemd
 ├─ inox-auth.service            root，常驻，Restart=always
 │    └─ /run/inox/auth.sock     root:greeter 0660，只接受 greeter 的 uid
 │       ├─ Authenticate         PAM 校验口令（/etc/pam.d/inox-auth）
 │       └─ StartSession         → 会话负责人（一次登录一个进程）
 │                                 ├─ pam_open_session（/etc/pam.d/inox-auth-session）
 │                                 ├─ logind.ActivateSession（用户会话的 VT）
 │                                 └─ exec inox-session（目标账号，已降权）
 └─ inox-greeter@<VT>.service    User=greeter，PAMName=inox-greeter，TTYPath=/dev/tty<VT>
      └─ inox-host
           ├─ inox-compositor --socket wayland-inox-greet
           └─ inox-greet          INOX_AUTH_SOCKET=/run/inox/auth.sock
```

- **`inox-auth`** 是唯一验口令、唯一能以别人身份启动进程的组件。它在启动时就拒绝「没有 root」的部署；
  系统账号与 root 一律拒绝，答复不泄漏账号是否存在。
- **`inox-greeter@<VT>`** 的实例名**就是 VT 号**：`inox-greeter@1.service` 是开机链，`inox-greeter@4.service`
  是试运行。单元只跑 `inox-host`，由宿主按顺序起合成器与登录界面。
- **`inox-host`** 只做编排：先确认边界套接字可连（不可连就在开窗之前以非零码退出），再起合成器、
  等它就绪、起登录界面，并在登录界面崩溃时按有界预算重启。它不认证、不碰凭据、不启动用户会话。
- **用户会话**落在 VT2..=6 中**第一个空闲**的 VT 上（跳过 greeter 占着的那个），由会话负责人建立并激活；
  会话结束时 `inox-auth` 把画面交回它启动前记录的那个会话。同时只允许一个用户会话。
- **VT3 永远是救援 console**（`getty@tty3.service`），安装时启用，回退与卸载都不删。

### 三个 PAM 栈

一次登录经过两个 PAM 事务，落在两个不同的栈上；第三个栈只服务登录界面自己。这三个名字都必须存在，
少任何一个的症状都是「登录失败」，不是「配置缺失」：

| 栈 | 谁加载 | 内容 | 少了会怎样 |
|----|--------|------|------------|
| `/etc/pam.d/inox-auth` | `inox-auth` 守护进程（`--service inox-auth`） | `auth`/`account`/`session` 走 `system-login`：口令在这里校验（`pam_unix` + `pam_faillock` + `pam_nologin` + `pam_shells`） | 连口令都验不了，每次尝试都被拒绝 |
| `/etc/pam.d/inox-auth-session` | 会话负责人（`inox-auth --session-leader`，`--session-service inox-auth-session`） | `auth required pam_permit.so` + `account`/`session` 走 `system-login`（`pam_systemd` 在这里注册 logind 会话） | `pam_start` 失败，`StartSession` 一律答复 `started:false`——看起来像口令错 |
| `/etc/pam.d/inox-greeter` | systemd（`PAMName=inox-greeter`） | `auth optional pam_permit.so` + `account`/`session` 走 `system-login` | 登录界面没有 logind 会话、没有 VT、拿不到 seat，屏幕不会亮 |

会话负责人**没有口令、也不接收口令**（口令在 `Authenticate` 结束时就被边界清零了），而安全的 `pam`
封装只允许在已认证的 client 上打开会话，所以它的 `auth` 行必须是无条件许可。这条许可**必须**待在第二个
文件里：把它写进 `/etc/pam.d/inox-auth` 就是让所有账号免密登录。

## 前置条件

1. 二进制已在 `/usr/bin`：`inox-auth`、`inox-host`、`inox-compositor`、`inox-greet`、`inox-session`。
   单元文件与脚本都按这个前缀写死；换前缀要改 `packaging/systemd/*.service`，脚本不会替你猜。
2. 本机原有显示管理器（这台机器是 greetd）**先保持原样**。安装脚本默认不动开机链。
3. 从仓库根执行本页的命令（脚本自己会解析所在目录，从别处执行也可以）。
4. 想先确认这套构建没问题：跑一遍[无 VT 预演](#先做无-vt-预演不需要-root)（不需要 root，也不会碰开机链、`/etc`、systemd 或 VT）。

从当前 checkout 装好二进制：

```bash
cargo build --workspace --release
sudo install -m 0755 target/release/inox-auth target/release/inox-host \
    target/release/inox-compositor target/release/inox-greet \
    target/release/inox-session /usr/bin/
```

## 安装（不动开机链）

```bash
sudo packaging/install.sh
```

它做的事：

- 装三个 PAM 栈：`/etc/pam.d/inox-auth`（校验口令）、`/etc/pam.d/inox-auth-session`（会话负责人打开会话）、
  `/etc/pam.d/inox-greeter`（登录界面自己的会话）。若目标位置已有**内容不同**的文件，
  先复制成 `<文件>.bak-<时间戳>` 再覆盖。
- 装三个单元：`inox-auth.service`、`inox-greeter@.service`（模板，实例名是 VT 号）与
  `inox-login-fallback.service`（开机时登录界面起不来就启动 greetd 的 oneshot；**只安装、从不启用**），
  然后 `daemon-reload`。
- `greeter` 系统账号不存在才创建（已存在则原样保留，uid 从账户库读，不写死）。
- `systemctl enable --now getty@tty3.service`：救援 console 就位。
- **不改** `display-manager.service`，**不启动**任何登录界面。

脚本在第一批写入之前就把能检查的都检查完（二进制、PAM 目录、packaging 文件），
所以一次被拒绝的运行不会留下任何改动。重复执行是幂等的。

## 先做无 VT 预演（不需要 root）

在碰任何 VT、`/etc` 或 systemd 之前，先把宿主这一层单独跑通：

```bash
packaging/rehearse-headless.sh
```

它在一个 `mktemp -d` 里造出私有的 `XDG_RUNTIME_DIR`、一个只 `accept()` 的**替身边界**（不实现 `auth-v1`，
不校验任何口令）、真实的 `inox-host` + 真实的 `inox-compositor --headless`（llvmpipe）+ 真实的
`inox-greet`，跑满 12 秒（`--seconds N` 可改）之后逐项判定：

- 合成器绑上了 Wayland socket；
- 宿主起了两个子进程（先合成器，后登录界面）；
- 登录界面**找到了认证边界**，并读到了系统账户列表；
- 登录界面真的映射出一个 toplevel（渲染器画出了帧，不是只有日志声称）；
- 到截止时间宿主仍然活着；`SIGTERM` 之后宿主以 0 退出，且**没有任何进程残留**（按本次运行的私有
  runtime 目录精确判定，不做 `pkill -f inox` 式的误伤）。

PASS 意味着**宿主编排、Wayland 握手、界面能画**这三件事已经被排除了。剩下的问题只会在设备、seat、
VT 与 PAM 那一侧——也就是下面需要真机的步骤。FAIL 会打印失败项、宿主的最后 60 行日志与替身边界的日志，
并以非零码退出，临时目录照样删掉。

它不需要 root（**拒绝**以 root 运行：root 能碰到 `/etc`、系统总线与真实 seat，而这些正是这一步要排除的），
不读写 `/etc`，不碰 systemd 与 VT，不联网。前提是 `target/debug`（或 `--bin-dir DIR`）里已经有
`inox-host`、`inox-compositor`、`inox-greet`；缺了会直接告诉你构建命令。`python3` 只用于那个替身边界。

**这是 E2 预演证据（验收 S07a），不是登录。** 替身边界不校验口令，没有会话、没有 VT 交接、没有 PAM；
它不能替代 S07–S09，也不构成任何「Inox 能登录」的结论。

## 试运行（VT4，不动开机链）

**先跑预检。** 它只读，不改任何东西，把环境与打包问题在你把屏幕切走之前指出来：

```bash
sudo packaging/preflight.sh
```

每一行是 `ok` / `WARN` / `FAIL`，FAIL 下面一行就是该怎么修：二进制缺了、三个 PAM 栈没装、
栈里 `include system-login` 走到的模块解析不到、单元没装或 `systemd-analyze verify` 有意见、
`greeter` 账户不合适、`/dev/tty4` 或 `/dev/dri/card*` 不存在（这两样就是黑屏的两种来源）、
seat/`ActiveSession` 不正常、两个显示管理器同时抢 VT1。**有 FAIL 就不要继续。**
`try-inox-greeter.sh` 自己也会在 `chvt` 之前跑一遍：FAIL 就不启动单元、不切屏。

预检通过之后：

```bash
sudo packaging/try-inox-greeter.sh
```

- 它只 `systemctl start inox-greeter@4.service`（该单元 `Wants=inox-auth.service`，边界会被一起拉起），
  确认单元**真的 active** 之后才 `chvt 4`。单元起不来时屏幕留在原处，并打印 `journalctl` 的尾巴。
- `display-manager.service` 前后不变；原来的桌面/登录界面仍在 VT1，`Ctrl+Alt+F1` 就回去了。
- 想换一个 VT：`sudo INOX_TRIAL_VT=2 packaging/try-inox-greeter.sh`（只允许 2、4、5、6；VT1 属于开机链，
  VT3 是救援 console）。

### 第一件要验证的事：VT 管道

登录成功之后**先看这两条**，它们告诉你 logind/libseat/VT 这条管道到底通不通：

```bash
loginctl list-sessions          # 你的会话必须在 VT2..=6 上，且不等于 greeter 的 VT（试运行是 4）
journalctl -u 'inox-greeter@*' -b --no-pager
```

期望：用户会话的 `VT` 列是一个**不同于 greeter 的**编号（2、5、6 之一）；`journalctl` 里能看到合成器
已经起来（宿主记录了合成器与登录界面的启动），且**没有** `session is inactive`，也没有 libseat/DRM
权限错误。如果会话的 VT 是空的、或者日志里报 seat/DRM 错误，问题在 VT 管道而不在登录逻辑，
先停下来看 [排障](#排障)，不要继续往后做切换。

### 登录之后

- 屏幕切到用户会话所在的 VT，`inox-session` 拉起合成器与桌面组件。
- 登录界面**不会关闭**：它留在 greeter 的 VT 上，收到「会话已启动」后显示 3 秒启动态，然后回到登录画面
  （保留上次选的账号、清空口令）。因此注销回来时屏幕上已经是登录画面，不需要谁重新拉起它。
- 注销后 `inox-auth` 把画面交回 greeter 的 VT；`loginctl list-sessions` 里用户会话消失，
  greeter 会话仍在。

在那个会话里跑一次（不需要 root）：

```bash
packaging/verify-login.sh                    # 试运行：登录界面在 VT4
packaging/verify-login.sh --greeter-vt 1     # 开机链：登录界面在 VT1
```

它把「看起来好了」变成一组要么成立要么不成立的事实：`$USER` 恰好**一个** `Class=user` 会话（`manager`
会话不算登录）、VT 在 2..=6 且不等于登录界面的 VT、`Service=inox-auth-session`、`Seat=seat0`、
`Active=yes`、`seat0` 的 `ActiveSession` 就是它、`/run/user/<uid>` 存在且属主与 `0700` 都对、会话进程
环境里的 `XDG_SESSION_ID` / `XDG_RUNTIME_DIR` / `XDG_VTNR` 与 logind 一致，以及**登录界面自己的会话
还在**（没有被交接杀掉）。每条 FAIL 下面写着它意味着什么：VT 没切 = `pam_systemd` 没拿到 `XDG_VTNR`；
`Service` 不是 `inox-auth-session` = 会话没走负责人；没有 seat = 合成器拿不到 DRM。

### 注销之后

注销，**回到登录画面**，然后在任意终端里跑：

```bash
packaging/verify-login.sh --after-logout                    # 试运行
packaging/verify-login.sh --after-logout --greeter-vt 1     # 开机链
```

它断言：`$USER` 的会话已经消失、`seat0` 的 `ActiveSession` 重新是登录界面那个会话、登录界面仍然活着。
FAIL 通常指向 `inox-auth` 没把画面交回（看 `journalctl -u inox-auth`，它记录了自己交回哪个会话）。

### 这一阶段已知的外观问题

`inox-compositor` 目前**没有处理全屏请求**，所以登录卡片是一个占满工作区的普通平铺窗口。
看起来不像「整屏登录界面」是**外观问题，不是登录失败**：能选用户、能输入口令、能登录，就说明这条路是通的。

### 只验证登录链：临时换掉会话程序

登录链有两段，失败时的表现完全不同：**登录界面/认证/VT 交接** 这一段，和 **登录之后 inox-session 拉起桌面** 那一段。
想先确认前一段通不通，可以把会话程序临时换成这台机器上已经能跑的会话（例如 niri）：

```bash
sudo systemctl edit inox-auth.service
```

在打开的 drop-in 里写（`ExecStart=` 空一行是清空原值，不是笔误）：

```ini
[Service]
ExecStart=
ExecStart=/usr/bin/inox-auth --exit-after-session --service inox-auth --session-service inox-auth-session --session-program /usr/bin/niri
```

```bash
sudo systemctl restart inox-auth.service
```

- 会话程序来自**启动配置**，永远不来自线路（`auth-v1` 的 `StartSession` 只带关联号），所以换它不改变认证与 VT 交接的任何一行；
  PAM、logind 会话、`XDG_*` 注入、`verify-login.sh` 的断言全部照旧。
- 这只测「登录链 + 一个已知可用的会话」。它**不**测 `inox-session`，不能据此说 Inox 桌面可用。
- 反过来，它也是一条可以日常使用的路：登录界面是 Inox 的，会话暂时是你的旧桌面，直到 `inox-session` 在真机上稳定为止。
- 恢复默认：

```bash
sudo systemctl revert inox-auth.service && sudo systemctl restart inox-auth.service
```

`packaging/preflight.sh` 会检查 `/usr/bin/inox-session` 存在，但不会因为你换了会话程序而报错。

### 结束试运行

```bash
sudo packaging/try-rollback.sh
```

停 `inox-greeter@4.service`，在没有其他 Inox 登录界面运行时一并停 `inox-auth.service`，
然后把画面切回开机链的 greeter 会话（按 `loginctl` 里 greeter 的 VT 判断，找不到就提示按 `Ctrl+Alt+F1`）。
不动开机链。没有东西在跑时它只说一句，不做别的事。

## 切换开机链

**先试运行通过再切换**（同样先跑 `sudo packaging/preflight.sh`，`0 FAIL` 再继续）。
切换本身不启动任何东西，它只改下一次开机用谁：

```bash
sudo packaging/install.sh --enable
```

顺序（顺序本身就是安全性的一部分）：

1. `systemctl enable inox-auth.service`。
2. `systemctl disable greetd.service`——**先**禁用 greetd，因为 `disable` 会按路径删掉
   `display-manager.service` 这个软链，必须在我们的软链建立之前发生。正在运行的 greetd 不受影响。
3. `ln -sfn /etc/systemd/system/inox-greeter@1.service /etc/systemd/system/display-manager.service`，
   `daemon-reload`，然后**读回验证**软链确实指向 `inox-greeter@1.service`。验证失败会明确报错并给出修法。

**切换后你有一条退路，不必靠 `Ctrl+Alt+F3` 救自己。** 安装时装了
`inox-login-fallback.service`：如果 Inox 登录界面连续起不来（`inox-greeter@1.service` 在 60 秒内重试
20 次仍失败，systemd 于是把它标成 `failed`），这个 oneshot 单元会**启动 greetd**，让这台机器仍然能登录。
它**不**重新启用 greetd，也**不**改 `display-manager.service`：`systemctl is-enabled greetd` 仍是 `disabled`，
下次开机仍然先试 Inox。也就是说，一次失败的启动不会悄悄把开机链改回去——它只是让你今天能进去修。

重启前可以先确认：

```bash
readlink -f /etc/systemd/system/display-manager.service   # .../inox-greeter@1.service
systemctl is-enabled inox-auth.service                    # enabled
systemctl is-enabled greetd.service                       # disabled
sudo reboot
```

开机后的期望：

- VT1 是 Inox 登录界面（卡片居中，账号是系统里可登录的账号）。
- `systemctl status inox-auth.service` 是 `active (running)`，`journalctl -u inox-auth` 里有
  「the greeter account may connect」与 socket 路径。
- `loginctl list-sessions` 有两个会话：greeter 在 VT1，你登录之后那个用户会话在一个空闲 VT（2、5、6 之一）。
- `systemctl --failed` 为空。

## 回退到 greetd

```bash
sudo packaging/rollback-to-greetd.sh
```

它**先切换、后停服务**：

1. 找到 greetd 单元的真实路径，`enable greetd.service`，把 `display-manager.service` 指回它，
   `daemon-reload`，**读回验证**。这一步不改动任何运行中的服务——此时 Inox 登录界面仍然在屏幕上。
   验证失败则立即退出，机器状态与执行前一致。
2. 验证通过之后才停 `inox-greeter@*` 与 `inox-auth.service`（并 `disable` 后者）。
3. `systemctl start greetd.service` 并确认它 active。若 greetd 起不来，脚本会打印恢复办法
   （`systemctl start inox-greeter@1.service` 回到 Inox 登录界面），而不是把你留在黑屏上。

PAM 栈与单元文件**保留**（不影响 greetd，之后试运行或再切换还要用）。要彻底删掉用
`sudo packaging/uninstall.sh`。回退与卸载都**不删** `getty@tty3.service`。

回退之后 `inox-login-fallback.service` 就不再相关了：greetd 重新成为开机链，它本来就是 greetd，
fallback 再启动它也只是一个 no-op。单元仍然留在系统里（和别的单元一起被安装/卸载），不会被启用，也不会
自己跑——它只有在 Inox 的登录单元进入 `failed` 时才触发。

## 救援 console（VT3）

`Ctrl+Alt+F3` 是一个常驻 getty。任何登录链故障都可以从那里登录、看日志、修文件：

```bash
journalctl -u inox-auth.service -b --no-pager
journalctl -u 'inox-greeter@*' -b --no-pager
loginctl list-sessions
readlink -f /etc/systemd/system/display-manager.service
```

从 VT3 回到图形界面用 `chvt 1`（开机链）或 `chvt 4`（试运行实例）。

## 卸载

```bash
sudo packaging/uninstall.sh
```

- 若 `display-manager.service` 仍指向 Inox，它**拒绝执行**并让你先跑 `rollback-to-greetd.sh`：
  在 Inox 仍是开机链的时候删掉它的单元，就是这套脚本存在的意义所要避免的半切换状态。
- 否则停并禁用 `inox-greeter@*` 与 `inox-auth.service`，删三个单元、三个 PAM 栈和残余的
  `*.wants` 软链，`daemon-reload`。
- **不删** `getty@tty3.service`、`greeter` 账户与 `/var/lib/inox-greeter`（后者是「最近使用账号」的记录）。

## 排障

| 现象 | 先看什么 | 常见原因 |
|------|----------|----------|
| 开机后等了大约一分钟，屏幕上出现的是 **greetd** 的登录界面 | `systemctl --failed`、`journalctl -u 'inox-greeter@1'` | 这是 `inox-login-fallback.service` 生效了：Inox 登录界面在 60 秒内连续失败 20 次，systemd 把它标成 failed 后启动了 greetd。**这不代表 greetd 被重新启用**（`is-enabled` 仍是 `disabled`，下次开机仍先试 Inox），也不代表链坏了不能修——日志里就是原因 |
| 无 VT 预演 PASS，但 `preflight.sh` 报 FAIL | `preflight.sh` 的 FAIL 行 | 问题在**环境/打包**而不是代码：`/dev/tty4`、`/dev/dri/card*`、PAM 栈或单元没装、seat/`ActiveSession` 不正常。先修环境，别去试 VT |
| VT4 黑屏，`inox-greeter@4` failed | `journalctl -u 'inox-greeter@*' -n 50` | 二进制缺失、`/run/inox/auth.sock` 连不上、合成器拿不到 DRM/seat |
| 登录界面显示「认证边界不可用」 | `systemctl status inox-auth`、`ls -l /run/inox/auth.sock` | `inox-auth` 没起来；socket 的属组不是 greeter 的组 |
| 每句话都被拒绝 | `journalctl -u inox-auth` | PAM 栈缺失（`--service` 与 `/etc/pam.d` 名字不一致）、账号被 `pam_faillock` 锁定 |
| 登录总以「会话无法启动」结束（`started:false`），`journalctl -u inox-auth` 里有 `pam_start` 失败 | `ls -l /etc/pam.d/inox-auth-session` | **会话负责人的栈不存在**（或 `--session-service` 与文件名不一致）：负责人打不开会话，表面症状像口令错。重新跑 `packaging/install.sh` 即可补齐 |
| 登录成功但屏幕不切换 | `loginctl list-sessions` 的 `VT` 列 | VT 管道：`XDG_VTNR` 没进 greeter 会话、logind 不可达 |
| 注销后回不到登录界面 | `loginctl list-sessions`、`journalctl -u inox-auth` | `inox-auth` 在会话期间重启过，交回失败；VT 仍可手动 `chvt 1` |
| 开机没有登录界面 | `systemctl status display-manager`、`systemctl --failed` | 开机链没切成、或 `inox-auth` 起不来导致宿主拒绝开窗 |
| 账号被锁 | `journalctl -u inox-auth`、`faillock --user <账号>` | `system-login` 自带 `pam_faillock`：多次失败会锁定账号，与 `inox-auth` 自己的滚动限额是两套预算 |

`inox-auth` 的 PAM 栈是 `include system-login`，因此它还继承 `pam_nologin`（`/etc/nologin` 存在时拒绝登录）
与 `pam_shells`（账号的 shell 必须在 `/etc/shells` 里）。这些都是有意的，不是「多余的行」。

## 安全边界（本页不改变的）

- 凭据只在 `Authenticate` 期间以 `Secret` 形式存在，答复之后由边界清零；会话负责人不接收口令，
  它只拿到父进程（root）已经验过的账号名与 VT，参数永不来自线路。
- 登录界面没有能替别人启动会话的代码（`inox-greet` 以 `default-features = false` 依赖 `inox-auth`）。
- 任何人不能通过这个链拿到 root：`inox-auth` 对 root 与系统账号一律拒绝；`greeter` 账号是
  `nologin` shell 的系统账号，只能跑宿主与合成器。
- 这套东西**不**新增网络监听、**不**上传任何东西。
