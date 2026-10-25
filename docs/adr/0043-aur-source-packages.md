# 0043 — AUR 源码包与发布后自动同步

- **状态**：proposed
- **日期**：2026-10-25
- **决策者**：YG（人类）/ Inox contributors
- **相关**：[ADR 0042｜发布制品与金丝雀渠道](./0042-release-artifacts-and-canary-channel.md)
  （单 tarball 渠道；本记录重新定义其中「渠道」与「校验和」部分）、
  [发布制品与金丝雀渠道](../book/src/development/release.md)、
  [AUR 包说明](../../packaging/aur/README.md)、
  `packaging/aur/PKGBUILD.in`、`packaging/aur/publish.sh`、
  `.github/workflows/release.yml`、`.github/workflows/ci.yml`

## 背景

ADR 0042 建立了一个渠道：CI 产出的金丝雀 tarball。它解决的是 E4「必须有 CI 产出的二进制」
这个前提，**不**解决安装方式：测试者拿到一个 tarball，手工解包、`install.sh`、重启。

对 Arch 用户，这个安装方式既不是他们习惯的路径，也不是这个发行版上可以长期使用的方式：

- tarball 里的二进制在 GitHub 的 `ubuntu-latest` 上构建，动态链接那里的 glibc 与 Qt。
  Arch 滚动更新，很快就会出现「glibc/Qt 比构建机新」或「ABI 不匹配」的安装失败。这不是
  Inox 代码的问题，但用户看到的是 Inox 装不上。
- `/usr/bin` 里手工放的文件不受 pacman 管理：升级、卸载、文件冲突、依赖记录全都没有。
  一个想要日常安装 Inox 的 Arch 用户需要的是包，不是 tar。

AUR 是 Arch 上这种包的入口。它不需要维护者自建仓库与签名，但代价是：**包由用户的机器构建**，
维护者能发布的只有配方（PKGBUILD），发布出去的配方立刻是公开可见的。

同时，ADR 0042 的「不能声称有两种分发渠道」这句话在本记录之后仍然成立，但含义要写清楚：
AUR 是**同一个 tag 的第二个安装路径**，不是第二种已验收的渠道；L02 要求的那套「全新环境里
安装/升级/回退/卸载」生命周期验证，AUR 侧一条都没做（见「后果」）。

## 决策

### 1. 两个 AUR 包名，一条配方

| 上游 tag | AUR 包 | `pkgver` | 关系 |
|----------|--------|----------|------|
| `vX.Y.Z` | `inox` | `X.Y.Z` | 稳定线 |
| `vX.Y.Z-canary.N` 及 `-alpha`/`-beta`/`-rc`/`-dev` | `inox-canary` | `X.Y.Z_canary.N` | `provides=('inox')`、`conflicts=('inox')` |

AUR 的 `pkgver` 只允许 `[0-9A-Za-z._+]`；连字符是 `pkgver-pkgrel` 的分隔符。因此预发布 tag
里的 `-` 在 `pkgver` 中写作 `_`，而 `source=` 仍指向原始 tag。渲染由
`packaging/aur/publish.sh` 完成，版本判定与 `release.yml` 的 prerelease 判定使用同一组后缀
（`-canary`/`-alpha`/`-beta`/`-rc`/`-dev`）。

稳定包名不能收预发布版本：`inox` 这个包名承诺的是稳定线，而当前的仓库只有 canary tag。
把 canary 塞进 `inox` 会让第一个稳定版看起来像「降级」，也会让用户的版本历史不再单调。
预发布单独成包，用户自己选择。

### 2. 源码包，不是 `-bin` 包

AUR 包从 tag 的源码归档构建（`build()` 跑 `cargo build --release --locked --workspace`，
默认特性），不使用 Release 的 tarball。理由与背景里的第一条相同：tarball 的二进制属于
Ubuntu 的 ABI，`-bin` 包在 Arch 上不可靠。AUR 用户预期的是「用我的系统库编译」，`-bin`
把维护者的构建环境问题变成了用户的运行期问题。

代价是包装一次要编译整个 workspace（AUR 用户的机器上也是）。这是这个决策接受的成本。

### 3. 配方是生成的，`.SRCINFO` 是 `makepkg` 生成的

`packaging/aur/PKGBUILD.in` 是唯一的手写来源；`publish.sh` 渲染占位符（包名、pkgver、
pkgrel、描述、source URL、sha256、解包目录名）。`.SRCINFO` **从不**手写，由
`makepkg --printsrcinfo` 生成。手写 `.SRCINFO` 是第二份真相，迟早与 PKGBUILD 分叉；而分叉的
`.SRCINFO` 恰恰是 AUR 服务器与用户最先看到的东西。

发布到 AUR 仓库的三个文件是 `PKGBUILD`、`.SRCINFO`、`inox.install`。生成物头部写明
「由 publish.sh 生成，手改会在下一次发布被覆盖」。

### 4. 发布顺序：Release 先，AUR 后；AUR 失败不让 Release 回滚

`release.yml` 的 `publish-aur` job `needs: package`，只在 tag push 时运行。顺序与语义是：

- **GitHub Release 先建**：AUR 失败不能把一个 CI 已经验证过的 Release 变成不可下载。发布
  通道之间不应该互相扣留；AUR 坏了是 AUR 的缺陷，不是 Release 的。
- **AUR 失败 = 这次 run 红**：AUR 停在上一个版本。修好后对同一次 run 用
  **Re-run failed jobs** 只重跑 AUR job，不会重复创建 Release。文档里写明这条恢复路径。
- **`workflow_dispatch` 不发布 AUR**：手工 dispatch 是「造个包来测」，没有 Release，也不该
  移动 AUR 包的版本。
- **推送幂等**：AUR 仓库里已经有同一份配方时不做空提交。

### 5. 推送之前必须真的构建过

配方是给 `makepkg` 用的，所以发布之前必须在 Arch 上跑一次 `makepkg`。`publish-aur` 在
`archlinux:base-devel` 容器里：

1. 下载 tag 源码归档并计算 sha256（校验和随版本变化，由发布时算出，不靠手工维护）；
2. 生成 `.SRCINFO`，并逐个用 `pacman -Si` 校验 `depends`/`makedepends` 在官方仓库中存在；
3. `makepkg --syncdeps` 完整构建，跑 PKGBUILD 的 `check()`；
4. 断言产物文件清单：14 个二进制、壁纸、三个 PAM 栈、三个 unit、sysusers 配置、LICENSE；
5. 通过之后才 `git push`。

`check()` 与 `packaging/package.sh` 对 tarball 的断言是同一条：运行构建出来的
`inox-compositor --test-input`，要求它拒绝该选项、错误信息点名 `test-input`。`--all-features`
会把注入套接字编进二进制；配方必须用默认特性，并且这一性质只能由「跑那个二进制」证明，
不能由源码树证明（ADR 0042 §4）。

CI 里另有一条 `aur (recipe)` job 在**每个 PR** 上跑：渲染两个渠道、生成 `.SRCINFO`、校验依赖名。
它不做完整构建——那是半小时级的成本，放在 release 当天、推送之前一次做完。这样配方里
「只有语法正确但依赖名打错」这类问题每天都在 CI 上暴露，而「能不能真的构建」在公开之前
一定验证过。

### 6. 凭据：AUR 账号绑定的 SSH key，存在 secret 里

AUR 没有 GitHub 那种 per-repository deploy key：认证身份就是 AUR 账号本身。因此：

- CI 用一把专用 ed25519 密钥（无口令），公钥加到 AUR 账号，私钥存仓库 secret
  `AUR_SSH_PRIVATE_KEY`；
- 提交作者默认 `Inox release bot <release@inox.invalid>`，可用 `AUR_GIT_NAME`/`AUR_GIT_EMAIL`
  覆盖；
- AUR 的 host key 由工作流 `ssh-keyscan` 获取，不在仓库里存一份会过期的副本；
- secret 缺失时 job 以明确的错误信息失败（而不是静默跳过），因为「发布成功但 AUR 没更新」
  必须是一次红灯。

该 AUR 账号需要是 `inox` / `inox-canary` 的维护者；包名尚无人占用时，首次 push 会自动建包。

### 7. 包安装了什么，不做什么

安装：14 个二进制、壁纸、三个 PAM 栈、三个 systemd unit、`greeter` 系统账户（sysusers）、
LICENSE。

**不启用开机链**：`inox.install` 只打印步骤。把机器从已有 display manager 切到 Inox 是
管理员决定，与 ADR 0042 / `packaging/install.sh --enable` 的立场一致（安装默认不动开机链）。

`greeter` 账户的定义在 `packaging/sysusers/inox.conf`，与 `install.sh` 的
`useradd --system` 参数必须一致（无登录 shell、home 在 `/var/lib/inox-greeter`）。两条安装
路径产生不同账户的话，差异只会在其中一条路径上暴露。

## 为什么不是其它方案

| 方案 | 否决理由 |
|------|----------|
| `-bin` 包，直接装 Release tarball | tarball 的二进制链接 Ubuntu 的 glibc/Qt，在 Arch 上不可靠；`-bin` 把维护者构建环境的问题转嫁给用户 |
| 只发 `inox`，canary 也算进去 | 稳定包名会被预发布占据，第一个稳定版看起来像降级；用户无法选择只要稳定线 |
| 发 `inox-git`（VCS 包） | `-git` 跟踪分支头，每次构建的提交都不同，与「发布 tag」的语义无关；本决策要的是「这个 tag 的配方」 |
| 手工维护 AUR 仓库 | 与 ADR 0042 §1「发布物只由 CI 产出」冲突；版本与校验和会在某个忙碌的发布日忘记更新，而 AUR 是公开页面 |
| 不构建就推送（只做静态校验） | 配方第一次真构建就发生在 AUR 用户机器上；Arch 的依赖名、Qt 工具、链接器差异都不是静态检查能覆盖的。用户装不上时才发现的费用远高于发布当天多跑一次构建 |
| 先发 AUR 再建 GitHub Release | AUR 失败会扣住一个已经验证过的 Release；两个通道互相阻塞没有收益 |
| 维护自建 pacman 仓库 | 需要仓库托管、签名、镜像与升级策略，工作量远大于「把配方推给 AUR」；那属于 L02 的完全体，不是起点 |

## 后果

### 正面

- **Arch 用户第一次有一个 pacman 管理的安装路径**：升级、卸载、文件归属、依赖记录都交给
  pacman；不再要求用户手工往 `/usr/bin` 放文件。
- **配方在每个 PR 上被渲染和检查**：依赖名、`.SRCINFO`、包名映射、`pkgver` 规则不再只在发布日
  才被执行。ADR 0042 §3 的同一条原则（只在发布日执行的脚本等于没被验证过）在这里也适用。
- **校验和与版本不会漂移**：sha256 由发布时下载的归档算出，`.SRCINFO` 由 makepkg 生成，没有
  手工维护的第二份数字。
- **`test-input` 后门在 AUR 路径上也有断言**：包里的合成器必须拒绝 `--test-input`，与 tarball
  的断言同源。
- **失败可恢复**：Release 与 AUR 解耦，AUR 失败可以只重跑一个 job。

### 负面 / 限制

- **AUR 不是 L02 的证据**。验收规范 L02 要求「至少两种已声明渠道在全新目标环境由 CI 安装、
  启动、注销、升级、回退、卸载」。AUR 配方会被构建，但安装后的生命周期没有被 CI 验证过
  （需要真实的 pacman 环境与 root 操作），所以 L02 仍然不满足。这一点写进了
  `release.md` 与 `packaging/aur/README.md`。
- **AUR 包不能作为 E4 证据**。E4 要求「CI 产出的二进制」；AUR 包由用户本机构建，比特与 CI
  验证过的 tarball 不同。E4 仍必须用 Release tarball。
- **构建成本**：canary 发布从「构建 Ubuntu tarball」变成「再在 Arch 容器里构建一遍整个
  workspace」。发布当天多约 20–40 分钟。失败时 AUR 落后于 Release，需要人工重跑。
- **维护面增加**：两个 AUR 包、一个容器 job、一个 release job、一把长期密钥。依赖清单
  （Qt、Smithay 的系统库）需要在 Arch 包名体系里正确，这由 `pacman -Si` 检查与 release 构建
  覆盖，但新增系统依赖时多一处要更新。
- **校验和钉的是 GitHub 生成的 tag 归档**：`sha256` 在发布时从 `github.com/.../archive/refs/tags/v*`
  下载算出。GitHub 若改变归档的生成方式，同一 tag 的字节会变，用户校验会失败；那时只能发一个
  `pkgrel` 递增的配方并重算校验和。这是 AUR 源码包常见的边界，不是这套流程引入的新风险。
- **AUR 包只声明 `x86_64`**：没有 aarch64 的构建证据，也没有对应 tag 的 Arch 构建记录
  （`BUILD-INFO` 是 tarball 的概念，AUR 包没有）。`pkgver` 只表示上游版本。
- **首次发布依赖 AUR 账号**：包名与密钥都属于一个具体的人类账号；账号失效则自动发布中断
  （红灯，不会静默）。

### 取代条件

出现以下任一情况时，本决策应被重新评估或取代：

- **L02 达成**：AUR（或任何第二渠道）在全新环境里通过 CI 的安装/升级/回退/卸载验证，
  「AUR 不算渠道」的限制随之失效，需要新的发布 ADR 描述已验证的渠道矩阵；
- **AUR 开始接受 CI 产出的二进制**（例如出现可验证的 `-bin` 构建链路，或上游提供 Arch 兼容的
  静态/固定基线二进制）：`-bin` 与源码包的取舍需要重写；
- **发布线出现真正的稳定版策略**（例如维护多个 release branch）：两个包名的映射与版本单调性
  需要重新定义；
- **AUR 的认证或 CI 集成方式变化**（deploy key 模型、AUR 托管政策）：第 6 条需要更新；
- **容器化构建被禁止或 runner 无法运行 `archlinux:base-devel`**：第 5 条的「发布前构建」必须
  换一个等价的验证手段，否则应回退到「不自动发布 AUR」。
