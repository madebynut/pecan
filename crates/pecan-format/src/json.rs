//! Lossless JSON and JSONC whitespace formatting with bounded lexical validation.
use crate::{FormattedCode, mapping::Token};
use anyhow::{Result, bail, ensure};
use serde::Deserialize as _;
use std::borrow::Cow;
pub(crate) const MAX_INPUT: usize = 2 * 1024 * 1024;
const MAX_OUTPUT: usize = 16 * 1024 * 1024;
const MAX_TOKENS: usize = 200_000;
/// Reformat whitespace only: strings, numeric lexemes, key order and duplicate keys survive.
pub(crate) fn json(source: &str, indent: usize) -> Result<FormattedCode> {
    ensure!(source.len() <= MAX_INPUT, "代码超过 2 MiB，保留原始显示");
    ensure!((1..=8).contains(&indent), "缩进宽度必须在 1–8 之间");
    let validated = validation_json(source)?;
    let mut parser = serde_json::Deserializer::from_str(&validated);
    serde::de::IgnoredAny::deserialize(&mut parser)?;
    parser.end()?;
    let bytes = source.as_bytes();
    let mut result = FormattedCode {
        text: String::with_capacity(source.len()),
        tokens: Vec::new(),
    };
    let mut offset = 0;
    let mut depth = 0usize;
    let mut previous = 0;
    let mut generated_indent = None;
    while offset < bytes.len() {
        let b = bytes[offset];
        if b.is_ascii_whitespace() {
            offset += 1;
            continue;
        }
        ensure!(
            result.tokens.len() < MAX_TOKENS,
            "代码过于复杂，保留原始显示"
        );
        let start = offset;
        offset = token_end(bytes, offset)?;
        let comment = bytes[start..].starts_with(b"//") || bytes[start..].starts_with(b"/*");
        if comment
            && result
                .text
                .as_bytes()
                .last()
                .is_some_and(|b| !b.is_ascii_whitespace())
        {
            result.text.push(' ');
        }
        if b == b'}' || b == b']' {
            depth = depth.saturating_sub(1);
            if previous != b'{' && previous != b'[' || generated_indent.is_some() {
                newline(&mut result.text, depth, indent, &mut generated_indent)?;
            }
        }
        ensure!(
            result.text.len() + offset - start <= MAX_OUTPUT,
            "格式化结果超过 16 MiB，保留原始显示"
        );
        result.tokens.push(Token {
            source: start..offset,
            display: result.text.len(),
        });
        result.text.push_str(&source[start..offset]);
        generated_indent = None;
        if comment {
            newline(&mut result.text, depth, indent, &mut generated_indent)?;
            continue;
        }
        match b {
            b'{' | b'[' => {
                depth += 1;
                ensure!(depth <= 128, "JSON 嵌套超过 128 层，保留原始显示");
                let next = bytes[offset..]
                    .iter()
                    .copied()
                    .find(|b| !b.is_ascii_whitespace());
                if next != Some(if b == b'{' { b'}' } else { b']' }) {
                    newline(&mut result.text, depth, indent, &mut generated_indent)?;
                }
            }
            b',' => {
                if source[offset..]
                    .trim_start_matches([' ', '\t'])
                    .starts_with("//")
                {
                    result.text.push(' ');
                } else {
                    newline(&mut result.text, depth, indent, &mut generated_indent)?;
                }
            }
            b':' => result.text.push(' '),
            _ => {}
        }
        ensure!(
            result.text.len() <= MAX_OUTPUT,
            "格式化结果超过 16 MiB，保留原始显示"
        );
        previous = b;
    }
    if let Some(spaces) = generated_indent {
        result.text.truncate(result.text.len() - spaces);
        if result.text.ends_with('\n') {
            result.text.pop();
        }
    }
    Ok(result)
}
fn newline(
    output: &mut String,
    depth: usize,
    indent: usize,
    generated: &mut Option<usize>,
) -> Result<()> {
    if let Some(spaces) = generated.take() {
        output.truncate(output.len() - spaces);
    }
    let spaces = depth * indent;
    if output.len() + 1 + spaces > MAX_OUTPUT {
        bail!("格式化结果超过 16 MiB，保留原始显示");
    }
    if !output.ends_with('\n') {
        output.push('\n');
    }
    *generated = Some(spaces);
    output.extend(std::iter::repeat_n(' ', spaces));
    Ok(())
}

/// Mask comments and trailing commas for serde validation, without changing source offsets.
fn validation_json(source: &str) -> Result<Cow<'_, str>> {
    let bytes = source.as_bytes();
    let mut masked: Option<Vec<u8>> = None;
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes[offset].is_ascii_whitespace() {
            offset += 1;
            continue;
        }
        let end = token_end(bytes, offset)?;
        if bytes[offset..].starts_with(b"//") || bytes[offset..].starts_with(b"/*") {
            let target = masked.get_or_insert_with(|| bytes.to_vec());
            for b in &mut target[offset..end] {
                if !matches!(*b, b'\r' | b'\n') {
                    *b = b' ';
                }
            }
        }
        offset = end;
    }
    let json = masked.as_deref().unwrap_or(bytes);
    let mut trailing = Vec::new();
    offset = 0;
    while offset < json.len() {
        if json[offset].is_ascii_whitespace() {
            offset += 1;
            continue;
        }
        let end = token_end(json, offset)?;
        if json[offset] == b','
            && json[end..]
                .iter()
                .copied()
                .find(|b| !b.is_ascii_whitespace())
                .is_some_and(|b| matches!(b, b'}' | b']'))
        {
            trailing.push(offset);
        }
        offset = end;
    }
    if !trailing.is_empty() {
        let target = masked.get_or_insert_with(|| bytes.to_vec());
        for offset in trailing {
            target[offset] = b' ';
        }
    }
    Ok(match masked {
        Some(bytes) => Cow::Owned(String::from_utf8(bytes)?),
        None => Cow::Borrowed(source),
    })
}
fn token_end(bytes: &[u8], start: usize) -> Result<usize> {
    let mut offset = start + 1;
    if bytes[start] == b'"' {
        while offset < bytes.len() {
            match bytes[offset] {
                b'\\' => offset += 2,
                b'"' => return Ok(offset + 1),
                _ => offset += 1,
            }
        }
        bail!("未闭合的 JSON 字符串");
    }
    if bytes[start..].starts_with(b"//") {
        return Ok(start
            + bytes[start..]
                .iter()
                .position(|&b| b == b'\n' || b == b'\r')
                .unwrap_or(bytes.len() - start));
    }
    if bytes[start..].starts_with(b"/*") {
        let end = bytes[start + 2..].windows(2).position(|pair| pair == b"*/");
        return end
            .map(|end| start + 4 + end)
            .ok_or_else(|| anyhow::anyhow!("未闭合的 JSON 注释"));
    }
    if b"{}[],:".contains(&bytes[start]) {
        return Ok(offset);
    }
    while offset < bytes.len()
        && !bytes[offset].is_ascii_whitespace()
        && !b"{}[],:".contains(&bytes[offset])
        && !bytes[offset..].starts_with(b"//")
        && !bytes[offset..].starts_with(b"/*")
    {
        offset += 1;
    }
    Ok(offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tab_width;
    #[test]
    fn detects_each_blocks_indentation_and_respects_override() {
        for n in [2, 4, 8] {
            let source = format!(
                "{{\n{}\"a\": {{\n{}\"b\": 1\n{}}}\n}}",
                " ".repeat(n),
                " ".repeat(n * 2),
                " ".repeat(n)
            );
            assert_eq!(tab_width(&source, "auto"), n);
            assert_eq!(tab_width(&source, "2"), 2);
        }
        assert_eq!(tab_width("{\n\t\"a\": 1\n}", "auto"), 4);
        assert_eq!(tab_width("x\n    y\n\n        z\n    q", "auto"), 4);
        assert_eq!(tab_width("\n".repeat(512).as_str(), "auto"), 4);
    }
    #[test]
    fn formatting_is_lossless_and_idempotent() {
        let source = r#" { "中文": "a\"b\\c", "z": 123456789012345678901234567890, "a": 1.2300e+04, "a": [true,{},[],null] } "#;
        let formatted = json(source, 4).unwrap();
        assert_eq!(json(&formatted.text, 4).unwrap().text, formatted.text);
        for token in &formatted.tokens {
            assert_eq!(
                &formatted.text[token.display..token.display + token.source.len()],
                &source[token.source.clone()]
            );
        }
        for needle in [
            "中文",
            "a\\\"b",
            "123456789012345678901234567890",
            "1.2300e+04",
        ] {
            let at = source.find(needle).unwrap();
            let mapped = formatted.range(at..at + needle.len());
            assert_eq!(&formatted.text[mapped], needle);
        }
        let all = formatted.range(0..source.len());
        assert_eq!(&formatted.text[all], formatted.text);
        assert!(formatted.text.contains("\n    \"中文\""));
    }
    #[test]
    fn comments_trailing_commas_and_comment_like_strings_survive() {
        let source = "// 配置\n{\n \"url\": \"https://host/*path*/\", // 地址\n /* block\n comment */ \"a\": [1,2,],\n} // 结束";
        let result = json(source, 4).unwrap();
        assert_eq!(json(&result.text, 4).unwrap().text, result.text);
        for token in &result.tokens {
            assert_eq!(
                &result.text[token.display..token.display + token.source.len()],
                &source[token.source.clone()]
            );
        }
        assert!(result.text.contains("// 地址\n"));
        assert!(result.text.contains("https://host/*path*/"));
        assert!(!result.text.ends_with('\n'));
        let at = source.find("地址").unwrap();
        assert_eq!(&result.text[result.range(at..at + "地址".len())], "地址");
        assert!(json("{\n//x\n}", 4).unwrap().text.ends_with("\n}"));
    }

    #[test]
    fn rejects_invalid_or_over_budget_input_without_partial_results() {
        for source in [
            "{",
            "[1,,]",
            "{} {}",
            "{\"a\":NaN}",
            "/* hi",
            "{\"a\":\"unterminated}",
        ] {
            assert!(json(source, 4).is_err(), "{source}");
        }
        assert!(json(&" ".repeat(MAX_INPUT + 1), 4).is_err());
        assert!(json("{}", 0).is_err());
        let tokens = format!("[{}0]", "0,".repeat(MAX_TOKENS / 2 + 1));
        assert!(json(&tokens, 2).is_err());
        assert!(json(&format!("[{}0{}]", "[".repeat(200), "]".repeat(200)), 4).is_err());
    }
}
