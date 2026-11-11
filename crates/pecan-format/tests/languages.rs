use pecan_format::{Format, tab_width};

#[test]
fn local_backends_are_idempotent_and_keep_literal_search_ranges() {
    for (language, source, needle) in [
        (
            "js",
            "function greet(name){const message='你好';return message+name;}",
            "你好",
        ),
        (
            "ts",
            "function greet(name:string):string{const message='你好';return message+name;}",
            "你好",
        ),
        (
            "jsx",
            "const Card=()=>{return <div title=\"你好\">Hello</div>;};",
            "你好",
        ),
        (
            "tsx",
            "const Card=(props:{name:string})=>{return <div>{props.name}</div>;};",
            "props.name",
        ),
        (
            "css",
            ".card{color:#abc;padding:2px;--label:'你好';}",
            "你好",
        ),
        (
            "scss",
            "$color:#abc;.card{color:$color;.title{padding:2px;}}",
            "$color",
        ),
        (
            "sass",
            ".card\n  color: #abc\n  .title\n    padding: 2px",
            "padding",
        ),
        (
            "less",
            "@color:#abc;.card{color:@color;.title{padding:2px;}}",
            "@color",
        ),
        (
            "html",
            "<div><section><p>你好 <strong>world</strong></p></section></div>",
            "你好",
        ),
        (
            "xml",
            "<items><item id=\"one\"><name>你好</name></item></items>",
            "你好",
        ),
        (
            "vue",
            "<template><div>{{ name }}</div></template>\n<script setup lang=\"ts\">const name:string='你好';</script>",
            "你好",
        ),
        (
            "svelte",
            "<script>const name='你好';</script>\n<div>{name}</div>",
            "你好",
        ),
        (
            "astro",
            "---\nconst name='你好';\n---\n<div>{name}</div>",
            "你好",
        ),
        (
            "sql",
            "select name from devices where owner = '你好' and id in (1,2);",
            "你好",
        ),
        (
            "pgsql",
            "select payload->>'name' from devices where id = $1;",
            "payload",
        ),
        (
            "tsql",
            "select [name] from [devices] where [id] = @id;",
            "devices",
        ),
    ] {
        let backend = Format::for_language(language).expect("supported language");
        for width in [2, 4, 8] {
            let output = backend
                .format(source, width)
                .unwrap_or_else(|error| panic!("{language}/{width}: {error}"));
            let again = backend
                .format(output.text(), width)
                .unwrap_or_else(|error| panic!("second {language}/{width}: {error}"));
            assert_eq!(output.text(), again.text(), "idempotent {language}/{width}");
            let at = source.find(needle).expect("test literal");
            assert_eq!(
                &output.text()[output.range(at..at + needle.len())],
                needle,
                "UTF-8 source mapping {language}/{width}"
            );
            assert!(
                !output.text().ends_with('\n'),
                "no extra display line {language}"
            );
        }
    }
}

#[test]
fn configuration_is_explicit_and_unknown_languages_remain_unsupported() {
    for (language, source) in [
        ("js", "function f(){if(true){return 1;}}"),
        ("css", ".a{color:red;}"),
        ("json", "{\"a\":{\"b\":1}}"),
        ("yaml", "reader:\n  port: 8080"),
        ("toml", "ports=[\n  8080,\n]"),
        (
            "sql",
            "select id from devices where id in (select id from allowed);",
        ),
    ] {
        let backend = Format::for_language(language).unwrap();
        let two = backend
            .format(source, 2)
            .unwrap_or_else(|e| panic!("{language}/2: {e}"));
        let four = backend
            .format(source, 4)
            .unwrap_or_else(|e| panic!("{language}/4: {e}"));
        assert_ne!(
            two.text(),
            four.text(),
            "indentation must affect {language}"
        );
        assert_eq!(tab_width(two.text(), "auto"), 2, "detect {language}");
        assert_eq!(tab_width(four.text(), "auto"), 4, "detect {language}");
    }
    assert!(Format::for_language("mermaid").is_none());
    assert!(Format::for_language("unknown").is_none());
    assert_eq!(
        Format::for_language("TS extra-info"),
        Some(Format::TypeScript)
    );
    assert_eq!(Format::for_language("pg"), Some(Format::PostgreSql));
    assert_eq!(tab_width("\ttext", "8"), 8);
}

#[test]
fn strings_comments_and_invalid_input_are_handled_conservatively() {
    for (language, source, literal) in [
        (
            "js",
            "// keep comment\nfunction f(){return 'a  b\\n中';}",
            "a  b\\n中",
        ),
        ("css", "/* keep comment */\n.a{content:'a  b';}", "a  b"),
        (
            "js",
            "// keep comment\nconst value=`a  b\n  c`;",
            "a  b\n  c",
        ),
        (
            "xml",
            "<p xml:space=\"preserve\">a  b</p><!-- keep comment -->",
            "a  b",
        ),
        ("html", "<pre>a  b\n c</pre><!-- keep comment -->", "a  b"),
        (
            "sql",
            "-- keep comment\nselect 'a  b', 'it''s fine' from devices;",
            "it''s fine",
        ),
    ] {
        let output = Format::for_language(language)
            .unwrap()
            .format(source, 4)
            .unwrap();
        assert!(
            output.text().contains(literal),
            "literal spacing {language}"
        );
        assert!(
            output.text().contains("keep comment"),
            "comments {language}"
        );
    }
    for (language, source) in [
        ("js", "const = ;"),
        ("ts", "function f( {"),
        ("css", ".a{color:"),
        ("xml", "<p><"),
    ] {
        assert!(
            Format::for_language(language)
                .unwrap()
                .format(source, 4)
                .is_err(),
            "invalid {language}"
        );
    }
    for language in ["js", "css", "html", "sql"] {
        let backend = Format::for_language(language).unwrap();
        assert!(backend.format(&"(".repeat(200), 4).is_err());
        assert!(
            backend
                .format(&"x".repeat(backend.input_limit() + 1), 4)
                .is_err()
        );
        assert!(backend.format("x", 0).is_err());
        assert!(backend.format("x", 9).is_err());
    }
}
