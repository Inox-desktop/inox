# Inox 设置 — zh-CN。
#
# 本文件必须与 `en-US/settings.ftl` 定义完全相同的消息 id 与占位符，
# `tests/catalog.rs` 用 `inox_common::i18n::compare_catalogs` 逐条断言（验收 Q03）。

settings-window-title = Inox 设置

settings-page-appearance = 外观
settings-page-panel = 面板
settings-page-notifications = 通知
settings-page-locale = 语言

settings-field-theme-appearance = 主题
settings-field-locale-language = 语言
settings-field-panel-position = 位置
settings-field-panel-height = 高度
settings-field-panel-autohide = 不使用时隐藏
settings-field-panel-show-clock = 时钟
settings-field-notifications-position = 弹出位置
settings-field-notifications-do-not-disturb = 勿扰模式
settings-field-notifications-low-timeout = 低优先级
settings-field-notifications-normal-timeout = 普通优先级
settings-field-notifications-critical-timeout = 紧急优先级

settings-choice-theme-light = 浅色
settings-choice-theme-dark = 深色
settings-choice-theme-system = 跟随会话
settings-choice-locale-environment = 跟随环境
settings-choice-locale-en-us = 英语（美国）
settings-choice-locale-zh-cn = 中文（简体）
settings-choice-panel-top = 顶部
settings-choice-panel-bottom = 底部
settings-choice-notifications-top-left = 左上
settings-choice-notifications-top-right = 右上
settings-choice-notifications-bottom-left = 左下
settings-choice-notifications-bottom-right = 右下

settings-toggle-on = 开
settings-toggle-off = 关
settings-number-with-unit = { $value } { $unit }
settings-number-decrease = −
settings-number-increase = +
settings-unit-px = 像素
settings-unit-ms = 毫秒

settings-action-apply = 应用
settings-action-cancel = 取消
settings-action-reset = 恢复默认
settings-action-reset-all = 全部恢复默认
settings-action-reload = 重新读取文件

settings-disabled-in-flight = 正在保存
settings-disabled-invalid = 请先修正标出的字段
settings-disabled-no-changes = 尚无修改

settings-status-unchanged = 文件未改变
settings-status-modified = 已在此修改，尚未写入
settings-status-saving = 正在写入文件
settings-status-pending = 已保存；正在等待各组件确认
settings-status-applied = 已保存并生效
settings-status-saved-only = 已保存；无法请求{ $component }应用
settings-status-conflict = { $file } 在本窗口读取之后被改动
settings-status-save-failed = 无法写入 { $file }
settings-status-component-rejected = { $component } 拒绝了该改动
settings-status-reload-failed = 无法重新读取文件
settings-status-unreachable = 无法请求{ $component }应用该改动

settings-detail-component-state = { $component }：{ $state }
settings-detail-conflict-digest = 文件当前内容为 { $current }
settings-detail-save-failed = { $file }：{ $reason }
settings-detail-fields = 拒绝的配置项：{ $fields }
settings-component-state-pending = 尚未确认
settings-component-state-applied = 已生效
settings-component-state-rejected = 已拒绝：{ $reason }
settings-component-state-unreachable = 无法请求：{ $reason }

settings-component-compositor = 合成器
settings-component-shell = Shell
settings-component-notifications = 通知守护进程

settings-error-field = { $path }：{ $message }
settings-error-height-range = 应为 { $min } 到 { $max } 逻辑像素，当前为 { $value }
settings-error-timeout-range = 应为 { $min } 到 { $max } 毫秒，当前为 { $value }
settings-error-locale = “{ $value }” 不是会话可用的语言标签
