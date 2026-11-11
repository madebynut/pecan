# 代码语言图标验收

图标位于语言文字左侧，尺寸为 16px。应用内语言栏在左侧，HTML、PDF 和 Word 的语言标识组在右侧。请切换亮暗主题，并导出这份文档检查图标、间距及源码完整性。

## 常用语言

```rust
fn main() {
    println!("碧果 Pecan");
}
```

```javascript
const greet = name => `Hello, ${name}`;
```

```typescript
interface Device {
    id: string;
    enabled: boolean;
}
```

```python
def greet(name):
    return f"Hello, {name}"
```

```bash
printf '%s\n' 'Hello, Pecan'
```

```yaml
device:
  enabled: true
  ports: [80, 443]
```

```html
<main class="reader">Hello, Pecan</main>
```

```css
.reader {
    max-width: 960px;
    margin: 0 auto;
}
```

## 别名与嵌套代码

JSONC 使用 JSON 图标，TSX 使用 TypeScript 图标，大小写不影响图标。提示块内代码复用正文代码栏。

```JSONC
{
    // 注释与源码应保留
    "device": "碧果",
    "enabled": true
}
```

```tsx
const App = () => <main>Hello, Pecan</main>;
```

:::tip
图标与语言文字应垂直居中，操作按钮保持原有位置。

```rs
let device = "Pecan";
```
:::

## SQL 与数据库图标

通用 `sql` 使用 Azure SQL Database 图标。指定数据库时使用对应 logo；`pg`、`pgsql`、`postgres` 和 `postgresql` 使用同一个 PostgreSQL 图标。未知语言和未指定语言仍使用 Lucide `</>`。Word 语言栏最小高度为 24pt，原生行高应完整容纳图标，PDF 保持 26pt。

```sql
SELECT id, name FROM devices WHERE enabled = TRUE;
```

```SQL
SELECT COUNT(*) FROM devices;
```

```mysql
SELECT VERSION();
```

```postgresql
SELECT current_database();
```

```pg
SELECT CURRENT_DATE;
```

```pgsql
SELECT now();
```

```sqlite
PRAGMA database_list;
```

```mssql
SELECT @@VERSION;
```

```oracle
SELECT * FROM dual;
```

```mariadb
SHOW DATABASES;
```

```mongodb
db.devices.find({ enabled: true });
```

```redis
GET device:status
```

## 未指定与未知语言

以下两块使用通用 `</>` 图标；复制仍应保留完整源码。

```
plain text
下一行
```

```unknown-language
unknown language source
```

跨页效果可使用 [代码块导出验收](code-export-appearance.md) 中的 90 行 Rust 代码检查：续页应重复图标与文字，正文及底边框完整；PDF 语言标识栏默认 26pt。
