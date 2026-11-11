# Pecan 0.8.4 布局与工程 Review

本轮按应用与内部功能库审查 11 个 crate 的职责、错误路径、资源生命周期及 vendor 扩展，重点追踪提示块从 Markdown 到 GPUI、HTML、Word、PDF 的整条渲染链。Rust edition 2024，MSRV 1.98。结论是保留已有原生布局、缓存和并发边界，把错误的视觉基准与重复约束收敛到其真正归属；不为了减少文件或行数改写有效的渲染机制。

## 必须修复：本轮已处理

### 1. 图文对齐使用了不同的基准

位置：`crates/pecan-app/src/notes.rs:47`、`vendor/gpui-pre/src/elements/text.rs:811`、`crates/pecan-export/src/typography.rs:89`、`crates/pecan-export/src/pdf/text.rs:72`、`crates/pecan-export/src/word.rs:700`、`crates/pecan-export/src/document.rs:508`。

**规则：** Rust best practices §4「Model the domain with types」、§4「Preserve public behavior」、§11「Test observable behavior」；[Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) 的 C-CUSTOM-TYPE。行盒、文字基线和可见字形中心是不同量，不能用统一的固定偏移冒充。

**问题：** 应用按行盒中心放图标，输出分别依赖 CSS 垂直居中或文字 position 补偿。中英字体回退、加粗和 code 改变指标后，文字与图标会向不同方向偏移。

**处理：** 对齐目标统一为首行可见字形的中心。应用读取原生排版后的 glyph bounds；输出共用 `typography` 中的字体回退、塑形和 `Ink` 指标。PDF 对 symbol 使用 `TextCenter`，数学保留自身 baseline；HTML 输出随字号缩放的 baseline 值；Word 将 DrawingML 图标锚定到段落，文字基线不移动。

```rust
let drop = ink.object_drop(icon_height);
let icon_top = baseline + ink.center() - icon_height / 2.;
```

Word 用原生 tab stop 与 hanging indent 为图标留 gutter，图标11pt、间距8pt；不再用空格猜宽。原生 Metal 回归覆盖中英混排、强调、放大标题、code 和换行；导出检查覆盖有／无标题提示块及嵌套 Markdown。

### 2. Word 实际字体与用于测量的字体不同

位置：`crates/pecan-export/src/word.rs:1355`、`crates/pecan-export/src/document.rs:479`。

**规则：** Rust best practices §11「Test observable behavior」和 §4「Preserve public behavior」。校验输出必须使用渲染器实际能获取的资源。

**问题：** macOS 的 PingFang MobileAsset 能被字体目录发现，但本机 Word 不枚举该字体，因而回退到另一字体；单纯继续调偏移无法解决差异。HTML 原先把 generic fallback 插在明确字体之间，也会提前跳过中文选择。

**处理：** Word 的抽象 macOS 中文默认值解析为 Office 可枚举的 Hiragino Sans GB，测量与样式使用同一选择。明确指定的用户字体保持原值。HTML 先列明确英文、中文字体，再列 generic fallback，并保留 CSS 字体名转义。

```css
font-family: "English Family", "Chinese Family", system-ui;
```

### 3. Word 容器叠加同页约束导致分页异常

位置：`crates/pecan-export/src/word.rs:398`、`crates/pecan-export/src/word.rs:596`、`crates/pecan-export/src/word.rs:1499`。

**规则：** Rust best practices §1「Application code should favor simplicity」、§4「Preserve public behavior」。一种布局职责应由一个机制处理，避免相互冲突的约束。

**问题：** 提示块外层 row 的 `cantSplit` 与内部 NoteTitle 的 `keepNext` 重复约束分页；长块会留下只有章节标题的空白页。全面禁止 row 拆分也不适用于超过一页的内容。

**处理：** 短块保留原生 `cantSplit`；逐段累计原生最低高度的保守下界，仅当块明确超过页内高度时解除外层不拆行约束。移除 NoteTitle 重复的 `keepNext`，保留 `keepLines`。自动换行、表格最终高度和分页仍由 Word 决定，没有引入另一个分页引擎。

```rust
if frame.minimum_height >= PAGE.1 - 2 * MARGIN {
    self.body.replace_range(frame.cant_split_at..frame.cant_split_at + CANT_SPLIT.len(), "");
}
```

样例从5页恢复为4页；短块内 JSON 语言标识与完整原生边框保留，长块尾部正文仍可见。Word 输出明确设置 compatibilityMode15，属性顺序和 OPC 关系有结构回归，表头底色不再被异常样式覆盖。

### 4. 网络 Runtime 的析构可能在异步环境中 panic

位置：`crates/pecan-network/src/lib.rs:22`、`crates/pecan-network/src/lib.rs:51`。

**规则：** Rust best practices §5「Destructors never fail, panic, or block」；[Rust Design Patterns](https://rust-unofficial.github.io/patterns/) 的 RAII 资源管理。

**问题：** Tokio Runtime 的默认析构会等待工作线程，在另一个异步 executor 中释放最后一个 Client 时可能 panic。TLS 初始化失败也不应留下需要阻塞析构的 Runtime。

**处理：** 先完成可失败的 TLS client 构造，再创建 Runtime；Client 单独持有 Handle，Drop 用 `Option::take` 转移 Runtime 并调用 `shutdown_background`。取消 guard、连接池、两项并发门限和一个工作线程保持。

```rust
if let Some(runtime) = self.runtime.take() {
    runtime.shutdown_background();
}
```

回归在外部 Tokio executor 内创建并释放 Client，另覆盖重定向、下载大小限制及取消。

## 应收敛的实现：已处理

字体回退和塑形从 PDF 私有模块迁到 `pecan-export::typography`，只在 crate 内共享，复用 Mermaid 已有系统字体目录；没有增加一个字体扫描器、公共抽象层或新的功能 crate。GPUI 首行墨迹观察为 opt-in，按已塑形 layout 缓存，普通 TextView 不触发字形查询。

撤掉提示文字固定 top／position 偏移及延后补偿数组；以 paragraph-relative symbol、原生 gutter 和共享 glyph 指标取代。vendor 扩展记录与 registry diff 同步，便于后续与上游合并。CI 工作区测试与本机验证统一为串行执行，避免 GPUI 测试 scheduler 经共享格式化 semaphore 相互唤醒；CI rustdoc 同时按 warnings-as-errors 检查。

**规则：** Rust best practices §4「Private by default」、§9「Small single-purpose crates」、§12「Optimize after measuring」。本轮未新增第三方 package 或常驻任务；已有输入、输出、栅格与缓存预算保持。

## 保留的有效修复

- GPUI 原生边框的坐标取整修复：纠正跨窗口原点的取整不一致，下一阅读行的背景不会覆盖代码卡片最后一个设备像素。已另行执行两个真实 Metal 滚动回归，不改成手绘独立底线。
- 原生 Editor 的可见行塑形、完整展开与水平滚动，以及表格 max-content tracks：直接服务性能和正确布局，不能仅因为位于 vendor 就删除。
- 行内 code 的可选择文字、源码范围映射、灰底及原生边框：保留用户已验收的内边距与强调语法。
- Mermaid 的矢量缓存、原有箭头／字体适配、预览栅格预算，格式化的两项共享门限及不可变准备结果，导出的同目录临时文件与原子提交。

## 后续边界

1. HTML 与 Word 的基线定位使用 leading paragraph 的字形指标，显式换行会停止首行测量；自动换行最终由浏览器／Word 完成。接收方缺少所选字体时仍可能回退，不宣称任意机器、任意字体都像素一致。Windows 字体映射有代码路径，本轮没有 Windows 实机验证。
2. `crates/pecan-network/src/lib.rs:94` 当前 request-body 读取有字节上限，时间限制主要覆盖 reqwest 请求；应用当前只发送图片 GET 的空 body。若增加上传或无限流输入，应把整个 body 读取放进截止时间，并增加 stalled-body 回归。规则：Rust best practices §7「Concurrency and async」，有界资源应包含等待时间；本轮不扩展上传协议。
3. `crates/pecan-export/src/typography.rs:222` 及既有系统字体渲染路径使用 fontdb 的共享映射，依赖导出期间字体文件不被原地修改。规则：Rust best practices §8「Unsafe and FFI」。保留现有生命周期与安全注释；若支持字体文件热编辑，应改为持有不可变字节快照并测量内存成本。
4. pedantic Clippy 提示还包括 must_use、文档标记、既有函数长度和数值转换等。没有把它们当成无差别重写的理由；项目已有统一 lint 表，严格配置的全部 warning 已清零。大规模重排 API 应独立变更并建立行为基线。

## 验证

244 项工作区测试通过，10 项环境专项默认 ignored；额外执行3项真实 Metal 专项通过。rustfmt、全目标全特性严格 Clippy 和 warnings-as-errors rustdoc 通过。三个源文档均执行生产四格式导出：两个无图片样例的 Markdown 与源文件逐字节一致；技术方案中的图片引用按既有便携导出行为改为随附资源目录，其余正文和代码保持原文。源文件不被导出修改。

综合／分页／技术方案的 PDF 分别2／4／16页，Word 分别2／4／17页，已逐页查看。HTML 在500px、1180px阅读宽度及打印模式下检查；Word 自动渲染使用 bundled LibreOffice，Microsoft Word 的对齐结果另由用户实际打开验收，最新反馈为对齐可接受、间距过小，随后间距调整为8pt。本轮没有新的 RSS 或持续 FPS 测量，局部按需缓存不等于已证明整体内存下降。

源码验收：`samples/callout-alignment.md`、`samples/callout-pagination.md`。日志与导出产物：`本地 QA/2026-11-05/pecan-callout-layout/final/`。用户技术方案仅用于本机 QA，不纳入源码与应用包。
