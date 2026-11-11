# Word 表格列表与代码边框验收

本页检查表格内的 Markdown 基础格式、原生列表和代码块边框。建议在 Word 75%、100% 和 125% 缩放下检查四边，随后核对跨页代码的最后一行。

导出 PDF 和 HTML 时，下表每个单元格应与 Word 一致：列表带原生标记，任务项显示复选框，说明与列表各自成段，源文中的 `-`、`[x]`、`3.` 不作为文字出现。

## 表格内的基础格式

| 场景 | 内容 |
| --- | --- |
| 截图原例 | * 提供配置拉取接口（HTTP）；<br><br>* 接收组件上报并转换为统一格式投递 MQ |
| 基础格式 | **加粗**、*斜体*、~~删除线~~、`代码`、[链接](https://example.com) |
| 有序列表 | 3. **第三项**<br>4. *第四项*<br>5. ~~第五项~~与`code` |
| 单个项目 | - 单项也应显示原生列表标记 |
| 任务列表 | - [x] **已完成**<br>- [ ] 待办 |
| 混合段落 | 列表前的说明<br><br>- **第一项**<br>- 第二项<br><br>列表后的说明 |
| HTML 列表 | <ul><li>父项<ul><li>子项</li></ul></li><li>同级项</li></ul> |
| 原样代码 | `* literal<br>`、`<br>`、a\|b，代码内标签不得变成换行 |
| 显式换行 | 第一行<br>第二行 |

## 普通代码块

```rust
fn main() {
    let value = 42;
    println!("value = {value}");
}
```

## 提示块内的表格与代码

:::
**表格内的样式与正文使用同一套 Markdown 解析。**

| 类型 | 内容 |
| --- | --- |
| 嵌套无序列表 | - **加粗**<br>- *斜体*<br>- ~~删除线~~与`代码` |
| 嵌套有序列表 | 7. 第一项<br>8. 第二项 |

```json
{"enabled":true,"servers":[{"name":"local","port":8080}]}
```
:::

> [!NOTE] 提示块中的代码
> **标题和图标间距保持 8pt。**
>
> ```yaml
> service:
>   name: pecan
>   enabled: true
> ```

## 跨页代码块

下面的代码保留全部行。语言栏应在续页重复，最后一行下面应有完整原生底边框。

```text
001: cross-page native Word code border verification
002: cross-page native Word code border verification
003: cross-page native Word code border verification
004: cross-page native Word code border verification
005: cross-page native Word code border verification
006: cross-page native Word code border verification
007: cross-page native Word code border verification
008: cross-page native Word code border verification
009: cross-page native Word code border verification
010: cross-page native Word code border verification
011: cross-page native Word code border verification
012: cross-page native Word code border verification
013: cross-page native Word code border verification
014: cross-page native Word code border verification
015: cross-page native Word code border verification
016: cross-page native Word code border verification
017: cross-page native Word code border verification
018: cross-page native Word code border verification
019: cross-page native Word code border verification
020: cross-page native Word code border verification
021: cross-page native Word code border verification
022: cross-page native Word code border verification
023: cross-page native Word code border verification
024: cross-page native Word code border verification
025: cross-page native Word code border verification
026: cross-page native Word code border verification
027: cross-page native Word code border verification
028: cross-page native Word code border verification
029: cross-page native Word code border verification
030: cross-page native Word code border verification
031: cross-page native Word code border verification
032: cross-page native Word code border verification
033: cross-page native Word code border verification
034: cross-page native Word code border verification
035: cross-page native Word code border verification
036: cross-page native Word code border verification
037: cross-page native Word code border verification
038: cross-page native Word code border verification
039: cross-page native Word code border verification
040: cross-page native Word code border verification
041: cross-page native Word code border verification
042: cross-page native Word code border verification
043: cross-page native Word code border verification
044: cross-page native Word code border verification
045: cross-page native Word code border verification
046: cross-page native Word code border verification
047: cross-page native Word code border verification
048: cross-page native Word code border verification
049: cross-page native Word code border verification
050: cross-page native Word code border verification
051: cross-page native Word code border verification
052: cross-page native Word code border verification
053: cross-page native Word code border verification
054: cross-page native Word code border verification
055: cross-page native Word code border verification
056: cross-page native Word code border verification
057: cross-page native Word code border verification
058: cross-page native Word code border verification
059: cross-page native Word code border verification
060: cross-page native Word code border verification
061: cross-page native Word code border verification
062: cross-page native Word code border verification
063: cross-page native Word code border verification
064: cross-page native Word code border verification
065: cross-page native Word code border verification
066: cross-page native Word code border verification
067: cross-page native Word code border verification
068: cross-page native Word code border verification
069: cross-page native Word code border verification
070: cross-page native Word code border verification
071: cross-page native Word code border verification
072: cross-page native Word code border verification
073: cross-page native Word code border verification
074: cross-page native Word code border verification
075: cross-page native Word code border verification
076: cross-page native Word code border verification
077: cross-page native Word code border verification
078: cross-page native Word code border verification
079: cross-page native Word code border verification
080: cross-page native Word code border verification
081: cross-page native Word code border verification
082: cross-page native Word code border verification
083: cross-page native Word code border verification
084: cross-page native Word code border verification
085: cross-page native Word code border verification
086: cross-page native Word code border verification
087: cross-page native Word code border verification
088: cross-page native Word code border verification
089: cross-page native Word code border verification
090: cross-page native Word code border verification
```

代码结束后的正文，用于核对底边框和间距。
