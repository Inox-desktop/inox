# inox-session

L4 会话监督器：通过真实 Wayland 往返等待合成器，再启动独立桌面子进程；提供有界重启、失败隔离、进程组退出与私有运行时目录清理。设计与限制见下文。

安装同一 CI 组合的二进制后运行 `inox-session`；软件渲染会话可显式使用 `inox-session --headless`。`--bin-dir PATH` 选择二进制目录，`--exit-after-seconds N` 在指定时长后正常注销。`XDG_RUNTIME_DIR` 必须由当前用户拥有且禁止其他用户访问。`config.toml` 的环境配置传给子进程，会话拥有的显示变量优先。

Shell 是一次性检查；面板是必需服务；壁纸、通知与空闲监视为可选服务。每服务滚动 60 秒最多自动重启 3 次；合成器退出会结束会话，不承诺恢复应用连接。SIGINT/SIGTERM 注销；SIGHUP 只说明环境需下次登录才能替换，各组件仍拥有自己的重载机制。

会话为每个会话生成一次性锁屏令牌：传给合成器（`--lock-token`）并只放进可信锁屏客户端的环境（`INOX_LOCK_TOKEN`），其它组件一概拿不到——得到它就等于拿到取锁权限。`INOX_SESSION_TAKE_LOCK=1` 时会话额外启动 `inox-lock --take-lock`（该模式没有解锁路径，因此不做默认启动）。当前只确认合成器 ready，尚无完整桌面 ready 握手。L3 的 layer-shell、窗口服务与通知 D-Bus 等现有缺口不会因为 session 拉起进程而消失。安全锁定、认证、logind 挂起及用户自启动未接线；主动脱离进程组的子孙进程不在首版归属保证内。

测试由 CI 运行：纯重启策略、真实子进程/Wayland peer 故障、信号注销与目录清理，以及独立 job 中的真实 Inox headless 启动/退出。本地仅 fmt/check/clippy，不运行 build/test，不以自检代替验收。

许可证 LGPL-3.0，见根 LICENSE。

## 输入法启动

在现有会话入口增加 `--input-method /usr/bin/fcitx5`，额外参数逐项用
`--input-method-arg` 传入。会话将 argv 交给合成器，由合成器授予私有 Wayland
连接并监督引擎；不向其他组件传播该连接。安装与工具包接线见
[输入法使用说明](../../docs/book/src/usage/input-methods.md)。
