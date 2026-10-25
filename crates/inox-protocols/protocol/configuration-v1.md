# Inox 配置接口 v1（会话 D-Bus）

- 对象路径：`/org/inox/Configuration`
- 接口：`org.inox.Configuration1`
- 总线名：`org.inox.Compositor`、`org.inox.Panel`、`org.inox.Launcher`、
  `org.inox.Wallpaper`、`org.inox.Notifications`
- `Reload(t generation, s expected) -> (t generation, s applied)`：读取完整候选，
  要求 `InoxConfig::snapshot()` 与 expected 逐字相等，准备并应用后回传实际快照。
  快照是当前 schema 的确定性 TOML（包含已解析主题与 locale），不是磁盘原文。
  不兼容版本应拒绝，不得忽略未知字段。接口变更使用新版本名。
  **输出模式、缩放与位置不属于 `Reload` 的能力**：它们会让屏幕可能变得不可操作，
  需要确认与回退，因此由同一对象路径上的
  [`org.inox.Configuration2`](./configuration-v2.md) 的 `Trial`/`Confirm`/`Rollback`
  承担。`Reload` 携带不同的 `outputs` 必须拒绝，并指向该接口——
  SIGHUP 与 `inoxctl` 没有确认通道。
- 失败返回 `org.freedesktop.DBus.Error.Failed`，原因含字段或资源；
  忙碌、队列满、退出、过期调用均不得回复成功。请求截止为 5 秒，
  设置调用超时为 6 秒；消费方提交前检查截止时间。
- 不接受路径、任意命令或资源正文，只重读消费方启动时发现的配置目录。
  仅同一用户的会话总线可达；不提供系统总线或网络服务，不给予额外权限。
- 消费方缺失时设置保留 saved-only。多消费方之间不承诺全局原子提交。
  SIGHUP 无客户端代次与回执，使用相同准备/应用入口并记录结果。
