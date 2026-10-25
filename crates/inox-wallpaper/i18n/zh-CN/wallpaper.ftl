# inox-wallpaper — zh-CN.
#
# 与 en-US 目录一一对应：id 与占位符必须完全一致，由 tests/catalog.rs 断言。

## 窗口化预览。
wallpaper-window-title = Inox 壁纸
wallpaper-windowed-notice = 当前以窗口方式预览：本合成器尚未提供 layer-shell 宿主，因此壁纸的布局经过计算与测试，但未实际应用。

## 单个输出的状态。
# 状态卡片自上而下读：这是哪个输出、上面是什么、来自哪里、如何缩放、组件正在做什么。
# 路径单独成行（caption + text_muted），因为它属于次要元信息，且必须能在很长时仍然可读。
wallpaper-output-heading = 输出 { $output }
wallpaper-output-image = { $width }×{ $height } 图像
wallpaper-output-source = 来自 { $path }
wallpaper-output-background = 主题背景
wallpaper-status-loading = 正在加载壁纸…
# 图像如何适配输出。缩放模式始终显示，因为它是几何中用户唯一能改变的部分（wallpaper.mode）。
wallpaper-scaling = 缩放：{ $mode }
# wallpaper.mode 接受的值，与用户写法一致。
wallpaper-mode-fill = 填充
wallpaper-mode-fit = 适应
wallpaper-mode-stretch = 拉伸
wallpaper-mode-center = 居中
wallpaper-mode-tile = 平铺
# 组件当前所处的阶段，写在上述事实下面。「已应用」是状态契约确认的事实：该帧已提交给此输出；
# 它并不表示存在 layer-shell 宿主。
wallpaper-phase-loading = 正在加载…
wallpaper-phase-ready = 已应用
wallpaper-phase-degraded = 未应用
wallpaper-phase-idle = 无图像
# 空态是一种状态，而不是没有状态：它说明当前显示什么、以及如何改变。只有
# `wallpaper.image = "none"` 会进入这个状态；**缺席**该键是内置默认壁纸，不是这里
# ，所以补救文案可以直接点明是哪个取值导致的。
wallpaper-empty-remedy = wallpaper.image 设为 none，因此显示主题背景。填写内置壁纸的名称或你自己的路径即可显示图像。

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
