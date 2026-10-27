# AGENTS.md — inox-lock

本 crate 遵循根仓库的**全部硬性规则**（[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)），本文件只补充本 crate 特有内容，不放宽任何硬性规则。

## 职责

会话锁屏。所属层级：**L3**（见根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

它是会话锁边界定下的**可信锁屏客户端**：锁状态由合成器所有，本 crate 拥有的是状态机、它的安全不变量、凭据卫生、限流策略、挂起前的锁定顺序，以及锁屏界面。它**不**拥有认证，也**不**拥有协议实现。

## crate 特有约束

- **`unsafe` 的唯一例外是 cxx-qt 生成的胶水。** `#[cxx_qt::bridge]`（`src/cxxqt_object.rs`）会展开出 `unsafe extern "C++"`，无法在 `forbid(unsafe_code)` 下编译，因此本 crate 的 `[lints.rust]` 把它放宽为 `allow`。**Inox 手写的 Rust 仍然不写 `unsafe`**；FFI 只存在于受控的 bridge 模块，安全逻辑在安全 Rust 一侧。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。`src/main.rs` 以 `std::process::exit` 结束，不是 panic。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- **crate 只能有 bin target。** cxx-qt 0.10 把桥的初始化符号放进「声明桥的那个 target」，被 `--gc-sections` 丢掉的静态库会链接失败（KDAB/cxx-qt#770）。因此没有 `src/lib.rs`：模块全挂在 `src/main.rs`，原本的 `tests/*.rs` 迁到 `src/headless_tests/` 的 `#[cfg(test)]` 模块（迁移契约 §2.1）。需要进程的测试（`cli`、`protocol`）把测试二进制自身再 spawn 一遍，子进程里被选中的「测试」直接调用 `crate::run`。
- **只有 `state::LockMachine::unlock` 能把会话推向 `Unlocked`**，而且只接受 `AuthOutcome::Success`。`LockEvent` 里没有成功变体，这是刻意设计。新增任何能改变锁阶段的代码路径前，先读会话锁边界与本模块文档。
- **来自认证边界的消息只对「在途代次」生效。** `app::update` 必须先问 `BoundaryState::is_current`，只有仍然在途的那一次尝试才接受 `AuthAccepted`/`AuthAnswered`；`is_stale` 只覆盖「更早的代次」，挡不住从未发出的代次与已被答复的那一次。
- **不允许出现「看起来像锁屏」的东西。** 会话锁边界拒绝 layer-shell 遮罩方案，本 crate 因此没有 `placement` 模块、没有锚点/exclusive zone/表面尺寸；不得引入 `LayerShellQt` 或任何 Qt Wayland 模块，也不得让二进制在协议缺失时假装已经锁定。
- **QML 是纯渲染器，且从不看凭据。** `qml/Main.qml` 只读 `bridge.modelJson`：没有颜色字面量、没有像素字面量、没有用户可见英文字符串（迁移契约 §4）；它不接输入、不持状态。**凭据只存在于 `credential::Secret`**：投影里只有由长度推出的掩码串，`Secret` 不在构建 JSON 的作用域内；不得把明文放进任何 `#[qproperty]`、`modelJson`、QML 变量或文本控件。
- **锁表面不是窗口。** `src/session.rs` 创建并持有 `ext_session_lock_v1` 表面；`src/render.rs` 只填它的缓冲区。离屏宿主（`src/offscreen.cpp`）只创建 `QGuiApplication`（**offscreen** 平台、**software** 场景图）+ `QQmlApplicationEngine` + `QQuickWindow` 并 `grabWindow()`；**不得**引入 Qt Wayland 客户端或 `LayerShellQt` 去「顺手」得到一个表面——那会创建第二个表面，正是会话锁边界拒绝的东西。
- **信号源必须先于任何线程。** `session::take` 先建 `Signals`（`SIGINT`/`SIGTERM`/`SIGHUP`）再连合成器、最后才创建 Qt 宿主：`signalfd` 只收到「在所有线程里都被阻塞」的信号，而 Qt 会起线程、线程继承创建者的掩码。测试子进程是测试二进制自己（入口在测试线程），`headless_tests` 因此在 spawn 前先于父线程阻塞这些信号、经 `exec` 让子进程继承。改动这条顺序会让 SIGTERM 变成直接杀进程，「会话仍锁定」的提示随之消失。
- **认证不在本 crate 内。** 不得实现口令校验、不得调用 PAM、不得添加认证 helper 或任何特权路径——校验永远在 `inox-auth`。`src/boundary.rs` 只是 `auth-v1` 的**客户端**：它把 `AuthAttempt` 发出去、把答复变成 `contract::AuthOutcome`，仅此而已；任何让它拥有「判断」能力的改动都要先改会话锁边界。
- **凭据不出安全边界。** 明文只能经 `credential::Secret::expose` 取出；唯一调用链是 `contract::AuthAttempt` → `boundary::ask`，在那里被编码进一帧并随两个缓冲区一起清零。禁止把凭据写入日志、通知、剪贴板或截图，禁止给 `Secret` 加 `Clone`/`PartialEq`/`Deref`/`AsRef<str>`。凭据在**取消、UI 隐藏或退出、解锁与挂起**时清零。改 `app::update` 时，新增任何会让会话离开屏幕的路径都必须先清零。**键盘路径同样不得记录字符**：`app::Key` / `app::Message` 的 `Debug` 里有明文，`?key` 就是一段口令进了日志。
- **限流文案不含计时。** 「还剩几次」「还要等 37 秒」都是限流器的内部状态，不得出现在任何面向用户的字符串里；等待时长只属于 `attempts::AttemptLimiter`。
- **用户可见文案只来自 Fluent**，`en-US` 与 `zh-CN` 同步；`headless_tests::catalog` 双向检查「代码提到的 id 都在目录里」与「目录里的 id 都被用到」。
- **颜色、尺寸、间距、圆角只来自 `inox_config::Theme`。** `view.rs` 与 `theme.rs` 里没有**设计色与设计尺寸**的字面量（结构性哨兵如 `TRANSPARENT`、边框宽 `0.0` 除外）；布局宽度用**比例**（无量纲，如卡片占 `2/(2+2×margin)`），字号四角色由 `spacing.lg × typography.scale` 派生，hairline 为 `spacing.xs / 2`，焦点环为 `spacing.xs`（`docs/quality/ui-guidelines.md` §1–§2）。QML 的初始尺寸由宿主以 context property 传入，QML 里不写像素。
- **掩码不是文本框。** 凭据字段画的是由**长度**推出的掩码，按键经 `app::key_binding` 以消息到达；不得把明文交给任何持有可能被复制的 `String` 的文本控件。
- 与工作区内其他 Inox crate 的依赖使用 **path 依赖**（根级 §8）。本 crate 依赖 L1：`inox-common`（i18n）、`inox-config`（设计令牌）；会话锁与通知总线第 7 条之后**另有一条向下的 L3 → L2 边**：`inox-protocols`（`ext-session-lock-v1` 与 `inox-ipc` 绑定）、`wayland-client` 与 `calloop`（协议腿的事件循环）、`rustix`（`memfd_create`）。界面侧：`cxx` / `cxx-qt` / `cxx-qt-lib`、`serde_json`（投影）、构建期 `cxx-qt-build`。**不得**新增 L3 依赖或依赖 `inox-compositor`。
- 新增依赖前先确认依赖方向合法（根级 §2），否则 `cargo test -p inox-integration --test layering` 会失败。
- **`--request-lock` 只发一条控制请求，不锁任何东西。** 它连会话的 `session-control-v1` 套接字（`INOX_SESSION_CONTROL` → `$XDG_RUNTIME_DIR/inox-session/control.sock`），写 `lock\n`，读 `ok|busy|unknown`，然后退出。`ok` 只表示会话已派发启动请求，**不是**锁定确认（那是合成器的 `session_lock_state`）。它不得退回到「自己画一个锁屏」；没有会话应答应报错并以非零退出。
- **解锁只有一条路：认证成功。** 只有 `state::LockMachine::unlock` 接受 `AuthOutcome::Success`，只有 `app::update` 的成功分支会排队 `OutgoingCommand::Unlock`，只有 `session::Client::unlock` 会发送 `unlock_and_destroy`，而且只在合成器已确认锁（`locked`）之后；发送后必须 `flush`，否则请求留在发送缓冲里、进程退出、会话仍然锁着。**信号仍然不解锁**：`SIGTERM`/`SIGINT` 停止进程、会话保持锁定并如实告知。`unlock_and_destroy` 没有对应的协议事件，因此不得伪造 `Message::UnlockConfirmed`。
- **本构建对状态说真话，界面因此可能画出「不可用」而不是「请输密码」。** 启动时探测三件事并把事实喂进状态：认证边界在不在听（`Disconnected`）、本次 uid 有没有账号名（`NotProvided`）、合成器有没有给键盘（`NoInput`）。任何一件不成立，投影就画 `Face::Unavailable`、凭据字段与提交控件一律禁用且带可读原因。**不得**让这条路径在没有边界/没有键盘时表现出可以认证的样子——`Message::BoundaryRetried` 也不得把边界复位成 `Idle`（重试需要重新探测，尚未接线）。改 `contract`/`view_model`/`app` 的这三处时必须同时改另两处。
- **锁表面只读键盘，不读指针与触摸。** `src/session.rs` 绑定 `wl_seat`/`wl_keyboard`，只在**自己的锁表面**获得焦点时把按键变成 `app::Message`；`src/input.rs` 是唯一读 keymap 的地方（`xkbcommon`，安全文本路径），`src/render.rs` 仍然不消费任何 `Message`。指针/触摸不做：布局在 QML 里，Rust 侧命中测试会是第二份布局数学。输入被接受的相位条件是**合成器已确认锁定**（`Phase::Locked`），`Locking`/`Covering` 不接受提交。IME、compose 死键与按键自动重复都还没有——这是已知限制，必须留在 README 里。
- **帧由 `src/render.rs` 画，不由别处画。** 锁表面的内容是 `view` 的投影经离屏 Qt Quick 渲染、再转成 `wl_shm` 的 `Argb8888` 字节；尺寸取自 configure、不取自本地推断；不被绘制的尺寸（0 或超过 `MAX_FRAME_BYTES`）拒绝提交，让输出回到合成器的锁定底色，**绝不**提交空缓冲或尺寸不符的缓冲。每个表面一个 `Interface`（各自的 `QQmlApplicationEngine` + `QQuickWindow`），进程级 `QGuiApplication` 只创建一次。
- **协议腿由 `app::State` 驱动，不得再长出第二份覆盖簿记。** 协议事件喂 `app::Message`，`State::take_outgoing()` 是唯一出口。合成器的 `locked` 与 `LockMachine::reports_locked()` 不一致时必须留下 warn，而不是静默接受。
- **令牌只经 `INOX_LOCK_TOKEN` 进入本进程，且不得打印。** 它由 `inox-session` 生成、只投递给本组件与合成器；日志、错误信息与用户可见文案里都不得出现它的值。
- **协议错误不得被当成解锁。** 合成器发来 `finished` 表示拒绝或撤回：没有 `locked` 时它是拒绝（退出非零），有 `locked` 时是合成器自己释放；两种情况都不由本 crate 决定会话是否安全。

## 主要依赖

- 上游 Inox crate：`inox-common`（`i18n::Localizer`/`Locale`、`i18n::compare_catalogs`）、`inox-config`（`Theme` 令牌、`InoxConfig`/`ConfigPaths`）、`inox-protocols`（`ext-session-lock-v1`、`inox-ipc`；L3 → L2 的合法向下边）。
- 界面：`cxx` / `cxx-qt` / `cxx-qt-lib`、`serde_json`（投影的序列化）、构建期 `cxx-qt-build`（QML 模块 + C++ 宿主）。
- 认证：`inox-auth`（`default-features = false`，只有 `wire` 契约，不链接 PAM/zbus；L3 → L2 的合法向下边）。
- 键盘：`xkbcommon`（`wayland` feature 关闭——`new_from_fd` 是 `unsafe fn`，本 crate 手写代码不用）。
- 第三方（协议与安全）：`wayland-client`、`calloop`、`rustix`（`fs` + `process`）、`users`（**只**用于 `get_user_by_uid`）、`zeroize`（凭据清零）、`thiserror`、`tracing`、`tracing-subscriber`、`anyhow`（仅 `main`）。
- 测试：`proptest`、`wayland-server`（协议测试扮演合成器一侧）、`tempfile`、`image`（仅在设置 `INOX_SCREENSHOT_DIR` 时写 PNG 工件）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。`CARGO_HOME` 指向仓库内缓存，`--offline` 避免网络。

| 目的 | 命令 |
|------|------|
| 自检 | `CARGO_HOME=$PWD/.cargo-home cargo check -p inox-lock --all-targets --all-features --offline` |
| 静态检查 | `CARGO_HOME=$PWD/.cargo-home cargo clippy -p inox-lock --all-targets --all-features --offline -- -D warnings` |
| 本 crate 测试 | `CARGO_HOME=$PWD/.cargo-home QT_QPA_PLATFORM=offscreen cargo test -p inox-lock --all-features --offline` |
| 协议腿（E2） | `QT_QPA_PLATFORM=offscreen cargo test -p inox-lock --all-features headless_tests::protocol` |
| 像素画面（Q02） | `QT_QPA_PLATFORM=offscreen cargo test -p inox-lock --all-features headless_tests::render` |
| 格式化 | `cargo fmt -p inox-lock` |
| 文档 | `RUSTDOCFLAGS='-D warnings' cargo doc -p inox-lock --no-deps --all-features` |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |

## 测试要求

- 纯逻辑必须有单元测试（`#[cfg(test)]`），**含拒绝与错误路径**，不只覆盖成功路径。
- 属性测试每项 ≥1000 样本（`ProptestConfig::with_cases(1000)`）：
  - **除认证成功外，任何事件序列都不会让会话离开锁定**（UI 退出、认证失败、认证超时、新增/移除输出、挂起请求、挂起完成）；
  - **覆盖簿记与独立写出的模型一致**，且「报锁定」必然跟随「所有现存输出已被遮蔽」。
- `headless_tests::render` 必须包含浅色、深色、限流等待、失败、无可信 UI、**本构建没有认证边界（`NotProvided`，无重试控件）**、协议缺失与 200% 字号等场景（1280×720），断言像素、四角为不透明 scrim、卡片用 `surface` 令牌、禁用控件与启用控件渲染不同；测试夹具经 `render::Interface` 渲染——那是二进制提交帧走的同一段代码——而不是另建一条渲染路径。Qt 每进程只允许一个 `QGuiApplication`，因此这些场景放在**同一个测试函数**里顺序渲染。
- `headless_tests::protocol` 除了认领/遮蔽/信号不解锁断言，还必须读回客户端交出的共享内存帧并断言：字节数等于 configure 尺寸的 4 倍、每个 alpha 为 `0xff`、像素不止一种颜色、**帧中心（卡片）与角落（scrim）不同**。只断言 buffer 的**格式**与**尺寸**不足以说明锁表面承载了界面；在协议测试用的小输出（320×240）上卡片会占满高度，因此「背景像素占多数」不是可靠判据。
- `headless_tests::cli` 覆盖默认路径：不带参数时非零退出、文案含会话锁边界与 `--take-lock`、**不含**密码提示、未知参数失败。
- 新增行为必须同时新增测试；测试代码不受「禁止 unwrap/expect/panic」约束。

## 目录结构

```
crates/inox-lock/
  Cargo.toml      工作区继承：字段一律 *.workspace = true
  build.rs        CxxQtBuilder：QML 模块 + 桥 + C++ 宿主
  src/            源码（模块划分见 README）
  qml/Main.qml    QML 渲染器
  i18n/           en-US / zh-CN 的 Fluent 目录
  src/headless_tests/  catalog / cli / protocol / render（原 tests/）
  README.md       本 crate 说明
  AGENTS.md       本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `lock`。
