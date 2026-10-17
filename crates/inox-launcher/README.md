# inox-launcher

应用启动器。属于 Inox 桌面环境的 **L3** 层（[ADR 0018](../../docs/adr/0018-l3-component-architecture.md)）。

启动器做三件事：把 XDG 应用条目读成一个索引、按查询给出确定的排序、把选中的条目启动起来并**验证**它真的起来了。这三件事分别在本 crate 的前三个模块组里，都不依赖合成器，因此可以脱离会话测试。

## 状态

**已实现**（证据等级 E0/E1：`fmt`、`clippy -D warnings`、`cargo doc -D warnings`、`cargo test` 全绿；纯逻辑有单元测试与属性测试）：

| 模块 | 内容 | 验收 |
|------|------|------|
| `desktop_entry` | desktop entry 格式解析：`[Desktop Entry]` 组、`Type=Application`、本地化 `Name`/`GenericName`/`Comment`、`Hidden`/`NoDisplay`、`OnlyShowIn`/`NotShowIn`、`TryExec`、`Categories`、`Actions`（含 `[Desktop Action …]` 组）；一行坏行被跳过并记录，一个坏文件不会中断其余文件的扫描 | D02 |
| `exec` | `Exec` 字段码展开为 **argv**：`%f %F %u %U %i %c %k %%` 与已废弃的 `%d %D %n %N %v %m`，双引号与反斜杠转义。返回 `Vec<String>`，本 crate 里**不存在**任何返回命令行字符串的函数——这就是「搜索词永远不会变成 shell 命令」的结构性保证 | D02 |
| `index` | XDG 数据目录扫描、目录优先级去重（靠前的目录胜出，`Hidden=true` 同样遮蔽后续同名条目）、子目录 id（`kde/foo.desktop` → `kde-foo.desktop`）、每个不可用文件的诊断 | D02 |
| `search` | 有界查询（`launcher.max_query_chars`）与有界结果（`launcher.max_results`）、确定性排序（全名 > 前缀 > 词前缀 > 子串 > 子序列，平手按名称再按 id）、Unicode 感知、请求代次（旧结果不能覆盖新查询） | D02、Q03 |
| `contract`、`activation` | ADR 0013 的 sent/accepted/applied/rejected/timeout 词汇、按**选择代次**幂等的激活（连按回车只启动一次）、失败原因与重试、有界等待后按观测证据给出就绪状态 | D02、D05 |
| `placement` | layer-shell 放置策略：overlay 层、居中、独占键盘、由配置与令牌推出的尺寸、每输出归属。**纯数据** | D02 前置 |
| `view_model`、`view`、`theme`、`i18n` | 从状态到画面的纯投影、Iced 组件树、令牌到 `iced::Theme` 的映射、Fluent 目录 | D05、Q01、Q02、Q03 |

**未实现 / 被阻塞**（不声称 E2/E3）：

- **它是普通窗口，不是 layer 表面。** `inox-compositor` 尚未广告 `zwlr_layer_shell_v1`，且 `iced_layershell` 0.19 与当前 iced 代次不兼容。`placement::Placement` 已经把该策略算好并写进日志，但没有任何宿主会应用它；启动时会以 `tracing::warn!` 说明这一点（ADR 0018）。
- **就绪以进程存活为证据。** ADR 0013 要的是「窗口或 D-Bus 就绪」，而两者当前都不可达：合成器没有窗口列表接口，多数应用也不持有 D-Bus 名字。现在用的是「宽限期后进程仍存活」，它排除了实际最常见的失败（`Exec` 指向一个启动即退出的程序），但比 ADR 0013 的claim 弱，`activation` 的模块文档里写明了。
- **不加载图标主题。** 条目有图标名时，图标列画名称首字符；没有图标名时画占位符。两种降级在颜色上可区分（D05）。
- **不导出语义树。** iced 0.14 没有 AT-SPI 桥，Q01 的 role/name/state/action 导出被该原型阻塞（ADR 0013 要求在 M4 之前完成）。
- **`Actions` 只解析不使用。** 交互式启动器只启动条目自身的 `Exec`；动作是索引的一部分，留给后续「右键菜单」。

## 目录结构

```
src/lib.rs            crate 文档与模块声明
src/desktop_entry.rs  desktop entry 格式解析
src/exec.rs           Exec 字段码 → argv
src/index.rs          XDG 数据目录扫描与索引
src/search.rs         排序、有界查询与请求代次
src/contract.rs       ADR 0013 的失败/状态词汇
src/activation.rs     至多一次的激活与就绪证据
src/placement.rs      layer-shell 放置策略（纯数据）
src/view_model.rs     状态 → 画面（纯投影）
src/app.rs            Elm：State / Message / update
src/view.rs           Iced 组件树
src/theme.rs          令牌 → iced::Theme 与样式闭包
src/i18n.rs           Fluent 目录装配
src/main.rs           日志、配置、目录，然后运行窗口
i18n/{en-US,zh-CN}/launcher.ftl
tests/{catalog.rs,render.rs,support/mod.rs}
```

## 测试

```sh
cargo test -p inox-launcher --all-features
```

- 单元测试：每条纯规则各一条，含拒绝与错误路径（解析器、字段码、目录优先级、排序键、激活状态机、放置几何、截断）。
- 属性测试：每项 ≥1000 个样本——`Exec` 展开不会拆分参数且引号往返保真、任何 `Exec` 值都被读取或给出理由、搜索确定性、结果有界且有序、乱序结果只有最新一代生效。
- `tests/catalog.rs`：`en-US` 与 `zh-CN` 的 id 与占位符一致；代码里出现的每个 `launcher-…` id 都在目录里，目录里每个 id 都被代码用到。
- `tests/render.rs`：1280×720 的浅色、深色、无匹配、索引不可用、空索引、长名称六个场景，用**真实的 Iced 组件树**经 tiny-skia 渲染后断言像素（表面色、强调色、警告色、颜色种类数）；一个交互测试在真实文本框里打字再按回车，断言只发出一次启动。

设置 `INOX_SCREENSHOT_DIR` 时，每个场景会额外落盘为 `inox-launcher-<场景>.png`，供 CI 收集为工件（验收 Q02）。

## 开发约定

- 遵循根级 [AGENTS.md](https://github.com/Inox-desktop/inox/blob/main/AGENTS.md) 与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径。**
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 用 `launcher`。

## 许可证

BSD 3-Clause，见仓库根的 [`LICENSE`](../../LICENSE)。
