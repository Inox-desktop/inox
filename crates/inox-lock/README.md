# inox-lock

会话锁屏。属于 Inox 桌面环境的 **L3** 层。

本 crate 的规范是会话锁边界，它比 PRD 更严格，并直接决定了两件事：

- **锁屏不是遮罩。** 会话锁边界明确拒绝「顶层全屏窗口或 layer-shell 遮罩当锁屏」，因为两者都不能证明输入与其他层的隔离、也不能证明 UI 崩溃后会话仍被遮蔽。锁状态由合成器所有，`inox-lock` 是 `ext-session-lock-v1` 的可信客户端。因此本 crate **没有摆放策略**：没有锚点、没有 exclusive zone、没有层表面尺寸——没有层表面可摆。
- **认证是人审项，不是本里程碑的产物。** PAM 或其他认证机制、helper 权限、凭据传递与清理都是会话锁边界要求人类签字后才落地的条目，Inox 不得自研口令哈希/校验；**测试桩不算认证**。

## 表面模式：真实锁表面 + 离屏 QML

锁表面是真实的 `ext_session_lock_v1` 表面（`src/session.rs` 创建、认领、按 configure 提交帧），界面由 QML 渲染，但渲染目标是**离屏**的：

```text
calloop 主循环（session.rs，信号源先于任何线程装好掩码）
  └─ 状态变化 → view.rs 投影成一份 JSON（标题、句子、掩码长度、控件与颜色）
       └─ render.rs → cxx-qt 桥（cxxqt_object.rs）→ C++ 宿主（offscreen.cpp）
            └─ QGuiApplication(offscreen) + QQmlApplicationEngine → QQuickWindow::grabWindow()
                 └─ RGBA8 预乘像素 → 转成 Argb8888 → wl_shm 帧 → 真实锁表面
```

进程里**没有** Qt Wayland 客户端、没有 `LayerShellQt`、没有 `xdg_toplevel`，也没有 Qt Wayland Compositor（GPL-only，许可证不允许引入）。每个锁表面一个 `render::Interface`，各自持有自己的 `QQmlApplicationEngine` 与 `QQuickWindow`，因此笔记本屏与 4K 显示器可以各自按 configure 尺寸出帧；进程级的 `QGuiApplication` 只创建一次。

**凭据从不进入 QML。** 投影里只有掩码的**长度**推出的掩码串与文案；`credential::Secret` 在构建 JSON 的地方根本不在作用域内。

## 状态

**已实现**（证据等级 E0/E1：`fmt`、`clippy -D warnings`、`cargo doc -D warnings`、`cargo test` 全绿；纯逻辑有单元测试与属性测试）：

| 模块 | 内容 | 验收 |
|------|------|------|
| `state` | 会话锁边界的锁状态机：`Unlocked / Locking / Locked / Covering / LockedWithoutUi / Unlocking`，输出覆盖门（`Locked → Covering → Locked`）、UI 崩溃与认证失败保持锁定、**`unlock()` 是通往 `Unlocked` 的唯一门**（`LockEvent` 里根本没有成功变体）。两条 ≥1000 样本的属性测试 | S02、S03 |
| `attempts` | 失败限流：滑动窗口内的有界尝试、等待上限正好一个窗口、内部计时不出现在任何面向用户的文案里；边界与拒绝路径有测试 | S03 |
| `credential` | 凭据缓冲：基于 `zeroize`，`clear` 就地写零而非缩短长度，`Debug`/`Display` 不打印值或长度，退格按**字符**而非字节并清零尾部，不可 `Clone`、不可 `PartialEq` | S03 |
| `suspend` | 挂起顺序：先锁定 → 收到**当前请求**的锁完成确认 → 才允许挂起；确认带代次，旧代次的确认不授权 | S03 |
| `contract` | 交互契约的边界词汇：请求代次、`Sent/Accepted/Answered/Unavailable`、答复只命中「在途代次」；`LockProtocol` 与 `ProtocolAbsence` 把「合成器没有这个协议」建模成值而不是错误 | D05 |
| `view_model`、`view`、`theme`、`i18n` | 状态到画面的纯投影（提示、验证中、失败与原因、限流等待、无可信 UI、不可用 + 重试；标题跟着画面走）、**Qt-free 的 JSON 文档**（颜色为 `#AARRGGBB`、尺寸为数值、句子来自 Fluent）、Fluent 目录 | D05、Q01、Q02、Q03 |
| `app` | Elm 循环：`State`/`Message`/`update`，键入、提交、限流、挂起编排、按键绑定；凭据以**移动**而非拷贝的方式离开状态进入待发命令 | D05 |
| `render` | 界面真正落到像素上：离屏 Qt Quick 宿主渲染 `view` 的投影，取回 `RGBA8888` 预乘帧并转成 `wl_shm` 的 `Argb8888` | S02、Q02 |
| `session` | `ext-session-lock-v1` 客户端：认领角色、取锁、每个输出一块锁表面、按 configure 提交渲染帧、输出消失时显式销毁表面；绑定 `wl_seat`/`wl_keyboard` 把按键喂进 `app::update`；成功后**唯一的**解锁请求（`unlock_and_destroy`，且发送后 flush）；信号不解锁 | S02、S03 |
| `main::--request-lock` | 手动触发的客户端：`session-control-v1` 的一条 `lock` 请求（`ok` 只表示已派发，不表示已锁），没有会话时如实报错并非零退出 | S02 |
| `boundary` | `auth-v1` 客户端：每次尝试一条连接、一次自有拷贝后随两段缓冲清零、代次对齐、答复只映射成 `contract::AuthOutcome`；失败分类（拒绝/超时/忙碌/限额/认证器失败/版本不符/不可达）；账号名取自会话 uid 的口令数据库条目 | D05、S03 |
| `input` | keymap 编译（`xkbcommon` 安全文本路径、拒绝内部 NUL、有界读取）与 keycode→`app::Key`：字符、空格、Backspace、Enter、Escape、Tab；控制字符（`Ctrl+…`）与死键不产生字符 | D05、Q01 |
| `runtime` | 最小的 `Task`（消息列表），保留 `update` → 后续工作的形状，不引入异步运行时 | — |

**未实现 / 被阻塞**（不声称上表之外的任何等级）：

- **空闲触发没有接。** 运行期触发已有一条通道（`--request-lock` → `session-control-v1`），但空闲链路还缺合成器的 `ext-idle-notify` 实现与 `inox-idle` 对 `session_lock_state` 的订阅；快捷键也还没有绑定到这条通道。
- **指针、触摸、IME 与按键重复没有接。** 键盘已接（见上表），但只读键盘：布局在 QML 里，Rust 侧命中测试会是第二份布局数学；`wl_surface` 没有 `text-input`，所以用输入法或 compose 死键打的字符（例如 `é`、CJK）打不出来；本会话没有任何东西合成按键重复，因此长按退格只删一个字符。这三条是**已知限制**，不是待修的 bug。
- **「重试」控件还按不到。** `Face::Unavailable { retry: Some(_) }` 会画出重试控件，但焦点里只有凭据与提交两项；重试需要一个重新探测边界的消息路径，尚未接线（`Message::BoundaryRetried` 存在，启动探测不会重跑）。
- **挂起传输没有接。** `suspend` 模块的「先锁定、确认后才允许挂起」是纯逻辑并已被测试；本 crate 没有 logind 传输，`OutgoingCommand::AllowSuspend` 目前只记一条 warn。
- **没有 AT-SPI 语义树。** 锁屏的 Qt 应用跑在 `offscreen` 平台上，没有 QPA 无障碍桥；QML 侧写了 `Accessible` 角色与名称，但今天没有任何东西把它们导出。
- **凭据字段不是文本框。** 掩码由凭据的**长度**推出，按键以消息形式到达；密码明文因此从不进入控件树。代价是没有光标、没有选区、没有 IME、字段内没有复制粘贴。

## 目录结构

```
src/main.rs            crate 文档、模块声明、日志、配置、目录；默认说明并退出，--take-lock 运行协议腿
src/render.rs          离屏 QML 帧：投影 → Qt Quick → Argb8888
src/session.rs         协议腿：取锁、覆盖每个输出、提交渲染帧、键盘、唯一的解锁请求
src/boundary.rs        `auth-v1` 客户端：一条连接一次尝试，答复经 channel 回到事件循环
src/input.rs           keymap → 键意：`wl_keyboard` 的 keycode 变成 `app::Key`
src/cxxqt_object.rs    cxx-qt 桥（唯一的 QML↔Rust 边界）
src/offscreen.cpp/h    C++ 宿主：QGuiApplication + QQmlApplicationEngine + grabWindow（每个表面一份引擎）
qml/Main.qml           QML 渲染器（无颜色/尺寸/文案字面量）
src/state.rs           锁状态机与不变量（纯）
src/attempts.rs        失败限流（纯）
src/credential.rs      会自我清零的凭据缓冲（纯）
src/suspend.rs         挂起前的锁定顺序（纯）
src/contract.rs        交互契约的边界词汇
src/view_model.rs      状态 → 画面（纯投影）
src/view.rs            画面 → JSON 文档（Qt-free）
src/theme.rs           设计令牌 → 文档的颜色、尺寸与控制状态
src/app.rs             Elm：State / Message / update / key_binding
src/runtime.rs         最小的 Task（消息列表）
src/i18n.rs            Fluent 目录装配
i18n/{en-US,zh-CN}/lock.ftl
src/headless_tests/{catalog,cli,protocol,render}.rs + mod.rs
```

## 测试

```sh
export CARGO_HOME=$PWD/.cargo-home
QT_QPA_PLATFORM=offscreen cargo test -p inox-lock --all-features --offline
```

crate 只能有 bin target（cxx-qt 0.10 的桥链接限制，迁移契约 §2.1），因此原来的 `tests/*.rs` 全部迁进 `src/headless_tests/`；需要**进程**的测试（`cli`、`protocol`）把测试二进制自身作为子进程再跑一遍，子进程里被选中的那个「测试」直接调用 `crate::run`。

- 单元测试：每条纯规则各一条，**含拒绝与错误路径**（重复覆盖、未知输出、重复锁定、无挂起在途却收到完成确认、陈旧答复、从未发出的代次、已被答复后重放的消息、超限尝试、退格与清零；取消、UI 退出与挂起都清零凭据）。
- 属性测试（每项 ≥1000 样本）：任意事件序列下「除认证成功外永不进入 `Unlocked`」，以及「输出覆盖簿记与独立模型一致、报锁定必然跟随覆盖」。
- `headless_tests::catalog`：`en-US` 与 `zh-CN` 的 id 与占位符一致；代码里出现的每个 `lock-…` id 都在目录里，目录里每个 id 都被代码用到。
- `headless_tests::cli`：真实子进程的默认路径——不带参数时非零退出、文案含会话锁边界与 `--take-lock`、**不含**密码提示；`--help` 不需要显示。
- `headless_tests::render`：一个测试函数（Qt 每个进程只允许一个 `QGuiApplication`）把 Q02 要求的每个画面——深色、浅色、已输入、限流、失败、无可信 UI、协议缺失、认证边界不可用、200% 字号——经**与二进制提交帧相同的** `render::Interface` 渲染成 1280×720 的真实像素并断言：帧长、四角是不透明的 scrim、卡片用 `surface` 令牌、禁用控件与启用控件不同、掩码的像素数等于长度；另有两个宿主同时渲染各自尺寸的用例。
- `headless_tests::protocol`：手写的 `ext-session-lock-v1`/`wl_shm`/`inox_ipc` 服务端 peer 驱动真实进程，除了认领、遮蔽与「信号不解锁」，还**读回客户端交出的帧**：字节数等于 configure 尺寸的 4 倍、每个 alpha 为 `0xff`、不止一种颜色、**画面中心（卡片）与角落（scrim）不同**（在 320×240 的小输出上卡片占满高度，所以不能用「背景像素占多数」来判定「画了东西」）。

**信号与子进程（容易踩的坑）：** `signalfd` 只收到「所有线程都阻塞」的信号。`session::take` 因此先建信号源、再连合成器、最后才创建 Qt 宿主（Qt 会起线程，线程继承创建者的掩码）；测试子进程是**测试二进制自己**，入口跑在测试线程而不是 `main` 线程，所以 `Fixtures` 在 spawn 之前先在父线程阻塞 SIGINT/SIGTERM/SIGHUP，让子进程经 `exec` 继承掩码。删掉任何一半，SIGTERM 都会变成直接杀进程，「会话仍保持锁定」的那句提示就再也不会出现。

## 开发约定

- 遵循[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**；本地命令只作自检。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 用 `lock`。

## 许可证

LGPL-3.0，见仓库根的 [`LICENSE`](../../LICENSE)。
