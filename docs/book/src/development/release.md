# 发布制品与金丝雀渠道

> **本页只讲「制品怎么产出、怎么拿到、怎么用」，不声称任何验收项已通过。**
> [验收规范](https://github.com/Inox-desktop/inox/blob/main/docs/quality/acceptance.md) 的发布门禁
> L01、L02、L03 **全部未满足**；E4 真机验收 **从未执行**。金丝雀制品是**开发者预览**，
> 不是发布合格物。决策依据见 [ADR 0042](../architecture/decisions.md)（`proposed`）。

## 这份制品是什么

一个 tarball，由 **CI** 用**默认特性**构建，用来解决一件具体的事：E4 真机验收按验收规范必须
「使用 CI 产出的二进制」，而在此之前仓库里根本没有这样的东西（CI 的 `build` job 构建完就把产物
丢掉了，仓库也没有任何 tag 或 release）。

**它不是**「发布物」的同义词，也不是任何人本地 `cargo build` 的产物。任何交给测试者的 tarball
都必须来自 CI 的一次运行，并能在 `BUILD-INFO` 里查到对应的提交 SHA。

## 从哪拿到它

有两条路径，都只由 CI 产出。

### 1. PR 或 `main` 上的 CI artifact

`ci.yml` 的 `package (canary tarball)` job 在**每个 PR 与每次 push 到 `main`** 都跑一遍：

1. Actions → **CI** → 选一次运行；
2. 在 job 列表里找 **`package (canary tarball)`**；
3. 页面底部的 **Artifacts** 下载 `inox-canary-tarball`。

artifact 保留 30 天。这条路径的用途是「我想现在试这个 PR 的构建」，它**不**产生任何 release，
也不需要版本号。

`.github/workflows/ci.yml` 里的 `package.sh` 执行与 `--help` 检查也在这个 job 里：打包脚本本身
是仓库里唯一没有任何别的 job 编译或测试的部分，语法错误必须在这里暴露，而不是等到有人机器处于
半安装状态时才暴露。

### 2. `Release` 工作流的预发布资产

`.github/workflows/release.yml` 由 `v*` tag 触发，把同一个 tarball 挂到 GitHub Release 上，
附件是 `inox-<version>-<target>.tar.gz` 与同名 `.sha256`。

也可以在 Actions → **Release** → **Run workflow** 手工触发（输入 `version`，留空则用
`Cargo.toml` 的版本加 `-dev.<sha12>`）：这条路**只上传 workflow artifact，不创建 Release**。
它的用途是「从某个提交造一个能测的包，但还没决定版本号」。

同一个 tag 还会发布成 AUR 源码包（见下面的「AUR：Arch 上的源码包」）。它是一条**安装路径**，不是
第二条被验收过的渠道：AUR 包由用户本机构建，E4 仍然只能用 CI 产出的 tarball。

### 拿到之后先校验

```bash
sha256sum --check inox-<version>-<target>.tar.gz.sha256
```

校验和不匹配就不要装——它说明下载坏了，或者文件不是它声称的那一个。

## 里面有什么

解包后是**单一顶层目录** `inox-<version>-<target>/`：

| 内容 | 是什么 |
|------|--------|
| `bin/` | 14 个交付二进制，默认特性构建、已 strip |
| `data/wallpapers/` | 内置壁纸；`install.sh --bin-dir` 会把它们装到 `/usr/share/inox/wallpapers` |
| `packaging/` | 安装面：`install.sh`、`uninstall.sh`、`preflight.sh`、`verify-login.sh`、`rehearse-headless.sh`、试运行与回退脚本、`README.md`、`pam.d/`、`systemd/`；以及 `package.sh` 本身——装一个系统用不到它，但读它就能知道这个包是怎么做出来的，不必为此克隆仓库 |
| `BUILD-INFO` | 构建元数据 |
| `SHA256SUMS` | 上述每个文件的校验和（不含自身） |

14 个二进制分两组，分法不是随意的：

- **必需 8 个**（`inox-auth`、`inox-host`、`inox-compositor`、`inox-greet`、`inox-session`、
  `inox-shell`、`inox-panel`、`inox-lock`）：缺任何一个，正确的口令会变成「会话立刻结束」，
  在登录界面上的症状与口令被拒完全相同。
- **可选 6 个**（`inox-wallpaper`、`inox-notifications`、`inox-idle`、`inox-xwayland`、
  `inox-launcher`、`inox-settings`）：会话缺它们也能活着，桌面只是少一块。

`inox-integration`（测试聚合器）与 `inox-qml-prototype`（cxx-qt 绑定原型）也有二进制，但**不随包
发布**：一个是用例装置，一个是证明绑定可用的原型。

### `BUILD-INFO`

字段固定：`name`、`version`、`stage`、`target`、`commit`、`commit-short`、`commit-date`、
`worktree`、`features`、`profile`、`stripped`、`rustc`、`cargo`、`built-at`、`builder`、`license`。

两条最有用：

- `commit` 是 40 位 SHA。真机验收记录里写它，就能唯一定位被测的源码状态。
- `features: default`。这是**发布物的核心性质**：`--all-features` 会编译进合成器的
  `test-input` feature，那是一个接受注入键盘与指针事件的套接字，不是能交给测试者的东西。

`packaging/package.sh` 不是靠约定来保证这件事的：它在打包前**真的运行**暂存出来的
`bin/inox-compositor --test-input <path>`，要求该选项被拒绝、且错误信息点名 `test-input`。
被接受（或进程挂住不返回）就让打包失败。这条断言没法用单元测试替代——被测性质属于**产物**，
不属于源码树。

`worktree: dirty` 意味着这个包不对应任何一个提交。看到它就别拿这份结果去登记验收。

## 怎么装

```bash
tar -xzf inox-<version>-<target>.tar.gz
cd inox-<version>-<target>

# 先看看这一份是什么
cat BUILD-INFO
sha256sum --check SHA256SUMS

# 装二进制 + 安装面
sudo ./packaging/install.sh --bin-dir ./bin

# 预检：只读，不改任何东西，有 FAIL 就别切屏幕
sudo ./packaging/preflight.sh
```

`install.sh --bin-dir ./bin` 会把 14 个二进制装到 `/usr/bin`、壁纸装到
`/usr/share/inox/wallpapers`，再装三个 PAM 栈、三个 systemd 单元、`greeter` 账户与 VT3 救援
console。它**默认不动开机链**：`display-manager.service` 保持原样，只有 `--enable` 才把开机链交给
`inox-greeter@1.service`。

预检之后是试运行（VT4，同样不动开机链）：

```bash
sudo packaging/try-inox-greeter.sh      # 起 inox-greeter@4.service 并切到 VT4
packaging/verify-login.sh               # 登录之后跑，检查 logind/PAM/VT/XDG_* 等不变量
sudo packaging/try-rollback.sh          # 结束试运行，把画面切回原来的登录界面
```

完整步骤、每一步验证什么、出问题从哪个 VT 进去修，见[登录与开机链](../usage/login.md)。
本页不重复那份内容：这里只负责「制品从哪来」。

**卸载**：`sudo packaging/uninstall.sh` 停用并移除单元与 PAM 栈，但**保留** `/usr/bin` 里的
二进制与壁纸；加 `--remove-binaries` 才连它们一起删。默认不删是有意的——那两个位置也可能是
本地构建装的。

## AUR：Arch 上的源码包

`Release` 工作流在 GitHub Release 创建之后，还会把同一个 tag 发布成 AUR 源码包
（决策见 [ADR 0043](../architecture/decisions.md)，实现见
[packaging/aur/README.md](https://github.com/Inox-desktop/inox/blob/main/packaging/aur/README.md)）。
它与上面的 tarball 是两条安装路径，不是两种已验证的渠道：包由用户本机编译，不是 CI 产出的
二进制。

| tag | AUR 包 | `pkgver` |
|-----|--------|----------|
| `vX.Y.Z` | `inox` | `X.Y.Z` |
| `vX.Y.Z-canary.N`（及 `-alpha`/`-beta`/`-rc`/`-dev`） | `inox-canary` | `X.Y.Z_canary.N` |

AUR 的 `pkgver` 不允许连字符，所以预发布 tag 的 `-` 在版本里写成 `_`；`source=` 仍指向原始 tag。
两个包互斥（`inox-canary` 提供/冲突 `inox`），装一个会替换另一个。

```bash
# 任选一个 AUR helper，或手工构建
yay -S inox                                  # 稳定线
yay -S inox-canary                           # 预发布
# 手工：
git clone https://aur.archlinux.org/inox.git && cd inox && makepkg -si
```

包安装 14 个二进制、壁纸、三个 PAM 栈、三个 systemd unit 与 `greeter` 系统账户（sysusers）；
**默认不启用开机链**，安装后的提示给出切换命令。它与 tarball 安装互斥：`install.sh` 放的
`/usr/bin/inox-*` 不归 pacman 管，pacman 会拒绝覆盖，先
`sudo packaging/uninstall.sh --remove-binaries` 再装包。

发布前，工作流在 `archlinux:base-devel` 容器里用 `makepkg` **完整构建**这个配方，跑 PKGBUILD 的
`check()`（与 tarball 同一条断言：构建出来的 `inox-compositor` 必须拒绝 `--test-input`），并断言
产物的文件清单；构建通过才推送。这一步失败不会回滚已经建好的 GitHub Release——对那次 run 点
**Re-run failed jobs** 只重跑 AUR job。

## 怎么切一个金丝雀

前提：**要打 tag 的提交必须已经在 CI 上全绿**。tag 是对某几份比特的承诺，不是对「应该差不多」的
承诺。

### 1. 需要时先 bump 版本

版本在根 `Cargo.toml` 的 `[workspace.package] version`。语义沿用已记录的 0.x 规则：
**minor 是不兼容变更，patch 是兼容变更**。只切一个构建、内容不变的话不需要 bump。

`Release` 工作流会检查 tag 与清单一致：发布版本必须以工作区版本开头。`0.1.1-canary.2` 可以，
`0.2.0-canary.1` 会在清单还是 `0.1.1` 时被**直接拒绝**——否则 `BUILD-INFO`、tarball 文件名与
release 标题会一起描述一个没人能复现的构建。

### 2. 打 annotated tag 并推送

```bash
# tag 信息写清这次切的是什么
git tag -a v0.1.1-canary.2 -m "canary: 0.1.1 + 发布制品流水线"

git push origin v0.1.1-canary.2
```

tag 必须是 **annotated**（`-a`）的：它要携带说明，而轻量 tag 只有一个提交指针。

### 3. CI 发布预发布

推送 tag 会触发 `Release` 工作流，它：

1. 从 tag 名解析版本（去掉 `v` 前缀）；
2. 要求版本以工作区版本开头；
3. 用 `packaging/package.sh --version <V> --output-dir target/dist` 打包；
4. 校验 `.sha256`、把 `BUILD-INFO` 从压缩包里读出来，并断言其中的 `commit` **就是**正在发布的那个提交；
5. 创建 GitHub Release，附件是 tarball 与 `.sha256`。版本里含 `-canary`、`-alpha`、`-beta`、
   `-rc`、`-dev` 时一律标为 **prerelease**；
6. 在 Arch 容器里构建 AUR 源码包（`makepkg` + `--test-input` 断言 + 产物清单断言），通过后推送到
   `aur.archlinux.org`（稳定 tag → `inox`，预发布 → `inox-canary`）。这一步只在 tag push 时运行；
   手工 dispatch 不发布 AUR。

release 说明正文本身写着这份制品不是什么（不声明硬件支持、不是稳定版、没有 SBOM、没有经过生命周期
验证的第二条渠道）。那句话与下面「这份制品不能声明什么」是同一份约束。

### 想造包但不想切版本

手工触发 `Release` 工作流，`version` 留空即可：产物只是 workflow artifact，**不会**创建 Release。

### 本地能不能跑 `packaging/package.sh`

能。它在本地是合法的调试工具：`--bin-dir DIR` 从已有二进制暂存（不重新构建）、`--version V`
与 `--target TRIPLE` 覆盖标注、`--output-dir DIR` 改输出位置（默认 `target/dist/`）、
`--no-verify` 跳过 `--test-input` 断言（**只**用于调试这个脚本本身，CI 从不传它）。

但本地跑出来的 tarball **不是发布物**：它没有对应 CI 运行的证据，也不能作为真机验收的对象。

## 用这个包做 E4 真机验收

这份 tarball 就是验收规范要求的那份「CI 产出的二进制」。操作步骤、每个场景要观察什么、要保留哪些
证据，全部以
[L2 真实后端硬件验收](https://github.com/Inox-desktop/inox/blob/main/docs/quality/l2-hardware-validation.md)
为准，本页不另立一份标准。要点是：

1. **准备**：在具有活动 VT 的本地用户会话里运行，具备 logind 或 seatd 设备授权。不要把另一个正
   占用 DRM master 的桌面当宿主编译器那样嵌套启动它。
2. **启动**：真实会话用 `inox-compositor --backend drm --socket inox-hardware`。`drm` 显式要求
   物理后端，**永不回落** headless；配置错了应该得到非零退出，而不是一次看起来成功的 headless 运行。
3. **记录**：日期、验收者、设备矩阵、**发布物身份**、操作次数、实际结果、原始证据链接。

「发布物身份」这一栏此前无处可填，现在填它：

```text
stage:    inox-0.1.1-x86_64-unknown-linux-gnu   （BUILD-INFO 的 stage 字段）
commit:   <BUILD-INFO 的 commit，40 位>
tarball:  <tarball 的 sha256>
```

失败项另建缺陷并记录可复现步骤。**没有执行或缺少证据一律保持未通过，不能用「CI 全绿」代填。**

## 这份制品不能声明什么

这一节是硬边界。任何 PR、文档、release 说明或对外描述，只要越过其中一条，就是把未取得的证据
当成已有能力。

- **不能声称真机登录可用。** 安装链、PAM 栈与登录界面都已接线，但**没有任何真机登录证据**。
  验收项 S06–S11 中除 S07a 以外的条目全部未执行。
- **不能声称可以当日常桌面用。** 物理 DRM/KMS + libinput 后端**已实现**，但 E4 真机验收
  **从未执行**，所以没有任何结论支撑「日常可用」。
- **不能声称支持任何硬件配置。** 没有 GPU、内核、显示器或输入设备组合因为「金丝雀能启动」而
  进入支持清单。支持声明只由验收项 H01–H03 建立。
- **不能声称有两种已验证的分发渠道。** 现在有两条安装路径：CI 产出的 tarball，和 AUR 上的源码包。
  但 AUR 包由用户本机编译，安装 → 启动 → 注销 → 升级 → 回退 → 卸载的生命周期没有在 CI 里验证过，
  所以 L02 仍然不满足；AUR 包也不能当作 E4 要求的「CI 产出的二进制」。
- **不能声称有 SBOM，或超出 tarball 自带 `SHA256SUMS` 的校验。** `cargo-deny` 只看 Cargo 依赖图，
  看不到实际链接的 Qt/Mesa 共享库；L01 要求的「SBOM 覆盖运行时系统组件」不满足。
- **不能称之为稳定发布。** 金丝雀是预发布。L03（人类发布审查、P0/P1 清零）完全没有被触及。
- **不能声称可移植到更旧的发行版。** 二进制是动态链接的，glibc 下限由构建 runner 决定
  （GitHub 的 `ubuntu-latest`，当前为 Ubuntu 24.04）。在没有声明基线之前，把它装到更旧的发行版上
  会失败，而这个失败与 Inox 的代码无关。

配套地：`docs/quality/acceptance.md` 的 L01、L02、L03 全部未满足，本页不改变这个事实。

## 相关

- 决策：[架构决策记录](../architecture/decisions.md)（ADR 0042 与 ADR 0043，状态均为 `proposed`）
- AUR 包（配方、凭据、边界）：
  [packaging/aur/README.md](https://github.com/Inox-desktop/inox/blob/main/packaging/aur/README.md)
- 安装与真机步骤：[登录与开机链](../usage/login.md)
- 构建命令与前置条件：[构建与运行](../usage/building.md)
- 安装面（PAM 栈、unit、脚本清单）：
  [packaging/README.md](https://github.com/Inox-desktop/inox/blob/main/packaging/README.md)
- 硬件验收记录表：
  [L2 真实后端硬件验收](https://github.com/Inox-desktop/inox/blob/main/docs/quality/l2-hardware-validation.md)
- 验收语义、证据等级与发布门禁：
  [验收规范](https://github.com/Inox-desktop/inox/blob/main/docs/quality/acceptance.md)
