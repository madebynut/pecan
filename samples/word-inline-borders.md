# Word 行内代码与细边框验收

用 Word 在 75%、100% 和 125% 下查看本页。胶囊应是一块完整的灰底，没有边框，上下左右都不出现多余的灰边或白缝；灰底覆盖左右内边距，在表头、引用和提示块底色上仍可辨认，表格文字保持垂直居中。代码卡片应使用浅灰细边框，语言栏与代码正文连续。

## 中英文与基础格式

| 场景 | `Header` 与 `中文表头` |
| --- | --- |
| 中文无空格 | 正文`代码`正文，接口`request_id`返回。 |
| 英文无空格 | before`code`after, `GET /api/devices` and `gjpqy`。 |
| 混合文字 | `设备ID device_id`、`状态Status`、`中文`与`English`。 |
| 胶囊内格式 | `**加粗 Bold**`、`*斜体 Italic*`、`~~删除 Strike~~`。 |
| 周围格式 | **加粗**`code`*斜体*，~~删除线~~`代码`[链接](https://example.com)。 |
| 列表与换行 | - 第一项`代码`<br>- 第二项 `code`<br><br>显式换行后 `gjpqy`。 |

## 嵌套表格与代码

:::
内容`code`与图标保持原有布局。

| 场景 | 内容 |
| --- | --- |
| 不透明白底 | 正文`代码`正文、`gjpqy`，不能透出外层提示块底色。 |
| 多行内容 | - **第一项**`code`<br>- *第二项*`代码` |

```json
{"enabled":true,"name":"碧果","description":"native editable code"}
```
:::

> [!NOTE] 提示块 `code` 标题
> 正文 `code` 与 `中文` 保留完整灰底。
>
> | 字段 | 说明 |
> | --- | --- |
> | `device_id` | 正文`代码`正文 |
>
> ```yaml
> service:
>   name: pecan
>   enabled: true
> ```

> 引用块 `code` 与 `中文` 保留完整灰底。
>
> | 字段 | 内容 |
> | --- | --- |
> | `id` | `device_id` |

长代码与续页语言栏另见 [表格列表与代码边框验收](word-table-lists.md)。
