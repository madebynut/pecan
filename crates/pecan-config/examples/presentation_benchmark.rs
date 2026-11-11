//! Compare the old clone-based settings check with the borrowed field comparison.
use pecan_config::PreferencesData;
use std::{hint::black_box, time::Instant};

fn legacy(left: &PreferencesData, right: &PreferencesData) -> bool {
    PreferencesData {
        auto_reload: right.auto_reload,
        theme: right.theme.clone(),
        ..left.clone()
    } == *right
}

fn main() {
    const CALLS: usize = 500_000;
    let base = PreferencesData::default();
    for (name, other) in [
        ("unchanged", base.clone()),
        (
            "body_changed",
            PreferencesData {
                font_size: 22.,
                ..base.clone()
            },
        ),
        (
            "unrelated_changed",
            PreferencesData {
                theme: "dark".into(),
                auto_reload: false,
                ..base.clone()
            },
        ),
    ] {
        assert_eq!(legacy(&base, &other), base.same_presentation(&other));
        for _ in 0..10_000 {
            black_box(legacy(black_box(&base), black_box(&other)));
            black_box(base.same_presentation(black_box(&other)));
        }
        let started = Instant::now();
        for _ in 0..CALLS {
            black_box(legacy(black_box(&base), black_box(&other)));
        }
        let previous = started.elapsed().as_nanos() as f64 / CALLS as f64;
        let started = Instant::now();
        for _ in 0..CALLS {
            black_box(base.same_presentation(black_box(&other)));
        }
        let current = started.elapsed().as_nanos() as f64 / CALLS as f64;
        println!("{name}: legacy {previous:.2} ns/call, current {current:.2} ns/call");
    }
}
