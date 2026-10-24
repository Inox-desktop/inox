# inox-host

greeter 宿主：把登录链放到 greeter 的 VT 上。属于 Inox 桌面环境的 **L4** 层，由 [ADR 0037](../../docs/adr/0037-greeter-host-vt-handoff-and-the-boot-login-chain.md) §决策 1/§决策 3 定义，`inox-greeter@.service`（`User=greeter`、`PAMName=inox-greeter`、`TTYPath=/dev/tty%i`）就是跑它。

它只做**编排**，一共四件事：

1. **先确认边界可连。** `connect(2)` 通 `auth-v1` 套接字（`--auth-socket`，默认 `$INOX_AUTH_SOCKET`，否则 `/run/inox/auth.sock`）；连不上就在**起合成器之前**以非零码退出（ADR 0037 §决策 3.1）。失败文案同时出现 `authentication boundary` 与 `inox-auth`，并带上试过的套接字路径——屏幕上不能出现一个骗人的登录框。
2. **起合成器。** `<bin-dir>/inox-compositor --socket wayland-inox-greet [--compositor-arg …]`，然后等 `$XDG_RUNTIME_DIR/wayland-inox-greet` **成为套接字**（`file_type().is_socket()`——同名的普通文件、目录或写了一半的产物都不算就绪，否则宿主会在合成器还接不了客户端时就起登录界面）**且**合成器仍存活，上限 **10 秒**；超时就停掉合成器并以非零码退出（§决策 3.2）。残留的**真 socket** 仍会通过探测：区分它与活着的监听需要建连，宿主故意不建，代价由「合成器是本宿主的活子进程」与「登录界面的重启预算」承担。
3. **起登录界面。** `<bin-dir>/inox-greet`，注入 `WAYLAND_DISPLAY=wayland-inox-greet` 与 `INOX_AUTH_SOCKET=<路径>`，其余环境继承，**stdin 为 null**（§决策 3.3）。
4. **监督。** 登录界面退出且合成器存活 → 按 ADR 0015 的有界预算重启（**60 秒内最多 3 次**，250/500/1000 ms 有界退避），超限整体退出让 systemd 记一次失败重启；合成器退出 → 宿主整体退出；`SIGTERM`/`SIGINT` → **逆序**停子进程（先登录界面，它走了之后再停合成器，宽限期 4 s / 5 s / 8 s 与 `inox-session` 的清理口径一致）。

它**不**判断「登录成功」：用户会话在别的 VT 上、由 `inox-auth` 的会话负责人启动（§决策 5）。宿主在自己的 VT 被暂停与恢复之间什么也看不到，也不需要看到——VT 恢复时屏幕上已经是登录画面（§决策 4）。

## 边界：它不做什么

- **不认证、不碰凭据。** 本 crate 没有 `libpam`、没有口令路径、不依赖 `inox-auth` 的任何类型，只对边界套接字做一次 `connect`。它连 `auth-v1` 的帧都不解析。
- **不启动用户会话。** 也没有 `server` feature、没有降权代码、没有 `uid`/`gid`：那是 `inox-auth` 的事。
- **不碰 VT 与设备。** 没有 ioctl、没有 DRM fd 转发：设备访问与 VT 仲裁在 logind + libseat，合成器自己处理 `PauseSession`/`ActivateSession`。
- **不横向依赖同层 crate。** 不依赖 `inox-auth`、`inox-compositor`、`inox-greet` 中任何一个；它只是按路径把它们**当成程序**启动。

## 命令行

```
inox-host [--bin-dir DIR] [--socket NAME] [--auth-socket PATH] [--compositor-arg ARG]…
```

| 选项 | 默认 | 说明 |
|------|------|------|
| `--bin-dir DIR` | 本可执行文件所在目录 | `inox-compositor` 与 `inox-greet` 的查找目录 |
| `--socket NAME` | `wayland-inox-greet` | Wayland 套接字名：传给合成器，并作为登录界面的 `WAYLAND_DISPLAY` |
| `--auth-socket PATH` | `$INOX_AUTH_SOCKET`，否则 `/run/inox/auth.sock` | 必须可连的认证边界套接字 |
| `--compositor-arg ARG` | 无（可重复） | 原样追加到合成器命令行（例如 `--compositor-arg --headless`） |
| `--help` / `-h` | | 用法行；不探测套接字、不解析目录，因此登录链坏着也能用 |
| `-V` / `--version` | | 版本行 |

未知参数、缺少取值、空取值一律报错并以非零码退出：静默忽略一个参数会让宿主跑起一个操作者没要求的阶段。

内部还有一个**不是**用户选项的模式 `--exec-child <program> [args…]`：calloop 的信号源是 `signalfd`，会在线程里屏蔽 `SIGINT`/`SIGTERM`，而 fork 出来的子进程会继承这个屏蔽字；该模式用安全 API 复位屏蔽字后 `exec` 目标程序（pid 不变），因此宿主发给子进程的 `SIGTERM` 真的会送到。`inox-session` 用同样的办法解决同一个问题。

## 退出码

`systemctl status` 因此能不看 journal 就分类失败：

| 码 | 含义 |
|----|------|
| `0` | 收到 `SIGTERM`/`SIGINT`，子进程已按逆序停掉 |
| `1` | 宿主自身起不来（参数、日志、`$XDG_RUNTIME_DIR`、事件循环） |
| `2` | 命令行用不了（未知参数、缺值、空值） |
| `3` | 认证边界不可连，什么都没启动（§决策 3.1） |
| `4` | 合成器退出，本阶段结束（§决策 3.4） |
| `5` | 合成器 10 秒内没有绑定 Wayland 套接字（路径必须是 socket；同名普通文件不算，§决策 3.2） |
| `6` | 某个子进程程序起不来（找不到或不可执行） |
| `7` | 登录界面用尽重启预算（§决策 3.4） |

## 状态

**已实现**（证据等级 E1+E2；纯逻辑单测 + 真实子进程集成测试）：

| 模块 | 内容 | 验收（ADR 0037 §验收 第 1、2 行） |
|------|------|------|
| `supervisor` | 编排规则作为纯状态机：`Preflight → CompositorSpawning → WaitingForSocket → Supervising → Stopping → Done`；边界拒绝、合成器超时/早退、登录界面有界重启、`SIGTERM` 逆序清理、宽限期升级为 `SIGKILL`。`Instant` 由调用方注入，无 I/O、无时钟 | 宿主编排（E1） |
| `process` | 薄适配层：每个子进程一个进程组、`--exec-child` 复位信号屏蔽字、非阻塞 reap、按组发信号、`Drop` 兜底 kill | 宿主编排（E2） |
| `runtime` | calloop 事件循环（50 ms tick + 信号源）、Wayland 套接字绑定探测（`is_socket`，不是「路径存在」）、边界 `connect` 探测、`Exit` → Fluent 文案 | 宿主编排（E2） |
| `cli` | 纯参数解析：`--bin-dir`/`--socket`/`--auth-socket`/`--compositor-arg`/`--help`/`--version`，未知与缺值拒绝 | 宿主 CLI 与文案（E1） |
| `i18n` | `en-US`/`zh-CN` 目录，`host-` 前缀，双向一致性 + 占位符一致 + 用词一致由 `tests/catalog.rs` 断言 | Q03 |

**未实现 / 未验证**（不声称上表之外的任何等级）：

- **真机登录（E4）未验证。** 裸 VT 上的 libseat/DRM 路径、`inox-greeter@.service` 的 `PAMName`/`TTYPath`、切换 VT 后的暂停与恢复，都要人在真机上按 ADR 0037 §验收 走一遍。本 crate 的测试用假合成器与假界面脚本，**不**证明真机能登录。
- **packaging 不在本 crate。** PAM 栈、systemd 单元、安装/回退/试运行脚本属于 `packaging/`（ADR 0037 §决策 6），本 crate 只管被那个单元拉起来之后的行为。
- **多用户、会话选择菜单、Suspend/VT 恢复矩阵**：ADR 0037 §未决项，v1 明确不做。
- **`inox-host` 是否最终并入 `inox-session`**：ADR 0037 未决项；两者编排规则目前各自独立。

## 目录结构

```
src/lib.rs        crate 文档、模块声明
src/supervisor.rs 编排规则（纯状态机）+ 重启预算
src/process.rs    子进程适配层（进程组、信号、reap、--exec-child）
src/runtime.rs    calloop 事件循环、套接字探测、Exit → 文案
src/cli.rs        纯参数解析
src/i18n.rs       Fluent 目录装配
src/main.rs       日志、参数、装配、退出码
i18n/{en-US,zh-CN}/messages.ftl
tests/catalog.rs  en-US/zh-CN 一致性与 id 双向检查
tests/cli.rs      --help/--version/未知参数/缺值/缺边界拒绝/缺 XDG_RUNTIME_DIR
tests/supervisor.rs 真实宿主子进程 + shell 脚本假合成器/假登录界面
```

## 测试

```sh
cargo test -p inox-host --all-features
```

- `src/supervisor.rs` 单测：每条规则一条，**含全部拒绝路径**——边界不可连（不产生任何 `Start`）、合成器在 socket 出现前退出、合成器 10 秒超时（含宽限期与 `SIGKILL` 升级）、登录界面连续崩溃超预算、启动失败也消耗预算、`SIGTERM` 逆序停止与重复信号幂等、结束后的事件全部被忽略；`Instant` 注入，因此时间规则不靠 sleep 验证。
- `tests/cli.rs`：`--help`/`-V` 不需要套接字且成功；未知参数、缺值、空值非零退出；没有边界时退出码 `3`、文案同时含 `authentication boundary` 与 `inox-auth`、且**没有**启动合成器（用会写标记文件的假 `inox-compositor` 证明）；缺 `$XDG_RUNTIME_DIR` 时明确拒绝。
- `tests/supervisor.rs`：用真实子进程（`CARGO_BIN_EXE_inox-host`）驱动 shell 脚本 fixture——假合成器报就绪（真 `UnixListener` 由 harness 绑定，shell 造不出套接字）或不绑、立刻退出、立刻崩溃，假登录界面记录环境并驻留——断言注入的 `WAYLAND_DISPLAY`/`INOX_AUTH_SOCKET`、stdin 为 EOF、`--compositor-arg` 原样透传、逆序停止的事件顺序、重启次数（1 次首启 + 3 次重启）、各拒绝路径的退出码，以及**没有留下任何进程**。另有一条「同名普通文件不算就绪」：decoy 文件存在时宿主仍按 10 s 超时以码 `5` 拒绝，登录界面从未启动。
- `tests/catalog.rs`：`en-US` 与 `zh-CN` 定义相同 id 与相同占位符；代码里出现的每个 `host-…` id 都在目录里，目录里每个 id 都被代码用到。

## 开发约定

- 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径。**
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 用 `host`。

## 许可证

LGPL-3.0，见仓库根的 [`LICENSE`](../../LICENSE)。
