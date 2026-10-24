# Inox greeter 宿主 — zh-CN。
#
# 本阶段会对人说的每一句话都在这里；代码里只有消息 id（根 AGENTS.md §7.3、PRD
# §11.2）。宿主是一个 systemd 单元，所以这些句子出现在 journal 里——读它的人是那个
# 在问「我的机器为什么没到登录画面」的人。`en-US` 必须定义完全相同的 id 与占位符，
# 由 tests/catalog.rs 双向断言（验收 Q03）。

# 用法行。命令行的语法也是给人读的，所以它同样是一条消息，而不是解析器里的一句英文。
host-usage = inox-host [--bin-dir DIR] [--socket NAME] [--auth-socket PATH] [--compositor-arg ARG]…

# 版本行。`inox-host` 是专有名词，不翻译。
host-version = inox-host { $version }

# ADR 0037 §决策 3.1：边界在合成器存在之前就被检查，拒绝必须同时说清缺的是什么与
# 由哪个进程提供——tests/cli.rs 依赖这两件事出现在同一句里。
host-boundary-missing = { $socket } 上的系统认证边界（inox-auth）不可用，因此 greeter 宿主没有启动合成器。

# 本阶段为什么结束。每条都点名失败的东西，并说明宿主正在停止——这是运维人员看到
# 单元重启时唯一的问题。
host-compositor-exited = 合成器已退出，greeter 宿主正在停止。登录界面的客户端已经没有可用的显示。
host-socket-timeout = 合成器没有在 { $seconds } 秒内创建 { $socket }，greeter 宿主正在停止。
host-compositor-unstartable = 无法启动 { $program } 处的合成器，greeter 宿主正在停止。
host-greeter-unstartable = 无法启动 { $program } 处的登录界面，greeter 宿主正在停止。
host-restarts-exhausted = 登录界面在 { $window } 秒内退出了 { $count } 次，greeter 宿主正在停止。systemd 会记录这次失败的启动并重试。

# 宿主对「有人要求它停止」唯一要说的话。这不是错误：子进程按逆序停止，单元以成功状态退出。
host-stopping = 有人要求 greeter 宿主停止；它正在先停登录界面，再停合成器。
