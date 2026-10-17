# Inox 登录组件 —— zh-CN。
#
# 与 en-US 保持同一组 id 与占位符（根 AGENTS.md §7.3、PRD §11.2）；
# `tests/catalog.rs` 用 `inox_common::i18n::compare_catalogs` 断言一致性（验收 Q03）。

greet-usage = inox-greet --preview

# 窗口与组件名。
greet-window-title = Inox 登录

# 提示本身。
greet-headline = 登录 Inox
greet-field-label = 用户名
greet-placeholder = 请输入用户名
greet-hint = 按回车登录；按 Esc 清空用户名
# 无法使用登录控件时键盘还剩下什么：承诺一次登录等于提示一个按了没反应的键。
greet-hint-clear = 按 Esc 清空用户名

# 控件。
greet-submit = 登录
greet-clear = 清空

# 登录将启动的会话。名称是专有名词，各语言一致；它作为目录条目而不是字面量，
# 是为了让「面向用户的文案不在目录之外」这条规则没有需要记住的例外。
greet-session-label = 会话
greet-session-name = Inox

# 本次构建是什么，先说清楚。
greet-banner = 仅供预览。不收集凭据，不启动会话。
greet-blocked = 系统认证边界通过安全审查前，登录功能不可用。

# 表单下方的状态。刻意不含次数、剩余尝试与剩余时间：限流信息自己泄露内部计时，
# 等于告诉猜它的人还剩多少预算（ADR 0015，与锁屏限流器同一条规则）。
greet-unavailable = 无法登录
greet-checking = 正在验证账户…
greet-starting = 正在启动会话…
greet-started = 会话正在启动
greet-failed = 该次登录未被接受

# 用户名本身的问题。
greet-invalid-user = 请输入有效的用户名。
greet-identity-too-long = 用户名过长。

# 为什么不能输入、为什么不能提交，显示在对应控件旁边（验收 D05）。
greet-disabled-unavailable = 系统认证服务不可用
greet-disabled-verifying = 已有一次登录正在验证
greet-disabled-started = 会话已在启动中
greet-disabled-invalid = 请输入有效的用户名
greet-disabled-rate-limited = 当前尝试次数过多
