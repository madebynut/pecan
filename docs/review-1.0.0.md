# Rust review：1.0.0 生产可靠性与资源控制

本次检查面向桌面应用和 11 个内部功能 crate，使用 Rust 1.98 / Edition 2024，重点覆盖文件加载、目录与标签生命周期、文档索引、图片与图表资源、格式化、配置持久化、导入导出及依赖。保留现有 UI 和导出版式，只修改有证据的可靠性、资源和平台问题。检查采用 rust-best-practices 与 rust-review；没有进行机械式 pedantic 重写或大规模替换渲染库。

## 必须修复：已修复

### 1. 图片和动画可绕过内存限制

位置：`vendor/gpui-pre/src/image_decode.rs:58`、`vendor/gpui-pre/src/svg_renderer.rs:190`、`vendor/usvg/src/parser/mod.rs:166`、`crates/pecan-app/src/raster.rs:18`。

**规则：** Microsoft Rust Guidelines `M-PANIC-ON-BUG`；Google Rust Style Guide 的错误处理原则；rust-best-practices §5、§12。外部输入超限应作为可处理的错误，不能依赖分配失败结束进程。

**问题：** 文件资源与内存图片有两套解码路径，本地读取、动画全部帧、SVGZ 展开缺少明确预算。内存图片的解码错误还会被转换成“仍在加载”。

**修复：** 资源和 Image 字节共用 `image_decode::decode`。编码输入上限 16 MiB，静态声明分配及动画累计像素 128 MiB，动画最多 4,096 帧；读取按上限 + 1 截止，解码器在分配前接收限制。SVG 同时约束外部/内嵌图片字节、内嵌像素和输出画布，SVGZ 在展开 16 MiB 后失败。失败会显示图片错误，并随可见状态释放缓存。正常 GIF 帧、时序、颜色、JPEG EXIF 方向与原分辨率保持原行为，不通过降采样或截帧掩盖超限。

```rust
let frame_bytes = check_decoder(&mut decoder)?;
animation(decoder.into_frames(), frame_bytes)
```

这些是每项资源预算，不代表整个进程 RSS 的绝对上限；解码器临时缓冲、GPU、字体和窗口也占用内存。

### 2. 后台重载改变了其他标签的文件监听，并能突破总源文本预算

位置：`crates/pecan-app/src/app.rs:323`、`:426`。

**规则：** Effective Rust 的所有权与资源生命周期原则；rust-best-practices §3、§7、§11。

**问题：** A 在重载期间切换到 B，A 完成后仍无条件设置 watcher，导致之后 B 的保存可能无法被发现。Replace 分支也没有重新检查 64 MiB 总源文本预算。

**修复：** 结果定位到原标签；非当前标签只更新自身文档。当前 watcher 使用独立变化信号，轮询读取当前信号，旧 watcher 排队的事件无法误触发新标签。重载超过总预算时保留旧文档及其他标签并提示，创建新的阅读视图前完成检查。

```rust
if index != reader.active {
    tab.view.update(cx, |view, cx| view.suspend(cx));
    return;
}
```

回归覆盖切换期间重载、旧信号迟到、当前信号触发刷新，以及超预算不丢旧视图。

### 3. 已有依赖命中可修复的安全通告

位置：`Cargo.toml`、`Cargo.lock`、`vendor/anydoc/Cargo.toml`。

**规则：** rust-best-practices §9，锁定并审计实际交付的依赖。

升级到 `lru 0.18.5`、`pulldown-cmark 0.13.6`、`zip 9.5.1`，同时将已撤回的 `async-executor 1.14.0` 更新为 `1.14.1`。分别处理 RUSTSEC-2026-0253、0433、0367/0368/0369/0429 和 yanked 版本。ZIP 读取测试适配新的可失败名称 API；实际导入依然使用有界 part 读取，没有调用 `ZipArchive::extract`。

此次通告里的 LRU 风险要求 panic 展开及会 panic 的键析构，wikilink 风险要求启用对应选项，ZIP 风险要求使用提取接口；当前主要应用路径不满足这些触发条件，但仍采用已修复版本，不以不可达判断代替依赖维护。

## 应当修复：已修复

### 4. 快速展开兄弟目录会丢失一次加载

位置：`crates/pecan-app/src/app.rs:506`、`crates/pecan-app/src/workspace.rs`。

**规则：** rust-best-practices §3、§7、§11：任务所有权必须与请求生命周期一致，先复现再修改。

单个 `tree_job` 被下一次展开覆盖，旧任务取消但目录仍标为展开。现在按目录保存任务，收起取消该子树、切换根目录取消全部。最多 8 个等待中的展开请求、2 个并行磁盘扫描、20,000 个展开条目；失败不改变已有条目，可收起后重试。两个展开在同一次 UI 更新中启动的回归，在修复前实际失败，修复后通过。

```rust
self.tree_jobs.insert(job_path, job);
```

### 5. 提示块索引反复扫描全部容器，密集结构占用不受源字节约束

位置：`crates/pecan-document/src/document.rs:407`、`:428`、`:740`。

**规则：** rust-best-practices §12：测量后优化；§5：将资源拒绝表达为 Result，原文保持不变。

排序且互不重叠的容器通过 `partition_point` 查找，替代每个 Markdown 事件的全量 `any/filter`。超长标题在收集时只保留用于标签的 UTF-8 前缀，避免先分配全文再截断。语义解析前限制围栏代码以外的 500,000 个非空行，并设置 64 MiB 索引计费预算，避免几 MiB 的密集标题制造数十万 UI 索引项；长围栏代码仍无损分页。

```rust
let end = spans.partition_point(|span| span.source.start < offset);
end > 0 && offset < spans[end - 1].source.end
```

基准采用本机 release 构建、5 次测量中位数（数字用于同机比较）：

| 输入 | 修改前 | 修改后 |
| --- | ---: | ---: |
| 16,000 个提示块，864,000 字节 | 653.0 ms | 12.5 ms |
| 500,001 个短标题，索引进程峰值 RSS | 161.5 MiB | 5.2 MiB（进入解析前拒绝） |
| 普通 10 MiB 文档，97,090 个标题 | — | 37.1 ms |

修改前基线来自修改前提交的独立 `pecan-document` 构建。RSS 是异常输入测试进程的测量，不是整款应用的降幅。CPU 调度及热缓存会影响计时；普通文档的版式没有因优化而改变。CRLF、容器内标题、代码围栏中的 `:::`、引用定义与 50 万行围栏代码都有回归覆盖。

### 6. Windows 回归中的 macOS 假设

位置：`crates/pecan-app/src/diagram.rs`、`recent.rs`、`app.rs`、`.github/workflows/ci.yml`。

**规则：** rust-best-practices §11：针对每个平台验证真实行为。

使用实际安装的 CJK 无衬线/衬线和等宽字体，继续比较实际预览像素；路径断言采用平台规范化后的路径；全屏验证分别遵循平台窗体边距；最近文件测试初始化完整 GPUI 菜单环境。没有通过跳过这些测试消除失败。Intel 原生测试已通过，但首次冷构建在 release 编译时触及原来的 60 分钟作业限制，调整为 120 分钟，保留 LTO 和全部质量检查。构建触发仍仅限 tag push。

三平台 CI 安装同一固定提交、SHA-256 校验的 Noto CJK 测试字体，避免托管 macOS/Windows 镜像缺失可选中文字体；下载校验通过后才原子替换。导入生命周期测试从入口就使用平台绝对路径，覆盖 Windows 规范化后查找和关闭标签的行为。字体安装只准备测试环境，不改变应用默认字体或打包内容。

## 需要持续跟进

依赖审计更新后没有 vulnerability、unsound 或 yanked 命中，但默认 `cargo deny check advisories` 仍因 9 类停止维护公告返回失败：instant、paste、rustls-pemfile、rustybuzz、smartstring、ttf-parser，以及 Linux 依赖链的 zbus_names、zvariant、zvariant_derive。没有增加忽略规则，也没有将此项报告为通过。后续应配合 GPUI、resvg、krilla 和相关上游升级；本轮不贸然迁移整形引擎、字体解析和宏依赖，以保护现有版式与跨平台行为。

资源防护不是完整的恶意输入隔离。格式解析仍运行在应用进程中，尚未进行长期 fuzz、Windows 实机高 DPI / IME / GPU 驱动压力测试，或整机长时间 RSS/FPS 对比。

## 保留的良好设计

- 11 个职责清晰的功能 crate，格式化、导入、导出、数学、Mermaid 与网络各自独立。
- 阅读块和图片随可见状态释放；搜索结果、格式化结果、图表与数学缓存已有边界，继续保留。
- 导出持有不可变文档和临时资源快照，写入临时文件后原子替换；格式化失败保留原文。
- 原生代码边框、提示块、表格、胶囊等版式沿用既有实现与回归，没有为本轮审查改写样式。

## 验证

- 工作区：281 项通过，11 项环境专项保持 ignored；没有新增跳过项。
- Rustfmt、严格全目标全特性 Clippy、Windows MSVC 交叉 Clippy、warnings-as-errors rustdoc 通过。
- actionlint、打包脚本 shellcheck、diff 空白检查通过。
- `samples/acceptance.md` 的 HTML / PDF / Word 与修改前逐字节相同；Markdown 仅临时资源目录名称不同，正文及对应图片字节相同。DOCX ZIP 完整性和 XML 解析通过。
- 原生 CI：前一轮 macOS ARM 完整通过；Intel 测试通过但冷构建超时；Windows 测试暴露的平台假设已修正。本轮修改仍须以新 tag 的 CI 结果为准，不将前一轮结果算作新提交验证。

本地 QA：`2026-11-11/pecan-production-review/`，包含四格式导出、逐项比较 JSON、基准与检查日志。
