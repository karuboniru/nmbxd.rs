# Windows 构建与测试包

[返回 README](../README.md) · [CI 与发布](ci.md)

## 构建与打包

Windows 使用 `x86_64-pc-windows-msvc`，需要 Rust MSVC 工具链、Visual Studio C++ Build Tools 和 Windows SDK。GPUI 的 release 着色器编译需要 `fxc.exe`，可通过 `GPUI_FXC_PATH` 指定完整路径；CI 从 Windows SDK 的 `bin/*/x64/fxc.exe` 中自动查找。

在仓库根目录的 PowerShell 中运行：

```powershell
rustup target add x86_64-pc-windows-msvc
./scripts/package-windows.ps1
# 已完成对应 target 的 release 构建时：
./scripts/package-windows.ps1 -SkipBuild
```

脚本默认执行 `cargo build --release --locked -p rdnmb --target x86_64-pc-windows-msvc`，通过 Cargo metadata 查找 target 目录并读取对应 release 程序。输出为 `dist/rdnmb-版本-windows-x86_64.zip` 和对应 `.zip.sha256`；版本来自 Cargo 包版本。

包内包含 `rdnmb.exe`、说明、许可证和空白凭据示例，使用明确的公开文件清单，不复制真实 `.env`、用户配置或整个构建目录。release 版本使用 Windows GUI 子系统，启动时不保留控制台窗口。

与 CI 相同的单元测试命令可在 PowerShell 中运行：

```powershell
$env:CARGO_BUILD_TARGET = 'x86_64-pc-windows-msvc'
cargo test --release --locked -p rdnmb
```

## 下载与运行

Windows 测试包可从 [Actions 构建记录](https://github.com/karuboniru/nmbxd.rs/actions/workflows/build.yml) 下载，并随 tag 的 Linux 包一起发布到 [GitHub Releases](https://github.com/karuboniru/nmbxd.rs/releases)。Actions 下载得到的外层 ZIP 需要先解开，才能取得真正的发行 ZIP 和校验文件。

在 PowerShell 中计算 SHA-256，与 `.zip.sha256` 内的哈希比较：

```powershell
Get-FileHash .\rdnmb-版本-windows-x86_64.zip -Algorithm SHA256
```

解压整个发行 ZIP 后运行其中的 `rdnmb.exe`，保留随附文档和许可证。

## 配置与验证边界

配置默认保存在 `%APPDATA%\rdnmb\settings.json` 和同目录的 `appearance.json`，也支持以绝对路径的 `XDG_CONFIG_HOME` 覆盖。本地历史位于配置目录中的 `history.sqlite3`，图片缓存位于 `media-cache/`。新版会迁移旧 `nmbxd` 设置；已有新版文件时优先使用新文件。

Windows 文件选择和图片另存为使用 rfd 原生对话框。虚拟机中的窗口渲染、中文输入、图片剪贴板和文件选择仍待实机验证；CI 仅构建及运行单元测试，不启动 GUI。构建成功不能代替桌面功能验证。
