# 行内代码与提示块导出验收

关于`**version**`，当前`接口文档`明确了接口外层`version`用于配置变更同步，但没有解释这里的内层`config.version`。两者不要直接当成同一含义。

左`code`右；左 `code` 右；左`API接口`右；左`设备文档`右。

`**粗体 Bold**`、`*斜体 Italic*`、`~~删除 Deprecated~~`、`***粗斜体 Both***`、`snake_case`、`# comment`。

<code>接口<strong>文档</strong><del>旧版</del></code> 与 <code>甲</code><code>乙</code>，都应只有各自完整的边框。

::: 
`needAddress=false` 时不调用 Geocoding，不返回 `address`；逆地理失败时坐标仍按成功返回（地址失败不推翻坐标成功）。

第二段包含**加粗**、*斜体*和~~删除~~，以及`API接口`。

- 列表包含 `request_id`。
- 第二条列表。

```json
{"needAddress":false,"address":"","version":2}
```
:::

> [!NOTE] 说明 `version`
> 正文与 `needAddress=false` 胶囊应完整。

> [!TIP] 建议
> 使用 `address`，同时保留 `request_id`。

> [!IMPORTANT] 重点
> 关注`**version**`与`接口文档`。

> [!WARNING] 警告
> 使用 `needAddress=false`。

> [!CAUTION] 注意
> `address` 为空时保留坐标。

> [!ERROR] 错误
> 不要覆盖 `request_id`。

| 场景 | 行内代码 |
| --- | --- |
| 没有外侧空格 | 左`API接口`右 |
| Markdown 样式 | `**version**`与`~~deprecated~~` |

```mermaid
flowchart LR
  A[获取配置] --> B[定位]
  B --> C[上报结果]
```
