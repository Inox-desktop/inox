# inox-wallpaper — zh-CN.
#
# 与 en-US 目录一一对应：id 与占位符必须完全一致，由 tests/catalog.rs 断言。

## 窗口化预览。
wallpaper-window-title = Inox 壁纸
wallpaper-windowed-notice = 当前以窗口方式预览：本合成器尚未提供 layer-shell 宿主，因此壁纸的布局经过计算与测试，但未实际应用（ADR 0018）。

## 单个输出的状态。
wallpaper-output-heading = 输出 { $output }
wallpaper-output-image = { $width }×{ $height } 图像，来自 { $path }
wallpaper-output-background = 主题背景
wallpaper-status-loading = 正在加载壁纸…

## 无法完成的请求。
wallpaper-problem-heading = 所选的壁纸无法使用
wallpaper-problem-subject = { $output }：{ $path }
wallpaper-remedy = 请把 wallpaper.image 指向确实存在的 PNG 或 JPEG，或删除它以使用主题背景。
wallpaper-reason-missing = 该文件不存在
wallpaper-reason-not-a-file-directory = 该路径是目录，不是文件
wallpaper-reason-not-a-file-other = 该路径不是普通文件
wallpaper-reason-unreadable = 无法读取该文件：{ $detail }
wallpaper-reason-undecodable = 无法解码该图像：{ $detail }
wallpaper-reason-too-large = 该图像有 { $pixels } 个像素，超过 { $limit } 的上限
wallpaper-reason-too-long-side = { $width }×{ $height } 的图像有一边超过 { $limit } 像素的上限
wallpaper-reason-degenerate = { $width }×{ $height } 的图像在此输出上没有可用的尺寸
wallpaper-reason-too-many-tiles = { $width }×{ $height } 的平铺需要 { $tiles } 份，超过 { $limit } 份的上限

## 整体配置。
wallpaper-config-error = 无法读取配置：{ $detail }
wallpaper-config-remedy = 请修正配置，然后重新启动 inox-wallpaper。
wallpaper-config-read-once = 配置只在组件启动时读取一次。

## 控件。
wallpaper-action-retry = 重试
wallpaper-action-nothing = 没有可重载的内容
