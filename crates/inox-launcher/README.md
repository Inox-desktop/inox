# inox-launcher

应用启动器。属于 Inox 桌面环境的 **L3** 层。

启动器做三件事：把 XDG 应用条目读成一个索引、按查询给出确定的排序、把选中的条目启动起来并**验证**它真的起来了。这三件事都不依赖合成器，因此可以脱离会话测试。界面是 **QML + Qt Quick 的普通窗口**，经 cxx-qt 绑定到 Rust：领域逻辑、状态机、设计令牌与 Fluent 文案全在 Rust，QML 只渲染一份 JSON 投影并把用户动作发回来（[迁移契约](../../docs/qml-migration-contract.md)）。

## 运行结构

```
main ─ QGuiApplication + QQmlApplicationEngine ─ qrc:/…/Main.qml (Window)
                                  │
        Launcher(#[cxx_qt::bridge]) ─┴─ modelJson  ──▶ QML 绑定（颜色/尺寸/文案/图标/网格几何）
                     ▲                ◀── dispatch(json) ── 动作
                     │                ◀── pump()（QML Timer 16 ms）
                State / update  ◀── Queue ◀── 工作线程（索引扫描、启动探针）
                     │
              reload::Host（SIGHUP + org.inox.Launcher 的 Reload 方法）
```

- **一条数据通道**：`view.rs` 把「状态 + 令牌 + Fluent 文案」投影成一份 JSON，桥把它作为只读属性 `modelJson` 交给 QML；每次状态变化整体重发。这是迁移契约 §3 明确接受的取舍：先保证正确与边界干净。
- **一条动作通道**：`dispatch("{...}")` 解析成 `UiAction` → 既有 `Message`；解析失败只 `warn!`，不 panic。
- **定时器**：QML 的 `Timer` 每 16 ms 调 `pump()`，排空工作线程与重载源送来的消息、跑 `update`、必要时重发投影。`app.rs` 的 `update` 仍然不做 I/O。
- **焦点**：Rust 只递增 `focus_serial`，桥发布成 `focusSerial`，QML 在变化时 `forceActiveFocus()`——搜索框在窗口打开时拿到光标，不靠把控件塞进状态。

## 状态

**已实现**（证据等级 E0/E1：`fmt`、`clippy -D warnings`、`cargo doc -D warnings`、`cargo test` 全绿；纯逻辑有单元测试与属性测试，界面有投影断言与离屏 QML 加载测试）：

| 模块 | 内容 | 验收 |
|------|------|------|
| `desktop_entry` | desktop entry 格式解析：`[Desktop Entry]` 组、`Type=Application`、本地化 `Name`/`GenericName`/`Comment`、`Hidden`/`NoDisplay`、`OnlyShowIn`/`NotShowIn`、`TryExec`、`Categories`、`Actions`（含 `[Desktop Action …]` 组）；一行坏行被跳过并记录，一个坏文件不会中断其余文件的扫描 | D02 |
| `exec` | `Exec` 字段码展开为 **argv**：`%f %F %u %U %i %c %k %%` 与已废弃的 `%d %D %n %N %v %m`，双引号与反斜杠转义。返回 `Vec<String>`，本 crate 里**不存在**任何返回命令行字符串的函数——这就是「搜索词永远不会变成 shell 命令」的结构性保证 | D02 |
| `index` | XDG 数据目录扫描、目录优先级去重（靠前的目录胜出，`Hidden=true` 同样遮蔽后续同名条目）、子目录 id（`kde/foo.desktop` → `kde-foo.desktop`）、扫描时解析每个条目的图标文件、每个不可用文件的诊断 | D02、D05 |
| `search` | 有界查询（`launcher.max_query_chars`）与有界结果（`launcher.max_results`）、确定性排序（全名 > 前缀 > 词前缀 > 子串 > 子序列，平手按名称再按 id）、Unicode 感知、请求代次（旧结果不能覆盖新查询） | D02、Q03 |
| `contract`、`activation` | 交互契约的 sent/accepted/applied/rejected/timeout 词汇、按**选择代次**幂等的激活（连按回车只启动一次）、失败原因与重试、有界等待后按观测证据给出就绪状态 | D02、D05 |
| `placement` | layer-shell 放置策略：overlay 层、居中、独占键盘、输出尺寸的份额（宽 4/5、高 9/10，至少六列、四行网格）、整行对齐的网格列数、每输出归属。**纯数据** | D02 前置 |
| `view_model` | 从状态到画面语义的纯投影：网格瓦片、选中、禁用原因、分类 chips、加载/空/无匹配/索引不可用四种横幅 | D05 |
| `view` | 上面那份 JSON 投影：颜色是令牌转换成的 `#AARRGGBB` 字符串，尺寸与字号是令牌推导值，文案是 Fluent 的完整句子 | D05、Q01、Q03 |
| `theme` | 令牌 → QML 颜色与尺寸的纯函数；字号 = Qt Quick 96 dpi 名义正文尺寸 × `typography.scale`，hairline = `spacing.xs / 2`，网格瓦片与列数由图标尺寸、字宽与卡内可用宽度推出 | Q02 |
| `icons` | `Icon=` → 图标文件：绝对路径原样使用；否则在 `$XDG_DATA_HOME/icons`、各 `$XDG_DATA_DIRS/icons`、`~/.local/share/icons` 中按 `hicolor` 优先、`{size}x{size}/apps` 接近 96 px 优先、PNG 优先于 `scalable/*.svg` 查找，最后退回 `*/pixmaps`；纯 `std`，路径变 `file://` URL 交给 QML | D05 |
| `categories` | 把 freedesktop `Categories=` 映射到固定的分类 chips（All、Development、Graphics、Internet、Office、AudioVideo、Game、Utility、System、Education、Science、Other）；chip 只列索引中确有条目的分类 | D05 |
| `runtime`、`reload` | 本 crate 自己的 `Queue`/`Task`/`oneshot`；SIGHUP 与 `org.inox.Launcher` 的 `Reload`（失败的重新加载保留旧配置） | — |
| `app`、`i18n`、`qml/Main.qml` | `State` / `Message` / `update`、Fluent 目录装配、声明式界面 | D05 |
| `open`、`open::mime`、`open::mimeapps` | **打开器**（`inox-open` / `inox-launcher --open`）：把参数分成 URI（按 `x-scheme-handler/<scheme>`）或文件路径（按共享 MIME 数据库的 `mime/globs2` glob 规则定类型），在 `mimeapps.list` 的规范顺序里取默认应用、退到 `Added Associations`、扣掉 `Removed Associations`，定位并解析 `.desktop`、用 `exec` 把目标填进 `%f`/`%u`/`%U`，再启动。退出码区分「没人能打开」(3) 与「找到了但起不来」(4) | D02 的打开一半 |

**打开器（`open`）**

启动器本身只管「按名字找应用」；打开器是同一件事的另一种选法——目标挑应用。它复用本 crate 已有的
`desktop_entry` 与 `exec`，因此 `Exec` 的字段码展开、终端前缀与「argv 永远不会变成 shell 命令」的
性质对打开器同样成立。两条入口都在 Qt 初始化**之前**处理，打开一个链接不会付 GUI 工具包的启动代价：

```
inox-open TARGET          # 打包时安装的链接，指向 inox-launcher
inox-launcher --open TARGET
```

| 退出码 | 含义 |
|--------|------|
| 0 | 应用已启动 |
| 1 | 命令行不对 |
| 2 | 目标不存在 |
| 3 | 没有应用能打开（类型未知 / 没有登记 / 登记的应用未安装） |
| 4 | 找到了应用但起不来（`Exec` 读不懂，或进程创建失败） |


**未实现 / 被阻塞**（不声称 E2/E3）：

- **它是普通窗口，不是 layer 表面。** `inox-compositor` 尚未广告 `zwlr_layer_shell_v1`，`placement::Placement` 已经把该策略算好并写进日志，但没有任何宿主会应用它；启动时会以**恰好一条** `tracing::warn!` 说明这一点。
- **没有像素级 UI 证据。** Iced 时代的 `tests/render.rs`（tiny-skia 离屏渲染 + `INOX_SCREENSHOT_DIR` 截图工件）已随迁移删除；现在断言的是投影值与「QML 模块能在 `QT_QPA_PLATFORM=offscreen` 下加载并回报 ready」。Q02 的截图证据需要 waydriver + 真实 Qt 平台，工作区还没有这样的宿主。
- **语义树只被「存在性」断言。** QML 用 `Accessible.role`/`Accessible.name` 标注控件，`reportReady` 会检查固定 chrome 的可用控件数量下限，离屏平台没有 AT-SPI 桥；Q01 不声称已验收。Qt 6.12 的 `Accessible` 附加类型也没有「不可用」状态，禁用行把原因放进 `Accessible.description`。
- **就绪以进程存活为证据。** 交互契约要的是「窗口或 D-Bus 就绪」，而两者当前都不可达：合成器没有窗口列表接口，多数应用也不持有 D-Bus 名字。现在用的是「宽限期后进程仍存活」，它排除了实际最常见的失败（`Exec` 指向一个启动即退出的程序），但比交互契约的 claim 弱，`activation` 的模块文档里写明了。
- **图标来自已安装的图标主题，不来自本 crate。** `icons` 只按 XDG 图标主题规范查找 `Icon=` 指向的文件；主题里没有该图标（或 Qt 无法解码该文件）时，瓦片退回名称首字符的字母块，两种降级在颜色上可区分（D05）。图标主题的 `index.theme` 继承链与 `symbolic` 目录不参与查找。
- **重载桥没有自动化测试。** SIGHUP 遮罩与 `org.inox.Launcher` 的 `Reload` 已接线（失败的重新加载保留旧配置），但驱动它们需要会话总线或信号，测试只覆盖了 SIGHUP 安装失败返回错误这一条；改动 `src/reload.rs` / `main::reload_state` 时要人工验证。
- **`Actions` 只解析不使用。** 交互式启动器只启动条目自身的 `Exec`；动作是索引的一部分，留给后续「右键菜单」。
- **打开器没有门户。** `org.freedesktop.portal.OpenURI` 需要 `xdg-desktop-portal` 后端，本仓库还没有
  （`inox-portal` 只是规划中的 crate）。`open` 走的是非沙箱路径：调用方自己的环境、调用方自己的权限。
- **打开器不嗅探文件内容。** 文件的类型只由名字（共享 MIME 数据库的 glob 规则）决定，不读 magic
  字节：扩展名说谎的文件会按扩展名选应用。`mime.cache` 也不读——它是带版本号的二进制格式，而
  `globs2` 是文本。
- **打开器不验证就绪。** `activation` 会等「窗口或 D-Bus 就绪」的证据，因为启动器要把状态显示给
  用户；打开器不能等——`xdg-open` 的契约就是启动完立刻返回。进程创建失败仍然以退出码报告。
- **`inox-open` 是链接，不是第二个二进制。** 打包脚本（`packaging/install.sh`、AUR 与 canary 的
  `PKGBUILD`）把它装成指向 `inox-launcher` 的符号链接；从源码直接 `cargo run` 时用 `--open`。
  仓库里没有覆盖系统 `xdg-open` 的钩子：那要靠发行版或用户自己的选择。

## 目录结构

```
Cargo.toml           工作区继承：字段一律 *.workspace = true
build.rs             cxx-qt：注册 com.inox.launcher.qml 模块并编译 bridge
qml/Main.qml         声明式 UI（Window 根），只绑定 vm.*
src/main.rs          入口、Qt 宿主、QML 加载自检；模块声明（bin-only，无 lib.rs）
src/cxxqt_object.rs  #[cxx_qt::bridge]：model_json / windowWidth / focusSerial / quitRequested + dispatch / pump / reportReady
src/view.rs          状态 + 令牌 + Fluent → JSON 投影
src/theme.rs         令牌 → QML 颜色与尺寸（纯函数）
src/runtime.rs       Queue / Task / oneshot（替代 iced::Task 与 Subscription）
src/reload.rs        SIGHUP + org.inox.Launcher 重新加载桥
src/app.rs           State / Message / update
src/view_model.rs    状态 → 画面语义
src/desktop_entry.rs desktop entry 格式解析
src/exec.rs          Exec 字段码 → argv
src/index.rs         XDG 数据目录扫描与索引
src/search.rs        排序、有界查询与请求代次
src/contract.rs      交互契约的失败/状态词汇
src/activation.rs    至多一次的激活与就绪证据
src/placement.rs     layer-shell 放置策略（纯数据）
src/i18n.rs          Fluent 目录装配
src/catalog_tests.rs 目录一致性（Q03，#[cfg(test)]）
src/headless_tests.rs 投影 + QML 加载 + 键盘路径的证据（#[cfg(test)]）
i18n/{en-US,zh-CN}/launcher.ftl
```

## 测试

```sh
CARGO_HOME=<repo>/.cargo-home QT_QPA_PLATFORM=offscreen \
  cargo test -p inox-launcher --all-features --offline
```

- 单元测试：每条纯规则各一条，含拒绝与错误路径（解析器、字段码、目录优先级、排序键、激活状态机、放置几何、截断）。
- 属性测试：每项 ≥1000 个样本——`Exec` 展开不会拆分参数且引号往返保真、任何 `Exec` 值都被读取或给出理由、搜索确定性、结果有界且有序、乱序结果只有最新一代生效。
- `catalog_tests`：`en-US` 与 `zh-CN` 的 id 与占位符一致；代码里出现的每个 `launcher-…` id 都在目录里，目录里每个 id 都被代码用到。
- `headless_tests`：浅色/深色/无匹配/加载中/空索引/索引不可用/长名称/200 % 字号等 Iced 渲染测试曾画过的场景，改为**投影断言**；QML 动作经 `dispatch` 进入真实 `update`（打字、Enter 只启动一次、Esc、上下键不环绕、指针选择、按住不重复）；最后在子进程里以 `QT_QPA_PLATFORM=offscreen` 加载 QML 模块，等 `reportReady` 确认根对象与语义树。

## 开发约定

- 遵循[硬性规则速查](https://github.com/Inox-desktop/inox/blob/main/README.md#硬性规则速查)与本 crate 的 [`AGENTS.md`](./AGENTS.md)。
- **CI（GitHub Actions）是唯一的构建与测试验证途径。**
- 颜色、尺寸、间距、圆角与文案只来自 `inox_config::Theme` 与 Fluent 目录；QML 里不允许出现字面量颜色、像素尺寸或用户可见英文串。
- 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)，scope 用 `launcher`。

## 许可证

LGPL-3.0，见仓库根的 [`LICENSE`](../../LICENSE)。
