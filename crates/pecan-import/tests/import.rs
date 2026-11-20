use std::{
    fs,
    path::{Path, PathBuf},
};

use pecan_import::{SourceKind, is_convertible, is_markdown, load, supports};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn office_reflow_preserves_text_images_math_and_original_bytes() {
    let path = fixture("rich.docx");
    let original = fs::read(&path).unwrap();
    let loaded = load(&path).unwrap();
    assert_eq!(loaded.resources.kind(), SourceKind::Office);
    let structured = load(&fixture("text.docx")).unwrap();
    assert_eq!(
        structured.document.headings()[0].title(),
        "Fixture Document",
        "Word heading styles become the outline"
    );
    assert!(
        loaded.document.source().contains("!["),
        "embedded images become real image references"
    );
    let files: Vec<_> = fs::read_dir(loaded.resources.directory())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(
        !files.is_empty(),
        "embedded image bytes are retained in local assets"
    );
    assert!(
        files
            .iter()
            .any(|file| fs::read(file).unwrap().starts_with(b"\x89PNG\r\n\x1a\n")),
        "the original PNG remains decodable"
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        original,
        "conversion never edits the source package"
    );
    let directory = loaded.resources.directory().to_path_buf();
    drop(loaded);
    assert!(
        !directory.exists(),
        "closing the document releases temporary assets"
    );

    let math = load(&fixture("math.docx")).unwrap();
    assert!(
        math.document.source().contains('$'),
        "Word equations preserve LaTeX delimiters"
    );
    assert!(
        math.document.source().contains("frac") || math.document.source().contains("sum"),
        "nontrivial equation structure survives"
    );
}

#[test]
fn text_pdf_reflows_and_scan_or_mixed_pdf_explicitly_requires_original_view() {
    let text = load(&fixture("text.pdf")).unwrap();
    assert_eq!(text.resources.kind(), SourceKind::Pdf);
    assert!(text.resources.notice().is_none());
    assert!(text.document.source().contains("Fixture Document"));
    assert!(text.document.source().contains("Plain paragraph"));
    for name in ["scan.pdf", "mixed.pdf"] {
        let loaded = load(&fixture(name)).unwrap();
        assert_eq!(loaded.resources.kind(), SourceKind::Pdf);
        assert!(
            loaded.resources.notice().unwrap().contains("原版查看"),
            "{name} must not silently omit scanned pages"
        );
    }
}

#[test]
fn unsupported_malformed_and_amplified_inputs_fail_with_resource_errors() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("invalid.pdf");
    fs::write(&path, b"not a PDF").unwrap();
    assert!(load(&path).is_err());
    let path = directory.path().join("file.xyz");
    fs::write(&path, b"unknown format").unwrap();
    assert!(load(&path).is_err());
    fs::File::create(&path)
        .unwrap()
        .set_len(32 * 1024 * 1024 + 1)
        .unwrap();
    assert!(load(&path).unwrap_err().to_string().contains("32 MiB"));
    let error = load(&fixture("zipbomb.docx")).unwrap_err();
    assert!(
        error.to_string().contains("resource") || error.to_string().contains("limit"),
        "expanded ZIP content must be rejected: {error}"
    );
    for name in [
        "note.MD",
        "report.docx",
        "book.epub",
        "slides.pptx",
        "data.xlsx",
        "scan.PDF",
    ] {
        assert!(
            supports(Path::new(name)),
            "{name} appears in the document workspace"
        );
    }
    assert!(!supports(Path::new("program.exe")));
}

/// Every extension listed under "Supported formats" in the AnyDoc README.
const ANYDOC_EXTENSIONS: [&str; 21] = [
    "doc", "docx", "docm", "ppt", "pps", "pot", "pptx", "pptm", "ppsx", "ppsm", "xls", "xlsx",
    "xlsm", "xlsb", "odt", "ods", "odp", "rtf", "epub", "csv", "pdf",
];

#[test]
fn every_anydoc_format_opens_as_converted_markdown() {
    for extension in ANYDOC_EXTENSIONS {
        for name in [
            format!("file.{extension}"),
            format!("FILE.{}", extension.to_uppercase()),
        ] {
            let path = Path::new(&name);
            assert!(supports(path), "{name} opens in the reader");
            assert!(is_convertible(path), "{name} converts through AnyDoc");
            assert!(!is_markdown(path), "{name} has no Markdown source");
        }
    }
    for name in ["note.md", "note.markdown", "note.mdown", "NOTE.MKD"] {
        assert!(supports(Path::new(name)));
        assert!(!is_convertible(Path::new(name)), "{name} is read directly");
    }
    assert!(!is_convertible(Path::new("program.exe")));

    let directory = tempfile::tempdir().unwrap();
    // Signature-less CSV falls back to its extension.
    let csv = directory.path().join("data.csv");
    fs::write(&csv, "name,count\nalpha,1\n").unwrap();
    let loaded = load(&csv).unwrap();
    assert_eq!(loaded.resources.kind(), SourceKind::Office);
    assert!(
        loaded.document.source().contains("alpha"),
        "{}",
        loaded.document.source()
    );
    let rtf = directory.path().join("note.rtf");
    fs::write(&rtf, r"{\rtf1\ansi{\b Bold} plain RTF text\par}").unwrap();
    let loaded = load(&rtf).unwrap();
    assert_eq!(loaded.resources.kind(), SourceKind::Office);
    assert!(
        loaded.document.source().contains("plain RTF text"),
        "{}",
        loaded.document.source()
    );
}

#[test]
fn embedded_images_are_referenced_by_file_urls_that_resolve_to_their_assets() {
    let loaded = load(&fixture("rich.docx")).unwrap();
    let source = loaded.document.source();
    let urls: Vec<_> = source
        .match_indices("](")
        .map(|(start, _)| {
            let rest = &source[start + 2..];
            let rest = rest.strip_prefix('<').unwrap_or(rest);
            &rest[..rest.find(['>', ')', ' ']).unwrap()]
        })
        .collect();
    assert!(!urls.is_empty(), "{source}");
    for url in urls {
        assert!(url.starts_with("file://"), "{url}");
        assert!(!url.contains('\\'), "no native separators: {url}");
        let path = url::Url::parse(url).unwrap().to_file_path().unwrap();
        assert!(path.starts_with(loaded.resources.directory()), "{url}");
        assert!(path.is_file(), "{url}");
    }
}
