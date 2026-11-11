use mermaid_rs_renderer::{LayoutConfig, Theme, layout::DiagramData};

#[test]
fn sequence_arrow_bases_are_perpendicular_to_horizontal_messages() {
    use resvg::{tiny_skia, usvg};

    fn triangles(group: &usvg::Group, result: &mut Vec<[tiny_skia::Point; 3]>) {
        for node in group.children() {
            match node {
                usvg::Node::Group(group) => triangles(group, result),
                usvg::Node::Path(path)
                    if path.fill().is_some()
                        && path.abs_bounding_box().width() < 16.
                        && path.abs_bounding_box().height() < 16. =>
                {
                    let transformed = path.data().clone().transform(path.abs_transform()).unwrap();
                    let vertices: Vec<_> = transformed
                        .segments()
                        .filter_map(|segment| match segment {
                            tiny_skia::PathSegment::MoveTo(point)
                            | tiny_skia::PathSegment::LineTo(point) => Some(point),
                            _ => None,
                        })
                        .collect();
                    if let Ok(vertices) = vertices.try_into() {
                        result.push(vertices);
                    }
                }
                _ => {}
            }
        }
    }

    let mut graph = mermaid_rs_renderer::parse_mermaid_strict(
        "sequenceDiagram\n participant A\n participant B\n A->>B: Forward\n B-->>A: Return\n B->>B: Retry\n",
    )
    .unwrap()
    .graph;
    graph.edges[0].arrow_start = true;
    let theme = Theme::modern();
    let config = LayoutConfig::default();
    let layout = mermaid_rs_renderer::compute_layout(&graph, &theme, &config);
    let svg = mermaid_rs_renderer::render_svg(&layout, &theme, &config);
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
    let mut arrows = Vec::new();
    triangles(tree.root(), &mut arrows);
    assert_eq!(
        arrows.len(),
        4,
        "forward, return, self-call and start arrow must render"
    );
    for (index, arrow) in arrows.iter().enumerate() {
        let top = arrow.iter().min_by(|a, b| a.y.total_cmp(&b.y)).unwrap();
        let bottom = arrow.iter().max_by(|a, b| a.y.total_cmp(&b.y)).unwrap();
        assert!(
            (top.x - bottom.x).abs() < 0.05,
            "arrow {index}: native rendered base must be vertical, got {top:?}–{bottom:?}"
        );
    }
}

#[test]
fn sequence_arrow_tips_meet_their_message_endpoints() {
    use resvg::usvg;
    fn filled_bounds(node: &usvg::Node, result: &mut Vec<usvg::Rect>) {
        match node {
            usvg::Node::Group(group) => {
                for child in group.children() {
                    filled_bounds(child, result);
                }
            }
            usvg::Node::Path(path) if path.fill().is_some() => {
                result.push(path.abs_stroke_bounding_box())
            }
            _ => {}
        }
    }
    let source = "sequenceDiagram\n participant A\n participant B\n A->>B: Forward\n B-->>A: Return\n B->>B: Retry\n";
    let mut graph = mermaid_rs_renderer::parse_mermaid_strict(source)
        .unwrap()
        .graph;
    graph.edges[0].arrow_start = true;
    let theme = Theme::modern();
    let config = LayoutConfig::default();
    let layout = mermaid_rs_renderer::compute_layout(&graph, &theme, &config);
    let svg = mermaid_rs_renderer::render_svg(&layout, &theme, &config);
    let view_box: Vec<f32> = svg
        .split_once("viewBox=\"")
        .unwrap()
        .1
        .split('"')
        .next()
        .unwrap()
        .split_whitespace()
        .map(|value| value.parse().unwrap())
        .collect();
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).unwrap();
    let mut markers = Vec::new();
    for node in tree.root().children() {
        filled_bounds(node, &mut markers);
    }
    markers.retain(|bounds| bounds.width() < 16. && bounds.height() < 16.);
    assert_eq!(
        markers.len(),
        layout
            .edges
            .iter()
            .map(|edge| usize::from(edge.arrow_start) + usize::from(edge.arrow_end))
            .sum::<usize>()
    );
    for (index, edge) in layout.edges.iter().enumerate() {
        for (at_start, present) in [(false, edge.arrow_end), (true, edge.arrow_start)] {
            if !present {
                continue;
            }
            let (tip, neighbor) = if at_start {
                (edge.points[0], edge.points[1])
            } else {
                (
                    edge.points[edge.points.len() - 1],
                    edge.points[edge.points.len() - 2],
                )
            };
            let x = tip.0 - view_box[0];
            let rightward = tip.0 > neighbor.0;
            assert!(
                markers
                    .iter()
                    .filter(|bounds| ((bounds.top() + bounds.bottom()) / 2.
                        - (tip.1 - view_box[1]))
                        .abs()
                        < 0.05)
                    .any(|bounds| (if rightward {
                        bounds.right()
                    } else {
                        bounds.left()
                    } - x)
                        .abs()
                        < 0.05),
                "edge {index}, start {at_start}: arrow paint must stop at message endpoint {x}, got {markers:?}"
            );
        }
    }
}

#[test]
fn zoomed_sequence_arrow_tips_have_no_protruding_shaft_pixels() {
    use resvg::{tiny_skia, usvg};
    let graph = mermaid_rs_renderer::parse_mermaid_strict(
        "sequenceDiagram\n participant A\n participant B\n A->>B: Forward\n B-->>A: Return\n B->>B: Retry\n"
    ).unwrap().graph;
    for theme in [Theme::modern(), Theme::dark()] {
        let config = LayoutConfig::default();
        let layout = mermaid_rs_renderer::compute_layout(&graph, &theme, &config);
        let svg = mermaid_rs_renderer::render_svg(&layout, &theme, &config);
        let defs = svg
            .split_once("<defs>")
            .unwrap()
            .1
            .split_once("</defs>")
            .unwrap()
            .0;
        for (index, edge) in layout.edges.iter().enumerate() {
            let id = format!("data-edge-id=\"edge-{index}\"");
            let paths: Vec<_> = svg
                .split("<path ")
                .skip(1)
                .filter_map(|part| {
                    let path = part.split_once("/>")?.0;
                    path.contains(&id).then(|| format!("<path {path}/>"))
                })
                .collect();
            assert_eq!(paths.len(), 2, "separate native shaft and marker paths");
            let tip = *edge.points.last().unwrap();
            let render = |only_marker: bool| {
                let paths = paths
                    .iter()
                    .filter(|path| !only_marker || path.contains("edgeMarkers"))
                    .cloned()
                    .collect::<String>();
                let fragment = format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"256\" height=\"96\" viewBox=\"{} {} 32 12\"><defs>{defs}</defs>{paths}</svg>",
                    tip.0 - 16.,
                    tip.1 - 6.
                );
                let tree = usvg::Tree::from_str(&fragment, &usvg::Options::default()).unwrap();
                let mut pixels = tiny_skia::Pixmap::new(256, 96).unwrap();
                resvg::render(
                    &tree,
                    tiny_skia::Transform::identity(),
                    &mut pixels.as_mut(),
                );
                pixels
            };
            let actual = render(false);
            let marker = render(true);
            assert!(
                marker.data().iter().any(|&value| value != 0),
                "native SVG markers must actually render"
            );
            // At 800% zoom compare the final 2 units of the tip with the
            // arrow alone. A full-length stroked path leaks outside its taper.
            for y in 30..66 {
                for x in 112..144 {
                    assert_eq!(
                        actual.pixel(x, y),
                        marker.pixel(x, y),
                        "edge {index}: line shaft must not leak outside the arrow taper at ({x}, {y})"
                    );
                }
            }
        }
    }
}

#[test]
fn sequence_markers_follow_actual_message_origins() {
    let graph = mermaid_rs_renderer::parse_mermaid_strict(
        "sequenceDiagram\n autonumber\n participant A\n participant B\n A->>B: Request\n B-->>A: Response\n B->>B: Retry\n"
    ).unwrap().graph;
    let layout =
        mermaid_rs_renderer::compute_layout(&graph, &Theme::modern(), &LayoutConfig::default());
    let DiagramData::Sequence(sequence) = &layout.diagram else {
        panic!("expected sequence")
    };
    assert_eq!(sequence.numbers.len(), layout.edges.len());
    for (edge, number) in layout.edges.iter().zip(&sequence.numbers) {
        let start = edge.points.first().unwrap();
        assert!(
            (number.x - start.0).abs() < 0.01 && (number.y - start.1).abs() < 0.01,
            "marker {} must follow the actual outgoing point {start:?}, got ({}, {})",
            number.value,
            number.x,
            number.y
        );
    }
}

#[test]
fn self_calls_keep_compact_geometry_with_long_captions() {
    let graph = mermaid_rs_renderer::parse_mermaid_strict(
        "sequenceDiagram\n autonumber\n participant A\n participant B\n alt 首次请求\n A->>A: 初始化\n A->>A: 写入定位历史，按定位时间更新设备最新位置\n else 重复请求\n A->>A: 丢弃并计数\n end\n A->>B: 完成\n"
    ).unwrap().graph;
    let layout =
        mermaid_rs_renderer::compute_layout(&graph, &Theme::modern(), &LayoutConfig::default());
    let DiagramData::Sequence(sequence) = &layout.diagram else {
        panic!("expected sequence")
    };
    let mut widths = Vec::new();
    for edge in &layout.edges[..3] {
        let width = edge.points[1].0 - edge.points[0].0;
        assert!(width <= 40., "self-call geometry remains compact: {width}");
        widths.push(width);
        let label = edge.label.as_ref().unwrap();
        let (x, _) = edge.label_anchor.unwrap();
        let frame = &sequence.frames[0];
        assert!(
            x - label.width / 2. >= edge.points[0].0,
            "self-call caption starts next to its participant"
        );
        assert!(
            x + label.width / 2. <= frame.x + frame.width,
            "branch frame contains the caption independently of loop width"
        );
    }
    assert!(
        widths
            .windows(2)
            .all(|pair| (pair[0] - pair[1]).abs() < 0.01),
        "short and long captions use the same loop width"
    );
}

#[test]
fn sequence_numbers_keep_equal_circles_and_fit_their_glyphs() {
    use resvg::usvg;
    let mut fonts = usvg::fontdb::Database::new();
    fonts.load_system_fonts();
    let options = usvg::Options {
        fontdb: std::sync::Arc::new(fonts),
        ..Default::default()
    };
    let themes = [Theme::modern(), Theme::dark()]
        .into_iter()
        .flat_map(|theme| {
            let mut system = theme.clone();
            system.font_family =
                "PingFang SC, Hiragino Sans GB, Noto Sans CJK SC, sans-serif".into();
            let mut mono = theme.clone();
            mono.font_family = "Menlo, Consolas, DejaVu Sans Mono, monospace".into();
            [theme, system, mono]
        });
    for theme in themes {
        let mut circle_radius = None;
        let mut diagram_size = None;
        let mut single_digit_size = 0.;
        for value in [1, 9, 10, 11, 20, 88, 99, 100, 999, usize::MAX] {
            let graph = mermaid_rs_renderer::parse_mermaid_strict(&format!(
            "sequenceDiagram\n autonumber {value}\n participant A\n participant B\n A->>B: Request\n"
        )).unwrap().graph;
            let config = LayoutConfig::default();
            let layout = mermaid_rs_renderer::compute_layout(&graph, &theme, &config);
            let DiagramData::Sequence(sequence) = &layout.diagram else {
                panic!("expected sequence")
            };
            let number = &sequence.numbers[0];
            let expected_size = *diagram_size.get_or_insert((layout.width, layout.height));
            assert_eq!(
                (layout.width, layout.height),
                expected_size,
                "digit count must not change diagram spacing for number {value}"
            );
            assert!(
                number.font_size.is_finite() && number.font_size > 0.,
                "even the largest sequence number needs valid fitted type"
            );
            let expected_radius = *circle_radius.get_or_insert(number.radius);
            assert_eq!(
                number.radius, expected_radius,
                "circle size must stay unchanged for number {value}"
            );
            if value == 1 {
                single_digit_size = number.font_size;
            } else if value < 100 {
                assert_eq!(
                    number.font_size, single_digit_size,
                    "two-digit sequence number {value} must keep the single-digit font size"
                );
            } else {
                assert!(
                    number.font_size < single_digit_size,
                    "three or more digits still fit within the uniform circle"
                );
            }
            let svg = mermaid_rs_renderer::render_svg(&layout, &theme, &config);
            let marker = format!("<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"", number.x, number.y);
            let radius: f32 = svg
                .split_once(&marker)
                .unwrap()
                .1
                .split('"')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            let tree = usvg::Tree::from_str(&svg, &options).unwrap();
            fn glyph_bounds(group: &usvg::Group, label: &str) -> Option<usvg::Rect> {
                for node in group.children() {
                    match node {
                        usvg::Node::Group(child) => {
                            if let Some(bounds) = glyph_bounds(child, label) {
                                return Some(bounds);
                            }
                        }
                        usvg::Node::Text(text)
                            if text.chunks().iter().any(|chunk| chunk.text() == label) =>
                        {
                            return Some(text.flattened().abs_bounding_box());
                        }
                        _ => {}
                    }
                }
                None
            }
            let glyphs =
                glyph_bounds(tree.root(), &value.to_string()).expect("number has real glyph paths");
            for x in [glyphs.left(), glyphs.right()] {
                for y in [glyphs.top(), glyphs.bottom()] {
                    assert!(
                        (x - number.x).hypot(y - number.y)
                            <= radius - if value < 10 { 1. } else { 0.5 },
                        "number {value} needs padding from the circular border: glyphs {glyphs:?}, radius {radius}"
                    );
                }
            }
        }
    }
}

const SOURCE: &str = "sequenceDiagram\n autonumber\n participant A as 客户端\n participant B as 数据服务\n participant C as 查询方\n A->>B: 上报结果\n B->>B: 鉴权、校验、转换为统一格式\n B->>B: 按设备编号和定位时间幂等\n alt 首次出现\n B->>B: 写入历史，按时间更新设备最新位置\n else 重复数据\n B->>B: 丢弃并计数\n end\n C->>B: 查询最新位置和历史轨迹\n";

#[test]
fn chinese_self_messages_and_branch_headers_have_separate_space() {
    let graph = mermaid_rs_renderer::parse_mermaid_strict(SOURCE)
        .unwrap()
        .graph;
    let theme = Theme::modern();
    let layout = mermaid_rs_renderer::compute_layout(&graph, &theme, &LayoutConfig::default());
    let DiagramData::Sequence(sequence) = &layout.diagram else {
        panic!("sequence source must produce sequence geometry")
    };
    for frame in &sequence.frames {
        for label in &frame.section_labels {
            assert!(
                label.y - label.text.height / 2. >= frame.y + frame.label_box.3 + 4.,
                "branch condition must clear the frame tag: {frame:?}"
            );
            assert!(
                label.x - label.text.width / 2. >= frame.x
                    && label.x + label.text.width / 2. <= frame.x + frame.width,
                "branch condition must fit its frame"
            );
        }
    }
    for pair in layout.edges.windows(2) {
        let previous_bottom = pair[0]
            .points
            .iter()
            .map(|point| point.1)
            .fold(0., f32::max);
        let next = &pair[1];
        if let (Some(label), Some((_, y))) = (&next.label, next.label_anchor) {
            assert!(
                y - label.height / 2. >= previous_bottom + 8.,
                "next message must clear the previous self loop: {pair:?}"
            );
        }
    }
    for (edge, number) in layout.edges.iter().zip(&sequence.numbers) {
        if let (Some(label), Some((x, y))) = (&edge.label, edge.label_anchor) {
            assert!(
                (number.x - x).abs() > label.width / 2. + number.radius + 2.
                    || (number.y - y).abs() > label.height / 2. + number.radius + 2.,
                "sequence number must not collide with message text"
            );
        }
    }
    let frame = &sequence.frames[0];
    for edge in &layout.edges[3..5] {
        for point in &edge.points {
            assert!(
                point.0 <= frame.x + frame.width && point.1 <= frame.y + frame.height,
                "self loop must fit inside its branch frame"
            );
        }
    }
}

#[test]
fn a_note_after_a_branch_is_outside_the_branch() {
    let graph = mermaid_rs_renderer::parse_mermaid_strict(
        "sequenceDiagram\n participant A\n participant B\n alt 初次请求\n A->>A: 初始化并更新状态\n A->>B: 请求\n end\n Note over A,B: 后续步骤说明\n B-->>A: 返回\n",
    ).unwrap().graph;
    let layout =
        mermaid_rs_renderer::compute_layout(&graph, &Theme::modern(), &LayoutConfig::default());
    let DiagramData::Sequence(sequence) = layout.diagram else {
        panic!("expected sequence")
    };
    let frame = &sequence.frames[0];
    let note = &sequence.notes[0];
    assert!(
        note.y > frame.y + frame.height,
        "a note after end cannot be pulled into the preceding alt frame"
    );
}

#[test]
fn sequence_frame_tabs_cover_lifelines_and_use_dash_borders() {
    use resvg::{tiny_skia, usvg};
    let graph = mermaid_rs_renderer::parse_mermaid_strict("sequenceDiagram\n participant A\n participant B\n alt Condition\n A->>B: Request\n else Retry\n B-->>A: Response\n end\n").unwrap().graph;
    for theme in [Theme::modern(), Theme::dark()] {
        let config = LayoutConfig::default();
        let layout = mermaid_rs_renderer::compute_layout(&graph, &theme, &config);
        let DiagramData::Sequence(sequence) = &layout.diagram else {
            panic!("expected sequence")
        };
        let svg = mermaid_rs_renderer::render_svg(&layout, &theme, &config);
        assert!(
            svg.contains("stroke-dasharray=\"6 4\""),
            "frames and dividers use short dashes"
        );
        let tab_index = svg.rfind("<polygon points=").unwrap();
        let lifeline_index = svg.rfind("stroke-width=\"1.5\"").unwrap();
        let border_index = svg.rfind("stroke-dasharray=\"6 4\"").unwrap();
        assert!(
            border_index > lifeline_index,
            "frame borders are painted above lifelines"
        );
        assert!(
            tab_index > lifeline_index,
            "frame tab is painted above lifelines"
        );
        let frame = &sequence.frames[0];
        let (x, y, w, h) = frame.label_box;
        let line = sequence
            .lifelines
            .iter()
            .find(|line| line.x > x + 2. && line.x < x + w - 2.)
            .expect("a lifeline crosses the tab");
        let view: Vec<f32> = svg
            .split_once("viewBox=\"")
            .unwrap()
            .1
            .split('"')
            .next()
            .unwrap()
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        let mut options = usvg::Options::default();
        options.fontdb_mut().load_system_fonts();
        let tree = usvg::Tree::from_str(&svg, &options).unwrap();
        let size = tree.size().to_int_size();
        let mut pixels = tiny_skia::Pixmap::new(size.width() * 2, size.height() * 2).unwrap();
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(2., 2.),
            &mut pixels.as_mut(),
        );
        let pixel = pixels
            .pixel(
                ((line.x - view[0]) * 2.).round() as u32,
                ((y + h * 0.15 - view[1]) * 2.).round() as u32,
            )
            .unwrap();
        let rgb = u32::from_str_radix(theme.primary_color.trim_start_matches('#'), 16).unwrap();
        assert_eq!(pixel.alpha(), 255, "tab is opaque");
        for (actual, expected) in [
            (pixel.red(), (rgb >> 16) as u8),
            (pixel.green(), (rgb >> 8) as u8),
            (pixel.blue(), rgb as u8),
        ] {
            assert!(
                actual.abs_diff(expected) <= 2,
                "tab fill must mask the lifeline: {pixel:?}"
            );
        }
    }
}
