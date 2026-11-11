//! Simplify unobstructed forward connectors without replacing obstacle routing.
use std::collections::BTreeMap;

use crate::ir::{Direction, NodeShape};
use crate::layout::{EdgeLayout, NodeLayout};

pub(in crate::layout) fn simplify(
    direction: Direction,
    nodes: &BTreeMap<String, NodeLayout>,
    edges: &mut [EdgeLayout],
) {
    let horizontal = matches!(direction, Direction::LeftRight | Direction::RightLeft);
    // Preserve distinct ports and captions on parallel relationships.
    let parallel: Vec<_> = {
        let mut counts = BTreeMap::new();
        for edge in edges.iter() {
            *counts.entry((&edge.from, &edge.to)).or_insert(0) += 1;
        }
        edges
            .iter()
            .map(|edge| counts[&(&edge.from, &edge.to)] > 1)
            .collect()
    };
    for (index, edge) in edges.iter_mut().enumerate() {
        if parallel[index] {
            continue;
        }
        let (Some(from), Some(to)) = (nodes.get(&edge.from), nodes.get(&edge.to)) else {
            continue;
        };
        let supported = |node: &NodeLayout| {
            !node.hidden
                && node.anchor_subgraph.is_none()
                && matches!(
                    node.shape,
                    NodeShape::Rectangle
                        | NodeShape::RoundRect
                        | NodeShape::Stadium
                        | NodeShape::Diamond
                        | NodeShape::Circle
                        | NodeShape::Subroutine
                        | NodeShape::Cylinder
                )
        };
        if !supported(from)
            || !supported(to)
            || edge.start_decoration.is_some()
            || edge.end_decoration.is_some()
            || edge.start_label.is_some()
            || edge.end_label.is_some()
        {
            continue;
        }
        let center = |n: &NodeLayout| (n.x + n.width * 0.5, n.y + n.height * 0.5);
        let (mut start, mut end) = (center(from), center(to));
        // RL/BT are mirrored by finalize after this canonical forward layout.
        if horizontal {
            start.0 += from.width * 0.5;
            end.0 -= to.width * 0.5;
        } else {
            start.1 += from.height * 0.5;
            end.1 -= to.height * 0.5;
        }
        let gap = if horizontal {
            end.0 - start.0
        } else {
            end.1 - start.1
        };
        if gap < 24. {
            continue;
        }
        let x1 = start.0.min(end.0);
        let x2 = start.0.max(end.0);
        let y1 = start.1.min(end.1);
        let y2 = start.1.max(end.1);
        // The full control hull must be empty, so smoothing cannot cut an obstacle.
        if nodes.values().any(|n| {
            !n.hidden
                && n.id != from.id
                && n.id != to.id
                && n.x < x2 + 8.
                && n.x + n.width > x1 - 8.
                && n.y < y2 + 8.
                && n.y + n.height > y1 - 8.
        }) {
            continue;
        }
        let middle = ((start.0 + end.0) * 0.5, (start.1 + end.1) * 0.5);
        edge.points = if (horizontal && (start.1 - end.1).abs() < 0.01)
            || (!horizontal && (start.0 - end.0).abs() < 0.01)
        {
            vec![start, end]
        } else if horizontal {
            vec![start, (middle.0, start.1), (middle.0, end.1), end]
        } else {
            vec![start, (start.0, middle.1), (end.0, middle.1), end]
        };
        // The existing final label-placement pass resolves collisions and expands bounds.
        if let Some(label) = &edge.label {
            edge.label_anchor = Some(if horizontal {
                (middle.0, middle.1 - label.height * 0.5 - 8.)
            } else {
                (middle.0 + label.width * 0.5 + 8., middle.1)
            });
        }
    }
}
