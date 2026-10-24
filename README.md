# R岛 · rdnmb

**R岛是 X 岛（nmbxd.com）的第三方桌面客户端，支持 Linux 和 Windows，并提供 macOS 实验性构建（Apple Silicon / Intel）。**

**macOS 需要大家帮忙测试：维护者完全没有 Mac 测试设备。** macOS 二进制由 GitHub Actions 构建，尚未进行实机运行和 GUI 验证，不代表已确认可正常使用。欢迎下载试用并提交 issue，反馈 macOS 版本、芯片架构、应用版本和复现步骤；尤其需要验证标题栏、文件选择器、图片复制粘贴和中文输入。

使用 **Rust + GPUI / gpui-component** 构建原生界面，基于 **reqwest** 访问 API，使用 **SQLite** 保存本地历史与阅读缓存。

[下载发布版](https://github.com/karuboniru/nmbxd.rs/releases) · [Actions 测试构建](https://github.com/karuboniru/nmbxd.rs/actions/workflows/build.yml)

## 已实现功能

- **浏览与阅读**：板块和串列表、滚动分页、页码跳转、时间顺序 / 引用树切换、嵌套引用预览。
- **发串与回复**：引用回复、图片附件与剪贴板粘贴、颜文字选择器；按串或板块独立保留本次运行中的草稿。
- **图片与复制**：缩略图、原图查看、图片复制与另存为、正文选择与复制、emoji 显示。
- **订阅收藏**：查看、添加和移除服务器订阅，通过订阅 ID 在设备间共享。
- **历史与缓存**：浏览 / 发串 / 回复历史、阅读位置恢复、已缓存内容离线阅读、图片磁盘缓存。
- **界面设置**：跟随系统 / 浅色 / 深色主题，可调整栏宽、字体和字号。

## 发串前配置 Cookie

**发串和回复需要有效的 X 岛 Cookie（饼干）。** 在网页版打开饼干二维码，对二维码右键选择 **“复制图像链接”**，然后在客户端“设置”中粘贴并导入，无需按 F12。链接形如：

```text
https://publicapi.ovear.info/qrcode.php?w=500&text=eyJj...&base64=1
```

链接支持 `http://` 和 `https://`。客户端在本地解码链接中的 Base64 JSON（`{"cookie":"%09%...","name":"c1VkhIS"}`），保留 cookie 的百分号编码。可连续导入多个饼干，再选择一个作为**认证饼干**并保存；浏览请求使用认证饼干，选择游客则匿名浏览。

设置中每个饼干旁可点击“删除”，待删除项以灰色虚线保留，可点击“恢复”。点击“保存”后才移除对应文件；关闭设置而不保存会撤回所有改动。删除当前认证饼干后切换为游客，也可在保存前选择另一个认证饼干。

回复框的下拉菜单可单独选择发送饼干。每个串首次回复默认使用认证饼干；成功回复后记住该串所用的饼干，下次回复（包括重启后）优先沿用，不影响其他串和浏览认证。发送失败不会更新这份记录。

饼干以明文分别保存在 `~/.config/rdnmb/cookie/${name}.cookie`，内容仅为 cookie 原文；旧配置中的饼干会自动迁移为 `default.cookie`，已有文件不会被覆盖。配置目录遵循 `XDG_CONFIG_HOME`，Windows 默认使用 `%APPDATA%/rdnmb`。Cookie 和二维码链接都是发帖凭据，请勿分享或提交到仓库。

## 文档

- [Linux 构建、运行与打包](docs/build-linux.md)
- [Windows 构建、运行与打包](docs/build-windows.md)
- [macOS 实验性构建、运行与打包](docs/build-macos.md)
- [GitHub Actions 与发布流程](docs/ci.md)
- [使用说明与功能细节](docs/usage.md)
- [技术说明、限制与验证记录](docs/technical.md)

界面截图：

![R岛界面截图](docs/screenshot.png)
