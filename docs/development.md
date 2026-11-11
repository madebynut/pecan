# 开发与发布

## 环境

- Rust 1.98+，工作区使用 Edition 2024。
- macOS：Xcode Command Line Tools，Metal 可用的运行环境。
- Windows x64：Visual Studio 2022 Build Tools，安装「使用 C++ 的桌面开发」和 Windows SDK（包括 `rc.exe`、`fxc.exe`）。运行时使用 DirectX 11。
- 打包脚本：Python 3.9+；Windows 使用 PowerShell。

所有核心格式化、Markdown、Mermaid、LaTeX、文档导入与导出均使用本地 Rust 库。macOS PDFKit／可选 Quick Look 代码与依赖仅在 macOS 编译；Windows 的 PDF 原版查看调用系统默认阅读器。图片／图表预览在两平台共用 GPUI。

Windows 使用 MSVC target、静态 CRT，图标和版本嵌入 EXE；GPUI 提供 PerMonitorV2 DPI manifest。设置和最近文件位于 `%APPDATA%/Pecan`，默认西文字体使用系统等宽字体。macOS 保留既有配置路径。

## 本地检查

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features -- --test-threads=1
cargo doc --locked --workspace --all-features --no-deps
cargo run --locked -p pecan-export --example export_document -- samples/acceptance.md target/export-smoke
```

GPUI 测试共享格式化信号量，须串行运行。Windows CI 使用固定提交和 SHA-256 校验的 Noto CJK 字体保证中英文渲染测试可重复；字体仅安装在临时 runner 的用户字体目录，不打包进应用。Windows 本地若缺少中文宋体／黑体，可运行 `./scripts/install-ci-fonts.ps1` 准备同一测试环境。标记为 ignored 的测试需要图形环境或外部验收文档，不会在普通 CI 自动执行。自动构建与测试不等同于 Windows 实机的高 DPI、驱动、输入法及 Office 显示验收；这些仍需按 [验收指南](../samples/acceptance-guide.md) 检查。

## 打包

```sh
# macOS 当前架构，也可传 aarch64-apple-darwin / x86_64-apple-darwin
sh scripts/bundle-macos.sh
```

```powershell
# Windows x64
./scripts/bundle-windows.ps1
```

产物写入 `dist/`。版本统一读取根 `Cargo.toml`，无需手动修改 plist 或 EXE 资源。Windows 压缩包保留许可证目录；macOS 应用包将许可证放在 Resources 内。Logo 的 SVG 是唯一设计源，运行 `sh scripts/build-icons.sh` 可在 macOS 重建 ICNS、ICO 和内部 PNG。

## GitHub Actions

**仅 `push.tags` 触发**，普通分支 push、pull request 均不运行构建，无定时或手动触发入口。

每个 tag 执行 macOS arm64、macOS x86_64、Windows x64 构建矩阵；所有平台先执行格式检查、Clippy、串行测试、rustdoc 和离线导出 smoke test，再打包上传 Actions artifacts。

正式发行步骤：

1. 更新根 `Cargo.toml` 的版本，并用 Cargo 更新 `Cargo.lock`；完成本地验证。
2. 提交并推送 `main`。此时不会触发构建。
3. 推送匹配版本的 tag：

   ```sh
   git tag -a v1.0.0 -m "Pecan 1.0.0"
   git push origin v1.0.0
   ```

4. 三个平台全部通过后，自动创建 GitHub Release，上传应用 ZIP 与 `SHA256SUMS.txt`。

`v` 前缀或数字开头的 tag 必须与 Cargo 版本完全一致，否则在构建前失败。其他 tag 仅构建并上传 artifacts，便于发布前验证，不创建 Release。不要移动已发布的版本 tag；修正发行问题应发布新版本。

当前未配置 Apple Developer / Authenticode 发布证书。macOS 使用 ad-hoc 签名，无公证；Windows 未签名。添加正式证书时应通过 GitHub Secrets 提供，不要放入源码。
