# 代码高亮与格式化导出验收

本页用于检查代码语法高亮、别名识别与格式化导出。先将「阅读 → 代码 → Tab 宽度」设为 4，再打开本页；切换为 2 或 8 可比较排版。

> [!NOTE] 验收要点
> SQL 和 TSX 的关键词、字符串、注释、数字以及 JSX 标签应有清晰颜色。PDF、Word、HTML 按全局 Tab 配置导出格式化代码；Markdown 导出保留源文件。

高亮与格式化的支持范围独立。Rust、Python 等暂未接入格式化器，仍有语法高亮。Vue/XML、SCSS/Sass/Less 使用宿主语法，专用扩展语法不保证全部着色。单块工具栏 Tab 选择只影响当前阅读块，导出采用全局配置。

## SQL 与数据库别名

```sql
-- 条件、字符串、数字
select id,name from devices where enabled=true and name='Pecan' order by id desc;
```

## PostgreSQL 别名

```pgsql
select id,name from devices where version>=42 and name='Pecan';
```

## TSX 组件

```tsx
// 类型、标签、属性、字符串
interface Props {name: string}
export const View=({name}:Props)=> <section title="Pecan">{name}</section>;
```

## JSX 组件

```jsx
// JSX 和 JavaScript 同时高亮
const View=({name})=> <div title="Pecan">{name}</div>;
```

## TypeScript

```TS
interface Settings {title: string; width: number}
const settings: Settings={title:'Pecan',width:960};
```

## JavaScript

```mjs
// 保留字符串中的空格
export function greet(name){return `Hello ${name}`;}
```

## JSON

```json
{"reader":{"title":"Pecan","width":960,"enabled":true}}
```

## YAML

```yaml
# 阅读设置
reader:
  title: Pecan
  width: 960
  enabled: true
```

## TOML

```toml
# 阅读设置
[reader]
title='Pecan'
width=960
enabled=true
```

## Rust

```rust
// 不支持的格式化语言保留原排版，仍有高亮
fn main() {
    let title = "Pecan";
    println!("{title}");
}
```

## Python

```py
# 注释、函数、字符串
def title():
    return "Pecan"
```

## C 与 C++

```cpp
// C 基础语法与 C++ 扩展
#include <string>
int main() {
    std::string title = "Pecan";
    return 0;
}
```

## C#

```C#
// 类、类型、字符串
class Reader {
    public string Title = "Pecan";
}
```

## Java

```java
class Reader {
    String title = "Pecan";
    int width = 960;
}
```

## Go

```go
package main
func main() {
    println("Pecan")
}
```

## Swift

```swift
// 函数、类型、字符串
func title() -> String {
    return "Pecan"
}
```

## Kotlin

```kt
fun main() {
    val title = "Pecan"
    println(title)
}
```

## GraphQL

```gql
# 查询与参数
query Reader {
  device(name: "Pecan") { id name }
}
```

## Protocol Buffers

```proto
syntax = "proto3";
message Reader {
  string title = 1;
  uint32 width = 2;
}
```

## HTML 与嵌入脚本

```html
<h1>Pecan</h1>
<script>const title = "Pecan";</script>
<style>h1 { color: #242424; }</style>
```

## Vue 宿主与 TypeScript

```vue
<script lang="ts">
interface Props { name: string }
const name: string = "Pecan";
</script>
<template><h1>{{ name }}</h1></template>
```

## Astro

```astro
---
const name: string = "Pecan";
---
<h1>{name}</h1>
```

## Svelte

```svelte
<script lang="ts">
const name: string = "Pecan";
</script>
<h1>{name}</h1>
```

## CSS

```css
/* 字体与颜色 */
.reader {color:#242424;font-size:16px;}
```

## Shell

```sh
# 关键字与字符串
if true; then
    echo "Pecan"
fi
```

## CMake

```cmake
# 构建选项
if(TRUE)
    message("Pecan")
endif()
```

## Ruby

```ruby
# 保留原排版
def title
  return "Pecan"
end
```

## Lua

```lua
-- 注释与变量
local title = "Pecan"
return title
```

## Scala

```scala
object Reader {
  val title = "Pecan"
}
```

## Elixir

```elixir
# 函数与块
def title do
  "Pecan"
end
```

## Zig

```zig
// 编译期常量
const title: []const u8 = "Pecan";
```

## Makefile

```make
# 构建目标
TITLE := Pecan
all:
	echo $(TITLE)
```

## Diff

```diff
diff --git a/reader.rs b/reader.rs
--- a/reader.rs
+++ b/reader.rs
@@ -1 +1 @@
-const WIDTH: usize = 800;
+const WIDTH: usize = 960;
```

## EJS

```ejs
<p><% if (ok) { %><%= "Pecan" %><% } %></p>
```

## ERB

```erb
<p><% if ok %><%= "Pecan" %><% end %></p>
```

## 未知语言保持原文

```unknown-language
keep   original
  do_not_reformat = true
```

## 嵌套代码保持一致

:::info
正文与提示块内的代码应使用同一格式化和高亮规则。

```json
{"container":{"name":"Pecan","width":960}}
```
:::

> ```yaml
> reader:
>   title: Pecan
>   width: 960
> ```

## 安全保留原文

```json
{"invalid": }
```

无效语法保持原文；超过自动格式化／高亮预算的代码完整保留。导出内容不能丢失，源 Markdown 不改写。
