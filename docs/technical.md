# 技术说明与验证记录

[返回 README](../README.md) · [使用说明](usage.md)

## 技术栈与界面

项目使用 Rust（Edition 2024），原生界面基于 GPUI / gpui-component；HTTP 请求使用 reqwest 与 rustls，JSON 数据使用 serde，历史记录和分页缓存使用 rusqlite / SQLite（随程序构建）。图片处理使用 image，图片剪贴板使用 arboard。

界面采用板块 → 串列表 → 阅读 → 发串 / 回复的四栏布局，分隔线可拖动，各栏保留最小宽度。使用客户端标题栏（CSD），支持拖动、双击最大化和窗口控制；打开串和编辑栏带有过渡动画。中文输入、选择和剪贴板由 gpui-component 输入组件提供。

Linux 文件对话框使用 XDG Desktop Portal（ashpd），其他平台使用 rfd。Linux 图片剪贴板优先使用 Wayland data-control；GNOME 等不支持该协议的环境需要 XWayland。

## 配置与凭据

Linux 配置目录默认为 `~/.config/rdnmb/`，Windows 为 `%APPDATA%\rdnmb\`；可通过绝对路径的 `XDG_CONFIG_HOME` 覆盖基础目录。`settings.json` 保存设置，`appearance.json` 保存主题，`history.sqlite3` 保存历史与分页快照，`media-cache/` 保存已加载图片。

未保存界面设置时，从项目目录启动会读取 `.env`。优先使用 `RDNMB_COOKIE`，兼容旧变量 `NMBXD_COOKIE`、`userhash` 或现有文件使用的 `usrhash`；值为原始百分号编码饼干，也支持 `userhash=…` 形式。保存的界面设置优先于环境变量和 `.env`，选择游客并保存后不会重新采用旧值。

Cookie 以本地明文保存，不使用密钥环；Linux 设置文件权限为 `0600`，保存使用临时文件原子替换。`.env` 已忽略，发行包不包含真实凭据。数据库不保存 Cookie。旧 `nmbxd` 目录的设置会自动迁移，具体行为见[使用说明](usage.md#设置)。

## 依赖补丁与锁文件

依赖固定为 GPUI 0.2.2 / gpui-component 0.5.1。`vendor/` 保留两个小补丁：cosmic-text 的完整 emoji 字符簇字体选择，以及 gpui-component 最大化 / 平铺时的窗口缩放命中区域（使用当前 viewport 而非还原尺寸）；详情见各目录 `NMBXD-PATCH.md`。保留 Cargo.lock：旧 xattr 0.2 的 Linux ENOATTR 引用与最新 libc 不兼容，因此锁文件使用 libc 0.2.186。

补丁说明：[cosmic-text](../vendor/cosmic-text/NMBXD-PATCH.md)、[gpui-component](../vendor/gpui-component/NMBXD-PATCH.md)。

API 参考 [TransparentLC/xdcmd 的文档](https://github.com/TransparentLC/xdcmd/wiki/自己整理的-X-岛匿名版-API-文档)，界面基于 [GPUI](https://gpui.rs/)。

## 当前限制

图片支持 JPEG / PNG / GIF / WebP，动图暂显示首帧；尚未实现时间线和跨重启草稿保存。HTML 转为文本，不执行页面脚本，也不自动加载任意 HTML 外链图片。

不会自动抓取整串，只在滚动、手动操作或每次浏览的一次末尾检查时请求相应页面。已加载回复使用普通滚动容器，超长串的列表虚拟化仍待实现。引用树只组织已加载子集，不追踪抓取未知引用。网络、缓存、上传限制及失败处理详见[使用说明](usage.md)。

## 历史验证记录

以下为原 README 保留的历史验证记录，不代表本次文档整理重新执行了测试；测试数量也不代表当前版本总数。

已在 Fedora 45 toolbox 完成 release 构建、83 项应用单元测试，另有 8 项 cosmic-text 测试、2 项窗口边缘回归测试和 `cargo clippy --all-targets -- -D warnings`。已检查 GNOME 下的 CSD 窗口和 Wayland 启动，以及夜间主题、系统字体下拉搜索、字体切换保存和设置文件 `0600` 权限。已在隔离 X11 / CSD 会话检查分隔线拖动、图片缩略图与原图、emoji 组合、Ctrl+V 图片草稿预览、自动翻页、末页刷新，以及最大化后关闭原图。Linux FileChooser portal 服务版本检查通过；尚未完成跨平台原生文件选择器实机测试。只读 API smoke 验证板块、串列表、回复页；未发送测试帖子。中文 IME 的实际输入仍需在桌面输入法环境中确认。

Windows CI 的构建和单元测试不覆盖 GUI 实机体验；窗口渲染、中文输入、图片剪贴板和文件选择仍需桌面环境验证。
