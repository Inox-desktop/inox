# AGENTS.md — inox-host

本 crate 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 的**全部硬性规则**，本文件只补充本 crate 特有内容，不放宽任何根级约束。

## 职责

greeter 宿主。所属层级：**L4**（见根 `Cargo.toml` 的 `[workspace.metadata.inox.layers]`）。

它拥有的是**登录阶段的进程编排**：[ADR 0037](../../docs/adr/0037-greeter-host-vt-handoff-and-the-boot-login-chain.md) §决策 1 / §决策 3 定义的 `inox-greeter@.service` 就是跑它。四件事：先确认 `auth-v1` 套接字可连否则在起合成器之前拒绝；起 `inox-compositor --socket wayland-inox-greet` 并等它把 Wayland 套接字绑定出来（路径必须真是 socket，上限 10 s）；起 `inox-greet`（注入 `WAYLAND_DISPLAY`、`INOX_AUTH_SOCKET`，stdin 为 null）；有界预算地监督登录界面、把合成器退出当作整阶段退出、收到 `SIGTERM`/`SIGINT` 时逆序停子进程。

它**不**拥有认证、凭据、用户会话与 VT/设备访问：那分别在 `inox-auth`（ADR 0029 / ADR 0037 §决策 5）与 logind + libseat（ADR 0037 §决策 1/§决策 2）。宿主甚至不 `use` `auth-v1` 的类型——它对边界的全部认知是「`connect` 成功没有」。

## crate 特有约束

- 默认禁止 `unsafe`：继承根 `Cargo.toml` 的 `[workspace.lints]`（`unsafe_code = "forbid"`）；本 crate 无例外，也不申请 ADR 0015 的特权接口。
- 非测试路径禁止 `unwrap()` / `expect()` / `panic!()` / `todo!()` / `unimplemented!()`。唯一的「永不返回」写法是 `main` 里对 `process::exec_child` 的 `match never {}`（`Infallible`），它不是 `panic!` 的变体。
- 日志统一使用 `tracing`，禁止 `println!` / `eprintln!`。
- 与工作区内其他 Inox crate 的依赖使用 **path 依赖**（根级 §8）。**本 crate 只依赖 L1 的 `inox-common`**：不得依赖 `inox-auth`、`inox-compositor`、`inox-greet` 中任何一个——它按**路径**把它们当程序启动，这是 L4 同层不横向依赖的具体形式（ADR 0037 §兼容、迁移与回退）。
- **规则与 I/O 必须分开。** `src/supervisor.rs` 是纯状态机：不读时钟（`Instant` 由调用方给）、不做 I/O、不打日志、不认识 Fluent；`src/process.rs` 只把 `Action` 变成进程操作；`src/runtime.rs` 只观察与执行；`src/main.rs` 只做参数与装配。把一条判定写进 `runtime.rs` 会让它无法被单测覆盖，这正是本 crate 的验收（ADR 0037 §验收 第 1 行）最依赖的东西。
- **边界必须比合成器更早。** 「能不能连上认证边界」是第一条判定：不可连就在任何进程存在之前以非零码退出（ADR 0037 §决策 3.1）。不得把这条检查推迟到合成器之后、不得在该状态下开任何窗口、也不得让它退化成一个可以跳过的开关。
- **子进程必须经本二进制的 `--exec-child` 启动。** calloop 的信号源是 `signalfd`，会屏蔽本线程的 `SIGINT`/`SIGTERM`，而 fork 出来的子进程继承该屏蔽字；跳过这一层等于让宿主发出的 `SIGTERM` 永远送不到子进程。`--exec-child` **不是**用户选项（不出现在 `--help`），也**不得**承载任何能力：宿主以 greeter 用户运行，它启动的每个进程都是同一个用户。
- **等的是「是 socket」，不是「路径存在」。** `runtime` 的探测用 `symlink_metadata(...).file_type().is_socket()`：同名的普通文件、目录、写了一半的产物都不算合成器就绪，否则宿主会在合成器还接不了客户端时就起登录界面。残留的**真 socket**（上一个合成器留下的）仍会通过探测——区分它与活着的监听要建连，而宿主故意不建；这一情形由「合成器是本宿主的活子进程」与「登录界面的重启预算」兜底，不要为了它把探测改成 connect。
- **每个子进程一个进程组**（`process_group(0)`），信号按组发；登录界面的 stdin 必须是 null，且必须移除继承来的 `WAYLAND_SOCKET`（它指向的是外面的合成器，不是本阶段的）。
- **重启预算只在登录界面上。** 60 秒 / 3 次、有界退避（ADR 0015）；合成器**永不**重启，它的退出是本阶段的结束（ADR 0037 §决策 3.4）。登录界面即使是**成功**退出也要重启：§决策 4 让它会话启动后驻留，所以任何退出都意味着画面没了。
- **用户可见文案只来自 Fluent**，`en-US` 与 `zh-CN` 同步，id 统一 `host-` 前缀；`tests/catalog.rs` 双向检查「代码提到的 id 都在目录里」与「目录里的 id 都被用到」。拒绝文案必须同时含 `authentication boundary` 与 `inox-auth`（`tests/cli.rs` 依赖这两个事实），并带上试过的套接字路径。
- **退出码是契约。** `Exit`（`src/supervisor.rs`）是唯一来源，`main` 只做转换；`0` 只在收到 `SIGTERM`/`SIGINT` 且子进程都停下时出现，其余每个失败各有专属非零码（见 README 的表）。新增失败路径必须同时给它一个码、一条 Fluent 文案与一条单测。

## 主要依赖

`anyhow`、`tracing`、`tracing-subscriber`；`calloop`（事件循环与信号源）、`rustix`（等待与向进程组发信号）、`inox-common`（日志与 i18n）。测试：`tempfile`。

**刻意不引入**：`inox-auth`（不解析 `auth-v1`）、`inox-config`（宿主不读主题/配置，locale 来自环境）、`thiserror`（可执行程序用 `anyhow`）。

## 命令

> 仅作参考，不作为本地验收依据；真正判定在 CI。

本 crate 是工作区成员，命令应尽量用 `-p` 限定范围（CI 的测试矩阵跑的就是同一批命令）。在 `crates/<name>/` 目录内或仓库根执行均可。

| 目的 | 命令 |
|------|------|
| 本 crate 测试 | `cargo test -p inox-host --all-features` |
| 本 crate 构建 | `cargo build -p inox-host --release --all-features` |
| 本 crate 文档 | `cargo doc -p inox-host --no-deps --all-features` |
| 全工作区测试 | `cargo test --workspace --all-features` |
| 静态检查 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 格式化 | `cargo fmt -p inox-host`（`--all` 会动到别的 crate，本 crate 的改动请锁范围） |
| 分层方向检查 | `cargo test -p inox-integration --test layering` |
| 依赖检查 | `cargo deny check` |

## 测试要求

- `src/supervisor.rs` 的纯逻辑单测**必须覆盖全部拒绝路径**：边界不可连（不得产生 `Start`）、合成器在套接字绑定出来之前退出、合成器 10 s 超时（含 `SIGKILL` 升级）、登录界面连续崩溃超预算、登录界面起不来也消耗预算、合成器起不来直接结束、`SIGTERM` 逆序停止与重复信号幂等、结束后的事件全部被忽略。时间规则用注入的 `Instant` 验证，**不得**靠 `sleep`。
- `tests/cli.rs` 覆盖 `--help`/`-V`（成功，且不需要边界）、未知参数、缺值、空值、缺边界时退出码 `3` 与文案，以及「拒绝时没有启动合成器」与「缺 `$XDG_RUNTIME_DIR` 明确拒绝」。
- `tests/supervisor.rs` 必须用**真实子进程**驱动 shell 脚本 fixture（假合成器/假登录界面），**不得** mock 宿主自己的规则；就绪态必须绑一个**真** `UnixListener`（普通文件不算就绪，那条也必须有测试）；断言注入的环境、`--compositor-arg` 透传、逆序停止的事件顺序、重启次数与各拒绝路径的退出码，并断言**没有留下任何进程**（宿主负责 reap 与按组 `SIGKILL`）。
- `tests/catalog.rs` 双向检查 id 与占位符，因此新增/删除文案必须两边一起改。
- 测试代码不受「禁止 unwrap/expect/panic」约束。
- **真机登录（E4）不在本 crate 的测试范围**：裸 VT、libseat/DRM、`PAMName`/`TTYPath` 与 VT 切换都要人按 ADR 0037 §验收 验证；本 crate 的测试用假程序，不证明真机能登录，也不得被读作已证明。

## 目录结构

```
crates/inox-host/
  Cargo.toml    工作区继承：字段一律 *.workspace = true
  src/          源码（supervisor / process / runtime / cli / i18n / main）
  i18n/         en-US / zh-CN 的 Fluent 目录
  tests/        catalog.rs、cli.rs、supervisor.rs
  README.md     本 crate 说明
  AGENTS.md     本文件（CLAUDE.md 是其符号链接）
```

工作区级文件（`Cargo.toml`、`Cargo.lock`、`deny.toml`、`rust-toolchain.toml`、`LICENSE`、CI 工作流）在仓库根目录，**各 crate 不再各自保留一份**。

## 提交信息格式

Conventional Commits，scope 使用 `host`。
