# Linux 构建与打包

[返回 README](../README.md) · [CI 与发布](ci.md)

## Fedora toolbox

需要 Rust 工具链、Wayland / X11 开发库、Vulkan 驱动和中文字体。首次编译较大，可在 toolbox 中安装系统依赖：

```sh
toolbox run sudo dnf install -y gcc-c++ cmake clang pkgconf-pkg-config \
  libxkbcommon-devel libxkbcommon-x11-devel wayland-devel \
  fontconfig-devel freetype-devel libxcb-devel openssl-devel \
  vulkan-loader mesa-vulkan-drivers google-noto-sans-cjk-fonts
toolbox run cargo run --locked
```

单元测试和只读 API smoke：

```sh
toolbox run cargo test --locked
toolbox run cargo run --locked -- --smoke
```

`--smoke` 只读请求板块列表、综合一首页和其中一串第一页，不输出凭据或帖子内容，不发送帖子。它需要网络，不能代替 GUI 和桌面输入法验证。

未保存界面设置时，从项目目录启动会读取 `.env`。优先使用 `RDNMB_COOKIE`，兼容 `NMBXD_COOKIE`、`userhash` 和 `usrhash`；值可为原始百分号编码饼干或 `userhash=…`。无饼干可尝试只读浏览，具体可见板块由服务器决定。`.env` 已忽略，不要提交真实凭据；已保存的界面设置优先于环境变量和 `.env`。

## Ubuntu 24.04

当前 CI 使用 Ubuntu 24.04 x86_64，并明确安装以下编译依赖：

```sh
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  build-essential pkg-config \
  libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev \
  libwayland-dev libfontconfig-dev libfreetype-dev
cargo test --release --locked -p rdnmb
cargo build --release --locked -p rdnmb
```

这是 CI 的依赖安装清单；桌面运行还需要可用的 Wayland / X11 会话、图形驱动和中文字体。CI 不启动 GUI。

## 打包

在仓库根目录构建并打包当前主机架构：

```sh
scripts/package-linux.sh
# 在 Fedora toolbox 中构建、打包：
toolbox run scripts/package-linux.sh
# 已完成 release 构建时：
scripts/package-linux.sh --skip-build
```

脚本默认执行 `cargo build --release --locked -p rdnmb`，从 `${CARGO_TARGET_DIR:-target}/release/rdnmb` 读取程序。输出位于 `dist/rdnmb-版本-linux-架构/`，以及同名 `.tar.gz` 和 `.tar.gz.sha256`；版本来自 `Cargo.toml`，架构来自 `uname -m`。

压缩包使用明确的公开文件清单，包含程序、图标、桌面入口、说明、许可证、安装 / 卸载脚本、空白凭据示例和构建环境记录，不包含真实 `.env`、帖子数据或整个 `target`。脚本依赖 GNU tar、gzip、ldd 和 sha256sum 等 Linux 工具。`SOURCE_DATE_EPOCH` 可指定归档时间，默认使用最近一次提交时间；归档文件顺序和元数据固定。二进制本身的可重现性仍取决于工具链、链接器和系统库。

## 使用发布包

```sh
sha256sum -c rdnmb-版本-linux-架构.tar.gz.sha256
tar -xzf rdnmb-版本-linux-架构.tar.gz
cd rdnmb-版本-linux-架构
./rdnmb
```

也可在解压目录执行 `./install.sh`，安装到当前用户的 `~/.local/bin` 及应用菜单，无需 sudo。应用 ID 为 `org.niconi.rdnmb`，桌面文件名为 `org.niconi.rdnmb.desktop`。通过菜单启动后，可在标题栏“设置”中导入饼干二维码的图像链接并选择认证饼干；从终端启动也可使用环境变量或当前目录的 `.env`。

从旧版升级时，请先在旧发行包目录运行旧版 `./uninstall.sh`，再安装新版；新版安装器保留旧 `nmbxd` 程序、桌面入口和图标，不会自动删除它们。新版 `./uninstall.sh` 删除新版安装文件，不删除用户凭据。

## 兼容性与桌面依赖

这是针对构建机器架构的动态链接 Linux 程序，尚未提供 AppImage、Flatpak 或跨发行版静态包。构建环境写入 `BUILD-OS.txt`，实际 `ldd` 输出写入 `DEPENDENCIES.txt`；打包遇到缺失库会失败。较新 Fedora 编译的程序可能要求较新的 glibc，不能保证在较旧 Ubuntu / Debian 等发行版运行。分发前应在目标发行版运行 `ldd ./rdnmb` 检查 `not found` / GLIBC 版本错误，并验证桌面启动。

Linux 文件选择和另存为严格使用 XDG Desktop Portal（ashpd），需要运行 `xdg-desktop-portal` 和桌面对应后端。图片复制使用 arboard：Linux 优先使用 Wayland data-control，不支持该协议的桌面（如 GNOME）需要可用的 XWayland；失败会显示错误，可改用“图片另存为”。退出应用后图片是否保留取决于桌面剪贴板管理器。

## 依赖约束

依赖固定为 GPUI 0.2.2 / gpui-component 0.5.1。`vendor/` 保留 cosmic-text 的完整 emoji 字符簇字体选择补丁，以及 gpui-component 最大化 / 平铺时窗口缩放命中区域补丁；详情见各目录的 `NMBXD-PATCH.md`。请保留 `Cargo.lock`：旧 xattr 0.2 的 Linux ENOATTR 引用与最新 libc 不兼容，锁文件使用 libc 0.2.186。
