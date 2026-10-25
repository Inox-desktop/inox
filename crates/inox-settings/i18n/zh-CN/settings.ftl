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

# 显示设置的试用与确认。模式、缩放或位置会立即生效，因此窗口必须先问一句：
# 只有确认过，合成器才保留这次改动；倒计时结束就恢复原来的显示设置。仅显示页如此。
settings-status-display-trial = 显示设置已应用，等待确认
settings-display-trial-title = 保留这些显示设置吗？
settings-display-trial-message = 显示设置已经生效。若不确认，{ $seconds } 秒后将自动恢复原来的设置。
settings-display-trial-confirm = 确认修改
settings-display-trial-revert = 恢复原设置
settings-display-trial-active = 显示设置正在等待确认
settings-display-trial-answering = 正在提交这次回答
settings-display-trial-notice-confirmed = 已确认这次的显示设置
settings-display-trial-notice-reverted = 已恢复原来的显示设置
settings-display-trial-notice-unreachable = 已在文件中恢复原来的显示设置，但无法通知合成器：{ $reason }
settings-display-trial-notice-failed = 无法恢复原来的显示设置：{ $reason }

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
settings-field-wallpaper-image = 背景图片
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
settings-choice-wallpaper-default = 默认壁纸
settings-choice-wallpaper-nebula = 星云
settings-choice-wallpaper-fuji = 富士山
settings-choice-wallpaper-bricks = 砖石地面
settings-choice-wallpaper-lake = 山湖倒影
settings-choice-wallpaper-rainbow = 彩虹公路
settings-choice-wallpaper-monument = 华盛顿纪念碑
settings-choice-wallpaper-background = 主题背景
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
settings-wallpaper-path = 壁纸
settings-list-add = 添加条目
settings-list-remove = 移除条目
settings-outputs-help = 修改后会立即切换模式、缩放与位置，并请你确认：未确认时 30 秒后自动恢复原来的设置。空列表使用后端默认值；两个位置均留空时自动排列，仅一项留空时该坐标为 0。
settings-terminal-help = 第一项填写可执行程序，之后每项填写一个参数。参数直接传递，不经 Shell 解释。空列表使用启动器默认值。
settings-wallpapers-help = 每项填写显示接口名称和壁纸（内置名称、none 或图片路径），* 表示其他显示器。空列表使用主壁纸。
settings-error-collection = 请检查第 { $row } 项：尺寸 1–16384，刷新率 1–1000，缩放 0.25–8，位置在 ±4194304 内。

settings-page-input-method = 输入法
settings-field-input-method-command = 引擎程序与参数
settings-input-method-help = 第一项填写 fcitx5 或兼容的可执行程序，之后每项填写一个参数。空列表禁用自动启动。改动在下次物理会话生效，命令行选项优先。请单独安装引擎，并使用引擎配置工具管理语言、词库和切换快捷键；不要添加后台运行选项。

settings-error-input-method-command = 请检查第 { $row } 项：首项必须为可执行程序，参数不能含有空字符。
settings-error-wallpaper-entry = 请检查第 { $row } 项：显示器名称不能重复。

settings-ctl-help = 用法：inoxctl reload（重载运行中的组件）或 inoxctl --help
settings-ctl-applied = { $component }：已生效
settings-ctl-absent = { $component }：未运行
settings-ctl-failed = { $component }：未确认重载成功
settings-ctl-incomplete = 配置未能全部重载成功。

# 每个页面是做什么的。一屏只有一个标题（页面名），标题下是这个说明句，
# 用 caption 角色绘制（UI 规范 §2）。
settings-page-appearance-description = 会话使用哪套主题。
settings-page-panel-description = 面板条在显示器边缘的位置，以及它显示什么。
settings-page-notifications-description = 弹出通知出现的位置、停留时间，以及会话启动时是否静音。
settings-page-locale-description = 会话使用的语言。
settings-page-input-description = 键盘与指针的行为。
settings-page-layout-description = 平铺窗口的排列方式。
settings-page-launcher-description = 启动器运行什么，以及显示多少。
settings-page-wallpaper-description = 每个窗口背后的画面。
settings-page-displays-description = 为下次启动合成器保存的显示器偏好。
settings-page-input-method-description = 会话自动启动的输入法。

# 页面内的卡片分组（UI 规范 §4）。一张卡片是一个主题的标题与字段，
# 卡片之间使用大号间距。
settings-group-appearance-theme = 主题
settings-group-panel-strip = 面板条
settings-group-panel-behaviour = 行为
settings-group-panel-clock = 时钟
settings-group-panel-tasks = 任务按钮
settings-group-notifications-display = 弹窗
settings-group-notifications-timeouts = 停留时间
settings-group-notifications-queue = 队列满时
settings-group-notifications-policy = 策略
settings-group-locale-language = 语言
settings-group-input-keyboard = 键盘
settings-group-input-repeat = 按键重复
settings-group-input-pointer = 指针
settings-group-layout-tiling = 平铺
settings-group-launcher-command = 命令
settings-group-launcher-results = 结果
settings-group-wallpaper-image = 背景图片
settings-group-wallpaper-outputs = 各显示器图片
settings-group-displays-outputs = 显示器
settings-group-input-method-command = 引擎

# 列表为空时要说的话。空态是用户必须能读到的状态，而不是一片空白（UI 规范 §6）。
settings-empty-outputs = 尚无显示器偏好，将使用后端默认值。
settings-empty-input-method = 没有引擎命令，自动启动已关闭。
settings-empty-launcher-terminal = 没有终端命令，将使用启动器默认值。
settings-empty-wallpaper-per-output = 没有各显示器图片，所有显示器都使用主壁纸。

# 被组件拒绝或无法请求的改动并没有卡住：这种情况正是「应用」再次可用的原因，
# 这一行把这件事说出来。
settings-status-retry-hint = 按「应用」可再次把这次改动发送给各组件。
