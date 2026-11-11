# 碧果 · Pecan

使用 Rust 与 GPUI 开发的原生文档阅读器。应用包名为 Pecan，内部显示「碧果」。「通用」设置位于第一项并默认打开。默认在关闭最后一个标签后保留窗口，显示 logo 背景；可在「设置 → 通用」启用关闭最后一个标签时退出应用。欢迎页只在首次启动自动打开，帮助菜单仍可手动打开。[验证记录](review-0.8.12.md) 说明标签与启动的回归范围。继续使用 GPUI Kit 0.9.0 与 GPUI 0.3.11。主 crate 聚焦原生窗口与交互，导出、格式化、高亮、配置、资源与网络由独立 crate 负责。samples/ 提供综合与多语言验收文档。

支持三栏布局、可拖动侧栏与标签页、代码高亮／缩进设置／折叠／多语言阅读格式化、Mermaid 中英独立字体与高清预览、统一图片预览、DOCX／PDF 重排阅读及 LaTeX 数学与化学式。系统「文件」菜单支持 PDF、Word、HTML、Markdown 导出，正文右键提供 PDF、Word、HTML 导出与「查看源码」；Mermaid 工具栏支持独立 SVG 与高清 PNG 导出。PDF 使用跨平台 Rust 排版与矢量输出，Word 保留可编辑文字及原生表格。

## 运行

需要 Rust 1.98 或更高版本；macOS 需要 Xcode Command Line Tools，Windows 需要 Visual Studio 2022 C++ Build Tools 与 Windows SDK。

```sh
cargo run --release --bin Pecan
cargo run --release --bin Pecan -- /path/to/document.md
cargo run --release --bin Pecan -- /path/to/folder
```

macOS 应用包：

```sh
./scripts/bundle-macos.sh
```

生成并验证 ad-hoc 签名的 `dist/Pecan.app`；尚未使用 Apple Developer 证书签名或公证。初次构建会编译 GPUI 和原生平台依赖，正式体验请使用 release 版本。

## Mermaid 字体

在「设置 → Mermaid」分别选择中文和英文字体。默认采用「系统默认字体」与「系统默认等宽字体」；按当前平台已安装的字体解析，设置文件保存平台无关的选项。中英文按字形分别排版，正文图表与已经打开的放大预览同步更新。界面、正文与代码也采用这些默认选项。旧版默认值一次性迁移，自定义字体和字号保留。LaTeX 保留独立 OpenType MATH 字体。Windows 默认字体映射已准备，当前交付和实测仍为 Apple Silicon macOS。

## 阅读操作

| 操作 | 功能 |
| --- | --- |
| ⌘O / ⌘⇧O | 打开文档 / 文件夹 |
| 文件 → 打开最近文件 / ⌃⌘R | 最近文档与文件夹；「更多…」搜索历史（Windows：Ctrl+Alt+R） |
| ⌥⌘P / 标签栏「原版查看」 | 查看当前 PDF 的原始页面（macOS） |
| ⌘F | 搜索正文，Enter / Shift+Enter 或按钮切换结果 |
| ⌘B / ⌘L | 展开／收起左侧文件栏 / 右侧大纲 |
| 拖动侧栏内侧边缘 | 调整宽度；拖至外缘收起，继续拖回展开；收起后可从窗口两侧拖开 |
| ⌘E / ⌘S / ⌘⇧S | 切换源码与阅读 / 保存 / 另存为 Markdown（Windows 使用 Ctrl） |
| ⌘R | 手动重新加载；未保存时提示 |
| ⌘, | 打开设置，自动保存并立即生效 |
| ⌃⌘F | 进入／退出全屏；顶部原生菜单由 AppKit 管理 |
| ⇧⌘F | 专注阅读，隐藏导航和标签栏 |
| ⌘+ / ⌘− / ⌘0 | 增大／减小／恢复正文字号 |
| ⌃Tab / ⌃⇧Tab | 切换标签页 |
| ⌘W | 关闭标签页；默认保留空窗口，设置中可改为退出应用 |
| 点击大纲标题 | 跳到对应阅读块和标题位置 |
| 代码块复制图标 | 复制完整代码，不包含围栏；格式化后复制当前排版 |
| 代码块缩进图标 | 自动识别或选择 2／4／8 列；支持的语言选择后自动格式化当前块，其它语言调整制表符显示和参考线 |
| 代码块折叠图标／左侧折叠箭头 | 折叠整块／按原生语法结构折叠，搜索自动展开目标 |
| Mermaid 右上角 `</>` 图标 | 带语法高亮的可选择只读源码视图，点击再次返回图表 |
| Mermaid 右上角复制图标 | 保留空格与换行，复制 Mermaid 原文 |
| 点击图表 / 展开图标 | 打开屏幕可用区域大小的独立窗口，默认完整适配、高清显示 |
| 系统红色窗口按钮 / Escape | 返回文档 |

## 源码编辑

正文右键「查看源码」、标签栏右端 `</>` 或视图菜单可将整个正文区域切换为 Markdown 编辑器。显示行号、语法高亮与横纵滚动，支持撤销／重做、查找和大纲定位；字体与字号沿用代码设置，顶部字号按钮在源码模式下调整代码字号。每个标签保留自己的编辑内容、光标与撤销历史。切回阅读时解析最新草稿，不会自动保存；导出也使用当前草稿。

「文件 → 保存」原样保存 Markdown 源码，包括代码缩进、UTF-8 BOM 和现有换行。另存为 Markdown 时一并保存所需图片资源；欢迎页和 Word／PDF 转换内容默认走另存为，不覆盖原始文档。关闭标签、关闭窗口、通过应用菜单／快捷键退出或手动重新加载时，未保存的修改会询问保存／不保存／取消。外部文件修改不会覆盖编辑会话；保存检测到冲突、文件删除或权限问题时保留草稿，可另存为或确认后重新加载。

编辑器按需创建，语法高亮复用现有后台引擎；输入不反复构建阅读组件。粘贴前检查单文档 32 MiB 与窗口总源文本 64 MiB 预算，保存与预览同样受文档预算约束；有修改或保存中的标签不会被自动回收。这些是源文本限制，不是整个进程内存上限。草稿当前仅保存在会话内，不提供崩溃恢复。验收文档见 [samples/source-editing.md](../samples/source-editing.md)。

## 最近文件

成功打开的文档与主动打开的文件夹自动保存，菜单分别显示最新十项；「更多…」可按文件名或完整路径搜索，使用方向键、Enter 或鼠标打开，Escape 关闭。已打开文档直接激活原标签，不重复创建；失效路径提示错误，不改动当前文档与目录。自动展开文档父目录、重新加载及欢迎页不写入历史。

历史独立于阅读设置，最多保存 100 条且文件不超过 256 KiB。写入在后台防抖并原子替换，退出前保存最后状态；仅保存路径和类型，不保存文档内容或缩略图。「清空最近记录」同步更新菜单和持久化文件。macOS 保存至 `~/Library/Application Support/Pecan/recent.json`，Windows 使用 `%APPDATA%/Pecan/recent.json`，Linux 使用 `$XDG_CONFIG_HOME/Pecan/recent.json` 或 `~/.config/Pecan/recent.json`。平台构建与验收范围见开发指南。

正文默认字号 16px、行高 1.6；旧版行高 1.8 升级到新默认，其它自定义行高、字体和字号保留。行高仍可在设置中调整，代码块使用独立配置。

## 导出

- 系统「文件」菜单：导出 PDF、Word、HTML、Markdown。
- 正文右键：PDF、Word、HTML 导出子菜单和「查看源码」；复制选中内容／Markdown 全文、查找、专注阅读、重新加载、复制路径、在 Finder 中显示。
- Mermaid 块下载图标：导出 SVG 或高清 PNG，不需要右键。

HTML 是单文件，嵌入本地／网络图片、Mermaid 和 LaTeX，保留代码高亮、表格、引用、列表和标题层级。使用白纸样式便于分享；不包含外部脚本、字体服务或 CDN。自定义系统字体使用 CSS 字体名与系统回退，未安装该字体的接收方可能看到回退字体。

Markdown 保留原始源文档及代码格式，并把图片复制到文件旁边新建的 `pecan-assets-*` 目录。移动或分享时请把 Markdown 文件和此目录一起带走；DOCX/PDF 重排文档的临时图片也会保存下来。

PDF 使用 krilla／krilla-svg 与 Rust 排版引擎输出 A4 页面，文字可选择，Mermaid 与 LaTeX 保留矢量。长代码换行并跨页，表格自动适配页宽并重复表头；图片等比例适配页面。导出主路径不依赖 Swift、WebKit 或 AppKit，也不启动外部浏览器或打印程序。Word 使用原生段落、标题、列表和表格，保留可编辑文本及图片。两种格式使用适合打印的 11 pt 正文、9 pt 代码和用户字体；各平台的自动验证范围见开发指南。

表格单元格里用 `<br>` 分行的列表、任务项和段落，在 PDF、Word 和 HTML 中都还原为原生列表与复选框。

每次只进行一项导出。图片按唯一 URI 复用，单张原始资源最大 16 MiB、合计 64 MiB，输出最大 128 MiB；下载复用现有网络客户端。PDF 与 Word 实际解码前检查图片尺寸及位深，单图像素缓冲预算 128 MiB、累计 256 MiB；16 位图片按 RGBA 8 B/px 计算，不缩小原图或降低清晰度。这是像素缓冲预算，不是整个进程峰值内存上限；HTML／Markdown 直接嵌入或复制原字节，无需解码。PDF 最多 5000 页，过高对象完整适配页宽、页高，无进展分页明确报错。保存取消不创建文件，资源缺失／渲染失败明确报错，保留已有目标文件。文档切换或关闭不会提前删除正在导出的临时资源。

PDF／Word 的 SVG 内嵌图片共用资源和解码预算，支持 data URL 与按各级 SVG 目录解析的本地图片，重复字节共享内存。SVG 文本每项最多 4 MiB、嵌套最多 32 层；按出现次数累计的解析文本最多 64 MiB，防止重复嵌套树放大内存。SVGZ 与 SVG 内部网络图片当前会明确报错。WebP 同时检查画布和压缩帧尺寸，避免不一致的畸形图片在报错前分配巨大缓冲。

## 阅读样式

浅色界面使用白色纸页、近黑文字和灰色侧栏、边框与悬停；深色界面使用深灰背景和浅灰文字。阅读区、Mermaid、数学和代码排版采用同一灰阶体系，代码通过克制的彩色语法高亮和灰色斜体注释保持语义层次。标题保留层级与留白，不加下划线。表格、代码块和行内 code 使用细边框及小圆角。表格保留原生行内 Markdown 渲染；使用 `<br>` 分行的单元格可显示真正的 Markdown 列表、强调、链接和行内代码，也支持简单的 `<ul>`／`<ol>` 列表。标准 GFM 表格本身仍要求每行单元格写在同一物理行。所有样式由 GPUI 原生绘制。

标题下方留白随正文字号调整：一级标题约 1.25 倍字号，其他标题约 1 倍字号；标题上方留白建立章节节奏。引用块使用注释灰文字。行内 code 使用浅灰底、1px 细边框和紧凑留白；按实际字形校正水平位置，使左右视觉留白均衡。代码块语言使用中性文字标签。状态栏固定 11px、高度 24px。

顶部仅保留居中的文档标题与两侧工具，左侧不放小 Logo；全屏时移除原生窗口按钮的固定预留，左侧栏按钮移至左边缘内侧。退出全屏恢复窗口按钮预留；AppKit 管理全屏窗口按钮和顶部菜单的显示。

## 字体与数学

设置按通用、外观、阅读、Mermaid、LaTeX排序，默认打开通用。中文、英文、界面、代码和 LaTeX 字体分别设置，另可调整各字号、行高、阅读宽度、主题和自动刷新。

中文、英文、界面和代码字体均使用系统已安装的完整公开字体列表，另提供系统默认字体。列表在设置窗口创建时读取并排序去重，不在阅读热路径扫描。字体选择不再受固定白名单限制，卸载后的字体由系统回退。中文字体作为回退，英文字体自带汉字时会优先使用其汉字字形。

正文默认 16px，代码块默认 14px；代码块字体与字号独立设置。界面中文／英文字体分别从完整系统字体列表选择，界面字号默认 14px，图标固定 16px，状态栏 11px。正文宽度默认 960px，可在设置 → 阅读中调整为 480–1600px；窄窗口自动收缩。块级图片按实际正文宽度等比例自适应，不再限制为 640 × 360，小图保持原尺寸上限，HTML 指定宽度仍作为上限；点击显示完整原图。Mermaid 正文预览仍保持适当紧凑，点击打开高清完整图表。

旧版默认正文宽度 800px 一次性迁移为 960px，自定义宽度保留；旧默认界面字号迁移为 14px，已迁移版本的自定义字号继续保留。恢复默认设置采用当前默认值。

LaTeX 默认字号为 18px；旧版未改过的 19px 迁移为 18px，自定义值保留。LaTeX 提供内置 STIX Two Math 与 Latin Modern Math，使用真实 MATH 排版指标；不会把普通正文／界面字体当作数学字体。行内 `$E=mc^2$` 和独占行的 `$$…$$` 可渲染分式、根号、上下标、求和、矩阵等，支持 `ce{N2 + 3H2 <=> 2NH3}`、`pu{…}` 和 `chemfig{…}`。独立公式提供「复制 LaTeX」按钮，行内公式可以选择复制。

数学阅读渲染采用纯 Rust mathra + resvg，无 JavaScript 或外部 LaTeX 安装。后台最多并行处理两项数学任务；插件只查询已准备的图像。完整 TeX 文档、任意宏包、宏定义和文件引入不受支持。源码、嵌套深度、公式数量和像素有明确上限；超出或不支持的语法显示可复制的错误提示。

设置保存于 macOS `~/Library/Application Support/Pecan/preferences.json`，写入防抖和原子替换，退出前刷新尚未保存的修改。数学字体许可证随应用包位于 `Contents/Resources/licenses/`。

新设置不存在时，Pecan 依次读取 Moye、MyMD 的设置并保存到自己的目录；已存在的 Pecan 设置优先，旧设置不会被改写。黑白坚果图标的 SVG、PNG 与 ICNS 位于 `assets/brand/`，可用 `sh scripts/build-icons.sh` 重建。

## 工程组织

| Crate | 职责 |
| --- | --- |
| `pecan-app` | GPUI 窗口、工作区、虚拟阅读列表、交互与后台任务生命周期 |
| `pecan-document` | 不可变源文档、分块、标题索引、搜索与 HTML 图片元数据 |
| `pecan-edit` | 原样 Markdown 保存、外部修改冲突检查、原子替换与权限保留 |
| `pecan-format` | 多语言格式化、资源限制、源码映射与阅读／导出共享结果 |
| `pecan-code` | 语言别名、GPUI Kit／Tree-sitter 高亮适配与缓存主题 |
| `pecan-assets` | 离线语言图标、提示块 SVG 与阅读／导出共享颜色 |
| `pecan-config` | 配置默认值、迁移、验证、持久化及最近路径历史 |
| `pecan-network` | 复用连接、可取消的异步图片下载与网络预算 |
| `pecan-export` | HTML、Markdown、Word、矢量 PDF、独立图表导出与原子写入 |
| `pecan-import` | anydoc 本地转换、资源预算与内嵌图片生命周期 |
| `pecan-mermaid` | Mermaid 布局、SVG、矢量缓存、PNG 与共享渲染队列 |
| `pecan-math` | LaTeX、化学公式、数学字体、预览缓存与共享渲染队列 |

所有内部版本、依赖配置与 lint 在工作区统一维护。功能 crate 不依赖 `pecan-app`；保存面板、GPUI Entity、用户通知、文件监视、防抖和退出刷新留在应用层。`pecan-code` 适配现有 GPUI Kit 高亮库，导出间接使用该库，但无需创建窗口；其余核心处理 crate 不直接依赖 GPUI。

职责和依赖边界见 [docs/architecture.md](architecture.md)。`samples/` 提供验收样例，`docs/` 保存设计与实测记录，`scripts/` 提供打包脚本，`vendor/` 保留已验证的上游局部补丁。

### 生成文件与清理

`target/` 只存放 Cargo 编译缓存、基准输出和临时验证文件；`dist/` 存放最新发行的应用、压缩包与校验值。正式的设计和验证结论保存在 `docs/`，可重复验收的文档保存在 `samples/`。

使用 `cargo clean --offline` 清除整个 `target/`，包括历史增量缓存、旧应用名的程序和临时导出文件。清理后首次构建需要重新编译依赖。更新发行产物时只保留最新一套；`vendor/` 中的补丁、原始清单及许可证属于源码和来源记录，应随工程保留。

## 性能

文档索引在后台建立。阅读区的外层虚拟列表按可见范围创建 Markdown 组件，避免完整文档组件的首次全量测量；最多保留 32 个阅读块、8 个图表预览。异步解析中的行保留占位高度，避免空行反复创建和淘汰。切换标签页释放旧页 UI 缓存，但保留源文档和滚动位置。

正文图片、公式与 Mermaid 的解码图像及 GPU 纹理随可见行或预览生命周期释放；缩放和主题变化替换旧图像，不积累历史栅格。系统字体数据库在进程内只初始化一次。代码使用彩色语义高亮，界面仍保持黑白灰。

Mermaid 使用 `mermaid-rs-renderer` 在 Rust 中解析、布局并输出 SVG，无需 Node.js、Chromium 或 WebView。`resvg` 解析后的矢量几何按源码与主题缓存，放大查看时重新栅格化；最多同时执行两项图表任务。搜索先防抖，再在后台执行，不复制或小写化整篇文档。

资源预算与实际测量见 [docs/performance.md](performance.md)。

## 验证

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
cargo doc --workspace --no-deps
# 按需用自己的本地技术文档验证图片和全部 Mermaid
PECAN_VERIFY_DOCUMENT=/path/to/technical-plan.md cargo test -p pecan-app external_document -- --ignored
cargo run --release -p pecan-document --example document_benchmark
cargo run --release -p pecan-document --example code_benchmark
cargo run --release -p pecan-mermaid --example mermaid_benchmark
cargo run --release -p pecan-math --example math_benchmark
cargo run --release -p pecan-config --example presentation_benchmark
cargo run --release -p pecan-export --example export_document -- samples/syntax-highlighting.md /tmp/pecan-exports
```

GPUI 测试使用独立调度器并共享格式化信号量，按 CI 配置串行运行，避免不同测试线程互相唤醒。

工作区自动化测试覆盖核心处理、数学／化学渲染、系统字体选项、旧默认字号迁移和自定义保留、Moye／MyMD 设置导入与新设置优先、设置保存、原生菜单全屏动作、Escape 优先级、标题居中／全屏空间回收、三栏位置和侧栏独立开关、长代码有限视口／两轴滚动／完整复制／末尾定位，以及 GPUI 大文档布局。本版新增 DOCX 图片／公式保留及临时资源清理、文本与扫描 PDF 分流、ZIP 解压放大与超限拒绝、原生 PDFKit 页面／可选择文本读取、正文宽度迁移和实际窄窗口图片自适应；Reader 集成测试验证导入标签、原文件目录、重载／关闭清理及窄窗口 PDF 操作区。图片和 Mermaid 的实际绘制／点击／悬停测试验证独立查看窗口、缩放、缓存复用、行内排版、引用式图片和关闭预览。专项测试读取外部文档中的图片与全部 Mermaid，在不同窗口大小和 2× 像素比例下核对完整可见、实际 PNG 清晰度，以及全屏 Escape 返回文档；文档内容不会保存在项目中。桌面核验范围详见性能记录。

## Word、PDF 与其他文档

使用 ⌘O 或文件栏直接打开 DOCX／DOC、PDF、ODT／ODP／ODS、RTF、EPUB、PPTX／PPT、XLSX／XLS 和 CSV 等 anydoc 支持的格式。导入内容在后台转换为 Markdown，复用现有字体、搜索、大纲、代码和公式渲染；原始文件不改写。

DOCX 等 Office 文档只解析一次，支持的内嵌图片提取到私有临时目录，并随标签页重载／关闭释放。Word 公式转换为 LaTeX；精确分页、批注、修订和所有复杂排版不保证保留。

PDF 默认重排提取的文本；多栏阅读顺序、字体编码和复杂表格受源 PDF 的结构影响。文本重排不提取 PDF 图片或复现原版排版。点击中间标签栏右侧的「原版查看」，或按 ⌥⌘P，使用 macOS PDFKit 打开独立可调整大小／全屏窗口，默认适配屏幕可用区域，保留页面、图片和可选择文本。⌘W 或 Escape 关闭原版窗口并返回阅读器。扫描或混合扫描 PDF 会明确提示使用原版查看；当前不进行 OCR，也不上传文档。Windows 原版查看使用系统默认 PDF 阅读器；重排转换不依赖 PDFium、WebView 或外部 Office 安装。

anydoc 固定为 0.2.4 的 MIT 源码版本，新增模型序列化入口以保留图片，收紧桌面资源预算；来源与补丁详见 `vendor/anydoc/MOYE-PATCH.md`。全应用同时最多两项转换，输入与重排 Markdown 各限 32 MiB、内嵌图片总计 32 MiB；ZIP 单项解压限 32 MiB、累计读取限 64 MiB，并限制条目、XML 节点及表格展开。PDFKit 原版支持 1–2000 页、最多 8 个窗口。预算用于限制资源消耗，不等于进程 RSS 上限或最坏耗时保证。

## 兼容范围

支持常见 Markdown 标题、段落、强调、列表、引用、任务列表、表格、代码块、链接和图片。相对图片路径以文档所在目录解析。

Markdown 图片（含引用式、行内和带链接的图片）与单独的 HTML `img` 标签支持点击放大。图片与 Mermaid 使用同一 GPUI 预览框架、主题和底部缩放栏，默认完整适配窗口，支持缩放、原始尺寸、拖动、触控板缩放、调整大小及全屏；Escape 返回文档。图片直接复用正文已解码的完整像素，不再次下载或编码临时 PNG；尚未完成加载的图片在预览窗口继续异步加载，使用窗口局部缓存。macOS 图片右键菜单保留「系统快速查看」作为可选操作。混合 HTML 片段继续由原有 HTML 阅读器排版。示例见 `samples/media.md`。

行内 code 使用浅灰底色、1px 细边框、左右各 5px 内边距和 3px 圆角；额外的字形位置校正保持原有占用宽度、换行及选择坐标，首尾显式空白保持原样。字形度量随现有排版缓存保存，普通重绘和滚动不重新测量。

高亮提示块使用 GPUI Kit 内置的 16px Lucide 矢量图标，正文和导出均复用同一组本地资源。HTML 嵌入 SVG，PDF 保留矢量，Word 复用 2× PNG；用户写在正文中的 emoji 保留原样。Lucide 许可随源码和应用包提供，无需网络图标服务。

图片、已渲染 Mermaid 预览和图表操作按钮使用 GPUI 的小手指针，避免原生自定义指针与 GPUI 恢复逻辑竞争。Mermaid 卡片使用源码、复制、展开图标并提供悬停提示；Mermaid 独立查看窗口按实际显示尺寸和屏幕像素比例从缓存矢量生成高清 PNG，默认完整适配；调整窗口大小自动重算，80ms 合并调整事件，64px 尺寸分桶，缩小时复用已有高清图。放大窗口仅用于查看图像：跟随应用亮暗主题的画布、系统窗口按钮与底部居中的缩小／比例／放大／适应窗口工具条，控件不遮挡图表。查看和复制源码在正文卡片中完成，源码按钮使用 gpui-kit 现成的 `CodeXml`（`</>`）图标。预览的背景、标题栏与底部控件复用应用主题；预览打开期间切换主题时，图表同步重新着色。按钮图标 16px，点击区域 28px，支持键盘焦点；独立查看窗口即使处于全屏，Escape 也直接关闭并返回原文档。

Mermaid 使用原生实现，覆盖流程图、时序图、类图、状态图、ER 图、饼图、甘特图、思维导图等图表；并不承诺与 Mermaid.js 的所有版本、扩展语法和布局完全一致。无法渲染时显示错误，仍可查看和复制源码。读取文档不会执行 JavaScript。

顶层与引用、列表、`:::` 内的代码块共用 GPUI 原生只读 Editor。保留全文展开、水平滚动、语法高亮、结构折叠和原生边框；纵向滚轮交给正文。代码块语言栏保留 Tab、折叠和复制，加载时自动格式化，选择 Tab 后重新格式化当前块。设置与工具栏支持自动识别及 2／4／8 列；无法推断的 Tab 使用 4 列。复制格式化后的完整排版，搜索仍按原始源码定位。

格式化由独立 `pecan-format` crate 完成，不需要 Node.js、外部命令、网络或平台专用组件。支持 JSON／JSONC／JSON-LD、YAML／YML、TOML、JavaScript／TypeScript／JSX／TSX、CSS／SCSS／Sass／Less、HTML／XML／Vue／Svelte／Astro，以及 SQL／MySQL／PostgreSQL／SQL Server 等别名。JSON 保留注释、重复键、顺序和原始数字写法；YAML 验证解析事件，TOML 验证字面量与注释；JS/TS 使用 dprint、样式使用 Malva、标记使用 markup_fmt、SQL 使用 sqlformat。SQL 是词元排版，不进行数据库语义校验。Rust、Python、Shell 等尚未接入专用格式化器，保留原排版；仍可设置实际 Tab 的显示宽度。

源 Markdown 与 Markdown 导出保留原文。PDF／Word／HTML 按全局 Tab 配置使用格式化代码；单块工具栏的临时宽度仅影响阅读块。导出优先复用同配置的阅读结果，缺失结果在后台安全准备。语法无效、不能可靠建立源码映射或超过资源预算时，保留原文；手动 Tab 操作失败会显示原因。JSON 输入最多 2 MiB／输出16 MiB，YAML/TOML 输入256 KiB／输出1 MiB，新后端输入64 KiB／输出1 MiB，均限制复杂度。整份文档最多缓存8 MiB格式化文本和映射、4096个唯一结果，批量任务预算2秒；超过预算的块保留原文。相同源码复用计算结果，折叠和 Tab 操作状态各块独立。最多两项后台格式化任务，快速切换仅最后一次结果生效；关闭文档取消后续工作，主题与字体变化复用准备结果，滚动不重复执行批量格式化。详细验证请打开 `samples/auto-formatting.md`。

后台文档索引仍按源页保留搜索位置，显示聚合为一个完整代码卡片；Editor 按祖先裁剪区只绘制可见行。其他 Markdown 语义块连同引用定义超过64 KiB时按页查看完整源码；超长标题仅在大纲中截短，正文保留原文。

## 依赖选择

- anydoc：本地多格式转换；锁定上游源码，并保留内嵌图片与有界资源消耗。
- PDFKit / objc2：macOS 原生 PDF 原版查看，无额外 PDF 引擎或全页栅格缓存。
- krilla / krilla-svg：跨平台 PDF 输出，文字、字体和 SVG 图表保留清晰度。
- GPUI Kit：统一 GPUI、组件和平台版本，原生 GPU 绘制。
- pulldown-cmark：快速流式 Markdown 事件与源位置索引。
- mermaid-rs-renderer：原生 Mermaid 解析、布局和 SVG 输出。
- resvg：可复用的 SVG 几何与高质量矢量栅格化；版本与 GPUI 对齐。
- mathra：原生数学与化学排版，内嵌 STIX Two Math 和 Latin Modern Math。
- Tree-sitter：通过 GPUI Kit 的现成语法及查询提供高亮；SQL／数据库别名、TSX／JSX、C／C++／C#、Java／Go／Rust／Python／Ruby／Swift／Kotlin／Scala／Lua／Elixir／Zig，JSON／YAML／TOML，CSS／HTML／Astro／Svelte，Bash／CMake／Make，GraphQL／Proto／Diff／EJS／ERB／JSDoc／Markdown 等。Vue／XML、SCSS／Sass／Less 使用宿主语法。未提供 grammar 的语言保留纯文本。
- regex：Unicode 大小写折叠和高效字面量搜索。
- BLAKE3、LRU：内容缓存身份与明确的缓存淘汰。
- notify：系统文件变化通知。

选择依据是使用场景、可测性能和版本兼容性；“最新版本”本身不代表更好的性能。


无标题高亮与 Markdown 结构示例见 [samples/callouts.md](../samples/callouts.md)。

## 手动验收

打开 `samples/acceptance.md`，按 `samples/acceptance-guide.md` 检查阅读、交互以及 Word／PDF 导出。长代码和网络图片分别使用 `samples/long-code.md`、`samples/network-image.md`；其余 samples 文件保留此前的分项样例。保持这些文件与 `media-preview.svg` 的相对目录结构。

多语言高亮及格式化导出专项验收见 [samples/syntax-highlighting.md](../samples/syntax-highlighting.md)。
