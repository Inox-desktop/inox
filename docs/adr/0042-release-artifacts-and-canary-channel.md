# 0042 — 发布制品与金丝雀渠道

- **状态**：proposed
- **日期**：2026-10-25
- **决策者**：YG（人类）/ Inox contributors
- **相关**：[验收规范](../quality/acceptance.md) §1（证据等级 E0–E4）与 §11（L01–L03）、
  [L2 真实后端硬件验收](../quality/l2-hardware-validation.md)、
  [发布制品与金丝雀渠道](../book/src/development/release.md)、
  [ADR 0043｜AUR 源码包与发布后自动同步](./0043-aur-source-packages.md)（第二个安装路径，
  重新定义本 ADR 的「渠道」与「校验和」部分）、
  `packaging/package.sh`、`.github/workflows/ci.yml`、`.github/workflows/release.yml`、
  根 `Cargo.toml` 的 `[workspace.package]` 版本注释（第 21–27 行）

## 背景

> 本仓库当前树中没有 `docs/adr/` 目录：更早的 27 份 ADR 正文在 `acc04dd` 中随目录一起被删除，
> 而[架构决策记录](../book/src/architecture/decisions.md)的索引仍在列出它们。本记录因此写成一份
> **自包含**的决策：它不依赖那些正文，也不再复述它们的结论。

### E4 的定义要求「CI 产出的二进制」，而 CI 不产出任何二进制

[验收规范](../quality/acceptance.md) 第 19 行把这一点写成硬前提：

> 不在本地构建或运行产品测试；E4 使用 CI 产出的二进制，由人类完成并在 Issue 留证。

同一份规范的第 17 行定义了 E4 是什么：人类使用 CI 发布物在登记硬件上操作，记录驱动、输出、
设备与结果。两句话合起来意味着：**没有 CI 产出的二进制，E4 一步都走不了。**

而在这份 ADR 之前，仓库里不存在任何发布物：

- `ci.yml` 的 `build` job（第 106–119 行）确实跑 `cargo build --workspace --release --all-features`，
  但它**不上传任何东西**。产物留在已经销毁的 runner 上，没有任何人拿得到它。这个 job 证明的是
  「发布构建能编译」，不是「有一个可交付的二进制」。
- 仓库**零 tag、零 release**：`git tag` 与 `git ls-remote --tags origin` 都返回空，而 GitHub Release
  必须挂在 tag 上，所以也不存在任何 Release。因此即使有人想下载，也没有地址可下。
- `packaging/` 是**安装面**，不是产出面：`install.sh` 装的是三个 PAM 栈、三个 systemd 单元与
  `greeter` 账户，它假定 `/usr/bin` 里已经有一整套二进制（或以 `--bin-dir` 指过来一套）。
  在没有任何发布物、又禁止本地构建作为验收依据的前提下，这个前提永远是假的。

结论：E4 的阻塞不是「硬件还没到」或「还没安排人测」，而是**被验收对象根本不存在**。

### `--all-features` 的产物不是可交付的二进制

最省事的做法是让 `build` job 把 `target/release` 打包上传。这条路是错的，原因在
`crates/inox-compositor/Cargo.toml` 的 `test-input` feature：

- 它给合成器加一个**接受注入键盘与指针事件的 UNIX 套接字**（`--test-input <PATH>`）。
  这是测试专用通道，M3 端到端用例靠它切焦点、向 X11 窗口输入、做指针拖放。
- `--all-features` 会把它**编译进二进制**。默认特性构建不含该代码，解析器会明确拒绝该选项；
  但在 `--all-features` 构建里，代码在、开关也在，只是默认没打开。
- 一个能在另一个进程请求下注入输入事件的合成器，交到测试者手里就变成了一个测试者无法评估的
  安全面。[验收规范](../quality/acceptance.md) 第 55 行已经把这条边界写死：该 feature 只能被
  表述为「运行期默认关闭、构建期默认不包含」，**不能据此声称在 all-features 构建中不存在**。

所以「打包 `build` job 的产物」这条捷径不是省事，是发布一个自带后门的二进制。

## 决策

### 1. 发布制品只由 CI 产出，本地永不是发布物

任何声称「这是给测试者的二进制」的东西都必须来自 CI 的一次运行，并带有该次运行的提交 SHA。
本地构建仍然允许（开发者需要一个能跑的二进制），但本地构建的结果**不是**发布物，也不构成任何
验收证据——这条与根级 `AGENTS.md` §3.1 的「本地构建不是验收依据」是同一条规则的两面。

### 2. 金丝雀制品的形态

一个 tarball，`target/dist/inox-<version>-<target>.tar.gz`，旁边放同名 `.sha256`。解包后是
**单一顶层目录**，内容固定为：

| 内容 | 说明 |
|------|------|
| `bin/` | 14 个交付二进制，**默认特性**构建，已 strip |
| `data/wallpapers/` | 内置壁纸。`inox-wallpaper` 在运行期经 `inox-common::paths::inox_wallpaper_dirs` 按 `<datadir>/inox/wallpapers` 查找，不随包发布就装出一个壁纸选择器空着的桌面 |
| `packaging/` | 安装面：`install.sh`、`uninstall.sh`、`preflight.sh`、`verify-login.sh`、`rehearse-headless.sh`、试运行与回退脚本、`package.sh`、`README.md`、`pam.d/`、`systemd/`。`install.sh` 按自身所在目录解析 `pam.d/` 与 `../data/wallpapers`，因此这三者在包内必须保持相对位置。`package.sh` 随包发布的目的不是安装（装一个系统用不到它），而是让收到包的人能直接读到制品是怎么做出来的 |
| `BUILD-INFO` | 构建元数据，见下 |
| `SHA256SUMS` | 上述每个文件的校验和（不含自身） |

`BUILD-INFO` 的字段是固定的：`name`、`version`、`stage`、`target`、`commit`、`commit-short`、
`commit-date`、`worktree`、`features`、`profile`、`stripped`、`rustc`、`cargo`、`built-at`、
`builder`、`license`。其中 `commit` 是 40 位 SHA、`features: default`，这两项是可追溯性的全部
意义所在：一条真机验收记录只要写出 `BUILD-INFO` 的 `commit` 与 `stage`，就能定位到唯一的源码状态。

只交付这 14 个二进制：`inox-integration`（测试聚合器）与 `inox-qml-prototype`（cxx-qt 绑定原型）
也有二进制，但一个是用例装置、一个是证明绑定可用的原型，两者都不该装进别人的系统。

### 3. `packaging/package.sh` 是唯一的打包实现，且每个 PR 都跑

打包逻辑放在 `packaging/package.sh`，由两处调用：

- `ci.yml` 的 `package` job：**每个 PR 与每次 push 到 `main`** 都跑一遍，产出物作为
  `inox-canary-tarball` artifact 保留 30 天；
- `.github/workflows/release.yml`：打 tag 或手工 dispatch 时用同一个脚本产出发布资产。

一个只在发布日才被执行的脚本等于没被验证过。让它在每个 PR 上跑，意味着「制品这条路是通的」由
每一次 CI 运行承担，而不是由发布当天的那个人承担。

脚本本身**不做**本地验收：它用 `cargo build --release --workspace --locked`（默认特性）构建，
`--bin-dir DIR` 可改为从已有二进制暂存，`--version V`、`--target TRIPLE`、`--output-dir DIR`
覆盖默认值。默认输出目录是 `target/dist/`。

### 4. 制品自己证明它不是测试构建

`package.sh` 在打包前会**真的运行暂存出来的 `bin/inox-compositor --test-input <path>`**，并且
要求它被拒绝、且错误信息里点名 `test-input`。若该选项被接受（或进程挂住不返回），打包失败。

这条断言无法用单元测试替代：被测性质属于**产物**而不是源码树。一个 CI 全是绿灯、但打包进去的是
all-features 二进制的事故，只有「跑一遍那个二进制」才能发现。

### 5. tag 语义：`vX.Y.Z-canary.N` 是预发布，不是稳定发布

金丝雀的 tag 形如 `v0.1.1-canary.2`。它必须满足：

- 版本以根 `Cargo.toml` 的 `[workspace.package] version` 开头（`release.yml` 会拒绝
  `0.2.0-canary.1` 这种「清单还没 bump」的 tag）；
- tag 是 annotated 的，说明这次切的是什么；
- GitHub Release 以 **prerelease** 发布。任何含 `-canary`、`-alpha`、`-beta`、`-rc`、`-dev`
  的版本都按预发布处理；
- **不登记进任何兼容矩阵**。工作区唯一的 `Cargo.lock` 是兼容性记录（验收项 L01 的不可变清单来源），
  金丝雀不往里加东西，也不产生第二份矩阵。

### 6. 版本语义不变

版本号语义沿用根 `Cargo.toml` 第 21–27 行已经记录的 0.x 规则：**minor 是不兼容变更，patch 是兼容
变更**。这份 ADR 不引入新规则，也不改动现有版本号；它只是给这条规则一个可下载的载体。

### 7. 金丝雀渠道的定位：开发者预览

金丝雀渠道是**开发者预览**，它的用途是让 E4 与手工排查有东西可跑。它**不声明任何硬件支持**：
没有任何 GPU、内核、显示器或输入设备组合因为「金丝雀能启动」而进入支持清单。支持声明只由
验收项 H01–H03 建立，且需要 E4 记录作依据。

## 为什么不是其它方案

| 方案 | 否决理由 |
|---|---|
| 打包 `build` job 的 `target/release` | 那是 `--all-features` 产物，会把 `test-input` 注入套接字一起发出去 |
| 本地打包后上传 Release | 违反「CI 是唯一构建与验证途径」，且没有任何东西能证明上传的二进制对应当前提交 |
| 在 PR 里就创建 GitHub Release | PR 是提案不是发布；每个 PR 都产生一个 Release 会把发布页变成构建日志 |
| 另开一份「发布专用」构建参数 | 两份构建参数必然分叉；「包里的二进制」与「CI 验证的二进制」必须是同一份 |
| 直接发稳定版（无预发布标记） | 稳定版意味着 L01/L02/L03 已通过，而它们全部未满足；用版本号假装通过是最坏的作假方式 |
| 用发行版打包（deb/rpm）作为第一阶段 | 那需要维护者、签名与仓库托管，工作量远大于一个 tarball；它属于 L02 的第二种渠道，不是起点 |

## 后果

### 正面

- **E4 第一次有了被验收对象**：`BUILD-INFO` 里的 `commit` 让「在哪份代码上测的」不再是一句回忆。
- **安装面的语法与入口被连续检查**：`packaging/*.sh` 与 `scripts/*.sh` 在 `package` job 里逐个
  `bash -n`，三个脚本的 `--help` 也被真的执行一次。注意这**不是**安装验证：`install.sh` 与
  `preflight.sh` 都需要 root，CI 从不运行它们，所以「解包 → 安装 → 预检」这条路仍只由人在真机上走。
- **`--all-features` 的误用无法静默出厂**：把测试后门编进制品会让 `package.sh` 直接失败，
  而不是在一周后被某个测试者的机器上发现。
- **制品形状被 CI 结构化检查**：单一顶层目录、14 个二进制、壁纸、安装面、`SHA256SUMS` 可校验、
  `BUILD-INFO` 能从压缩包里读出来——这些都写成失败即红的断言，而不是文档里的约定。
- **真机结果可追溯**：[L2 真实后端硬件验收](../quality/l2-hardware-validation.md) 要求登记
  「发布物 SHA」，此前这个字段无处可填。

### 负面 / 限制

- **这份 ADR 不满足 L01**：没有覆盖运行时系统组件的 SBOM（`cargo-deny` 只看 Cargo 依赖图，
  看不到实际链接的 Qt/Mesa 共享库），也没有完整的 E3/E4 组合结论。tarball 只解决了
  「二进制从哪来」，不解决「哪个组合被声明为兼容」。
- **这份 ADR 不满足 L02**：只有**一个**渠道，而且没有「全新目标环境里安装 → 升级 → 回退 → 卸载」
  的生命周期测试。`uninstall.sh` 存在，但卸载后机器是否真的回到安装前的状态，没有被自动验证过。
  > 2026-10-25 补充：ADR 0043 增加了 AUR 源码配方，即第二个**安装路径**，并重新定义了渠道矩阵与
  > 校验和策略。它改变了「只有一个渠道」这句话，但不改变 L02 的结论：AUR 侧的安装后生命周期
  > 同样没有被 CI 验证过。
- **金丝雀 tarball 不是「发布合格」**：它是开发者预览。把它称作发布、稳定版或「可以日常用」的
  版本，都是这份 ADR 明确禁止的说法；L03 也完全没有被触及（没有人类发布审查、没有 P0/P1 清零）。
- **二进制不具可移植性**：它动态链接构建 runner 的系统库。runner 是 GitHub 的 `ubuntu-latest`
  （当前为 Ubuntu 24.04），因此制品的 glibc 版本下限由那台机器决定。在没有声明基线之前，
  把它装到更旧的发行版上会失败，而这个失败与 Inox 的代码无关。
- **多了一件要保持绿色的东西**：`package` job 与 `Release` workflow 都需要维护。它们红了会阻塞
  合并，而它们的失败原因（打包脚本、tar 参数、资产命名）与产品正确性无关。
- **可能掩盖「构建能过但装不上」**：CI 只验证到「脚本语法正确、`--help` 能答、tarball 形状正确」。
  `install.sh` 与 `preflight.sh` 都需要 root，CI 不执行它们；真正的安装、切换屏幕、注销后回到登录
  画面仍然只由人在真机上做，而 E4 至今**未执行**。

### 取代条件

出现以下任一情况时，本决策应被重新评估或取代：

- **L01/L02 达成**：一旦 SBOM 覆盖运行时系统组件、完整 E3/E4 组合有结论、且至少两种渠道在
  全新环境里通过安装/升级/回退/卸载的自动化验证，金丝雀应升级为正式发布流程，本 ADR 由新的
  发布 ADR 取代；
- **出现第二种渠道**（发行版包、容器镜像、Flatpak 等）：渠道矩阵与校验和策略需要重新定义，
  不能沿用「一个 tarball」的模型（ADR 0043 已部分触发这一条：它增加了 AUR 源码配方）；
- **构建基线与可移植性被正式声明**：若改用固定的旧 glibc 基线、静态链接或容器化构建，
  「不具可移植性」这条限制随之失效，制品的适用面需要重新写；
- **`test-input` feature 消失或被移到不可发布的构建路径**：第 4 条的运行时断言从「必需」降为
  「冗余」，但仍应保留到替换它的机制存在为止；
- **执行 E4 后发现制品本身不可用**（例如缺少运行时依赖、动态链接不满足）：制品形态需要重新设计，
  而不仅仅是修脚本。
