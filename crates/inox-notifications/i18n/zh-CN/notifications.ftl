# Inox 通知守护进程 — zh-CN。
#
# 与 en-US 目录保持完全相同的 id 与占位符：`tests/catalog.rs` 用
# `inox_common::i18n::compare_catalogs` 断言（验收 Q03）。应用名、通知标题、
# 正文与动作标签是随通知到达的**数据**，不在这里翻译。

# 守护进程自身窗口的标题（ADR 0018：目前是普通窗口，而非 layer 表面）。
notifications-window-title = Inox 通知

# 守护进程自身的状态：弹窗位置为空时说明原因，而不是留一块空白。
notifications-empty = 没有通知
notifications-suppressed = 勿扰模式已开启，通知已静音

# 锁定会话时替代摘要、应用名与图标（ADR 0015，验收 D06）：敏感内容根本不存在于视图模型里。
notifications-generic-summary = 收到一条通知

# 单条通知的关闭按钮。
notifications-close = 关闭

# 动作按钮不可用的原因：`policy.show_actions` 已关闭（D05）。
notifications-action-disabled-policy = 动作已在 notifications.toml 中关闭

# 持有的通知多于弹窗能显示的数量（D05，验收 D06）。
notifications-overflow = 还有 { $count } 条在等待

# 本守护进程不解码正文图片，改用 alt 文本；图片没有 alt 时显示这一条。
notifications-image-placeholder = 图片
