<div align="center">
  <img src="assets/brand/pecan-icon.svg" width="112" height="112" alt="Pecan Logo" />
  <h1>Pecan · 碧果</h1>
  <p><strong>让技术文档，回归阅读。</strong></p>
  <p>以 Rust 与 GPUI 构建的原生桌面阅读器。<br />清晰的排版、流畅的图表、可分享的文档，收在一个安静的窗口里。</p>
  <p>
    <a href="https://github.com/madebynut/pecan/releases">下载应用</a> ·
    <a href="docs/guide.md">使用指南</a> ·
    <a href="samples/welcome.md">功能示例</a> ·
    <a href="https://github.com/madebynut/pecan/issues">反馈问题</a>
  </p>
  <p>
    <img src="https://img.shields.io/badge/version-1.0.0-242424" alt="Version 1.0.0" />
    <img src="https://img.shields.io/badge/macOS_·_Windows-native-555555" alt="macOS and Windows" />
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-777777" alt="MIT license" /></a>
  </p>
</div>

---

## 为认真阅读而设计

**Pecan 是阅读器。** 打开 Markdown，也能把 Word、PDF 等文档转换为适合屏幕阅读的内容。它不会改写你的原文件，不需要账户，也不会将文档上传到转换服务。

黑、白、灰构成界面的底色。文件栏、标签页和大纲各有位置；收起两侧面板，就只留下文档本身。从一份接口说明到一整篇技术方案，阅读、检索、预览与导出都在手边。

| 阅读与导航 | 内容与表达 | 分享与输出 |
| :--- | :--- | :--- |
| 多标签、拖动排序、最近文件 | Markdown、表格、任务列表、提示块 | PDF、Word、HTML、Markdown |
| 可拖动侧栏、标题大纲、全文查找 | Mermaid 原生渲染与高清缩放 | Mermaid 独立 SVG / PNG 导出 |
| 浅色 / 深色 / 跟随系统 | LaTeX 数学与化学公式 | 可选择的 PDF 文字与矢量图表 |
| 中英文字体、字号、行高分别设置 | 代码高亮、折叠、缩进线、一键复制 | 可编辑的 Word 段落与表格 |

## 值得多看一眼的细节

### 图表清楚，思路才清楚

Mermaid 无需浏览器或 JavaScript 运行时。正文里直接阅读，点击进入独立预览；缩放会按窗口尺寸与显示密度重新栅格化，保持线条和文字清晰。图表的源码、高亮、复制与导出均可从块工具栏访问。普通图片使用同样的预览交互。

### 代码不只是有颜色

Tree-sitter 提供多语言语法高亮，语言标识搭配离线 Devicon 图标。代码保留完整高度，支持水平滚动、结构折叠与一键复制。Tab 宽度可选自动识别、2、4、8；JSON、YAML、TOML、JS / TS / TSX、SQL 等已接入本地格式化器。格式化只影响阅读与排版，源文件保持原样。

### 排版可以有自己的习惯

正文、代码、界面、Mermaid 的字体分别配置，并支持中英文独立选择系统字体。数学使用专用数学字体。默认正文 **16px / 1.6 倍行高 / 960px 宽度**，随窗口自动收缩；代码默认 14px。标题、引用、表格、行内代码和提示块都有统一的留白与层级。

### 导出后，依然是一份好读的文档

PDF 保留可选择文字，Mermaid 和公式保留矢量；Word 输出原生可编辑内容；HTML 将图片、图表和公式嵌入单文件，便于离线分享。Markdown 导出保留源文，并把引用图片保存到配套资源目录。

## 原生，也克制

- **GPU 绘制**：GPUI 直接绘制桌面界面，不打包 Chromium 或 WebView。
- **按需呈现**：文档采用虚拟列表，可见块才创建阅读组件；缓存有明确上限。
- **后台处理**：索引、搜索、图表、公式、格式化与转换在后台执行，任务并发和资源占用受到约束。
- **重复利用**：内容寻址缓存、共享字体库、复用网络连接，减少重复解析与分配。
- **本地优先**：内置图标和数学字体离线可用。只有文档引用网络图片或你点击外部链接时才需要网络。

性能取决于文档结构、字体和图表复杂度。具体预算、测量条件与历史结果见 [性能说明](docs/performance.md)。

## 下载与运行

在 [Releases](https://github.com/madebynut/pecan/releases) 下载对应压缩包：

| 平台 | 文件 | 使用方式 |
| :--- | :--- | :--- |
| macOS 12+ · Apple Silicon | `Pecan-1.0.0-macOS-arm64.zip` | 解压，将 `Pecan.app` 放入应用程序目录 |
| macOS 12+ · Intel | `Pecan-1.0.0-macOS-x86_64.zip` | 同上 |
| Windows 10 / 11 · x64 | `Pecan-1.0.0-Windows-x64.zip` | 解压整个目录，运行 `Pecan.exe` |

Windows 需要支持 DirectX 11 的图形环境。发布包附第三方许可证与 SHA-256 校验文件。当前 macOS 包使用 ad-hoc 签名，尚未经过 Apple 公证；Windows 包尚未进行 Authenticode 签名，系统可能显示来源确认。

常用操作在 macOS 使用 `⌘`，Windows 使用 `Ctrl`：打开 `O`、查找 `F`、文件栏 `B`、大纲 `L`、关闭标签 `W`、设置 `,`。Windows 全屏使用 `F11`。首次启动显示欢迎文档；关闭最后一个标签默认保留空窗口，可在通用设置中更改。

## 支持范围

- **Markdown**：常见基础语法、表格内 Markdown、提示块与 `:::` 高亮块，以及图片、公式和 Mermaid 扩展。
- **文档导入**：DOCX / DOC、PDF、RTF、ODT、EPUB、PPTX、XLSX、CSV 等由 anydoc 在本地转换。复杂分页、批注、修订及多栏布局可能无法完整重排。
- **PDF 阅读**：默认提取文字重排，不执行 OCR。原版查看在 macOS 使用 PDFKit，在 Windows 使用系统默认 PDF 阅读器。
- **Mermaid / LaTeX**：原生实现覆盖常见图表和数学、化学表达式；不保证兼容 Mermaid.js 全部扩展或任意 TeX 宏包。无法渲染时保留可查看、可复制的源码与错误信息。
- **格式化与导出**：未接入格式化器、语法无效或超过预算的代码保留原文。PDF / Word / HTML 使用全局 Tab 配置；单块临时设置只作用于阅读，Markdown 导出保留原文。

## 从源码构建

需要 **Rust 1.98+**。macOS 安装 Xcode Command Line Tools；Windows 安装 Visual Studio 2022 Build Tools 的 **使用 C++ 的桌面开发**与 Windows SDK。不需要 Node.js、Python 文档转换器或外部 Office。

```sh
git clone https://github.com/madebynut/pecan.git
cd pecan
cargo run --locked --release --bin Pecan
```

打包另需 Python 3.9+；macOS 执行 `sh scripts/bundle-macos.sh`，Windows 在 PowerShell 执行 `./scripts/bundle-windows.ps1`。

工作区按功能划分为 11 个 crate：界面、文档、格式化、高亮、资源、配置、网络、导入、导出、图表与数学。模块边界见 [架构说明](docs/architecture.md)，构建、验收和发布步骤见 [开发指南](docs/development.md)。`vendor/` 中的上游补丁与许可证是构建的一部分，请一并保留。

## 参与与致谢

欢迎提交可复现的问题、排版样例和改进建议。阅读效果问题请附应用版本、操作系统、缩放比例以及去除敏感内容后的最小文档。

Pecan 由 [GPUI Kit](https://github.com/longbridge/gpui-kit)、pulldown-cmark、Tree-sitter、mermaid-rs-renderer、resvg、mathra、krilla 和 anydoc 等开源项目支持；图标采用 Lucide 与 Devicon。完整许可证随源码及应用包提供，见 [licenses](licenses)。

项目以 [MIT License](LICENSE) 开源。
