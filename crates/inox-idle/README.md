# inox-idle

L4 空闲监视器，使用 `ext-idle-notify-v1` 的 inhibitor-respecting 通知和 calloop 事件循环。默认阈值 300 秒，`inox-idle --timeout-seconds N` 可显式修改。只选择第一个 seat；合成器缺少 notifier/seat 或断开连接时显式失败，不轮询进程推测活动。

纯策略实现抑制、手动锁定、锁确认代次与超时、锁后允许挂起。**可信锁屏和 logind 通道尚未获批/实现**，运行中的监视器只报告锁定不可用，不执行锁屏或挂起命令。模型里的确认只能由将来的可信 transport 提供，不能把进程退出码当作确认。当前 Inox 合成器尚未提供 idle notifier，作为可选服务启动后会按预算失败退化。

设计与审查前提见、。CI 覆盖策略、缺能力拒绝以及真实 Wayland 线协议的 idled/resumed 往返；不代表 S03 的真实挂起/恢复通过。本地仅 fmt/check/clippy，测试与验收交给 CI。

许可证 LGPL-3.0，见根 LICENSE。
