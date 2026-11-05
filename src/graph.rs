//! Incremental graph geometry, bounded by the configured visible lane count.
use crate::git::{Commit, CommitId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: u16,
    pub to: u16,
    /// Other lanes span the row; parent edges start at its middle.
    pub parent: bool,
    pub color: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphRow {
    pub node: Option<u16>,
    pub incoming: bool,
    pub color: u8,
    pub edges: Box<[Edge]>,
    pub width: u16,
    pub overflow: bool,
}

/// Fit the visible frontier inside its column. One shared spacing must be used
/// for every visible row so edges meet across row boundaries.
pub fn lane_spacing(available_width: f32, lanes: u16) -> f32 {
    ((available_width - 18.).max(0.) / lanes.max(1) as f32).min(12.)
}

/// A bounded text fallback centered on the current node, with explicit markers
/// for lanes outside the column or beyond the graph frontier cap.
pub fn text_graph(row: &GraphRow, limit: usize) -> String {
    let count = row.width as usize;
    let limit = limit.clamp(1, 32);
    let node = row.node.map_or(0, usize::from);
    let start = node
        .saturating_sub(limit / 2)
        .min(count.saturating_sub(limit));
    let end = (start + limit).min(count);
    let mut text = if start > 0 {
        "… ".to_owned()
    } else {
        String::new()
    };
    for lane in start..end {
        text.push_str(if row.node == Some(lane as u16) {
            "● "
        } else {
            "│ "
        });
    }
    if end < count || row.overflow {
        text.push('…');
    }
    text
}

/// One shared layout for visible rows. Runs of idle, occupied lanes become a
/// single dashed continuation column; every node and junction remains distinct.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphLayout {
    columns: Vec<u16>,
    collapsed: Vec<bool>,
    pub spacing: f32,
}
impl GraphLayout {
    pub fn new<'a>(rows: impl IntoIterator<Item = &'a GraphRow>, width: f32) -> Self {
        let mut occupied = [false; 200];
        let mut active = [false; 200];
        for row in rows {
            if let Some(node) = row.node {
                occupied[node as usize] = true;
                active[node as usize] = true;
            }
            for edge in &row.edges {
                occupied[edge.from as usize] = true;
                occupied[edge.to as usize] = true;
                if edge.parent || edge.from != edge.to {
                    active[edge.from as usize] = true;
                    active[edge.to as usize] = true;
                }
            }
        }
        let mut columns = vec![0; 200];
        let mut collapsed = Vec::new();
        let mut in_run = false;
        for lane in 0..200 {
            if !occupied[lane] {
                continue;
            }
            if active[lane] || !in_run {
                collapsed.push(!active[lane]);
            }
            columns[lane] = (collapsed.len() - 1) as u16;
            in_run = !active[lane];
        }
        Self {
            spacing: lane_spacing(width, collapsed.len().max(1) as u16),
            columns,
            collapsed,
        }
    }
    pub fn column(&self, lane: u16) -> usize {
        self.columns.get(lane as usize).copied().unwrap_or(0) as usize
    }
    pub fn x(&self, lane: u16) -> f32 {
        9. + self.column(lane) as f32 * self.spacing
    }
    pub fn is_collapsed(&self, lane: u16) -> bool {
        self.collapsed
            .get(self.column(lane))
            .copied()
            .unwrap_or(false)
    }
    pub fn has_collapsed(&self) -> bool {
        self.collapsed.iter().any(|value| *value)
    }
    pub fn width(&self) -> f32 {
        self.collapsed.len() as f32 * self.spacing + 18.
    }
}

struct Lane {
    target: CommitId,
    color: u8,
}

pub struct GraphBuilder {
    // Reuse holes instead of shifting every unrelated branch on each row.
    lanes: Vec<Option<Lane>>,
    limit: usize,
    next_color: u8,
}
impl GraphBuilder {
    pub fn new(limit: usize) -> Self {
        Self {
            lanes: Vec::new(),
            limit: limit.clamp(1, 200),
            next_color: 0,
        }
    }
    fn color(&mut self) -> u8 {
        let color = self.next_color;
        self.next_color = (color + 1) % 6;
        color
    }
    fn find(&self, target: &CommitId) -> Option<usize> {
        self.lanes
            .iter()
            .position(|lane| lane.as_ref().is_some_and(|lane| &lane.target == target))
    }
    pub fn push(&mut self, commit: &Commit) -> GraphRow {
        let before_width = self.lanes.len();
        // Lanes never shift. Remember only colors for unchanged continuation
        // edges; moving the replaced node below preserves its one displaced ID.
        let mut before = [None; 200];
        let mut occupied = 0;
        for (color, lane) in before.iter_mut().zip(&self.lanes) {
            *color = lane.as_ref().map(|lane| lane.color);
            occupied += usize::from(color.is_some());
        }
        let existing = self.find(&commit.id);
        let mut overflow = false;
        let node = existing.unwrap_or_else(|| {
            let slot = self
                .lanes
                .iter()
                .position(Option::is_none)
                .unwrap_or(self.lanes.len());
            if slot >= self.limit {
                overflow = true;
                self.limit - 1
            } else {
                if slot == self.lanes.len() {
                    self.lanes.push(None);
                }
                slot
            }
        });
        let displaced = self.lanes[node].take();
        let color = if existing.is_some() {
            displaced.as_ref().unwrap().color
        } else {
            self.color()
        };
        let displaced = if existing.is_none() { displaced } else { None };
        let mut row = GraphRow {
            node: Some(node as u16),
            incoming: existing.is_some(),
            color,
            width: before_width.max(self.lanes.len()).max(1) as u16,
            overflow,
            edges: Box::default(),
        };
        let mut edges = Vec::with_capacity(occupied + commit.parents.len());
        for (index, parent) in commit.parents.iter().enumerate() {
            let to = if let Some(to) = self.find(parent) {
                Some(to)
            } else {
                let slot = if index == 0 {
                    Some(node)
                } else {
                    self.lanes.iter().position(Option::is_none).or_else(|| {
                        if self.lanes.len() < self.limit {
                            self.lanes.push(None);
                            Some(self.lanes.len() - 1)
                        } else {
                            None
                        }
                    })
                };
                slot.inspect(|&slot| {
                    let color = if index == 0 { color } else { self.color() };
                    self.lanes[slot] = Some(Lane {
                        target: parent.clone(),
                        color,
                    });
                })
            };
            if let Some(to) = to {
                edges.push(Edge {
                    from: node as u16,
                    to: to as u16,
                    parent: true,
                    color: self.lanes[to].as_ref().unwrap().color,
                });
            } else {
                row.overflow = true;
            }
        }
        for (from, color) in before[..before_width].iter().enumerate() {
            let Some(color) = color else { continue };
            let to = if from != node {
                // Only the current node can replace an occupied lane. Every
                // other target and its color are still at their original index.
                Some(from)
            } else {
                // At the lane cap a disconnected commit may evict a target,
                // then immediately reintroduce it as a parent. Preserve the
                // original continuation edge and color in that case too.
                displaced.as_ref().and_then(|lane| self.find(&lane.target))
            };
            if let Some(to) = to {
                edges.push(Edge {
                    from: from as u16,
                    to: to as u16,
                    parent: false,
                    color: *color,
                });
            }
        }
        row.edges = edges.into_boxed_slice();
        while self.lanes.last().is_some_and(Option::is_none) {
            self.lanes.pop();
        }
        row.width = row.width.max(self.lanes.len() as u16);
        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: u64, parents: &[u64]) -> Commit {
        let parents = parents
            .iter()
            .map(|id| format!("{id:040x}"))
            .collect::<Vec<_>>()
            .join(" ");
        crate::git::parse_commit(format!("{id:040x}\n{parents}\nA\na@b\n2026-01-01T00:00:00Z\nA\na@b\n2026-01-01T00:00:00Z\nMessage\n").as_bytes()).unwrap()
    }

    #[test]
    fn generated_dags_keep_geometry_within_the_frontier() {
        for cap in [1, 2, 3, 8, 32, 200] {
            let mut graph = GraphBuilder::new(cap);
            let mut state = 0x23456789_u64;
            for id in (1..=512_u64).rev() {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let parents = (0..(state >> 60).min(8))
                    .filter_map(|index| {
                        id.checked_sub(1 + (state.rotate_right(index as u32 * 7) % 31))
                    })
                    .map(|parent| format!("{parent:040x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                let commit = crate::git::parse_commit(format!("{id:040x}\n{parents}\nA\na@b\n2026-01-01T00:00:00Z\nA\na@b\n2026-01-01T00:00:00Z\nMessage\n").as_bytes()).unwrap();
                let actual = graph.push(&commit);
                assert!(actual.node.is_some_and(|node| usize::from(node) < cap));
                assert!(usize::from(actual.width) <= cap);
                assert!(actual.edges.len() <= commit.parents.len() + cap);
                assert!(actual.edges.iter().all(|edge| usize::from(edge.from) < cap
                    && usize::from(edge.to) < cap
                    && edge.color < 6));
                assert!(
                    actual
                        .edges
                        .iter()
                        .filter(|edge| edge.parent)
                        .all(|edge| Some(edge.from) == actual.node)
                );
                assert!(graph.lanes.len() <= cap);
            }
        }
    }

    #[test]
    fn full_width_octopus_merges_and_sparse_roots_respect_the_cap() {
        for cap in [1, 2, 8, 32, 200] {
            let mut builder = GraphBuilder::new(cap);
            let merge = commit(1000, &(1..=256).collect::<Vec<_>>());
            let merged = builder.push(&merge);
            assert_eq!(merged.edges.iter().filter(|edge| edge.parent).count(), cap);
            assert!(merged.overflow);
            assert_eq!(usize::from(merged.width), cap);
            for id in (1..=256).rev() {
                let parents = if id % 3 == 0 { vec![] } else { vec![id / 2] };
                let commit = commit(id, &parents);
                let row = builder.push(&commit);
                assert!(row.node.is_some());
                assert!(
                    row.edges
                        .iter()
                        .all(|edge| usize::from(edge.from) < cap && usize::from(edge.to) < cap)
                );
                assert!(builder.lanes.len() <= cap);
            }
            builder.push(&commit(0, &[]));
            assert!(builder.lanes.is_empty());
        }
    }

    #[test]
    fn overflow_can_reintroduce_the_displaced_target_with_its_old_color() {
        let mut builder = GraphBuilder::new(2);
        builder.push(&commit(10, &[9, 8]));
        let row = builder.push(&commit(7, &[8, 6]));
        assert!(row.overflow);
        assert!(!row.incoming);
        assert_eq!(row.node, Some(1));
        assert_eq!(row.color, 2);
        assert_eq!(
            row.edges.as_ref(),
            &[
                Edge {
                    from: 1,
                    to: 1,
                    parent: true,
                    color: 2
                },
                Edge {
                    from: 0,
                    to: 0,
                    parent: false,
                    color: 0
                },
                Edge {
                    from: 1,
                    to: 1,
                    parent: false,
                    color: 1
                },
            ]
        );
        let restored = builder.push(&commit(8, &[5]));
        assert!(restored.incoming);
        assert_eq!((restored.node, restored.color), (Some(1), 2));
        let joined = builder.push(&commit(9, &[5]));
        assert!(
            joined
                .edges
                .iter()
                .any(|edge| edge.parent && edge.from == 0 && edge.to == 1 && edge.color == 2)
        );
    }

    #[test]
    fn chain_ends_at_the_root() {
        let mut builder = GraphBuilder::new(200);
        assert!(!builder.push(&commit(3, &[2])).incoming);
        let second = builder.push(&commit(2, &[1]));
        assert!(second.incoming);
        assert_eq!(second.node, Some(0));
        assert!(builder.push(&commit(1, &[])).edges.is_empty());
        assert!(builder.lanes.is_empty());
    }

    #[test]
    fn merge_routes_parents_and_deduplicates_shared_ancestors() {
        let mut builder = GraphBuilder::new(200);
        let merge = builder.push(&commit(4, &[3, 2]));
        assert_eq!(merge.edges.iter().filter(|e| e.parent).count(), 2);
        builder.push(&commit(3, &[1]));
        let branch = builder.push(&commit(2, &[1]));
        assert_eq!(branch.edges.iter().filter(|e| e.parent).count(), 1);
        assert_eq!(builder.lanes.len(), 1);
        builder.push(&commit(1, &[]));
        assert!(builder.lanes.is_empty());
    }

    #[test]
    fn wide_merges_report_overflow_and_bound_frontier() {
        let mut builder = GraphBuilder::new(2);
        let row = builder.push(&commit(9, &[1, 2, 3, 4]));
        assert!(row.overflow);
        assert!(builder.lanes.len() <= 2);
        assert!(row.edges.iter().all(|e| e.from < 2 && e.to < 2));
        let disconnected = builder.push(&commit(8, &[7]));
        assert!(disconnected.node.is_some());
        assert!(disconnected.overflow);
    }

    #[test]
    fn unrelated_lanes_keep_positions_and_colors_when_branches_join() {
        let mut builder = GraphBuilder::new(200);
        builder.push(&commit(10, &[9, 8, 7]));
        let first = builder.push(&commit(9, &[6]));
        let join = builder.push(&commit(8, &[6]));
        assert_eq!(join.node, Some(1));
        assert!(
            join.edges
                .iter()
                .any(|e| e.parent && e.from == 1 && e.to == 0)
        );
        let side = join
            .edges
            .iter()
            .find(|e| !e.parent && e.from == 2)
            .unwrap();
        assert_eq!(side.to, 2);
        let continued = builder.push(&commit(7, &[5]));
        assert_eq!(continued.node, Some(2));
        assert_eq!(continued.color, side.color);
        let shared = continued
            .edges
            .iter()
            .find(|e| e.from == 0 && !e.parent)
            .unwrap();
        assert_eq!(shared.color, first.color);
        builder.push(&commit(11, &[4]));
        assert_eq!(builder.find(&commit(6, &[]).id), Some(0));
        assert_eq!(builder.find(&commit(4, &[]).id), Some(1));
    }

    #[test]
    fn folded_context_preserves_distinct_nodes_and_merge_endpoints() {
        let mut first = GraphRow {
            node: Some(150),
            width: 200,
            ..GraphRow::default()
        };
        first.edges = (0..200)
            .map(|lane| Edge {
                from: lane,
                to: lane,
                parent: false,
                color: 0,
            })
            .collect();
        let next = GraphRow {
            node: Some(151),
            width: 200,
            edges: vec![Edge {
                from: 151,
                to: 196,
                parent: true,
                color: 1,
            }]
            .into_boxed_slice(),
            ..GraphRow::default()
        };
        let layout = GraphLayout::new([&first, &next], 280.);
        assert!(layout.has_collapsed());
        assert_eq!(layout.spacing, 12.);
        assert!(layout.width() < 100.);
        assert!(layout.x(150) < layout.x(151) && layout.x(151) < layout.x(196));
        for lane in [150, 151, 196] {
            assert!(!layout.is_collapsed(lane));
        }
        assert!(layout.is_collapsed(0) && layout.is_collapsed(199));
    }
    #[test]
    fn wide_frontiers_fit_and_small_frontiers_keep_normal_spacing() {
        assert_eq!(lane_spacing(280., 3), 12.);
        for lanes in [24, 100, 200] {
            let spacing = lane_spacing(280., lanes);
            assert!(9. + (lanes - 1) as f32 * spacing + 3.5 < 280.);
        }
    }
    #[test]
    fn text_fallback_keeps_wide_nodes_and_continuations_visible() {
        let mut row = GraphRow {
            node: Some(150),
            width: 200,
            ..GraphRow::default()
        };
        let text = text_graph(&row, 12);
        assert!(text.contains('●'));
        assert!(text.starts_with('…') && text.ends_with('…'));
        assert!(text.chars().count() <= 28);
        row.width = 2;
        row.node = Some(1);
        row.overflow = true;
        assert_eq!(text_graph(&row, 12), "│ ● …");
    }
}
