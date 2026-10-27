# packaging/

安装面（deployment surface），不属于任何 crate，不参与依赖图、`cargo deny` 或分层检查。它做的只有三件事：
把三个 PAM 栈与三个 systemd 单元放到系统里，让它们能按需启动，以及在需要时**完整地**退回 greetd。
决策见  §决策 5/§决策 6。

## 文件

| 文件 | 装到 | 作用 |
|------|------|------|
| `pam.d/inox-auth` | `/etc/pam.d/inox-auth` | **口令在这里校验**：`auth`/`account`/`session` 走 `system-login`（含 `pam_systemd`），不含 `pam_securetty` |
| `pam.d/inox-auth-session` | `/etc/pam.d/inox-auth-session` | **会话在这里打开**：会话负责人（`inox-auth --session-leader`）用它 `pam_open_session`。`auth required pam_permit.so`（负责人没有口令）+ `account`/`session` 走 `system-login` |
| `pam.d/inox-greeter` | `/etc/pam.d/inox-greeter` | greeter 自己的 logind 会话：`auth optional pam_permit.so`，`account`/`session` 走 `system-login` |
| `systemd/inox-auth.service` | `/etc/systemd/system/` | 以 root 常驻的认证边界，`/run/inox/auth.sock` |
| `systemd/inox-greeter@.service` | `/etc/systemd/system/` | 模板单元，**实例名就是 VT 号**；`User=greeter`、`PAMName=inox-greeter`、`ExecStart=/usr/bin/inox-host` |
| `systemd/inox-login-fallback.service` | `/etc/systemd/system/` | oneshot，**从不启用**：由 `inox-greeter@.service` 的 `OnFailure=` 触发，登录界面 60 秒内失败 20 次后启动 greetd，让一次失败的启动仍然可登录；不重新启用 greetd，也不改 `display-manager.service` |
| `rehearse-headless.sh` | — | **不需要 root**的无 VT 预演：私有 `XDG_RUNTIME_DIR` + 替身边界 + 真实 `inox-host`/`inox-compositor`/`inox-greet`，判定宿主编排、Wayland 握手与界面渲染（E2，S07a） |
| `preflight.sh` | — | 以 root 运行的**只读**预检：14 个二进制（8 个 required 缺一即 FAIL、6 个 optional 缺了只 WARN）、三个 PAM 栈及其 `include` 链上的模块、三个单元与 `systemd-analyze verify`、`greeter` 账户、VT/DRM/seat、两个显示管理器抢 VT1；每项 `ok`/`WARN`/`FAIL`，有 FAIL 即非零退出（S06） |
| `verify-login.sh` | — | **不需要 root**，登录之后跑：把「登录成功了吗」拆成 logind/PAM 服务/VT/`XDG_*`/登录界面存活等不变量；`--after-logout` 检查注销后画面确实交回（S09 的自动化一半） |
| `install.sh` | — | 装上面的文件 + `greeter` 账户 + VT3 救援 console；默认**不动开机链**，`--enable` 才切换。`--bin-dir DIR` 额外把 14 个二进制装进 `/usr/bin`（tarball 的安装方式） |
| `package.sh` | — | **产出发行 tarball**：默认特性 `cargo build --release --workspace --locked` → 暂存 14 个二进制 + 壁纸 + 本目录 → 断言暂存的合成器拒绝 `--test-input`（否则说明后端被编进去了）→ `BUILD-INFO` + `SHA256SUMS` → `target/dist/inox-<version>-<target>.tar.gz`。`--bin-dir` 复用已有二进制。决策见 ADR 0042 |
| `try-inox-greeter.sh` | — | 在 VT4 起 `inox-greeter@4.service` 并 `chvt 4`，不动开机链 |
| `try-rollback.sh` | — | 结束试运行：停该实例与边界，把画面切回开机链的 greeter |
| `rollback-to-greetd.sh` | — | **先**把 `display-manager.service` 与 `enabled` 交回 greetd、**再**停 Inox 单元 |
| `uninstall.sh` | — | 停/禁用并移除单元与 PAM 栈；**不删** `getty@tty3`、greeter 账户与状态目录。`--remove-binaries` 才删 `/usr/bin/inox-*` 与 `/usr/share/inox/wallpapers`（发行版包拥有的同一批路径，默认不碰） |
| `sysusers/inox.conf` | `/usr/lib/sysusers.d/inox.conf` | `greeter` 系统账户的声明，供 AUR 包经 pacman 的 `systemd-sysusers` 钩子创建；必须与 `install.sh` 的 `useradd --system` 参数一致 |
| `aur/PKGBUILD.in` 与 `aur/inox.install` | — | AUR 配方模板与 pacman 安装钩子（钩子只打印切换开机链的步骤，不启用任何东西）。渲染、构建验证与推送见 `aur/README.md` |
| `aur/publish.sh` | — | AUR 配方的唯一实现：渲染 PKGBUILD、`makepkg --printsrcinfo` 生成 `.SRCINFO`、校验依赖名、`makepkg` 构建与产物断言、幂等推送。决策见 ADR 0043 |

## 为什么是三个栈

一次登录经过两个 PAM 事务，落在两个不同的栈上，这不是冗余：

1. `inox-auth`（守护进程）用 `--service inox-auth` 验口令：这里是唯一出现 `pam_unix`、`pam_faillock`、
   `pam_nologin`、`pam_shells` 的地方。
2. `inox-auth --session-leader`（一次登录一个进程）用 `--session-service inox-auth-session` 打开会话：
   它**没有口令、也不接收口令**，所以它的 `auth` 行必须是无条件许可。把许可行放进第一个栈等于让所有人
   免密登录，因此它必须是第二个文件。`pam_systemd` 在这个栈上注册 logind 会话（VT、`/run/user/<uid>`、
   cgroup），会话进程因此拿到 seat、合成器才能拿到 DRM。
3. `inox-greeter` 只给登录界面自己的 logind 会话用（`PAMName=inox-greeter`），不校验任何口令。

少任何一个栈的症状都是「登录失败」，而不是「配置缺失」：`pam_start` 失败在 `journalctl -u inox-auth` 里，
屏幕上表现为会话启动失败（`started:false`）。

## 三条不变量

1. **默认不动开机链。** `display-manager.service` 只有在 `install.sh --enable` 时才会被改到
   `inox-greeter@1.service`。安装、试运行、结束试运行、卸载都不会改它。
2. **先切换再停服务。** `rollback-to-greetd.sh` 的第一步是让 greetd 重新拥有别名并 `enable`，此时运行中的
   Inox 登录界面仍然占着屏幕；只有在别名已被验证指向 greetd 之后才停 Inox 单元。任一步失败都不会留下
   「既不指向 greetd 也不指向 Inox」的开机链。
3. **VT3 永远是救援 console。** 安装时 `enable --now getty@tty3.service`；试运行用 VT4，用户会话落在
   VT2..=6 且跳过 greeter 的 VT。回退与卸载都不删它。

## 二进制从哪来

安装面本身**不产出二进制**。两种来源，都由 `install.sh --bin-dir` 落到 `/usr/bin`：

- **发行 tarball（测试者用这个）**：CI 的 `package` job 每次 PR 都跑 `packaging/package.sh`，
  产出 `target/dist/inox-<version>-<target>.tar.gz`；打 `v*` tag 时 `release.yml` 把它挂到
  GitHub Release 上（预发布）。解包后 `sudo ./packaging/install.sh --bin-dir ./bin`。
- **本地构建**：`cargo build --release --workspace`，然后 `sudo ./packaging/install.sh --bin-dir target/release`。

`package.sh` 用**默认特性**构建，并且会拒绝把 `--all-features` 的产物打成包：那个 feature 会在
合成器里加一个接受注入键鼠事件的套接字（`inox-compositor/test-input`），不是能交给测试者的东西。
它会真的运行暂存的 `inox-compositor --test-input` 来证明这一点。

第三条路径是 **AUR 源码包**（`packaging/aur/`）：`Release` 工作流在 tag 发布后把同一个 tag 的
PKGBUILD 推到 AUR（稳定 tag → `inox`，预发布 → `inox-canary`）。它由 Arch 用户本机用 `makepkg`
构建，不是 CI 产出的二进制，所以**不能**当 E4 证据；它与 `install.sh` 装的 `/usr/bin` 文件互斥，
pacman 会拒绝覆盖非包所有的文件。发布前工作流会在 `archlinux:base-devel` 里完整构建一遍，
并通过同一套 `--test-input` 断言。决策：ADR 0043；配方与凭据见 `aur/README.md`。

决策：ADR 0042（tarball）与 ADR 0043（AUR）。

## 与代码的接缝

- `inox-greeter@.service` 只跑 `inox-host`。宿主自己按顺序起 `inox-compositor` 与 `inox-greet`，
  单元里**不得**再写它们——顺序只应有一个所有者。
- `install.sh` / `preflight.sh` / `uninstall.sh` 里的二进制清单必须一致，而且要覆盖 `inox-session`
  标成 `required` 的每个组件（`inox-shell`、两个 `inox-panel` 表面、`inox-lock`）。少一个的症状是
  「口令正确但会话立刻结束」——看起来像密码被拒。清单曾经是五个，这就是把它写进预检的原因。
- 单元里的 `Environment=XDG_VTNR=%i` 是必需的：systemd 不会给 tty 单元推导这个变量（`getty@tty3.service`
  的 `Environment=` 是空的），而 `pam_systemd` 只从进程环境读 VT。没有它，greeter 的 logind 会话没有 VT、
  libseat 拿不到 seat、合成器上不了屏。
- `inox-auth.service` 显式传 `--service inox-auth`、`--session-service inox-auth-session` 与
  `--session-program /usr/bin/inox-session`：前两个让「装进来的 PAM 栈」与「被加载的 PAM 栈」是同一个名字
  （两者都与 `server::DEFAULT_*_SERVICE` 相同，写出来是为了让脚本安装的文件与单元加载的文件不能悄悄分叉，
  登录链），最后一个是绝对路径——程序默认值是相对名字，而系统单元的 CWD 是 `/`。

## 自检

所有脚本都可以只做静态检查：

```bash
for script in packaging/*.sh packaging/aur/*.sh; do bash -n "$script"; done
```

唯一会真的启动东西的是 `rehearse-headless.sh`，而它只在自己的 `mktemp -d` 里启动真实组件：
不需要 root、不读写 `/etc`、不碰 systemd 与 VT，退出时不会留下进程。前提是
`target/debug`（或 `--bin-dir`）里有 `inox-host`、`inox-compositor`、`inox-greet`：

```bash
packaging/rehearse-headless.sh            # 默认 12 秒
packaging/rehearse-headless.sh --seconds 5 --bin-dir target/debug
```

它给出的是 **E2** 证据（验收 S07a）：排除宿主编排、Wayland 握手与界面渲染三件事，
**不**覆盖 DRM/seat、VT 交接、PAM 与会话启动。

真机上则是这一对：

```bash
sudo packaging/preflight.sh     # 只读，切换屏幕之前；有 FAIL 就别继续
packaging/verify-login.sh       # 登录之后（--greeter-vt 1 用于开机链）
packaging/verify-login.sh --after-logout
````packaging/*.sh` 的其余部分不做本地验收——真机登录链的
验收（S06–S11 中除 S07a 以外的条目）必须由人在这台机器上完成，见
[登录与开机链](../docs/book/src/usage/login.md) 与 [验收规范 §8](../docs/quality/acceptance.md)。
