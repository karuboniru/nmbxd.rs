# GitHub Actions 构建与发布

[返回 README](../README.md) · [Linux 构建与打包](build-linux.md) · [Windows 构建与测试包](build-windows.md) · [macOS 社区测试包](build-macos.md)

[构建记录与下载](https://github.com/karuboniru/nmbxd.rs/actions/workflows/build.yml) · [正式发布](https://github.com/karuboniru/nmbxd.rs/releases)

工作流定义在 [`.github/workflows/build.yml`](../.github/workflows/build.yml)。每次向任意分支 push、推送任意 tag，以及 PR 更新，自动运行构建；也可在 Actions 页面手动运行。一次 push 多个提交时构建该 push 的最终提交；工作流没有取消旧构建的并发规则。

## 平台与检查

| 平台 | Runner | Rust target | 超时 |
| --- | --- | --- | --- |
| Linux x86_64 | Ubuntu 24.04 | 主机默认 target | 60 分钟 |
| Linux aarch64 | Ubuntu 24.04 ARM | 主机默认 target | 60 分钟 |
| Windows x86_64 | Windows Server 2022 | `x86_64-pc-windows-msvc` | 90 分钟 |
| Windows aarch64 | Windows 11 ARM | `aarch64-pc-windows-msvc` | 90 分钟 |
| macOS aarch64 | `macos-15`（Apple Silicon） | `aarch64-apple-darwin` | 90 分钟 |
| macOS x86_64 | `macos-15-intel`（Intel） | `x86_64-apple-darwin` | 90 分钟 |

所有平台均安装 Rust stable，使用 `CARGO_BUILD_JOBS=2` 和 `CARGO_INCREMENTAL=0`，缓存 Cargo 下载与 release 构建产物，然后依次执行：

```sh
cargo test --release --locked -p rdnmb
cargo build --release --locked -p rdnmb
```

测试使用 release profile，以复用 GPUI 构建。Linux 随后执行 `bash scripts/package-linux.sh --skip-build` 并检查归档 SHA-256；Windows 执行 `./scripts/package-windows.ps1 -SkipBuild -Target $env:CARGO_BUILD_TARGET`；macOS 执行 `bash scripts/package-macos.sh --skip-build --target "$CARGO_BUILD_TARGET"` 并用 `shasum -a 256 --check` 校验。Ubuntu 的确切系统依赖见 [Linux 构建说明](build-linux.md#ubuntu-2404)。Windows 会自动查找 Windows SDK 的 `fxc.exe` 并设置 `GPUI_FXC_PATH`。macOS 明确选择 Xcode 16.4，检查 Metal 工具，设置 SDKROOT、LIBCLANG_PATH 以及 `MACOSX_DEPLOYMENT_TARGET=15.0`，详情见 [macOS 构建说明](build-macos.md)。

CI 不启动 GUI、不要求桌面会话或 cookie，也不向岛上发送内容。Windows 的 GUI 和原生桌面功能仍需实机验证。维护者没有 Mac 硬件，macOS 包用于社区实机测试，窗口渲染、中文输入、文件对话框和图片剪贴板仍待反馈。Ubuntu 产物是动态链接程序，运行目标需有兼容的 glibc / 系统库；包内 `BUILD-OS.txt` 与 `DEPENDENCIES.txt` 可用于检查。

## 下载构建产物

成功后，在该次运行页面的 **Artifacts** 下载：

| Artifact 名称 | 内容 |
| --- | --- |
| `rdnmb-linux-x86_64-提交SHA` | `.tar.gz` 和 `.tar.gz.sha256` |
| `rdnmb-linux-aarch64-提交SHA` | `.tar.gz` 和 `.tar.gz.sha256` |
| `rdnmb-windows-x86_64-提交SHA` | `.zip` 和 `.zip.sha256` |
| `rdnmb-windows-aarch64-提交SHA` | `.zip` 和 `.zip.sha256` |
| `rdnmb-macos-aarch64-提交SHA` | `.tar.gz` 和 `.tar.gz.sha256` |
| `rdnmb-macos-x86_64-提交SHA` | `.tar.gz` 和 `.tar.gz.sha256` |

所有产物保留 30 天。GitHub artifact 下载的外层是 ZIP；解开后再解压里面的平台发行包。Linux 可按 [安装说明](build-linux.md#使用发布包) 运行安装脚本，Windows 解压整个发行 ZIP 后运行程序，macOS 解压 tar.gz 后从终端运行原始二进制；macOS 包未提供 `.app`、开发者签名或公证。

## Tag 发布

推送任意 tag 时，等待三个平台、各两个架构的构建全部成功后，release job 下载本次运行的六份已测试产物，再次核对 `.tar.gz` 和 `.zip` 归档的 SHA-256，并上传到该 tag 的 GitHub Release。release job 不重新构建或执行产物，超时为 10 分钟。

工作流不会创建 tag；手动运行 tag 工作流也不会额外发布 Release。首次发布先创建草稿、上传附件，再公开 Release；重跑 tag 构建会替换同一 Release 的同名附件。Release 标题为 `R岛 tag`，压缩包名使用 Cargo 版本，二者需自行保持一致。

建议先更新 `Cargo.toml` / `Cargo.lock` 版本并提交，再按实际版本执行：

```sh
git tag v0.1.7
git push origin v0.1.7
```

## 权限与凭据

构建只使用仓库源码，不需要配置 cookie 或任何自定义 secret。工作流默认只有 `contents: read`，包括 fork PR 的构建 job；checkout 不保留凭据。仅 tag 发布 job 获得 `contents: write`，使用 GitHub 自动提供的 `GITHUB_TOKEN` 写入 Release。
