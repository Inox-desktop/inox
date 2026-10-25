# inox-xwayland

XWayland 集成服务。属于 Inox 桌面环境的 **L2** 层。

## 状态

**M1**：以「监管外部进程」的方式把 rootless XWayland 接入 Inox 会话。

`xwayland-satellite`（MPL-2.0、应用形态、不在 crates.io 上）被视为**运行期系统依赖**，与 `Xwayland` 本身同级：
本 crate **不链接**它，只负责拉起、转发生命周期与退出清理，因此依赖图中不新增任何编译期或系统级耦合。

**就绪判定 = 真实 X11 连接建立往返**（connection setup：请求 + setup 应答），**不是** socket 存在。这是验收项 X01 的硬性要求
（[`docs/quality/acceptance.md`](../../docs/quality/acceptance.md) §5：「就绪需实际 X11 往返，socket 存在不足以通过」）；
握手仅用 `std` 实现，不链接任何 X11 库。

## M1 行为

1. **拉起**：以 `Command` 启动 `xwayland-satellite`，命令行第一个参数是显示号（如 `:1`），其后是 `--satellite-arg` 透传的额外参数；子进程继承当前环境，`WAYLAND_DISPLAY` 因此可达。
2. **就绪探测**：每 100 ms 对 `<x11-socket-dir>/X<N>` 做一次**真实 X11 连接建立往返**——发送 12 字节连接请求（不含授权数据），再解析服务端应答：只有 status 为 1 且应答结构合法（含可读出的厂商字符串）才算就绪。**socket 存在或可连接不足以通过**（验收项 X01 明确要求实际 X11 往返）。就绪后以 `info` 记录厂商字符串（如 `The X.Org Foundation`），运行时另记录 `XWayland ready on :N`，并在配置了 `--ready-file` 时写入 `:N\n`。
3. **监管**：轮询子进程存活；子进程意外退出时以其退出码作为服务退出码。
4. **退出**：收到 `SIGINT` / `SIGTERM` 时先向子进程发送 `SIGTERM`，等待 `--shutdown-grace` 秒后升级为 `SIGKILL`，随后以状态 `0` 退出。
5. **超时**：`--ready-timeout` 秒内 XWayland 仍未就绪则以非零状态退出。

就绪探测的失败面全部视为「未就绪」并按下一次轮询重试，从不 panic：socket 不存在 / 连接被拒、应答超时（每次往返总时长受上限约束）、连接被对端关闭、应答被截断、status 0（拒绝，附原因）、status 2（要求授权）、status 非法、协议主版本不是 11、厂商字符串越界、应答之后还有多余数据。

M1 **不包含**：剪贴板 / 拖放、合成器 `-listenfd` 按需激活、崩溃重启监管、DRM/KMS、输入、X11 授权（连接请求不带授权数据，要求授权的显示一律按未就绪处理）。

## 命令行

```
--display <:N>          [默认: :1]
--satellite <PATH>      [默认: $INOX_XWAYLAND_SATELLITE 或 "xwayland-satellite"]
--satellite-arg <ARG>   可重复，按顺序透传
--x11-socket-dir <DIR>  [默认: /tmp/.X11-unix]
--ready-timeout <SECS>  [默认: 10]（>= 1）
--shutdown-grace <SECS> [默认: 5]（>= 1）
--ready-file <PATH>     可选；就绪后写入 DISPLAY 名
-h | --help             打印帮助并以 0 退出
-V | --version          打印 `inox-xwayland <VERSION>` 并以 0 退出
```

退出码：干净关闭 `0`；子进程自行退出时沿用其退出码（被信号杀死时为 `1`）；其他错误为 `1`。

## 运行

需要一个正在运行的 Wayland 合成器（`WAYLAND_DISPLAY` 指向它）以及 `xwayland-satellite` 可执行文件：

```sh
WAYLAND_DISPLAY=wayland-1 inox-xwayland --display :1 --ready-file /tmp/inox-xwayland.ready
```

仅探测逻辑可以脱离真实 XWayland 验证：把 `--satellite` 指向任意长驻命令，并在 `--x11-socket-dir` 下自行创建 `X<N>` 监听套接字；该套接字必须**应答连接建立请求**（回一个 status 1、结构合法的 setup 应答），只监听不应答会被判为未就绪。

## 开发约定

- 本项目遵循[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径**，本地命令仅作快速自检。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)。
- MSRV：Rust **1.86**（edition 2024）。

## 测试分层

| 层级 | 位置 | 说明 |
|------|------|------|
| 单元测试 | `src/**` 内 `#[cfg(test)]` | CLI 解析、显示号解析、X11 连接建立往返（成功 / 拒绝 / 要求授权 / 截断 / 垃圾 / 尾随数据 / 静默对端超时）、`satellite_args`、以注入的假实现驱动的服务状态机 |
| 进程级生命周期 | `tests/lifecycle.rs` | 用 shell 替身与「会应答 / 不应答连接建立请求」的 X11 替身驱动真实二进制：就绪文件、`SIGTERM` 清理退出、就绪超时、子进程提前退出 |
| headless 端到端 | `tests/headless_x11_mapping.rs` | 需设 `INOX_XWAYLAND_E2E=1`；在 weston headless 中映射一个真实 X11 应用 |

## 许可证

LGPL-3.0，见仓库根的 [`LICENSE`](../../LICENSE)。
