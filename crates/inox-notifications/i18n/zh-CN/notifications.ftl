# Inox 通知守护进程 — zh-CN。
#
# 与 en-US 目录保持完全相同的 id 与占位符：`src/headless_tests.rs` 用
# `inox_common::i18n::compare_catalogs` 断言（验收 Q03）。应用名、通知标题、
# 正文与动作标签是随通知到达的**数据**，不在这里翻译。

# 守护进程自身窗口的标题。
notifications-window-title = Inox 通知

# 守护进程自身的状态：弹窗位置为空时说明原因，而不是留一块空白。
notifications-empty = 没有通知
notifications-suppressed = 勿扰模式已开启，通知已静音

# 锁定会话时替代摘要、应用名与图标：敏感内容根本不存在于视图模型里。
notifications-generic-summary = 收到一条通知

# 单条通知的关闭按钮。
notifications-close = 关闭

# 动作按钮不可用的原因：`policy.show_actions` 已关闭（D05）。
notifications-action-disabled-policy = 动作已在 notifications.toml 中关闭

# 持有的通知多于弹窗能显示的数量（D05，验收 D06）。
notifications-overflow = 还有 { $count } 条在等待

# 通知自称的紧急程度（非普通情形才显示）。
#
# ui-guidelines §5/§6：紧急度 = 一种颜色 + 一句话。句子是分不清颜色的用户读到的内容，
# 也是紧急通知不只「变红」而已的原因。
notifications-urgency-low = 低优先级
notifications-urgency-critical = 紧急

# 通知到达至今的时间（ui-guidelines §3：卡片状态行用 caption）。刻意粗糙：只说到
# 「刚刚」而不数秒，这样相隔片刻绘制的两帧说的是同一句话。
notifications-age-just-now = 刚刚
notifications-age-minutes = { $count } 分钟前
notifications-age-hours = { $count } 小时前
notifications-age-days = { $count } 天前

# 本守护进程不解码正文图片，改用 alt 文本；图片没有 alt 时显示这一条。
notifications-image-placeholder = 图片
