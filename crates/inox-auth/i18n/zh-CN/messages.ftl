# Inox 认证边界 —— zh-CN。
#
# 与 en-US 保持同一组 id 与占位符（根 AGENTS.md §7.3、PRD §11.2）；
# `tests/catalog.rs` 用 `inox_common::i18n::compare_catalogs` 断言一致性。
# 这里只有 helper 面向人的两类文本：用法行与启动前的拒绝；
# 运行期的一切都走 tracing。

auth-usage = inox-auth [--socket PATH] [--service NAME] [--session-service NAME] [--session-program PATH] [--session-vt-min N] [--session-vt-max N] [--seat NAME] [--allow-uid UID] [--socket-group GID] [--minimum-uid UID] [--attempts N] [--window-seconds N] [--exit-after-session]

# 启动前的拒绝。
auth-requires-root = 系统套接字需要以 root 运行；用 --socket 可以运行非特权实例
auth-session-program = 会话程序不可执行：{ $path }
auth-argument = { $value } 不是 { $option } 可接受的值
auth-vt-range = 会话 VT 范围没有可用的 VT：最小值是 { $min }，最大值是 { $max }

# 内部会话负责人模式的拒绝。
auth-leader-root = 会话负责人必须以 root 运行；只由 inox-auth 为单次登录启动，其他任何方式都不允许
auth-leader-vt = 会话 VT { $vt } 不在本边界允许的范围内（{ $range }）
auth-leader-account = { $user } 不是本边界会为其启动会话的账号
auth-leader-arguments = --session-leader 需要同时给出 --user 与 --vt
auth-leader-only = --user 与 --vt 只能和 --session-leader 一起使用
