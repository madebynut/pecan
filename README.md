<div align="center">
  <img src="assets/brand/pecan-icon.svg" width="112" height="112" alt="Pecan Logo" />
  <h1>Pecan · 碧果</h1>
  <p><strong>让技术文档，回归阅读。</strong></p>
  <p>
    <a href="samples/welcome.md">功能示例</a> ·
    <a href="https://github.com/madebynut/pecan/issues">反馈问题</a>
  </p>
</div>

---

Pecan 是 macOS 与 Windows 上的原生 Markdown 阅读器，用 Rust 与 GPUI 绘制。也可以编辑 Markdown，并在本地把 Word、PDF 等文档转成适合阅读的内容。阅读和格式化保持原文件不变；源码在保存时才写入。

| 阅读与导航 | 内容与表达 | 分享与输出 |
| :--- | :--- | :--- |
| 阅读与源码切换 | Markdown 编辑、高亮、撤销与查找 | 保存、另存为与外部修改冲突检查 |
| 多标签、拖动排序、最近文件 | 表格、任务列表、提示块 | PDF、Word、HTML、Markdown |
| 可拖动侧栏、标题大纲、全文查找 | Mermaid 原生渲染与高清缩放 | Mermaid 独立 SVG / PNG |
| 浅色 / 深色 / 跟随系统 | LaTeX 数学与化学公式 | 可选中的 PDF 文字与矢量图表 |
| 中英文字体、字号、行高分别设置 | 代码高亮、折叠、缩进线、一键复制 | 可编辑的 Word 段落与表格 |

正文右键「查看源码」，或按 `⌘E` / `Ctrl+E`，即可在阅读和带行号的 Markdown 编辑器之间切换。`⌘S` / `Ctrl+S` 保存，配合 Shift 另存为。有未保存修改时，关闭或切回阅读都会询问保存、不保存或取消；不保存仍用当前草稿预览，不写入文件。Word、PDF 和欢迎页的修改另存为 Markdown。

Mermaid 和 LaTeX 在本机绘制。图表可独立预览，并导出 SVG / PNG。PDF 保留可选中的文字和矢量图表，Word 保留可编辑的段落与表格，HTML 把图片、图表和公式嵌进单个文件。

macOS 使用 `⌘`，Windows 使用 `Ctrl`：打开 `O`、查找 `F`、文件栏 `B`、大纲 `L`、关闭标签 `W`、设置 `,`。Windows 全屏使用 `F11`。

导入覆盖 DOCX、PDF、RTF、ODT、EPUB、PPTX、XLSX、CSV 等，复杂分页和批注可能无法完整重排。PDF 默认提取文字，不做 OCR。Mermaid 与 LaTeX 覆盖常见图表和数学、化学表达式，不覆盖 Mermaid.js 的全部扩展或任意 TeX 宏包。

## 从源码构建

需要 **Rust 1.98+**。macOS 安装 Xcode Command Line Tools；Windows 安装 Visual Studio 2022 Build Tools 的 **使用 C++ 的桌面开发**与 Windows SDK。

```sh
git clone https://github.com/madebynut/pecan.git
cd pecan
cargo run --locked --release --bin Pecan
```

项目以 [Apache License 2.0](LICENSE) 开源。
