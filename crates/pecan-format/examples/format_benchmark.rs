//! Reproducible release-mode measurements; formats only fixed synthetic snippets.
use pecan_format::Format;
use std::{hint::black_box, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cases = [
        (
            "json",
            "{\"reader\":{\"width\":960,\"ports\":[8080,8081]},\"enabled\":true}",
        ),
        (
            "yaml",
            "reader:\n  width: 960\n  ports: [8080, 8081]\nenabled: true",
        ),
        (
            "toml",
            "[reader]\nwidth=960\nports=[\n  8080,\n  8081,\n]\nenabled=true",
        ),
        (
            "ts",
            "function load(id:string){const options={id,enabled:true};return options;}",
        ),
        (
            "css",
            ".reader{color:#333;background:#fff;padding:16px;}.reader code{font-size:14px;}",
        ),
        (
            "html",
            "<div><p>Hello <strong>world</strong></p></div>\n<script>const options={enabled:true};</script>",
        ),
        (
            "sql",
            "select id,name from devices where active=true and id in (select device_id from sessions);",
        ),
    ];
    println!("language,input_bytes,first_us,warm_mean_us,retained_bytes");
    let mut batch = 0.;
    for (language, source) in cases {
        let backend = Format::for_language(language).ok_or("benchmark language unavailable")?;
        let first = Instant::now();
        let output = backend.format(black_box(source), 4)?;
        let first_us = first.elapsed().as_secs_f64() * 1e6;
        let iterations = 200;
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(backend.format(black_box(source), 4)?);
        }
        let mean_us = start.elapsed().as_secs_f64() * 1e6 / f64::from(iterations);
        batch += mean_us;
        println!(
            "{language},{},{first_us:.2},{mean_us:.2},{}",
            source.len(),
            output.estimated_size()
        );
    }
    println!(
        "100_mixed_fences_estimate_ms,{:.2}",
        batch * 100. / 7. / 1000.
    );
    let large = [
        ("json", format!("[{}]", (0..256).map(|i| format!(r#"{{"id":{i},"name":"device_{i}","ports":[8080,8081]}}"#)).collect::<Vec<_>>().join(","))),
        ("yaml", (0..64).map(|i| format!("device_{i}:\n  name: device_{i}\n  settings:\n    port: 8080\n")).collect::<String>()),
        ("ts", (0..32).map(|i| format!("function device_{i}(id:string){{const options={{id,active:true,port:8080}};return options;}}\n")).collect::<String>()),
        ("css", (0..64).map(|i| format!(".device_{i}{{color:#333;background:#fff;padding:16px;}}\n")).collect::<String>()),
        ("sql", (0..32).map(|i| format!("select id,name from devices where active=true and group_id={i};\n")).collect::<String>()),
    ];
    for (language, source) in large {
        let backend = Format::for_language(language).ok_or("benchmark language unavailable")?;
        let first = Instant::now();
        let output = backend.format(black_box(&source), 4)?;
        let first_us = first.elapsed().as_secs_f64() * 1e6;
        let start = Instant::now();
        for _ in 0..20 {
            black_box(backend.format(black_box(&source), 4)?);
        }
        let mean_us = start.elapsed().as_secs_f64() * 1e6 / 20.;
        println!(
            "large_{language},{},{first_us:.2},{mean_us:.2},{}",
            source.len(),
            output.estimated_size()
        );
    }
    Ok(())
}
