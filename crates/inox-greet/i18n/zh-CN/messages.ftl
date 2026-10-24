# Inox 登录组件 —— zh-CN。
#
# 与 en-US 保持同一组 id 与占位符（根 AGENTS.md §7.3、PRD §11.2）；
# `tests/catalog.rs` 用 `inox_common::i18n::compare_catalogs` 断言一致性（验收 Q03）。

greet-usage = inox-greet [--preview]

# 窗口与组件名。
greet-window-title = Inox 登录

# 口令字段与正在登录的账户。账户名不是文案：它是数据，来自系统的账户数据库。
greet-credential-label = 口令
greet-credential-placeholder = 请输入口令
greet-hint = 按回车登录；按 Esc 清空口令
# 无法使用登录控件时键盘还剩下什么：承诺一次登录等于提示一个按了没反应的键。
greet-hint-clear = 按 Esc 清空口令

# 控件。
greet-submit = 登录
greet-clear = 清空
greet-switch-user = 切换用户
greet-back = 返回

# 账户列表：它是什么，以及它对不在其中的人说什么。
greet-choose-account = 选择账户

# 登录将启动的会话。名称是专有名词，各语言一致；它作为目录条目而不是字面量，
# 是为了让「面向用户的文案不在目录之外」这条规则没有需要记住的例外。
greet-session-label = 会话
greet-session-name = Inox

# 本次构建是什么，先说清楚。
greet-banner = 仅供预览。不收集凭据，不启动会话。
greet-banner-live = 登录后将启动 Inox 会话。
greet-blocked = 系统认证边界不可用，无法校验登录。

# 表单下方的状态。刻意不含次数、剩余尝试与剩余时间：限流信息自己泄露内部计时，
# 等于告诉猜它的人还剩多少预算（ADR 0015，与锁屏限流器同一条规则）。
greet-unavailable = 无法登录
greet-checking = 正在验证账户…
greet-starting = 正在启动会话…
greet-started = 会话正在启动
greet-failed = 该次登录未被接受

# 屏幕拿到的东西本身有问题，而不是谁打错了：账户数据库不是本屏幕该信的东西。
greet-identity-too-long = 该账户名过长。
greet-credential-too-long = 口令过长。

# 为什么不能输入、为什么不能提交，显示在对应控件旁边（验收 D05）。
greet-disabled-unavailable = 系统认证服务不可用
greet-disabled-verifying = 已有一次登录正在验证
greet-disabled-started = 会话已在启动中
greet-disabled-invalid = 该账户名无法交给认证服务
greet-disabled-rate-limited = 当前尝试次数过多
greet-disabled-no-accounts = 本系统没有可登录的账户
greet-disabled-accounts-unreadable = 无法读取账户列表
greet-disabled-account-unselected = 请选择要登录的账户
