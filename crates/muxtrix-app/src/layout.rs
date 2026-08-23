//! Pane-tree geometry: pure functions over [`PaneTree`] with no UI dependency.
//!
//! Everything here maps a tree (plus the focused pane) to layout facts —
//! rectangles, neighbours, balanced splits — so the view layer only has to
//! draw what these decide.

use std::collections::BTreeSet;

use muxtrix_domain::{PaneId, PaneTree, SplitAxis, SplitRatio, WorkspaceTab};

use crate::app::{NavDirection, PaneLayout, PaneRect};

pub(crate) fn pane_ids_in_layout(tree: &PaneTree) -> Vec<PaneId> {
    tree.pane_ids()
}

/// Keeps the tree's spatial order while recovering panes that are still owned
/// by the tab but missing from a stale layout projection.
pub(crate) fn pane_ids_for_layout(tab: &WorkspaceTab) -> Vec<PaneId> {
    let mut seen = BTreeSet::new();
    let mut pane_ids = tab
        .root
        .pane_ids()
        .into_iter()
        .filter(|pane_id| tab.panes.contains_key(pane_id) && seen.insert(*pane_id))
        .collect::<Vec<_>>();
    pane_ids.extend(
        tab.panes
            .keys()
            .copied()
            .filter(|pane_id| seen.insert(*pane_id)),
    );
    pane_ids
}

pub(crate) fn same_panes(first: &[PaneId], second: &[PaneId]) -> bool {
    if first.len() != second.len() {
        return false;
    }
    let mut first = first.to_vec();
    let mut second = second.to_vec();
    first.sort_unstable();
    second.sort_unstable();
    first == second
}

/// A stack always needs one live body. In a half-stacked layout the global
/// focus may be in the sibling leaf, so the stack falls back to its first pane
/// instead of rendering only collapsed headers above an empty field.
pub(crate) fn expanded_stack_pane(pane_ids: &[PaneId], focused_pane_id: PaneId) -> Option<PaneId> {
    pane_ids
        .contains(&focused_pane_id)
        .then_some(focused_pane_id)
        .or_else(|| pane_ids.first().copied())
}

pub(crate) fn pane_layout_tree(layout: PaneLayout, pane_ids: &[PaneId]) -> PaneTree {
    match layout {
        PaneLayout::Base | PaneLayout::Vertical => {
            grid_pane_layout(pane_ids, SplitAxis::Horizontal, SplitAxis::Vertical)
        }
        PaneLayout::Horizontal => {
            grid_pane_layout(pane_ids, SplitAxis::Vertical, SplitAxis::Horizontal)
        }
        PaneLayout::Stacked => PaneTree::stack(pane_ids.to_vec())
            .expect("a pane layout is only built from non-empty pane lists"),
        PaneLayout::HalfStacked => {
            let Some((first, rest)) = pane_ids.split_first() else {
                unreachable!("a pane layout is only built from non-empty pane lists");
            };
            if rest.is_empty() {
                return PaneTree::leaf(*first);
            }
            PaneTree::Split {
                axis: SplitAxis::Horizontal,
                ratio: SplitRatio::EQUAL,
                first: Box::new(PaneTree::leaf(*first)),
                second: Box::new(
                    PaneTree::stack(rest.to_vec())
                        .expect("half-stacked layouts always have a right-hand stack"),
                ),
            }
        }
    }
}

/// Mirrors Zellij's default tiled layouts: a leading remainder group followed
/// by groups of four, laid out as columns for Vertical and rows for Horizontal.
pub(crate) fn grid_pane_layout(
    pane_ids: &[PaneId],
    group_axis: SplitAxis,
    within_group_axis: SplitAxis,
) -> PaneTree {
    if pane_ids.len() == 1 {
        return PaneTree::leaf(pane_ids[0]);
    }
    // Zellij's first Vertical constraint is a full-height leading pane with
    // the remaining panes sharing the second column.
    if group_axis == SplitAxis::Horizontal && pane_ids.len() <= 5 {
        return PaneTree::Split {
            axis: group_axis,
            ratio: SplitRatio::EQUAL,
            first: Box::new(PaneTree::leaf(pane_ids[0])),
            second: Box::new(balanced_pane_layout(&pane_ids[1..], within_group_axis)),
        };
    }
    // The first Horizontal constraint is a simple sequence of rows.
    if group_axis == SplitAxis::Vertical && pane_ids.len() <= 4 {
        return balanced_pane_layout(pane_ids, group_axis);
    }
    let group_count = pane_ids.len().div_ceil(4);
    let leading = pane_ids.len() - (group_count - 1) * 4;
    let mut groups = Vec::with_capacity(group_count);
    groups.push(balanced_pane_layout(
        &pane_ids[..leading],
        within_group_axis,
    ));
    for group in pane_ids[leading..].chunks(4) {
        groups.push(balanced_pane_layout(group, within_group_axis));
    }
    balanced_tree_layout(groups, group_axis)
}

pub(crate) fn balanced_pane_layout(pane_ids: &[PaneId], axis: SplitAxis) -> PaneTree {
    match pane_ids {
        [] => unreachable!("pane layouts require at least one pane"),
        [pane_id] => PaneTree::leaf(*pane_id),
        _ => {
            let split = pane_ids.len().div_ceil(2);
            let ratio = SplitRatio::new(((split * 1_000) / pane_ids.len()) as u16)
                .expect("balanced pane groups stay inside split ratio bounds");
            PaneTree::Split {
                axis,
                ratio,
                first: Box::new(balanced_pane_layout(&pane_ids[..split], axis)),
                second: Box::new(balanced_pane_layout(&pane_ids[split..], axis)),
            }
        }
    }
}

pub(crate) fn balanced_tree_layout(mut trees: Vec<PaneTree>, axis: SplitAxis) -> PaneTree {
    match trees.len() {
        0 => unreachable!("pane layouts require at least one pane group"),
        1 => trees.pop().expect("the only pane group remains"),
        len => {
            let split = len.div_ceil(2);
            let second = trees.split_off(split);
            let ratio = SplitRatio::new(((split * 1_000) / len) as u16)
                .expect("balanced pane groups stay inside split ratio bounds");
            PaneTree::Split {
                axis,
                ratio,
                first: Box::new(balanced_tree_layout(trees, axis)),
                second: Box::new(balanced_tree_layout(second, axis)),
            }
        }
    }
}

/// Grow the deepest split containing `target` by about 30% of its footprint.
/// Once that would consume its sibling, both sides become a title-sheet stack.
pub(crate) fn enlarge_focused_tree(tree: &mut PaneTree, target: PaneId) -> bool {
    let PaneTree::Split {
        ratio,
        first,
        second,
        ..
    } = tree
    else {
        return false;
    };
    let in_first = first.contains(target);
    let branch = if in_first { first } else { second };
    if enlarge_focused_tree(branch, target) {
        return true;
    }

    let current = ratio.permille();
    let next = if in_first {
        current
            .checked_add(300)
            .filter(|next| *next <= SplitRatio::MAX)
    } else {
        current
            .checked_sub(300)
            .filter(|next| *next >= SplitRatio::MIN)
    };
    if let Some(next) = next {
        *ratio = SplitRatio::new(next).expect("bounded resize ratios remain valid");
        return true;
    }

    let pane_ids = tree.pane_ids();
    *tree = PaneTree::stack(pane_ids).expect("a split always contains at least two panes");
    true
}

/// Grow `target` across the nearest split on one side of it. The direction is
/// chosen from rendered pane geometry, while this tree walk keeps the mutation
/// attached to the exact split that owns that boundary.
pub(crate) fn enlarge_focused_tree_toward(
    tree: &mut PaneTree,
    target: PaneId,
    direction: NavDirection,
) -> bool {
    let PaneTree::Split {
        axis,
        ratio,
        first,
        second,
    } = tree
    else {
        return false;
    };
    let in_first = first.contains(target);
    let branch = if in_first {
        &mut **first
    } else {
        &mut **second
    };
    if enlarge_focused_tree_toward(branch, target, direction) {
        return true;
    }

    let owns_boundary = matches!(
        (axis, in_first, direction),
        (SplitAxis::Vertical, false, NavDirection::Up)
            | (SplitAxis::Vertical, true, NavDirection::Down)
            | (SplitAxis::Horizontal, false, NavDirection::Left)
            | (SplitAxis::Horizontal, true, NavDirection::Right)
    );
    if !owns_boundary {
        return false;
    }

    let current = ratio.permille();
    let next = if in_first {
        current
            .checked_add(300)
            .filter(|next| *next <= SplitRatio::MAX)
    } else {
        current
            .checked_sub(300)
            .filter(|next| *next >= SplitRatio::MIN)
    };
    if let Some(next) = next {
        *ratio = SplitRatio::new(next).expect("bounded resize ratios remain valid");
    } else {
        let pane_ids = tree.pane_ids();
        *tree = PaneTree::stack(pane_ids).expect("a split always contains at least two panes");
    }
    true
}

pub(crate) fn pane_rects(tree: &PaneTree) -> Vec<PaneRect> {
    pub(crate) fn visit(
        tree: &PaneTree,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        rects: &mut Vec<PaneRect>,
    ) {
        match tree {
            PaneTree::Leaf { pane_id } => rects.push(PaneRect {
                pane_id: *pane_id,
                x,
                y,
                width,
                height,
            }),
            PaneTree::Stack { pane_ids } => rects.extend(pane_ids.iter().map(|pane_id| PaneRect {
                pane_id: *pane_id,
                x,
                y,
                width,
                height,
            })),
            PaneTree::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let fraction = ratio.fraction();
                match axis {
                    SplitAxis::Horizontal => {
                        let first_width = width * fraction;
                        visit(first, x, y, first_width, height, rects);
                        visit(
                            second,
                            x + first_width,
                            y,
                            width - first_width,
                            height,
                            rects,
                        );
                    }
                    SplitAxis::Vertical => {
                        let first_height = height * fraction;
                        visit(first, x, y, width, first_height, rects);
                        visit(
                            second,
                            x,
                            y + first_height,
                            width,
                            height - first_height,
                            rects,
                        );
                    }
                }
            }
        }
    }

    let mut rects = Vec::new();
    visit(tree, 0.0, 0.0, 1.0, 1.0, &mut rects);
    rects
}

/// Zellij's undirected grow heuristic prefers a fully aligned neighbor above,
/// then below, left, and right. A pane merely touching part of an edge does not
/// claim that direction; the original deepest-split behavior remains the
/// fallback for irregular layouts.
pub(crate) fn zellij_resize_direction(rects: &[PaneRect], current: PaneId) -> Option<NavDirection> {
    [
        NavDirection::Up,
        NavDirection::Down,
        NavDirection::Left,
        NavDirection::Right,
    ]
    .into_iter()
    .find(|direction| has_aligned_direct_neighbors(rects, current, *direction))
}

pub(crate) fn has_aligned_direct_neighbors(
    rects: &[PaneRect],
    current: PaneId,
    direction: NavDirection,
) -> bool {
    const EPSILON: f32 = 1e-3;
    let Some(origin) = rects.iter().find(|rect| rect.pane_id == current) else {
        return false;
    };
    let (origin_start, origin_end) = match direction {
        NavDirection::Left | NavDirection::Right => (origin.y, origin.y + origin.height),
        NavDirection::Up | NavDirection::Down => (origin.x, origin.x + origin.width),
    };
    let mut spans: Vec<_> = rects
        .iter()
        .filter(|rect| rect.pane_id != current)
        .filter(|rect| match direction {
            NavDirection::Left => (rect.x + rect.width - origin.x).abs() <= EPSILON,
            NavDirection::Right => (rect.x - (origin.x + origin.width)).abs() <= EPSILON,
            NavDirection::Up => (rect.y + rect.height - origin.y).abs() <= EPSILON,
            NavDirection::Down => (rect.y - (origin.y + origin.height)).abs() <= EPSILON,
        })
        .filter_map(|rect| {
            let (start, end) = match direction {
                NavDirection::Left | NavDirection::Right => (rect.y, rect.y + rect.height),
                NavDirection::Up | NavDirection::Down => (rect.x, rect.x + rect.width),
            };
            (start >= origin_start - EPSILON && end <= origin_end + EPSILON).then_some((start, end))
        })
        .collect();
    spans.sort_by(|first, second| {
        first
            .0
            .partial_cmp(&second.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut covered_until = origin_start;
    for (start, end) in spans {
        if start > covered_until + EPSILON {
            return false;
        }
        covered_until = covered_until.max(end);
        if covered_until >= origin_end - EPSILON {
            return true;
        }
    }
    false
}

pub(crate) fn stacked_neighbor(
    tree: &PaneTree,
    current: PaneId,
    direction: NavDirection,
) -> Option<PaneId> {
    match tree {
        PaneTree::Leaf { .. } => None,
        PaneTree::Stack { pane_ids } => {
            let index = pane_ids.iter().position(|pane_id| *pane_id == current)?;
            match direction {
                NavDirection::Up if index > 0 => pane_ids.get(index - 1).copied(),
                NavDirection::Down => pane_ids.get(index + 1).copied(),
                NavDirection::Left | NavDirection::Right | NavDirection::Up => None,
            }
        }
        PaneTree::Split { first, second, .. } => stacked_neighbor(first, current, direction)
            .or_else(|| stacked_neighbor(second, current, direction)),
    }
}

/// The nearest pane strictly in `direction` from `current`, requiring overlap
/// on the orthogonal axis so diagonal panes are not surprising jumps.
pub(crate) fn neighbor_pane(
    rects: &[PaneRect],
    current: PaneId,
    direction: NavDirection,
) -> Option<PaneId> {
    const EPSILON: f32 = 1e-3;
    let origin = rects.iter().find(|rect| rect.pane_id == current)?;
    let candidates = rects.iter().filter(|rect| {
        let ahead = match direction {
            NavDirection::Right => rect.x >= origin.x + origin.width - EPSILON,
            NavDirection::Left => rect.x + rect.width <= origin.x + EPSILON,
            NavDirection::Down => rect.y >= origin.y + origin.height - EPSILON,
            NavDirection::Up => rect.y + rect.height <= origin.y + EPSILON,
        };
        let overlaps = match direction {
            NavDirection::Left | NavDirection::Right => {
                rect.y < origin.y + origin.height - EPSILON
                    && rect.y + rect.height > origin.y + EPSILON
            }
            NavDirection::Up | NavDirection::Down => {
                rect.x < origin.x + origin.width - EPSILON
                    && rect.x + rect.width > origin.x + EPSILON
            }
        };
        rect.pane_id != current && ahead && overlaps
    });
    candidates
        .min_by(|first, second| {
            let distance = |rect: &PaneRect| match direction {
                NavDirection::Right => rect.x - (origin.x + origin.width),
                NavDirection::Left => origin.x - (rect.x + rect.width),
                NavDirection::Down => rect.y - (origin.y + origin.height),
                NavDirection::Up => origin.y - (rect.y + rect.height),
            };
            let drift = |rect: &PaneRect| match direction {
                NavDirection::Left | NavDirection::Right => {
                    ((rect.y + rect.height / 2.0) - (origin.y + origin.height / 2.0)).abs()
                }
                NavDirection::Up | NavDirection::Down => {
                    ((rect.x + rect.width / 2.0) - (origin.x + origin.width / 2.0)).abs()
                }
            };
            (distance(first), drift(first))
                .partial_cmp(&(distance(second), drift(second)))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|rect| rect.pane_id)
}
