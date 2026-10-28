# Inox 面板 — zh-CN。
#
# 面板可能显示在屏幕上的每一段文案都在这里，代码只持有消息 id
# （根级 AGENTS.md §7.3、PRD §11.2）。`zh-CN` 必须定义完全相同的 id 与占位符，
# `tests/catalog.rs` 用 `inox_common::i18n::compare_catalogs` 断言（验收 Q03）。
#
# 星期与月份的编号与 `chrono` 一致：1 = 周一 … 7 = 周日，1 = 一月 … 12 = 十二月。

# 面板自身表面的标题：面板画的每一种表面都是 wlr-layer-shell 表面。
panel-window-title = Inox 面板

# 面板级状态（验收 D05）。
panel-state-starting = 正在等待窗口服务
panel-state-empty = 没有打开的窗口
panel-service-unavailable = 窗口服务不可用
panel-service-retry = 重试

# 任务按钮（验收 D03）。
panel-task-unknown-app = 未知应用
panel-task-disabled-service = 窗口服务不可用
panel-task-tooltip = { $app }：{ $title }
panel-task-tooltip-empty = { $app }
panel-task-title-with-count = { $title }（{ $count }）

# 坞栏图块：面板没有图标主题解码器，因此每块是一枚写着应用名首字的徽标；
# 没有名字的应用用这条占位文案。它会被画在方形图块里，所以必须很短。
panel-dock-icon-placeholder = ？

# 托盘项（验收 D03）。
panel-tray-unknown-item = 状态项
panel-tray-tooltip = { $title }（{ $status }）
panel-tray-status-passive = 空闲
panel-tray-status-active = 活动中
panel-tray-status-attention = 需要关注

# 时钟（验收 Q02、Q03）。
panel-clock-tooltip = { $year } 年 { $month } { $day } 日 { $weekday }
panel-clock-am = 上午
panel-clock-pm = 下午
panel-clock-zone-utc = UTC
panel-clock-zone-offset = UTC{ $sign }{ $hours }:{ $minutes }

panel-clock-weekday-abbr-1 = 周一
panel-clock-weekday-abbr-2 = 周二
panel-clock-weekday-abbr-3 = 周三
panel-clock-weekday-abbr-4 = 周四
panel-clock-weekday-abbr-5 = 周五
panel-clock-weekday-abbr-6 = 周六
panel-clock-weekday-abbr-7 = 周日
panel-clock-weekday-full-1 = 星期一
panel-clock-weekday-full-2 = 星期二
panel-clock-weekday-full-3 = 星期三
panel-clock-weekday-full-4 = 星期四
panel-clock-weekday-full-5 = 星期五
panel-clock-weekday-full-6 = 星期六
panel-clock-weekday-full-7 = 星期日
panel-clock-month-abbr-1 = 1 月
panel-clock-month-abbr-2 = 2 月
panel-clock-month-abbr-3 = 3 月
panel-clock-month-abbr-4 = 4 月
panel-clock-month-abbr-5 = 5 月
panel-clock-month-abbr-6 = 6 月
panel-clock-month-abbr-7 = 7 月
panel-clock-month-abbr-8 = 8 月
panel-clock-month-abbr-9 = 9 月
panel-clock-month-abbr-10 = 10 月
panel-clock-month-abbr-11 = 11 月
panel-clock-month-abbr-12 = 12 月
panel-clock-month-full-1 = 1 月
panel-clock-month-full-2 = 2 月
panel-clock-month-full-3 = 3 月
panel-clock-month-full-4 = 4 月
panel-clock-month-full-5 = 5 月
panel-clock-month-full-6 = 6 月
panel-clock-month-full-7 = 7 月
panel-clock-month-full-8 = 8 月
panel-clock-month-full-9 = 9 月
panel-clock-month-full-10 = 10 月
panel-clock-month-full-11 = 11 月
panel-clock-month-full-12 = 12 月

# 任务区左侧的工作区指示器（ADR 0045 §4）。`active` 与 `next` 从 1 开始计数，
# 与用户数工作区的方式一致；`count` 是工作区总数。`next` 是点击后会显示的
# 工作区，不是已经显示过的。
panel-workspace-indicator = { $active }/{ $count }
panel-workspace-tooltip = 第 { $active } 个工作区，共 { $count } 个；点击切换到第 { $next } 个
panel-workspace-disabled-service = 窗口服务不可用

# 窗口模式（ADR 0045 §2）。它们命名合成器的四种状态；在任务项上右键会按
# 这个顺序循环切换。
panel-task-mode-tiled = 平铺
panel-task-mode-floating = 浮动
panel-task-mode-maximized = 最大化
panel-task-mode-fullscreen = 全屏
panel-task-tooltip-mode = { $app }：{ $title }（{ $mode }）
panel-task-tooltip-mode-empty = { $app }（{ $mode }）
