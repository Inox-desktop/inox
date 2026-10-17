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

settings-field-panel-margin = 边缘间距
settings-field-panel-exclusive = 预留面板空间
settings-field-panel-clock-format = 时钟格式
settings-field-panel-max-task-buttons = 任务按钮上限
settings-field-panel-icon-size = 图标大小
settings-field-launcher-max-results = 结果数量上限
settings-field-launcher-icon-size = 结果图标大小
settings-field-launcher-include-hidden = 显示隐藏应用
settings-field-launcher-max-query-chars = 搜索字数上限
settings-field-wallpaper-image = 图片路径（留空使用主题背景）
settings-field-wallpaper-mode = 图片布局
settings-field-keyboard-layout = 键盘布局
settings-field-keyboard-variant = 键盘变体
settings-field-keyboard-repeat-delay = 按键重复延迟
settings-field-keyboard-repeat-rate = 按键重复速率（0 为禁用）
settings-field-tap-to-click = 轻触点击
settings-field-natural-scroll = 自然滚动
settings-field-layout-master-ratio = 主区域宽度
settings-field-layout-master-count = 主区域窗口数
settings-field-layout-gap = 窗口间距
settings-field-notifications-max-visible = 同时显示数量
settings-field-notifications-width = 弹窗宽度
settings-field-notifications-margin = 弹窗边距
settings-field-notifications-icon-size = 通知图标大小
settings-field-notifications-max-pending = 等待通知上限
settings-field-notifications-queue-policy = 队列满时
settings-field-notifications-show-actions = 显示通知操作
settings-field-notifications-hide-body-when-locked = 锁屏时隐藏消息正文
settings-page-input = 键盘与鼠标
settings-page-layout = 窗口布局
settings-page-launcher = 启动器
settings-page-wallpaper = 壁纸
settings-choice-fill = 填充
settings-choice-fit = 适应
settings-choice-stretch = 拉伸
settings-choice-center = 居中
settings-choice-tile = 平铺
settings-choice-drop-oldest = 丢弃最早通知
settings-choice-coalesce-by-app = 按应用合并
settings-unit-count = 项
settings-unit-hz = Hz
settings-unit-percent = %
settings-error-number-range = 请输入 { $min } 至 { $max } 之间的值，当前为 { $value }。
settings-error-clock-format = 请使用面板支持的时钟格式，例如 %H:%M。
settings-error-integer = 请输入整数。

settings-field-outputs = 显示器偏好
settings-field-launcherterminal = 终端程序与参数
settings-field-wallpaperperoutput = 各显示器壁纸
settings-page-displays = 显示器
settings-output-name = 接口名称（留空匹配任意显示器）
settings-output-width = 宽度（像素）
settings-output-height = 高度（像素）
settings-output-refresh = 刷新率（Hz）
settings-output-scale = 缩放（0.25–8）
settings-output-x = 水平位置（留空自动排列）
settings-output-y = 垂直位置（留空自动排列）
settings-terminal-argument = 可执行程序或一个参数
settings-wallpaper-path = 图片路径
settings-list-add = 添加条目
settings-list-remove = 移除条目
settings-outputs-help = 保存的偏好将在下次会话启动时读取，此处不会实时切换显示模式。空列表使用后端默认值；两个位置均留空时自动排列。
settings-terminal-help = 第一项填写可执行程序，之后每项填写一个参数。参数直接传递，不经 Shell 解释。空列表使用启动器默认值。
settings-wallpapers-help = 每项填写显示接口名称和图片路径，* 表示其他显示器。空列表使用主壁纸。
settings-error-collection = 请检查第 { $row } 项：尺寸 1–16384，刷新率 1–1000，缩放 0.25–8，位置在 ±4194304 内且同时填写或同时留空。壁纸覆盖项名称须唯一，路径不能为空。

settings-page-input-method = 输入法
settings-field-input-method-command = 引擎程序与参数
settings-input-method-help = 第一项填写 fcitx5 或兼容的可执行程序，之后每项填写一个参数。空列表禁用自动启动。改动在下次物理会话生效，命令行选项优先。请单独安装引擎，并使用引擎配置工具管理语言、词库和切换快捷键；不要添加后台运行选项。
