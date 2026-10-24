# macOS 构建与社区测试包

[返回 README](../README.md) · [CI 与发布](ci.md)

维护者没有 Mac 硬件，macOS 目前提供社区测试用二进制。CI 负责原生构建和单元测试，不启动 GUI；窗口渲染、中文输入法、文件对话框、图片剪贴板及桌面操作仍需社区实机反馈。构建成功不代表这些功能已验证。

## CI 与工具链

CI 分别使用 `macos-15`（Apple Silicon，`aarch64-apple-darwin`）和 `macos-15-intel`（Intel，`x86_64-apple-darwin`）原生编译与运行测试，不依赖维护者自有机器。Runner 标签与架构对应关系见 [GitHub 官方说明](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)。

两个 job 选择 Xcode 16.4，并检查 `metal`、`metallib` 是否可用。GPUI 0.2.2 的构建脚本会调用这两个工具编译 Metal 着色器，还需要 Xcode 的 libclang 生成绑定。构建应选择完整 Xcode，不能只依赖独立 Command Line Tools；如所选新版 Xcode 缺少 Metal 工具链，按 [Apple 官方说明](https://developer.apple.com/documentation/xcode/downloading-and-installing-additional-xcode-components) 在 Xcode Components 中安装 Metal Toolchain，或运行 `xcodebuild -downloadComponent metalToolchain`。

初始 CI 包设置 `MACOSX_DEPLOYMENT_TARGET=15.0`，面向 macOS 15 及以上；旧版 macOS 尚未验证。包内 `BUILD-OS.txt` 记录构建系统、Rust target、部署目标和 Xcode 版本，`DEPENDENCIES.txt` 记录 `otool -L` 输出。

## 本机构建与打包

安装 Rust stable、完整 Xcode 及 Metal 工具链后，在仓库根目录运行。以下 Apple Silicon 示例与 CI 的环境一致；Intel Mac 将 target 改为 `x86_64-apple-darwin`。

```sh
export DEVELOPER_DIR=/Applications/Xcode_16.4.app/Contents/Developer
export MACOSX_DEPLOYMENT_TARGET=15.0
export CARGO_BUILD_TARGET=aarch64-apple-darwin
export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
export LIBCLANG_PATH="$(dirname "$(xcrun --find clang)")/../lib"
xcrun --sdk macosx metal --version
rustup target add "$CARGO_BUILD_TARGET"
cargo test --release --locked -p rdnmb
bash scripts/package-macos.sh
# 已完成对应原生 target 的 release 构建时：
bash scripts/package-macos.sh --skip-build --target "$CARGO_BUILD_TARGET"
```

脚本默认执行 `cargo build --release --locked -p rdnmb --target 原生target`，读取 `${CARGO_TARGET_DIR:-target}/target名称/release/rdnmb`。`--target` 或 `CARGO_BUILD_TARGET` 可指定当前原生 target；脚本拒绝不同架构，并用 `lipo -verify_arch` 检查实际二进制。脚本使用 macOS 自带 tar、gzip、shasum 及 Xcode 命令行工具。

输出为 `dist/rdnmb-版本-macos-aarch64.tar.gz` 或 `dist/rdnmb-版本-macos-x86_64.tar.gz`，附同名 `.tar.gz.sha256` 和解包目录。包内有原始 `rdnmb` 可执行文件、README、公开文档、许可证、空白 `.env.example` 和构建记录，使用明确公开文件清单，不复制真实凭据、用户数据或整个构建树。产物不包含 `.app`、DMG、开发者签名或公证，归档也不承诺逐字节可重现。

## 下载与运行

在 [Actions 构建记录](https://github.com/karuboniru/nmbxd.rs/actions/workflows/build.yml) 下载对应架构的 artifact，先解开 GitHub 的外层 ZIP，取得实际 `.tar.gz` 和校验文件。tag 构建成功后，这些文件也会发布到 [GitHub Releases](https://github.com/karuboniru/nmbxd.rs/releases)。Apple Silicon 选择 `aarch64`，Intel 选择 `x86_64`。

```sh
shasum -a 256 --check rdnmb-版本-macos-aarch64.tar.gz.sha256
tar -xzf rdnmb-版本-macos-aarch64.tar.gz
cd rdnmb-版本-macos-aarch64
./rdnmb
```

通过终端启动。下载的测试程序可能受 macOS 安全检查限制；请先确认来源和校验和，再按系统提示决定是否允许该程序运行。终端启动后的设置、饼干导入及使用方式见 [使用说明](usage.md)。

## 反馈

请在 issue 中附上提交 SHA / tag、Mac 芯片架构、macOS 版本、启动方式、复现步骤及相关错误。优先反馈能否启动与渲染、中文输入、原生文件选择 / 另存为、图片复制和链接打开；日志或截图请先移除饼干与个人内容。
