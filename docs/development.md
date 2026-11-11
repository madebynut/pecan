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

本地与 tag 发布 CI 均执行全工作区、全特性测试，GPUI 测试共享格式化信号量，须使用 `--test-threads=1` 串行运行。标记为 ignored 的测试需要图形环境或外部验收文档，应单独验收。macOS 和 Windows CI 使用固定提交和 SHA-256 校验的 Noto CJK 字体保证测试及导出 smoke test 的中文覆盖；字体仅安装在临时 runner 的用户字体目录，不打包进应用。本地若缺少中文宋体／黑体，可运行 `python scripts/install-ci-fonts.py` 准备同一字体环境。自动构建与本地测试不等同于 Windows 实机的高 DPI、驱动、输入法及 Office 显示验收；这些仍需按 [验收指南](../samples/acceptance-guide.md) 检查。

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

每个 tag 执行 macOS arm64、macOS x86_64、Windows x64 构建矩阵；所有平台先执行格式检查、Clippy、全工作区全特性测试、rustdoc 和离线导出 smoke test，再打包上传 Actions artifacts。测试使用各平台原生目标、`--no-fail-fast` 和 `--test-threads=1`，一个 crate 失败后继续运行其他 crate，以收集完整结果；任何测试失败仍会阻止该平台打包及最终发布。其他平台也继续完成检查。标记为 ignored 的环境专项不自动运行，须单独验收。

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


## 资源预算与审查记录

文档原始文本上限 32 MiB；单窗口最多 8 个标签、64 MiB 总源文本，重载也执行总量检查。索引额外限制围栏代码外 500,000 个非空行和 64 MiB 计费预算；超限会返回错误并保留已打开的旧内容。目录最多保留 20,000 个展开条目，8 个等待请求共用 2 个扫描槽。

图片编码输入 16 MiB，静态声明分配或动画累计像素 128 MiB、4,096 帧；SVGZ 展开 16 MiB。这些限制与已有可见行释放、图表及数学缓存预算互补，不代表整机 RSS 的硬上限。完整依据、基准、已修问题和仍需跟进事项见 [1.0.0 生产审查](review-1.0.0.md)。
