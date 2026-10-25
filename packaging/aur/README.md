# AUR 包

这个目录把一个 Inox 的 `v*` tag 变成 AUR 上的**源码包配方**。它是发布流程的一部分，
不是独立渠道：GitHub Release 建好之后，`.github/workflows/release.yml` 的 `publish-aur`
job 用这里的脚本烘焙 PKGBUILD、在 Arch 容器里真的构建一遍，构建通过才推送。
决策与边界见 [ADR 0043](../../docs/adr/0043-aur-source-packages.md)。

## 两个包，一条配方

| tag | AUR 包名 | pkgver | 关系 |
|-----|----------|--------|------|
| `vX.Y.Z` | `inox` | `X.Y.Z` | 稳定线 |
| `vX.Y.Z-canary.N`（以及 `-alpha` / `-beta` / `-rc` / `-dev`） | `inox-canary` | `X.Y.Z_canary.N` | `provides=('inox')`、`conflicts=('inox')` |

AUR 的 `pkgver` 不允许连字符（连字符是 `pkgver-pkgrel` 的分隔符），所以预发布 tag 里的
`-` 在 pkgver 里写成 `_`；`source=` 仍然指向原始 tag。两个包互斥，装一个会替换另一个。

**为什么不是 `-bin`**：GitHub Release 里的 tarball 是 `ubuntu-latest` 上构建的，动态链接
Ubuntu 的 glibc 与 Qt；装到 Arch 上不是「换个路径」的问题。AUR 包必须从源码构建，链接
用户自己的系统库。

**为什么不把 canary 发进 `inox`**：`inox` 这个包名承诺的是稳定线。预发布另开一个包，
用户自己选；这也让 `inox` 的版本历史保持单调。

## 文件

| 文件 | 作用 |
|------|------|
| `PKGBUILD.in` | 配方模板。占位符由 `publish.sh` 渲染；**不要**在 AUR 仓库里手改生成物 |
| `inox.install` | pacman 的 `install=` 钩子。只打印如何切换开机链，**不启用、不启动**任何东西 |
| `publish.sh` | 唯一的实现：渲染、生成 `.SRCINFO`、校验依赖名、`makepkg` 构建与产物断言、幂等推送 |
| `../sysusers/inox.conf` | `greeter` 系统账户的定义，由 pacman 的 `systemd-sysusers` 钩子创建 |

`PKGBUILD.in` 里的 `check()` 是承载性质的一步，不是装饰：`inox-compositor` 的
`test-input` feature 会编译一个接受注入键盘/指针事件的套接字。配方用默认特性构建，
`check()` 真的运行构建出来的合成器、要求它拒绝 `--test-input` 并在错误里点名该选项——
与 `packaging/package.sh` 对发布 tarball 做的是同一条断言（ADR 0042 §4）。

## 自动发布

`release.yml` 的顺序是：`package` job 构建 tarball 并创建 GitHub Release，然后
`publish-aur` job：

1. 在 `archlinux:base-devel` 容器里下载 tag 源码包、算 sha256；
2. 渲染配方、`makepkg --printsrcinfo` 生成 `.SRCINFO`、校验每个 `depends`/`makedepends`
   都能在官方仓库里解析；
3. `makepkg --syncdeps` 完整构建整个 workspace，跑 `check()`，并断言产物的文件清单
   （14 个二进制、壁纸、三个 PAM 栈、三个 unit、sysusers 配置、LICENSE）；
4. 推送 `PKGBUILD`、`.SRCINFO`、`inox.install` 到 `aur.archlinux.org/<包名>.git`。

**失败语义**：GitHub Release 先于 AUR 发布，所以 AUR 失败不会让已经验证过的 Release 失效；
它只会让这次 run 变红，AUR 停在上一个版本。修好之后在 Actions 里对这次 run 点
**Re-run failed jobs** 即可只重跑 AUR job（`package` job 不会重复创建 Release）。
推送是幂等的：同一份配方再推一次不会产生空提交。

如果失败原因是脚本本身而不是网络/凭据，重跑同一个 run 会再次执行旧代码：此时在一份包含修复的
checkout 上手动跑 `packaging/aur/publish.sh --version <V> --build --push`。配方的源码来自 tag
归档，与这次手动执行所用的提交无关，所以补发不会改变已发布 Release 的内容。

只有 tag push 会走这条路。`workflow_dispatch` 是「造个包来测」，不是发布，不碰 AUR。

## 凭据

AUR 只认**绑定在 AUR 账号上的 SSH key**，没有 GitHub 那种 per-repo deploy key。所以：

1. 生成一把只给 CI 用的密钥（不要带口令）：

   ```bash
   ssh-keygen -t ed25519 -C 'inox-release-ci' -f ~/.ssh/inox-aur-ci -N ''
   ```

2. 把 `~/.ssh/inox-aur-ci.pub` 的内容加到 AUR 账号的
   [SSH Public Key](https://aur.archlinux.org/account) 页。
3. 把**私钥**存成仓库 secret：

   ```bash
   gh secret set AUR_SSH_PRIVATE_KEY -R Inox-desktop/inox < ~/.ssh/inox-aur-ci
   ```

4. 该 AUR 账号必须是 `inox` / `inox-canary` 的维护者（或包名尚无人占用，首次 push
   会自动建包）。

可选变量：`AUR_GIT_NAME`、`AUR_GIT_EMAIL` 控制 AUR 提交作者，默认
`Inox release bot <release@inox.invalid>`。工作流用 `ssh-keyscan` 获取 AUR 的 host key，
不在仓库里存一份会过期的副本。

## 手动发布与本地演练

```bash
# 完整流程：下载 tag 源码、构建验证、推送
packaging/aur/publish.sh --version 0.1.1 --build --push

# 只渲染和校验，不构建、不推送（CI 的 aur job 就是这条）
packaging/aur/publish.sh --version 0.1.1 --sha256 SKIP

# 用当前 worktree 的 HEAD 造源码包来演练，不联网、不推 AUR
packaging/aur/publish.sh --version 0.1.1 --local-source . --build --no-syncdeps
```

`--local-source` 与 `--sha256 SKIP` 是演练/校验模式，`--push` 会拒绝它们：发布出去的
配方必须钉住真实远端归档与真实校验和。`--pkgrel N` 用于同一版本重发配方（AUR 侧
`pkgrel` 加一，`pkgver` 不变）。`--keep` 保留 scratch 目录，方便看构建日志。

需要 Arch（`makepkg`）才能渲染之外的部分：`.SRCINFO` 由 `makepkg` 生成，构建也在
`makepkg` 里跑。非 Arch 机器上只能读脚本，不能执行。

## 包装了什么

| 路径 | 内容 |
|------|------|
| `/usr/bin/` | 14 个交付二进制（`packaging/package.sh` 的 `SHIPPED_BINARIES` 同一份清单） |
| `/usr/share/inox/wallpapers/` | 内置壁纸 |
| `/etc/pam.d/inox-auth`、`inox-auth-session`、`inox-greeter` | 三个 PAM 栈 |
| `/usr/lib/systemd/system/inox-{auth,greeter@,login-fallback}.service` | 三个 unit |
| `/usr/lib/sysusers.d/inox.conf` | `greeter` 系统账户；已存在时（例如 greetd 建过）sysusers 不改动它，Inox 直接复用，与 `install.sh` 一致 |
| `/usr/share/licenses/inox/LICENSE` | 许可证 |

**不启用开机链**：包安装完只打印步骤（见 `inox.install`）。把 Inox 设为登录界面要自己
`ln -sf /usr/lib/systemd/system/inox-greeter@1.service /etc/systemd/system/display-manager.service`。
`graphical.target` 的 `Wants=display-manager.service` 会拉起它；先停用其他 display manager。

**与 tarball 安装互斥**：`packaging/install.sh` 装的 `/usr/bin/inox-*` 不受 pacman 管理，
pacman 会以「exists in filesystem」拒绝安装本包。从 tarball 装的机器先用
`sudo packaging/uninstall.sh --remove-binaries` 卸掉，再 `pacman -S`。

## 这个包不能声明什么

- **不是 L02 的证据。** AUR 只是同一个 tag 的第二个安装路径，安装 → 启动 → 注销 → 升级 →
  回退 → 卸载的生命周期没有被 CI 在全新环境里验证过，L02 仍不满足。
- **AUR 构建的二进制不能当 E4 证据。** 验收规范要求 E4 使用 **CI 产出的二进制**；AUR 包
  由用户本机构建，代码相同但比特不是 CI 验证过的那一份。E4 请用 Release tarball。
- **不声明任何硬件支持，也不声明「可日常使用」。** 与 Release 页的边界一致。
- **只有 `x86_64`。** Release 流水线只构建并验证这个架构；没有 aarch64 的构建证据。

CI 覆盖见 [持续集成](../../docs/book/src/development/ci.md)：每个 PR 跑 `aur (recipe)` job
（渲染两个渠道、生成 `.SRCINFO`、检查依赖名），完整的 `makepkg` 构建在 release 当天、
推送之前完成。发布制品与渠道的完整说明见
[发布制品与金丝雀渠道](../../docs/book/src/development/release.md)。
