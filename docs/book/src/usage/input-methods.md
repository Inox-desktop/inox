# 输入法

Inox 提前接入 `text-input-v3`、`input-method-v2` 和 `virtual-keyboard-v1`，
提供预编辑、提交文本、删除周边文本、光标定位候选窗，以及焦点和锁屏隔离。
协议管理器当前广告接口版本 1；不广告旧的 `text-input-v1/v2`。
实现与验收边界见 [ADR 0023](https://github.com/Inox-desktop/inox/blob/main/docs/adr/0023-input-method-support.md)。

## 启动 Fcitx5

先通过发行版安装 Fcitx5、所需语言引擎与字体，例如拼音或 Rime。
Inox 不内置引擎、不下载词库、不默认启动任何输入法。可在系统设置的「输入法」页保存引擎程序和逐项参数，
例如第一项 `/usr/bin/fcitx5`；空列表禁用自动启动。设置在下次物理会话启动时读取。
显式启动参数优先，headless 测试仍只使用命令行。参见[系统设置](settings.md)。
在已有 D-Bus 会话中使用 CI 产物：

```bash
inox-session --input-method /usr/bin/fcitx5
```

单独运行合成器时也可传相同参数：

```bash
inox-compositor --input-method /usr/bin/fcitx5
```

额外参数逐项传递，例如 `--input-method-arg=--verbose=debug`，不经 shell 解释。
不要传入使引擎后台化的参数，也不要同时从其他自启动入口再启动一个引擎。
合成器通过专用 Wayland 连接授予输入法权限；从普通终端另行启动的引擎没有
`input-method-v2` 权限。引擎退出后最多重启三次，每次重新授予独立连接；
关闭会话时终止并回收引擎。停用时同时清空设置中的输入法列表并移除 `--input-method`。
降级到不认识新字段的版本前，删除 `compositor.toml` 的 `[input] input_method`。

用引擎自己的配置工具添加所需语言、设置切换快捷键和拼音/词库偏好。
Fcitx5 中的键盘组布局应与 Inox 的键盘配置一致；不要在 Inox 里复制一套引擎快捷键。

## 应用兼容

| 应用 | 输入路径 |
|------|----------|
| Iced 0.14 的普通文本框、支持 v3 的原生 Wayland 应用 | 通过 `text-input-v3`；无需额外 Inox 客户端代码 |
| GTK 3/4 的 Wayland 应用 | 使用 GTK 内置 Wayland 输入上下文；按应用检查已有 `GTK_IM_MODULE` 覆盖 |
| 较旧的 Qt 或不支持 v3 的应用 | 安装发行版的 Fcitx5 工具包模块，并按应用设置 `QT_IM_MODULE=fcitx` |
| XWayland 应用 | 使用引擎的 XIM/工具包模块，设置 `XMODIFIERS=@im=fcitx`，并确保引擎能连接对应 `DISPLAY` |

这些是上游提供的接线方式，**不等于上述应用均已在 Inox 验收通过**。
具体工具包版本及变量选择参见 [Fcitx5 Wayland 文档](https://fcitx-im.org/wiki/Using_Fcitx_5_on_Wayland)。
不要全局强制所有应用使用同一个工具包模块，否则可能绕过原生文本输入路径。

IBus 可通过其工具包模块/XIM 使用；本次未实现 GNOME 的 IBus D-Bus 合成器桥，
不承诺 `ibus-daemon` 能直接充当 `input-method-v2` 服务。也未提供输入法选择面板、
托盘状态或引擎安装界面。

## 验收状态

CI 中的真实 Wayland 客户端用例覆盖文本事务、过期序号、多个文本对象、
焦点、禁用、断连、权限拒绝、锁屏隔离、候选窗的 1.5 倍像素与鼠标交互。
进程测试验证私有连接、重启次数与退出回收；`Fcitx5 Chinese input on Inox`
任务另安装真实拼音引擎，验证 `nihao` 的预编辑及 `你好` 的提交。
新增用例的运行结果仍须以对应提交的 CI run 为准。

GTK/Qt、浏览器、XWayland、日文/韩文引擎和真实多屏的组合尚待逐版本验收。
锁定期间外部输入法不能接收按键、读取锁屏文本、注入虚拟按键或显示候选窗；
锁屏密码输入仍受当前锁屏组件交付范围限制，不能由输入法协议测试推断认证已完成。
