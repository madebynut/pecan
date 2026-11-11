# 源码编辑 · Source editing

在标签栏点击 `</>`，或按 **⌘E / Ctrl+E**，切换阅读与源码模式。

这是一份可复制的验收文档。试试 **加粗**、*斜体*、~~删除线~~、`inline code` 和 [链接](https://github.com/madebynut/pecan)。

## 编辑与保存

- [ ] 修改这个标题，切回阅读确认预览。
- [ ] 按 ⌘S / Ctrl+S 保存，再重新打开检查内容。
- [ ] 输入文字后关闭标签，分别检查「取消」「保存」「不保存」。
- [ ] 修改后切换标签，再回来检查草稿和撤销。

```rust
fn main() {
    let greeting = "你好，Pecan";
    println!("{greeting}");
}
```

```json
{"source":true,"indent":  4,"preserve_spacing":true}
```

> [!TIP]
> 源码保存会保留你写下的 Markdown。代码块在阅读视图中的格式化不会改写源文件。

::: info
源码模式同样保留 **提示块**、列表和其他扩展语法。
:::

| 功能 | 操作 |
| --- | --- |
| 查找 | ⌘F / Ctrl+F |
| 另存为 | ⇧⌘S / Ctrl+Shift+S |

```mermaid
flowchart LR
    A[源码编辑] --> B[阅读预览]
    B --> C[保存 Markdown]
```
