# inox-idle

L4 空闲监视器，使用 `ext-idle-notify-v1` 的 inhibitor-respecting 通知和 calloop 事件循环。默认阈值 300 秒，`inox-idle --timeout-seconds N` 可显式修改。只选择第一个 seat；合成器缺少 notifier/seat 或断开连接时显式失败，不轮询进程推测活动。

纯策略实现抑制、手动锁定、锁确认代次与超时、锁后允许挂起。**锁请求**已接上可信通道：`Idled` 经 `session-control-v1`（`inox_protocols::control`，`src/transport.rs`）请会话启动锁屏客户端，`ok`/`busy` 只算派发；**锁确认**只来自合成器在 `inox-ipc` 上广播的 `session_lock_state`（`Locked` 才算确认，`Unlocked` 才复位，未知值一律忽略）。若请求发出时合成器已报告锁定，就用这条观测确认——已锁时不会再有状态变化事件。`logind` 挂起传输仍未接线，因此「允许挂起」目前只记一条 warn。当前 Inox 合成器尚未实现 `ext-idle-notify`，作为可选服务启动后会按预算失败退化——这是合成器侧的缺口，不是本 crate 的。

设计与审查前提见、。CI 覆盖策略、缺能力拒绝以及真实 Wayland 线协议的 idled/resumed 往返；不代表 S03 的真实挂起/恢复通过。本地仅 fmt/check/clippy，测试与验收交给 CI。

许可证 LGPL-3.0，见根 LICENSE。
