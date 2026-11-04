// Frozen pre-optimisation algorithm: an independent oracle for logical rows.
use super::{Edge, GraphRow, Lane};
use crate::git::{Commit, CommitId};

pub struct ReferenceBuilder {
    // Reuse holes instead of shifting every unrelated branch on each row.
    lanes: Vec<Option<Lane>>,
    limit: usize,
    next_color: u8,
}
impl ReferenceBuilder {
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
        let before = self.lanes.clone();
        let existing = self.find(&commit.id);
        let mut overflow = false;
        let node = existing.unwrap_or_else(|| {
            let node = self
                .lanes
                .iter()
                .position(Option::is_none)
                .unwrap_or(self.lanes.len());
            let node = if node >= self.limit {
                overflow = true;
                self.limit - 1
            } else {
                if node == self.lanes.len() {
                    self.lanes.push(None);
                }
                node
            };
            let color = self.color();
            self.lanes[node] = Some(Lane {
                target: commit.id.clone(),
                color,
            });
            node
        });
        let color = self.lanes[node].take().unwrap().color;
        let mut row = GraphRow {
            node: Some(node as u16),
            incoming: existing.is_some(),
            color,
            width: before.len().max(self.lanes.len()).max(1) as u16,
            overflow,
            edges: Box::default(),
        };
        let mut edges = Vec::new();
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
        for (from, lane) in before.iter().enumerate() {
            let Some(lane) = lane else {
                continue;
            };
            if lane.target != commit.id
                && let Some(to) = self.find(&lane.target)
            {
                edges.push(Edge {
                    from: from as u16,
                    to: to as u16,
                    parent: false,
                    color: lane.color,
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
