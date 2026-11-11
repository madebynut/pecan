use pecan_mermaid::{Appearance, Renderer};

#[test]
fn native_diagrams_produce_svg_and_png_and_reuse_cached_geometry() {
    let renderer = Renderer::new();
    for source in [
        "flowchart LR\n A[开始] --> B{选择}\n B -->|是| C[完成]",
        "sequenceDiagram\n participant A as 阅读器\n participant B as 缓存\n A->>B: 请求\n B-->>A: 返回",
        "classDiagram\n class Reader\n class Document\n Reader --> Document",
        "stateDiagram-v2\n [*] --> Reading\n Reading --> Closed\n Closed --> [*]",
        "erDiagram\n READER ||--o{ DOCUMENT : opens",
        "pie title 占比\n \"阅读\" : 80\n \"图表\" : 20",
        "gantt\n title 计划\n dateFormat YYYY-MM-DD\n section 开发\n 阅读器 :a, 2026-10-30, 2d",
        "mindmap\n root((Pecan))\n  阅读\n  图表",
    ] {
        let diagram = renderer
            .render(source, Appearance::Light)
            .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert!(diagram.svg().contains("<svg"));
        let png = diagram.png(640.).unwrap();
        assert!(
            png.starts_with(b"\x89PNG\r\n\x1a\n"),
            "preview must be an actual PNG"
        );
        let cached = renderer.render(source, Appearance::Light).unwrap();
        assert!(
            std::ptr::eq(diagram.svg().as_ptr(), cached.svg().as_ptr()),
            "cache hit must reuse parsed vector geometry"
        );
    }
    assert_eq!(renderer.stats().hits, 8);
}

#[test]
fn cache_identity_includes_theme_and_invalid_source_is_recoverable() {
    let renderer = Renderer::new();
    let source = "flowchart LR\n A --> B";
    let light = renderer.render(source, Appearance::Light).unwrap();
    let dark = renderer.render(source, Appearance::Dark).unwrap();
    assert_ne!(light.svg(), dark.svg());
    assert!(
        renderer
            .render("definitely not Mermaid", Appearance::Light)
            .is_err()
    );
    assert!(
        renderer
            .render(&"x".repeat(128 * 1024 + 1), Appearance::Light)
            .is_err()
    );
    assert!(light.png(f32::NAN).is_err());
}

#[test]
fn cache_remains_bounded_after_many_distinct_diagrams() {
    let renderer = Renderer::new();
    for index in 0..80 {
        renderer
            .render(
                &format!("flowchart LR\n A[图 {index}] --> B"),
                Appearance::Light,
            )
            .unwrap();
    }
    let stats = renderer.stats();
    assert!(stats.entries <= 64);
    assert!(stats.bytes <= 32 * 1024 * 1024);
}

#[test]
fn oversized_graphs_are_rejected_before_expensive_layout() {
    use std::fmt::Write as _;
    let renderer = Renderer::new();
    let mut source = "flowchart LR\n".to_string();
    for index in 0..600 {
        writeln!(source, "N{index}[Node {index}]").unwrap();
    }
    let error = renderer.render(&source, Appearance::Light).unwrap_err();
    assert!(error.to_string().contains("512 nodes"));
    assert_eq!(renderer.stats().entries, 0);
}

#[test]
fn light_sequence_keeps_distinct_colors_and_visible_strokes_when_reduced() {
    use mermaid_rs_renderer::{LayoutConfig, Theme, layout::DiagramData};
    use resvg::tiny_skia;

    let source = "sequenceDiagram\n participant A\n participant B\n A->>B: Request\n B-->>A: Response\n Note over A,B: Keep this information\n";
    let diagram = Renderer::new().render(source, Appearance::Light).unwrap();
    let graph = mermaid_rs_renderer::parse_mermaid_strict(source)
        .unwrap()
        .graph;
    // Palette changes must not change the measured geometry or the selected font size.
    let mut theme = Theme::modern();
    let xml = roxmltree::Document::parse(diagram.svg()).unwrap();
    theme.font_family = xml
        .descendants()
        .find(|node| node.has_tag_name("text"))
        .unwrap()
        .attribute("font-family")
        .unwrap()
        .into();
    let layout = mermaid_rs_renderer::compute_layout(&graph, &theme, &LayoutConfig::default());
    let DiagramData::Sequence(sequence) = &layout.diagram else {
        panic!("expected a sequence diagram")
    };
    let scale = 0.5;
    let image =
        tiny_skia::Pixmap::decode_png(&diagram.png(diagram.dimensions().0 * scale).unwrap())
            .unwrap();
    let view_box: Vec<f32> = diagram
        .svg()
        .split_once("viewBox=\"")
        .unwrap()
        .1
        .split('"')
        .next()
        .unwrap()
        .split_whitespace()
        .map(|value| value.parse().unwrap())
        .collect();
    let on_white = |x: f32, y: f32| {
        let pixel = image
            .pixel(
                ((x - view_box[0]) * scale).floor() as u32,
                ((y - view_box[1]) * scale).floor() as u32,
            )
            .unwrap();
        let white = 255 - u16::from(pixel.alpha());
        [
            u16::from(pixel.red()) + white,
            u16::from(pixel.green()) + white,
            u16::from(pixel.blue()) + white,
        ]
    };
    let darkness = |rgb: [u16; 3]| {
        255. - (f32::from(rgb[0]) * 0.2126
            + f32::from(rgb[1]) * 0.7152
            + f32::from(rgb[2]) * 0.0722)
    };
    let edge = &layout.edges[0];
    let x = (edge.points[0].0 + edge.points[1].0) / 2.;
    let y = edge.points[0].1;
    let message_contrast: f32 = (-2..=2)
        .map(|offset| darkness(on_white(x, y + offset as f32 / scale)))
        .sum();
    assert!(
        message_contrast > 100.,
        "a reduced message stroke must remain legible on white: {message_contrast}"
    );
    let lifeline = &sequence.lifelines[0];
    let line_contrast: f32 = (-2..=2)
        .map(|offset| darkness(on_white(lifeline.x + offset as f32 / scale, y + 16.)))
        .sum();
    assert!(
        line_contrast > 40.,
        "a reduced lifeline must retain visible coverage: {line_contrast}"
    );
    let actor = layout.nodes.get("A").unwrap();
    let actor_fill = on_white(actor.x + 8., actor.y + 8.);
    assert!(
        actor_fill[2] > actor_fill[0] + 8,
        "actor fill should be pale lavender: {actor_fill:?}"
    );
    let note = &sequence.notes[0];
    let note_fill = on_white(note.x + 8., note.y + 8.);
    assert!(
        note_fill[0] > note_fill[2] + 30 && note_fill[1] > note_fill[2] + 30,
        "note fill should be pale yellow: {note_fill:?}"
    );
}

#[test]
fn independent_fonts_change_actual_glyphs_and_partition_the_cache() {
    use pecan_mermaid::DiagramFonts;
    let renderer = Renderer::new();
    let source = "sequenceDiagram\n autonumber\n participant A as Reader 阅读器\n participant B as Worker 任务\n A->>B: Request 请求\n B-->>A: Result 结果\n";
    let database = pecan_mermaid::system_fonts();
    let installed = |candidates: &[&str]| {
        candidates
            .iter()
            .find(|name| {
                database
                    .faces()
                    .any(|face| face.families.iter().any(|(family, _)| family == *name))
            })
            .unwrap_or_else(|| panic!("font regression needs one of {candidates:?}"))
            .to_string()
    };
    let sans = installed(&["PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC"]);
    let serif = installed(&["Songti SC", "SimSun", "Noto Serif CJK SC"]);
    let mono = installed(&["Menlo", "Consolas", "DejaVu Sans Mono"]);
    let fonts = [
        DiagramFonts::new(&sans, &sans),
        DiagramFonts::new(&sans, &serif),
        DiagramFonts::new(&mono, &serif),
    ];
    for appearance in [Appearance::Light, Appearance::Dark] {
        let mut previous = None;
        for fonts in &fonts {
            let diagram = renderer
                .render_with_fonts(source, appearance, fonts)
                .unwrap();
            let png = diagram.png(1200.).unwrap();
            if let Some(previous) = previous {
                assert_ne!(
                    png, previous,
                    "changing either family changes actual raster glyphs"
                );
            }
            previous = Some(png);
            let cached = renderer
                .render_with_fonts(source, appearance, fonts)
                .unwrap();
            assert!(std::ptr::eq(diagram.svg().as_ptr(), cached.svg().as_ptr()));
        }
    }
    assert_eq!(renderer.stats().entries, 6);
    assert_eq!(renderer.stats().hits, 6);
}

#[test]
fn connector_tips_do_not_paint_past_their_anchors() {
    for source in [
        "flowchart LR\n A[Start] <--> B{Choose}\n B -->|Text| C[Native]\n B ==> D[Diagram]",
        "classDiagram\n A --> B\n B ..> C\n C --|> D\n D <|-- E",
        "stateDiagram-v2\n [*] --> Ready\n Ready --> Done",
        "architecture-beta\n service a(server)[Server]\n service b(database)[Database]\n a:R --> L:b",
        "C4Context\n Person(a, \"User\")\n System(b, \"App\")\n BiRel(a, b, \"Uses\")",
    ] {
        let diagram = Renderer::new().render(source, Appearance::Light).unwrap();
        let svg = roxmltree::Document::parse(diagram.svg()).unwrap();
        for polygon in svg
            .descendants()
            .filter(|node| node.has_tag_name("polygon"))
        {
            if polygon
                .attribute("points")
                .is_some_and(|value| value.starts_with("0,0 -"))
            {
                assert!(
                    polygon.attribute("stroke").is_none(),
                    "a flowchart tip outline crosses its anchor: {source}"
                );
            }
        }
        for edge in svg
            .descendants()
            .filter(|node| node.has_tag_name("path") || node.has_tag_name("line"))
        {
            for attr in ["marker-start", "marker-end"] {
                let Some(reference) = edge.attribute(attr) else {
                    continue;
                };
                let id = reference.trim_start_matches("url(#").trim_end_matches(')');
                let marker = svg
                    .descendants()
                    .find(|node| node.attribute("id") == Some(id))
                    .unwrap();
                let path = marker
                    .children()
                    .find(|node| node.has_tag_name("path"))
                    .unwrap();
                // The triangle tip must be at the reference, including any outer stroke.
                let d = path.attribute("d").unwrap();
                let xs: Vec<f32> = d
                    .split(['M', 'L', 'Z', 'z'])
                    .filter_map(|part| part.split_whitespace().next()?.parse().ok())
                    .collect();
                let tip = if attr == "marker-end"
                    || marker.attribute("orient") == Some("auto-start-reverse")
                {
                    xs.into_iter().fold(f32::NEG_INFINITY, f32::max)
                } else {
                    xs.into_iter().fold(f32::INFINITY, f32::min)
                };
                let reference: f32 = marker.attribute("refX").unwrap().parse().unwrap();
                assert!(
                    (tip - reference).abs() <= 0.6,
                    "{id} tip must meet its endpoint: {source}"
                );
                if path.attribute("fill") != Some("none") {
                    assert!(
                        path.attribute("stroke").is_none(),
                        "filled tips must not overshoot: {id}"
                    );
                }
            }
        }
    }
}

#[test]
fn clear_forward_flowcharts_use_centered_ports_without_staircases() {
    use mermaid_rs_renderer::{LayoutConfig, Theme};
    for direction in ["LR", "RL", "TB", "BT"] {
        for body in [
            "A[Open] --> B[Index]\n B --> C[Render]\n C --> D{Type}\n D -->|Text and code| E[Native]\n D -->|Mermaid| F[Diagram]\n F --> G[Preview]",
        ] {
            let source = format!("flowchart {direction}\n{body}");
            let graph = mermaid_rs_renderer::parse_mermaid_strict(&source)
                .unwrap()
                .graph;
            let layout = mermaid_rs_renderer::compute_layout(
                &graph,
                &Theme::modern(),
                &LayoutConfig::default(),
            );
            for edge in &layout.edges {
                if let Some((label, (x, y))) = edge.label.as_ref().zip(edge.label_anchor) {
                    assert!(
                        !layout.nodes.values().any(|node| {
                            x - label.width * 0.5 < node.x + node.width
                                && x + label.width * 0.5 > node.x
                                && y - label.height * 0.5 < node.y + node.height
                                && y + label.height * 0.5 > node.y
                        }),
                        "{direction}: branch captions must stay clear of node contents"
                    );
                }
            }
            for edge in &layout.edges {
                for (node_id, endpoint, outside) in [
                    (&edge.from, edge.points[0], edge.points[1]),
                    (
                        &edge.to,
                        *edge.points.last().unwrap(),
                        edge.points[edge.points.len() - 2],
                    ),
                ] {
                    let node = &layout.nodes[node_id];
                    if node.shape != mermaid_rs_renderer::ir::NodeShape::Rectangle {
                        continue;
                    }
                    let on_left = (endpoint.0 - node.x).abs() < 0.01;
                    let on_right = (endpoint.0 - node.x - node.width).abs() < 0.01;
                    if on_left || on_right {
                        assert!(
                            (outside.1 - endpoint.1).abs() < 0.01,
                            "{direction}: arrow must enter normally, not along a side: {:?}",
                            edge.points
                        );
                        assert!(if on_left {
                            outside.0 < endpoint.0
                        } else {
                            outside.0 > endpoint.0
                        });
                        assert!(
                            endpoint.1 - node.y > 5. && node.y + node.height - endpoint.1 > 5.,
                            "{direction}: arrow wings need corner clearance at {node_id}: {:?}",
                            edge.points
                        );
                    } else {
                        assert!(
                            (outside.0 - endpoint.0).abs() < 0.01,
                            "{direction}: vertical port must enter normally"
                        );
                    }
                }
                for segment in edge.points.windows(2) {
                    assert!(
                        (segment[0].0 - segment[1].0).abs() < 0.01
                            || (segment[0].1 - segment[1].1).abs() < 0.01,
                        "{direction} route must be horizontal or vertical: {:?}",
                        edge.points
                    );
                }
            }
        }
    }
}

#[test]
fn flowchart_shaft_stops_inside_the_arrowhead() {
    let diagram = Renderer::new()
        .render("flowchart LR\n A[Source] --> B[Target]", Appearance::Light)
        .unwrap();
    let xml = roxmltree::Document::parse(diagram.svg()).unwrap();
    let shaft = xml
        .descendants()
        .find(|node| node.attribute("class") == Some("edgePath"))
        .unwrap();
    let d = shaft.attribute("d").unwrap();
    let last: Vec<f32> = d
        .rsplit('L')
        .next()
        .unwrap()
        .trim()
        .split(',')
        .map(|x| x.trim().parse().unwrap())
        .collect();
    let head = shaft.next_sibling_element().unwrap();
    let transform = head.attribute("transform").unwrap();
    let tip: Vec<f32> = transform
        .split_once("translate(")
        .unwrap()
        .1
        .split(')')
        .next()
        .unwrap()
        .split_whitespace()
        .map(|x| x.parse().unwrap())
        .collect();
    assert!(
        tip[0] - last[0] > 4.,
        "the shaft must not paint a rectangular tail at the arrow tip"
    );
    // Inspect the painted edge alone: a node's outline must not mask a protruding shaft.
    let snippet = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"160\" height=\"160\" viewBox=\"{} {} 20 20\">{}{}</svg>",
        tip[0] - 10.,
        tip[1] - 10.,
        &diagram.svg()[shaft.range()],
        &diagram.svg()[head.range()]
    );
    let tree = resvg::usvg::Tree::from_str(&snippet, &resvg::usvg::Options::default()).unwrap();
    let mut pixels = resvg::tiny_skia::Pixmap::new(160, 160).unwrap();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixels.as_mut(),
    );
    assert!(
        pixels.pixel(72, 86).unwrap().alpha() < 80,
        "line ink outside the triangular tip must be absent"
    );
    assert!(
        pixels.pixel(24, 80).unwrap().alpha() > 200,
        "shaft and arrowhead must join without a gap"
    );
}

#[test]
fn flowchart_smoothing_preserves_obstacles_and_distinct_parallel_edges() {
    use mermaid_rs_renderer::{LayoutConfig, Theme};
    let theme = Theme::modern();
    let config = LayoutConfig::default();
    let graph = mermaid_rs_renderer::parse_mermaid_strict("flowchart LR\n A --> B")
        .unwrap()
        .graph;
    let mut layout = mermaid_rs_renderer::compute_layout(&graph, &theme, &config);
    let from = &layout.nodes["A"];
    let start = (from.x + from.width, from.y + from.height * 0.5);
    let to = layout.nodes.get_mut("B").unwrap();
    to.y += 100.;
    let end = (to.x, to.y + to.height * 0.5);
    let mid_x = (start.0 + end.0) * 0.5;
    layout.edges[0].points = vec![start, (mid_x, start.1), (mid_x, end.1), end];
    let path = |svg: &str| {
        let tree = roxmltree::Document::parse(svg).unwrap();
        tree.descendants()
            .find(|node| node.attribute("class") == Some("edgePath"))
            .unwrap()
            .attribute("d")
            .unwrap()
            .to_owned()
    };
    assert!(
        path(&mermaid_rs_renderer::render_svg(&layout, &theme, &config)).contains(" C "),
        "a clear forward branch should have one smooth curve"
    );
    let mut obstacle = layout.nodes["A"].clone();
    obstacle.id = "obstacle".into();
    obstacle.x = mid_x - 10.;
    obstacle.y = (start.1 + end.1) * 0.5 - 10.;
    obstacle.width = 20.;
    obstacle.height = 20.;
    layout.nodes.insert(obstacle.id.clone(), obstacle);
    assert!(
        !path(&mermaid_rs_renderer::render_svg(&layout, &theme, &config)).contains(" C "),
        "a curve cannot cut across an occupied control hull"
    );

    let parallel = mermaid_rs_renderer::parse_mermaid_strict(
        "flowchart LR\n A -->|first| B\n A -->|second| B",
    )
    .unwrap()
    .graph;
    let layout = mermaid_rs_renderer::compute_layout(&parallel, &theme, &config);
    assert_ne!(
        layout.edges[0].points, layout.edges[1].points,
        "parallel relationships must keep distinct routes"
    );
    assert_ne!(
        layout.edges[0].label_anchor, layout.edges[1].label_anchor,
        "parallel captions must stay apart"
    );
}
