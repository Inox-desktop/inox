# 0041 — `session-control-v1`: 手动与空闲触发锁定的私有控制通道

- 状态：**accepted**
- 日期：2026-10-25
- 相关：会话锁边界（已删除的 ADR 0022 系列）、`crates/inox-protocols/protocol/session-control-v1.md`、
  `crates/inox-session/src/control.rs`、`crates/inox-lock --request-lock`

## 背景

截至 M5，锁屏已经能被**启动**（`inox-session` 在 `INOX_SESSION_TAKE_LOCK=1` 时拉起
`inox-lock --take-lock`），协议腿、界面、认证与解锁路径都已落地。缺的是**谁有资格要求锁定**：

- 会话监督进程是唯一持有会话锁令牌的进程，因此只有它能启动可信锁屏客户端；
- 合成器不 spawn 进程，也不该为「请去锁」新增 Wayland 请求——`inox-ipc` 是任何能连合成器的
  客户端都能绑定的全局对象；
- 空闲守护（`inox-idle`）没有令牌，也不该有（令牌只投递给两个进程）；
- 手动锁定（快捷键、CLI）同样需要一条通往会话的通道。

没有这条通道，锁定只能靠「重开会话并带上环境变量」，那不是一个可用的产品行为。

## 决策

新增一条私有 Unix 套接字契约 `session-control-v1`，由 `inox-session` 监听：

- 路径 `$XDG_RUNTIME_DIR/inox-session/control.sock`，目录 `0700`（属主为会话用户），
  套接字文件 `0600`；可用 `INOX_SESSION_CONTROL` 覆盖（测试与非常规部署）。
- 请求只有一个动词：`lock\n`。答复 `ok` / `busy` / `unknown`。
- 服务端在**读任何字节之前**用 `SO_PEERCRED` 校验对端 uid 必须等于自身 uid；不匹配则不回复、关闭。
- 类型与帧在 `inox-protocols::control`（L2）：客户端是 L3（`inox-lock`）、服务端是 L4
  （`inox-session`），共享类型只能落在两者都合法的 L2 边界上；规范文本在
  `inox-protocols/protocol/session-control-v1.md`。
- 服务的启动模式由 `ServiceSpec::start` 表达：`AtStartup`（现状）与 `OnDemand`
  （只由请求启动，且**从不自动重启**）。锁客户端是 `OnDemand` + `oneshot`：
  重启它要么被「已锁」的合成器拒绝，要么把用户刚解开的桌面重新锁上。

## 为什么不是其它方案

| 方案 | 否决理由 |
|---|---|
| `inox-ipc` v3 新增 `request_lock` | 该全局对象任何 Wayland 客户端都能绑定，等于对所有应用开放「锁我屏幕」；沙箱应用连不上 `0700` 的 runtime 目录，但连得上 Wayland socket |
| 空闲/面板直接 spawn 锁客户端 | 需要把会话锁令牌发给第三个进程，破坏「令牌只投递给两个进程」的能力模型 |
| 信号（SIGUSR1） | 无回执、需要发现 pid、不可扩展；只能当一天 spike |
| 让合成器自己启动客户端 | L2 不 spawn 进程，也不该拥有会话生命周期 |

## 后果

- 手动锁定与空闲触发共用同一条通道；`inox-lock --request-lock` 是它的命令行客户端。
- `ok` **不是**锁定确认：会话是否被遮蔽由合成器的 `inox-ipc` `session_lock_state` 报告。
  把 `ok` 当成「已锁」的调用方信任了错误的一半。
- 这条通道**不能解锁**：解锁只由 `auth-v1` 的一次成功答复构成（`unlock_and_destroy` 仅由
  `AuthOutcome::Success` 触发）。它的最坏后果是让会话把自己锁起来。
- 空闲触发的完整链路还缺合成器的 `ext-idle-notify` 实现与 `inox-idle` 对
  `session_lock_state` 的订阅；本 ADR 只负责「谁有资格要求锁定」这一段。
- 新增私有 ABI 意味着：破坏性变更必须先改规范文本，并同时改三处实现与测试。
