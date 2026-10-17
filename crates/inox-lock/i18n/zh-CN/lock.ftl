# Inox 锁屏 —— zh-CN。
#
# 与 en-US 保持同一组 id 与占位符（根 AGENTS.md §7.3、PRD §11.2）；
# `tests/catalog.rs` 用 `inox_common::i18n::compare_catalogs` 断言一致性（验收 Q03）。

# 表面与组件名。
lock-window-title = Inox 锁屏

# 提示本身。
lock-prompt = 请输入密码
lock-field-label = 密码
lock-field-placeholder = 输入密码
lock-hint = 按回车解锁
lock-submit = 解锁

# 提示下方的状态。刻意不含次数、剩余尝试与剩余时间：限流信息自己泄露内部计时，
# 等于告诉攻击者还剩多少预算（ADR 0015）。
lock-verifying = 正在验证密码…
lock-attempts-waiting = 尝试次数过多，请稍候再试。
lock-failed = 密码未被接受

# 认证边界给出的失败原因。
lock-failure-rejected = 认证服务拒绝了该密码
lock-failure-timed-out = 认证服务未在时限内应答
lock-failure-unavailable = 认证服务不可达
lock-failure-cancelled = 该次尝试在得到答复前已被放弃

# 无法输入时的可读原因（验收 D05）。
lock-disabled-boundary = 认证服务不可用
lock-disabled-waiting = 当前尝试次数过多
lock-disabled-verifying = 已有一次尝试正在验证
lock-disabled-unlocked = 会话未处于锁定状态
lock-disabled-protocol = 会话锁定协议不可用
lock-disabled-no-ui = 锁屏界面未在运行

# 已锁定但无可信界面：会话保持锁定，直到界面重启（ADR 0015）。
lock-held-title = 会话已锁定
lock-held-detail = 锁屏界面正在重启，会话保持锁定。

# 无法准备好的挂起（ADR 0015：先锁定，再挂起）。
lock-suspend-refused-lock-failed = 会话未挂起：无法确认锁定已完成
lock-suspend-refused-timeout = 会话未挂起：锁定未在时限内确认
lock-suspend-refused-no-protocol = 会话未挂起：该合成器无法锁定会话

# 锁屏完全无法运行。
lock-unavailable = 会话锁定无法运行
lock-unavailable-retry = 重试
lock-protocol-missing-global = 合成器未提供会话锁定协议（ext-session-lock-v1）
lock-protocol-registry-unavailable = 无法读取合成器注册表
lock-boundary-disconnected = 认证服务不可达
lock-boundary-timeout = 认证服务未在时限内应答

# 二进制在非零退出前报告的内容（ADR 0015、ADR 0018）。
lock-start-blocked = 本次构建无法启动会话锁定：{ $reason }
lock-start-blocked-reason = 合成器未提供 ext-session-lock-v1，且 ADR 0015 要求先对认证边界完成人类安全审查，本界面在此之前不得认证任何用户
