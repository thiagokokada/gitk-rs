//! Incremental graph geometry, bounded by the configured visible lane count.
use crate::git::{Commit, CommitId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: u16,
    pub to: u16,
    /// Other lanes span the row; parent edges start at its middle.
    pub parent: bool,
}

#[derive(Clone, Debug, Default)]
pub struct GraphRow {
    pub node: Option<u16>,
    pub incoming: bool,
    pub edges: Vec<Edge>,
    pub width: u16,
    pub overflow: bool,
}

pub struct GraphBuilder {
    lanes: Vec<CommitId>,
    limit: usize,
}

impl GraphBuilder {
    pub fn new(limit: usize) -> Self {
        Self {
            lanes: Vec::new(),
            limit: limit.clamp(1, 200),
        }
    }

    pub fn push(&mut self, commit: &Commit) -> GraphRow {
        let before = self.lanes.clone();
        let existing = self.lanes.iter().position(|id| id == &commit.id);
        let node = existing.or_else(|| {
            if self.lanes.len() < self.limit {
                self.lanes.push(commit.id.clone());
                Some(self.lanes.len() - 1)
            } else {
                None
            }
        });
        let mut row = GraphRow {
            node: node.map(|i| i as u16),
            incoming: existing.is_some(),
            width: self.lanes.len() as u16,
            overflow: node.is_none(),
            edges: Vec::new(),
        };
        if let Some(node) = node {
            self.lanes.remove(node);
            for (offset, parent) in commit.parents.iter().enumerate() {
                if !self.lanes.contains(parent) {
                    if self.lanes.len() < self.limit {
                        self.lanes
                            .insert((node + offset).min(self.lanes.len()), parent.clone());
                    } else {
                        row.overflow = true;
                    }
                }
            }
            for parent in &commit.parents {
                if let Some(to) = self.lanes.iter().position(|id| id == parent) {
                    row.edges.push(Edge {
                        from: node as u16,
                        to: to as u16,
                        parent: true,
                    });
                }
            }
        }
        for (from, id) in before.iter().enumerate() {
            if id == &commit.id {
                continue;
            }
            if let Some(to) = self.lanes.iter().position(|lane| lane == id) {
                row.edges.push(Edge {
                    from: from as u16,
                    to: to as u16,
                    parent: false,
                });
            }
        }
        row.width = row.width.max(self.lanes.len() as u16).max(1);
        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: u8, parents: &[u8]) -> Commit {
        let parents = parents
            .iter()
            .map(|id| format!("{id:040x}"))
            .collect::<Vec<_>>()
            .join(" ");
        crate::git::parse_commit(format!("{id:040x}\n{parents}\nA\na@b\n2026-01-01T00:00:00Z\nA\na@b\n2026-01-01T00:00:00Z\nMessage\n").as_bytes()).unwrap()
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
    }
}
