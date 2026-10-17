# ADR 0027: 迁移 zbus 6 并统一提升 MSRV 至 1.89

- 状态：accepted
- 日期：2026-10-17
- 决策者：当前开发会话的人类维护者
- 批准记录：在说明 zbus 6 及 Rust 1.89 的迁移影响后，用户回复「帮我去迁移下」，明确批准本次迁移范围。
- 关联：ADR 0022 §5、ADR 0026、PR #23；验收 B02、L01、C02、D06。

## 背景与选择

现有 zbus 5 通过 zvariant、zbus_names、zvariant_derive 引入
RUSTSEC-2026-0370、0371、0372 停止维护告警，使完整依赖图的 cargo-deny 失败。
这些库已合并进 zbus 6；继续停留在 5.x 无法解除阻塞，忽略告警不符合本项目要求。
本次选择迁移维护中的 API，保留现有 D-Bus 接口、状态所有权和错误边界。

## 决策

- 根 workspace.dependencies 使用 zbus 6.0，许可证仍为 MIT；不新增 Inox Cargo 依赖边。
- 工作区 rust-version 从 1.88 提升至 1.89，所有成员继续继承；CI 主工具链仍跟随 stable。
- 显式启用 default-rt、proxy、service、p2p、tracing；用 zbus::block_on 驱动异步 API，
  代替已经移除的 blocking API。沿用通知调用与信号的独立传输线程、顺序处理及有界等待。
  配置重载的文件准备、回执和 Iced/calloop 状态应用边界保持不变。
- 使用 MessageStream、Variant/OwnedVariant、Header::primary().serial_num() 和延迟到 build 的构造器校验。
  标准 D-Bus 错误通过 Error::FDO 处理，继续区分消费方拒绝和不可达。
- 不修改 D-Bus/Wayland 线上接口、配置格式或权限；不降低 cargo-deny 门槛。

## 验证与回退

CI 验证整个工作区、真实通知 p2p 往返/信号/超时、配置应用回执和依赖检查。
增加 Rust 1.89 的工作区静态检查，验证声明的工具链下限。
只有 CI 全绿后才视为验证完成；真实硬件证据边界不变。

回退需同时恢复依赖、锁文件和 API，并恢复 MSRV 声明；恢复 zbus 5 将重新暴露维护状态阻塞，
不能用降低检查要求作为回退方案。旧工具链 1.88 不再受支持，系统接口及用户配置无需迁移。

## 后果与复审

收益是删除停止维护的依赖，恢复完整依赖检查；成本是 API 调整、运行时驱动和工具链下限提升。
本记录只替代 ADR 0022 的 zbus 5 阻塞 API 选择，不把该提案的其它决策自动标为 accepted。
后续更换异步运行时、引入跨进程事务或修改协议时另行评审。
