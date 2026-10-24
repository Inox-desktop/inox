# Plasma Desktop 对照与 Inox 差距

- 对照日期：2026-10-16
- 本地来源：`../plasma-desktop`，提交 `90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a`，工作区干净。
- 用途：为[验收规范](./acceptance.md)与 ADR 0011–0015 提供可追溯的工程依据。下表描述该快照中的代码与测试，不代表 KDE 的全部质量标准，也不代表这些测试已由 Inox 运行。

## 参考边界

该仓库的 [README](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/README.md) 明确区分桌面专属组件与其他 Plasma 仓库。不能从这里推导 KWin 的 DRM、输入、合成性能或会话锁安全实现；这些领域的 Inox 决策仍需 Smithay/协议依据与自身验证。`design/` 包含历史提案与 future work，不应当作当前产品保证。

参考的是行为、边界和验证方法。Inox 在本计划快照期遵守 Rust + Smithay + calloop + Iced、Fluent 与 BSD 3-Clause 的既有约束；随后的 ADR 0033/0034 决定 Shell GUI 迁往 QML + Qt Quick 并将项目重许可为 LGPL-3.0（见 [ADR 0033](../adr/0033-qml-qt-quick-shell-evaluation.md)、[ADR 0034](../adr/0034-lgplv3-relicensing-evaluation.md)）。本次未复制上游代码、测试或资源；上游各文件许可证不同，引用不构成引入依赖或再许可。

## 可核实的对照

| 编号 | 快照证据 | 从证据得到的启发 | Inox 当前标准的缺口 | 对应决策与验收 |
|------|----------|------------------|--------------------|----------------|
| PL-01 | [Kickoff 测试](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/appiumtests/applets/kickofftest/kickofftest.py) 的 `test_1_keyboard_navigation` 逐步断言唯一焦点，`test_3_search_app` 检查真实 D-Bus 服务出现 | 启动器要测完整任务与焦点轨迹 | “搜索并启动”没有就绪证据、取消路径和键盘约束 | ADR 0013；D02、Q01 |
| PL-02 | [面板焦点测试](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/appiumtests/desktop/panelfocustest.py) 分别检查点击桌面操作不取得面板焦点、显式快捷键取得面板焦点 | 区分展示、激活和输入焦点 | 面板“显示出来”没有说明会不会抢键盘输入 | ADR 0012、0013；W03、D01 |
| PL-03 | [任务管理器测试](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/appiumtests/applets/taskmanagertest.py) 启动测试窗口，验证角标与双窗口分组；[Task.qml](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/applets/taskmanager/qml/Task.qml) 消费任务模型并提供可访问名称 | UI 应消费可更新的任务状态，支持真实窗口生命周期 | 没有窗口出现、销毁、重连后的任务列表一致性标准 | ADR 0013；D03 |
| PL-04 | [屏幕映射测试](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/containments/desktop/plugins/folder/autotests/screenmappertest.cpp) 的 `tst_addRemoveScreenWithItems`、`tst_readAndSaveItemsOnActivitiesOnDisabledScreens` 检查移除、恢复及持久化 | 显示器暂时不存在与用户删除配置是两件事 | 没有输出身份、断连后的状态保留与再接入断言 | ADR 0012、0014；O01、O02、C03 |
| PL-05 | [位置管理测试](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/containments/desktop/plugins/folder/autotests/positionertest.cpp) 检查保存/加载、分辨率变化，以及没有屏幕时不覆盖配置 | 几何变化与保存策略必须一同测试 | 平铺属性测试未覆盖输出变化造成的持久化损失 | ADR 0012、0014；W02、O02、C03 |
| PL-06 | [后台服务设置测试](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/appiumtests/kcms/kcm_kded_test.py) 同时检查按钮变化和 D-Bus 中实际加载的服务 | 设置成功应以服务端真实状态为准 | “读写并热重载”未区分写盘、请求成功与生效 | ADR 0014；D04、C02 |
| PL-07 | [时钟设置迁移](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/desktoppackage/contents/updates/digitalclock_migrate_showseconds_setting.js) 将旧布尔值转成新的显示策略；[屏幕映射迁移](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/desktoppackage/contents/updates/folderview_fix_recursive_screenmapping.js) 清理历史数据 | 默认值与类型演进需要显式迁移及历史样本 | 旧配置升级、失败保留、回退流程未定义 | ADR 0014；C04 |
| PL-08 | [AppletError.qml](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/desktoppackage/contents/applet/AppletError.qml) 提供适配面板方向的错误呈现与复制错误入口 | 故障是有交互和布局约束的产品状态 | 无服务不可用、重试、超时和错误态验收 | ADR 0013；D05、Q02 |
| PL-09 | [桌面 GUI 测试注册](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/appiumtests/desktop/CMakeLists.txt) 声明隔离总线、Wayland 环境与超时；[桌面测试](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/appiumtests/desktop/desktoptest.py) 失败时截屏并检查进程退出 | 测试基础设施、失败证据与清理也属于质量契约 | 只要求截图，没有运行上下文、超时或残留检查 | ADR 0011；B02、R03 |
| PL-10 | [合并请求模板](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/.gitlab/merge_request_templates/Default.md) 要求原因、测试计划、UI 前后对照、关联缺陷 | 修复要有可复现案例；视觉变化要有审查证据 | CI 绿灯没有关联需求或回归用例 | ADR 0011；B01、Q02 |
| PL-11 | [.kde-ci.yml](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/.kde-ci.yml) 区分运行依赖、要求测试通过并启用 QML lint；同时配置 `--repeat until-pass:5` | 环境与运行依赖须版本化；重试策略需要独立判断 | 未约束缺依赖跳过、零用例或重跑后的报告 | ADR 0011；B02、L01。Inox 拟保留首次失败，不采用重试即通过 |
| PL-12 | [LockScreenUi.qml](https://invent.kde.org/plasma/plasma-desktop/-/blob/90f507b7e2fe8a8a02e9000c6a3c8adfd9bde94a/desktoppackage/contents/lockscreen/LockScreenUi.qml) 区分认证提示、成功、错误和挂起前清理密码 | 锁屏 UI 应覆盖认证生命周期 | “可解锁”未定义认证失败、挂起与敏感信息清理 | ADR 0015；S02、S03。该 QML 不证明合成器锁屏安全 |

## 对 Inox 的取舍

M0/M1 保留为基础设施与渲染里程碑，不再被用来证明完整桌面质量。M2 起要求真实客户端可观察的窗口/输入断言；M4 起要求完成任务、状态恢复、语义与错误反馈；M5 才可声明日常可用，且必须通过锁屏、会话故障与硬件矩阵；M6 才能声明满足指定发布配置的质量门槛。

活动管理、桌面文件夹视图、在线组件市场、任意第三方插件、KRunner 提供器全集均不因本次对照进入 Inox 的最低范围。独立的新需求仍需产品排期。基础键盘操作、核心语义、字体缩放与中英文界面不能再用“1.0 后完整支持”整体延期。

以下是 Inox 自己提出的要求，不能归因于 Plasma 已保证：多输出事务与回退、DRM/VT 生命周期、协议权限、锁屏失败时保持封锁、配置原子写入、性能阈值、硬件支持矩阵。这些要求的依据、替代方案和证据义务分别在 ADR 0011–0015 中展开。
