# 配置代码格式化验收

本页检查 JSON、YAML／YML 与 TOML 的阅读格式化。文档加载时按配置自动格式化；点击 Tab 图标选择 2、4、8 列或自动识别后，立即重新格式化本块。格式化只改变当前阅读显示，不修改 Markdown 文件或导出的原始配置。原始状态复制源码，格式化状态复制当前排版。

## YAML 配置

依次选择 4 列、2 列、8 列，观察层级与缩进线一致；注释、锚点、字符串和数值应保留。搜索「中文设备」时，应定位到格式化后的对应文字。再测试整块折叠、复制。

```yaml
# 服务配置
defaults: &defaults
  port: 8080 # 监听端口
  retry: 3
  description: '中文设备'
servers:
  - name: "alpha"
    config: *defaults
  - name: "beta"
    config:
      port: 8081
      enabled: true
message: |+
  第一行
    第二行

```

## YML 别名

Tab 图标应同 YAML 一样可用，语法高亮、复制一致。

```YML
reader:
  width: 960
  fonts:
    body: "系统默认"
    code: "系统等宽"
```

## TOML 配置

亮暗主题下，检查表头、键、字符串、数字、布尔值和注释有清晰的语法颜色；导出 HTML、PDF、Word 后也应保留高亮。选择缩进后，多行数组按选定宽度排版。TOML 表头与普通键值默认仍靠左，保持常见配置风格；注释、浮点数写法、多行字符串中的空格不能被改写。

```toml
# 阅读配置
[reader]
name="中文设备" # 标题
enabled=true
scale=1.2300e+04
updated=2026-11-04T09:00:00+08:00
ports=[
  8080, # 主端口
  8081,
]
message='''第一行
    第二行'''

[[reader.servers]]
name="alpha"
address="https://example.com"
```

### 大小写与嵌套

下面使用大写语言标识，提示块中的 TOML 也应高亮。缩进格式化、复制、整块折叠均应正常。

::: info
```TOML
# 嵌套配置
[reader.fonts]
body = "系统默认"
code = "系统等宽"
sizes = [
  16,
  14,
]
```
:::

## JSONC 保持原有功能

选择缩进后直接格式化，保留注释、字段顺序与数字写法。

```jsonc
{
    // 配置项
    "reader": {"width":960,"scale":1.2300e+04},
    "ports": [8080,8081]
}
```

## 无效配置

点击缩进后应出现明确错误，当前显示与源文保留，不出现部分格式化结果。

```yaml
reader: [
```

```toml
ports = [
```

## 右键与导出

在代码文字上右键，只有正文菜单，不再额外弹出编辑器的「跳转到定义／剪切／粘贴」菜单。鼠标选择、键盘复制、工具栏复制和折叠仍正常。

分别导出 Word 与 PDF：PDF 代码语言栏为 26pt；Word 采用24pt 的原生语言栏最小高度，图标完整。导出不包含操作按钮，语言组在右侧，代码正文与语言栏底色有层次。
