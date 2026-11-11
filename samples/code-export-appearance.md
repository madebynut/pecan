# 代码块导出验收

此文档验证代码块在应用、HTML、PDF 和 Word 中的外观，以及提示块内代码与长代码分页。语言标识位于导出代码块右侧，HTML 语言栏最小高度 40px、PDF 最小高度 26pt，文字垂直居中；语言栏使用浅灰底色，代码正文使用更淡的灰色；源代码保持可选中和可复制。

## 普通代码

```json
{ "device": "设备", "enabled": true, "ports": [80, 443] }
```

```rust
fn main() {
    let message = "碧果 Pecan";
    println!("{message}");
}
```

```
第一行 plain text
    第二行保留缩进
最后一行 end
```

## 提示块中的代码

:::info
说明中的 **Markdown** 与 `code` 保持原有样式，下面的代码具有独立语言栏与背景。

```jsonc
{
  // 注释和字段顺序保持不变
  "device": "设备",
  "options": { "debug": true }
}
```
:::

> [!TIP]
> 导出后代码仍可编辑。
>
> ```python
> def greet(name):
>     return f"Hello {name}"
> ```

## 长代码分页

下面有 90 行变量和首尾函数行。PDF 和 Word 应完整保留内容，并在续页显示语言栏；语言栏不能单独停留在上一页，正文底色与边框应持续可见。

```rust
fn pagination_check() {
    let step_01 = 1; // 保留第 1 行
    let step_02 = 2; // 保留第 2 行
    let step_03 = 3; // 保留第 3 行
    let step_04 = 4; // 保留第 4 行
    let step_05 = 5; // 保留第 5 行
    let step_06 = 6; // 保留第 6 行
    let step_07 = 7; // 保留第 7 行
    let step_08 = 8; // 保留第 8 行
    let step_09 = 9; // 保留第 9 行
    let step_10 = 10; // 保留第 10 行
    let step_11 = 11; // 保留第 11 行
    let step_12 = 12; // 保留第 12 行
    let step_13 = 13; // 保留第 13 行
    let step_14 = 14; // 保留第 14 行
    let step_15 = 15; // 保留第 15 行
    let step_16 = 16; // 保留第 16 行
    let step_17 = 17; // 保留第 17 行
    let step_18 = 18; // 保留第 18 行
    let step_19 = 19; // 保留第 19 行
    let step_20 = 20; // 保留第 20 行
    let step_21 = 21; // 保留第 21 行
    let step_22 = 22; // 保留第 22 行
    let step_23 = 23; // 保留第 23 行
    let step_24 = 24; // 保留第 24 行
    let step_25 = 25; // 保留第 25 行
    let step_26 = 26; // 保留第 26 行
    let step_27 = 27; // 保留第 27 行
    let step_28 = 28; // 保留第 28 行
    let step_29 = 29; // 保留第 29 行
    let step_30 = 30; // 保留第 30 行
    let step_31 = 31; // 保留第 31 行
    let step_32 = 32; // 保留第 32 行
    let step_33 = 33; // 保留第 33 行
    let step_34 = 34; // 保留第 34 行
    let step_35 = 35; // 保留第 35 行
    let step_36 = 36; // 保留第 36 行
    let step_37 = 37; // 保留第 37 行
    let step_38 = 38; // 保留第 38 行
    let step_39 = 39; // 保留第 39 行
    let step_40 = 40; // 保留第 40 行
    let step_41 = 41; // 保留第 41 行
    let step_42 = 42; // 保留第 42 行
    let step_43 = 43; // 保留第 43 行
    let step_44 = 44; // 保留第 44 行
    let step_45 = 45; // 保留第 45 行
    let step_46 = 46; // 保留第 46 行
    let step_47 = 47; // 保留第 47 行
    let step_48 = 48; // 保留第 48 行
    let step_49 = 49; // 保留第 49 行
    let step_50 = 50; // 保留第 50 行
    let step_51 = 51; // 保留第 51 行
    let step_52 = 52; // 保留第 52 行
    let step_53 = 53; // 保留第 53 行
    let step_54 = 54; // 保留第 54 行
    let step_55 = 55; // 保留第 55 行
    let step_56 = 56; // 保留第 56 行
    let step_57 = 57; // 保留第 57 行
    let step_58 = 58; // 保留第 58 行
    let step_59 = 59; // 保留第 59 行
    let step_60 = 60; // 保留第 60 行
    let step_61 = 61; // 保留第 61 行
    let step_62 = 62; // 保留第 62 行
    let step_63 = 63; // 保留第 63 行
    let step_64 = 64; // 保留第 64 行
    let step_65 = 65; // 保留第 65 行
    let step_66 = 66; // 保留第 66 行
    let step_67 = 67; // 保留第 67 行
    let step_68 = 68; // 保留第 68 行
    let step_69 = 69; // 保留第 69 行
    let step_70 = 70; // 保留第 70 行
    let step_71 = 71; // 保留第 71 行
    let step_72 = 72; // 保留第 72 行
    let step_73 = 73; // 保留第 73 行
    let step_74 = 74; // 保留第 74 行
    let step_75 = 75; // 保留第 75 行
    let step_76 = 76; // 保留第 76 行
    let step_77 = 77; // 保留第 77 行
    let step_78 = 78; // 保留第 78 行
    let step_79 = 79; // 保留第 79 行
    let step_80 = 80; // 保留第 80 行
    let step_81 = 81; // 保留第 81 行
    let step_82 = 82; // 保留第 82 行
    let step_83 = 83; // 保留第 83 行
    let step_84 = 84; // 保留第 84 行
    let step_85 = 85; // 保留第 85 行
    let step_86 = 86; // 保留第 86 行
    let step_87 = 87; // 保留第 87 行
    let step_88 = 88; // 保留第 88 行
    let step_89 = 89; // 保留第 89 行
    let step_90 = 90; // 保留第 90 行
}
```

结束标记：确认能找到 `step_90` 和此段正文。HTML 保留水平滚动；PDF 和 Word 按纸页宽度排版。JSON 系列可选择缩进宽度并自动格式化，Rust、Python 等尚无格式化器，工具栏保留 Tab、折叠和复制。阅读格式化不修改 Markdown 源文或导出源代码。
