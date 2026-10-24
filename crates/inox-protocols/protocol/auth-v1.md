# `auth-v1` — the private authentication contract

- 状态：**normative**（规范文本）。实现见 `crates/inox-auth/src/wire.rs`；决策见
  [ADR 0029](../../../docs/adr/0029-inox-authentication-boundary.md)。
- 传输：Unix 域套接字，默认 `/run/inox/auth.sock`（可用 `INOX_AUTH_SOCKET` 覆盖）。
- 对端：服务端为 `inox-auth`（特权 helper），客户端为 `inox-greet`（登录界面）。锁屏
  （`inox-lock`）将来的解锁路径复用同一契约。
- 版本：接口名 `inox-auth-v1`，`version = 1`。握手帧携带版本，不匹配即拒绝。

## 1. 为什么不是 Wayland 协议

`inox-ipc` 走 Wayland 连接，而认证边界必须**早于任何合成器**存在：登录界面本身还没有
合成器可连，helper 也不持有 Wayland 连接。因此这是一条单纯的 Unix 套接字私有协议，
按 ADR 0026 对配置契约的同一做法记录在 `inox-protocols/protocol/` 下：**契约文档在这里，
类型与实现留在说它的 crate 里**，不新增层级边，也不把 helper 拉进 L2 依赖图。

## 2. 帧格式

```
frame   = payload_len: u32 (big-endian) , payload
payload = tag: u8 , fields…
```

- `payload_len` 上限 `MAX_FRAME_BYTES = 4096`，**在分配缓冲之前**校验：超出即断开连接，
  不回复。长度前缀说谎的对端与我们在「下一帧从哪里开始」上没有共识，因此没有重同步。
- 字段为大端定长整数与定长前缀字节串；每个 tag 的字段之后不得有多余字节。
- 未知 tag、长度不符、字段截断、多余字节：可回复时回 `Error`，不可回复时断开。
- **一条连接一条请求**：客户端连接、握手、发一条请求、读一条答复，然后连接结束。
  服务端也主动关闭，不做连接复用。

### 请求（客户端 → 服务端）

| tag | 名称 | 字段 |
|-----|------|------|
| 1 | `Hello` | `version: u32`（必须等于 1） |
| 2 | `Authenticate` | `attempt: u64`, `name_len: u16`, `name: bytes`, `secret_len: u16`, `secret: bytes` |
| 3 | `StartSession` | `attempt: u64` |
| 4 | `Cancel` | `attempt: u64` |

- `attempt` 由客户端选择、服务端原样回显；它**不是能力**，模型只认自己发出的那一个。
- `name` 是账号名，语法见 §4；`secret` 是口令的字节（不要求 UTF-8）。
- `secret_len` 上限 `MAX_CREDENTIAL_BYTES = 1024`。超限是客户端错误，**不截断**：被截断的
  口令是另一个口令。

### 答复（服务端 → 客户端）

| tag | 名称 | 字段 |
|-----|------|------|
| 16 | `Welcome` | `version: u32` |
| 17 | `Accepted` | `attempt: u64` |
| 18 | `Rejected` | `attempt: u64` |
| 19 | `Unavailable` | `attempt: u64`, `reason: u8` |
| 20 | `SessionStarted` | `attempt: u64`, `started: u8`（0/1） |
| 21 | `Error` | `code: u8` |

`Unavailable.reason`：`0` 无边界（客户端自用）、`1` 忙、`2` 超限、`3` 认证器失败。
**没有「用户不存在」**：不存在的账号、root 与系统账号一律按 `Rejected` 答复，否则屏幕上
的文案就成了账号枚举探针。

`Error.code`：`0` 格式错误、`1` 版本不支持、`2` 对端不被服务。

## 3. 一次登录的时序

```
greeter ── connect ──► helper
        ── Hello{1} ─►                       版本校验
        ◄─ Welcome{1} ─
        ── Authenticate{attempt,name,secret} ─►
                                              SO_PEERCRED 校验 uid/gid
                                              账号策略（非 root、uid ≥ 阈值）
                                              滚动限额（60 s 内 3 次）
                                              PAM（service = inox-greet）
        ◄─ Accepted | Rejected | Unavailable ─
   （连接结束；被接受的尝试在服务端按 (对端 uid, attempt, 期限) 记住）
        ── connect ──► helper
        ── Hello{1} ─►
        ◄─ Welcome{1} ─
        ── StartSession{attempt} ────────────►  pam_open_session
                                               以该账号的 uid/gid/附加组启动固定会话
        ◄─ SessionStarted{attempt,started} ─
```

- **为什么要分两条连接。** 登录界面必须能在「密码对了」之后仍然取消（关窗、Esc）。若
  被接受的尝试绑定在连接上，客户端就只有两种选择：要么在用户还没确认时就把会话启动，
  要么把一条套接字挂在界面的决定上。绑定到 `(对端 uid, attempt, 期限)` 让两条都成立：
  续接是**同 uid 的另一个进程**也能做的动作，而它本来就有权发起登录；取消则是一条
  `Cancel`（或干脆不再续接，等它过期）。
- 续接**只能成功一次**：服务端在转发 `StartSession` 之前就把该尝试取走。
- 未被续接的尝试在 `attempt` 期限（默认 30 s）后过期并由服务端撤销；它没有打开过
  PAM 会话（`pam_open_session` 只在真正启动会话时调用）。
- 会话运行期间，新的 `Authenticate` 得到 `Unavailable{Busy}`：认证器线程正在等待该会话
  结束（PAM 会话必须与进程生命周期一致）。

## 4. 账号名语法

```
name = ( letter | "_" ) , { letter | digit | "_" | "-" | "." }
letter = "a"…"z" | "A"…"Z"      digit = "0"…"9"
```

长度 1…128 字节，不含空字节、不区分路径语义。客户端与服务端**都必须**用同一条规则
校验（实现里是同名函数 `wire::valid_username`）：屏幕能输入什么、边界会接受什么，是同一
句话，而不是两处各自的看法。

## 5. 安全不变量（实现必须保持）

1. **对端凭据来自内核**：`SO_PEERCRED` 的 uid/gid，而不是报文里的任何字段。不在允许集合
   内的对端在任何读取之前就被拒绝（`Error{UnauthorizedPeer}`）。
2. **一条报文里的口令不进日志**：口令在协议层就进 `Secret`（清零、不可打印、唯一出口），
   解码用的临时缓冲在一段作用域内自清零；`Debug` 输出里只有 `<redacted>`。
3. **会话程序不来自报文**：`StartSession` 只带 `attempt`。要启动什么由 helper 的配置决定。
4. **失败不泄漏账号是否存在**：见 §2 的 `Unavailable.reason`。
5. **限额在服务端也在客户端**：客户端限流是给人看的礼貌，服务端限流才是边界。
6. **不支持即拒绝**：版本不匹配、未知 tag、超限、截断都不猜测、不继续。

## 6. 兼容性

`auth-v1` 是 Inox 1.0 之前的私有契约，允许破坏性变更，但必须：改本文件、改
`crates/inox-auth/src/wire.rs` 的往返测试、并按根级 AGENTS.md §4/§7.2 先写 ADR。
`Cargo.lock` 之外没有版本矩阵：两侧同仓、同一次 CI。
