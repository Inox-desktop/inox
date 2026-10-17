# inox-launcher — 面向用户的文案（根级 AGENTS.md §7.3）。
#
# id 形如 `<component>-<area>-<name>`；尾部的 `.attribute` 选择消息的属性，见
# `inox_common::i18n`。本文件必须与 en-US 目录保持 id 与占位符一致（验收 Q03）。

launcher-window-title = 应用程序
launcher-search-placeholder = 搜索应用程序
launcher-search-hint = 上下键选择 · 回车启动 · Esc 关闭

launcher-state-loading = 正在读取应用程序索引…
launcher-state-empty-title = 没有找到应用程序
launcher-state-empty-body = 在 { $dirs } 中没有可用的 desktop entry。
launcher-state-empty-hint = 请检查 XDG_DATA_DIRS 是否指向你的应用程序目录。
launcher-state-nomatch-title = 没有匹配项
launcher-state-nomatch-body = 没有应用程序匹配“{ $query }”。
launcher-state-unavailable-title = 无法使用应用程序索引
launcher-state-action-retry = 重试
launcher-state-truncated = 只搜索前 { $limit } 个字符。

launcher-row-icon-placeholder = ？
launcher-row-disabled = 无法使用：{ $reason }

launcher-launch-requested = 正在启动 { $name }…
launcher-launch-started = { $name } 正在运行。
launcher-launch-failed = 无法启动 { $name }。

launcher-skipped-note = 已跳过 { $count } 个不可用的条目。

launcher-failure-index-unavailable = 无法读取应用程序索引：{ $detail }
launcher-failure-no-command = 该条目没有定义 Exec 命令
launcher-failure-unknown-field-code = Exec 命令使用了未定义的字段码 { $code }
launcher-failure-misplaced-field-code = Exec 命令在更大的词中使用了 { $code }
launcher-failure-unterminated-quote = Exec 命令的双引号不配对
launcher-failure-trailing-escape = Exec 命令以落单的反斜杠结尾
launcher-failure-trailing-percent = Exec 命令以落单的 % 结尾
launcher-failure-missing-terminal = 该条目需要终端，但 shell.toml 未设置 launcher.terminal
launcher-failure-spawn-failed = 无法启动进程：{ $detail }
launcher-failure-exited-before-ready = 进程在就绪之前就退出了：{ $detail }
launcher-failure-not-ready-in-time = 进程在 { $seconds } 秒内没有任何运行迹象
