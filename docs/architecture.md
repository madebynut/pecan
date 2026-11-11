# Pecan 工程边界

工作区采用 12 个平级 crate。主 crate 负责桌面事件与界面；可独立调用的处理、持久化和输出各有明确归属。各 crate 使用具体类型和少量函数，不引入插件注册、通用服务层或额外调度框架。

## 依赖与职责

`pecan-app` 组合其余功能 crate，功能 crate 均不依赖应用。

- `pecan-document` 提供源文档、分块、搜索和 HTML 图片元数据。
- `pecan-edit` 负责 UTF-8 Markdown 的原子保存、外部修改冲突检查和权限保留，不依赖 UI 或平台专有组件。
- `pecan-format` 依赖文档模型，拥有格式化结果、源码映射、批量准备和缺失导出结果的缓存。
- `pecan-code` 适配现成的 GPUI Kit／Tree-sitter 语法库，阅读和导出共用语言解析与主题。
- `pecan-assets` 依赖文档中的提示块类型，提供离线 SVG、语言标识和共享颜色。
- `pecan-config` 使用 Mermaid 的平台字体选项与图表字体类型，负责默认值、迁移、验证、原子保存、最近文件数据和首次启动记录。
- `pecan-network` 实现现有 HTTP client 协议，拥有连接池、一个网络工作线程、并发门限、超时和取消处理。
- `pecan-import` 依赖文档模型，负责本地格式转换与临时资源。
- `pecan-mermaid`、`pecan-math` 各拥有渲染器、矢量缓存和同领域共享队列。
- `pecan-export` 组合文档、配置、格式化、高亮、图标与渲染，负责资源收集、排版和最终原子提交；网络客户端由调用方提供。

`pecan-code` 仍依赖 GPUI Kit，这是复用其现成语法库的适配边界。导出不依赖应用的 App、窗口和保存面板，可以通过 example 在无窗口进程内运行。全工作区当前实测平台为 Apple Silicon macOS。

## 生命周期留在桌面层

首次启动使用独立的 `launched` 标记，不受恢复默认设置影响；使用原生配置目录和文件独占创建，已有设置或历史的升级安装不会再次自动显示欢迎页。标签为空是合法状态，关闭全部时取消加载与搜索、释放文件监视和文档实体；查找、复制、导出及标签切换安全停用，打开文件和帮助仍可用。

GPUI Entity 与 Task、设置／历史保存防抖、文件监视、原生保存面板、通知、预览窗口与退出刷新属于 `pecan-app`。后台调用由应用或 example 的执行器安排，渲染与格式化 crate 不启动新的常驻线程。

源码编辑复用 GPUI Kit 的 Rope、撤销和 Tree-sitter；编辑会话按标签保留，只在首次进入源码模式创建。输入不触发整篇文档重新解析，切回阅读、大纲导航、保存和导出时在后台准备最新快照。关闭、退出和重新加载共用未保存修改保护；异步回调按标签实体和版本匹配，不覆盖新输入。

导出捕获不可变 Snapshot：源文档、配置与导入资源通过 Arc 延长生命周期，代码结果共享不可变缓存。标签关闭不影响已开始的导出。阅读、图表预览和导出共用原渲染队列，队列克隆共享缓存及信号量。格式化阅读与导出共用同一全局并发门限和原有预算。

`pecan-export::typography` 在 crate 内共享平台字体解析、字形塑形和可见墨迹／字体指标。PDF 直接使用这些字形；Word 用同一指标定位原生 DrawingML 图标，文字保留 Word 基线；HTML 将相同指标转为随字号缩放的 baseline 对齐值。三种输出复用 Mermaid 已有的系统字体目录，不增加第二套全局字体扫描或渲染队列。Word 的抽象 macOS 中文默认值使用 Office 可枚举的 Hiragino Sans GB；显式字体选择保持。

应用的提示块观察 GPUI 已排版的首行可见字形；图标放在独立 gutter 中，正文的选择、换行和链接仍由原生文本布局负责。导出指标只在导出阶段测量，应用首行指标仅在请求观察时按布局缓存。Word 短容器使用原生不拆行约束，已知最低高度超过页内高度时解除此约束，最终字体回退、换行和分页仍交给 Word。

## 新功能的归属

新增格式化语言放在 `pecan-format`；语言别名和高亮适配放在 `pecan-code`；输出格式放在 `pecan-export`；设置数据和迁移放在 `pecan-config`。交互式控件留在应用层。只有职责、生命周期或复用边界清晰的处理模块才另开 crate，避免为了拆文件引入新框架。

## 可复现验证

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps
cargo run --release -p pecan-config --example presentation_benchmark
cargo run --release -p pecan-export --example export_document -- samples/syntax-highlighting.md /tmp/pecan-exports
```

所有测试位于其所属功能 crate；原生交互和组件绘制测试仍在应用。重构以源文档、导出 XML／媒体／PDF 页面及渲染结果的对照验证保持现有行为。
