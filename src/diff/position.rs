//! Preserve a relative viewport position within a changed file.
use super::DiffSnapshot;
use std::{ops::Range, path::PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct DiffPosition {
    path: Option<PathBuf>,
    fraction: f64,
}

fn file_range(snapshot: &DiffSnapshot, section: Option<usize>) -> Range<usize> {
    let start = section
        .and_then(|i| snapshot.files[i].start_line)
        .unwrap_or(0);
    let end = section
        .and_then(|i| snapshot.files.get(i + 1))
        .and_then(|file| file.start_line)
        .unwrap_or(snapshot.count);
    start..end
}

impl DiffPosition {
    pub fn capture(snapshot: &DiffSnapshot, line: usize) -> Self {
        let section = snapshot
            .files
            .iter()
            .rposition(|file| file.start_line.is_some_and(|start| start <= line));
        let range = file_range(snapshot, section);
        let fraction = line.saturating_sub(range.start) as f64
            / range.end.saturating_sub(range.start + 1).max(1) as f64;
        Self {
            path: section.map(|i| snapshot.files[i].path.clone()),
            fraction: fraction.clamp(0., 1.),
        }
    }

    pub fn restore(&self, snapshot: &DiffSnapshot) -> usize {
        let section = self
            .path
            .as_ref()
            .and_then(|path| snapshot.files.iter().position(|file| &file.path == path));
        let range = file_range(snapshot, section);
        range.start
            + (range.end.saturating_sub(range.start + 1) as f64 * self.fraction).round() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileSection, SelectionId};
    use std::sync::Arc;

    fn snapshot(count: usize, files: &[(&str, Option<usize>)]) -> DiffSnapshot {
        DiffSnapshot {
            selection: SelectionId(1),
            request: 1,
            start: 0,
            lines: Vec::new(),
            count,
            last_line_len: 0,
            max_width: 0,
            files: Arc::new(
                files
                    .iter()
                    .map(|(path, start_line)| FileSection {
                        path: path.into(),
                        old_path: None,
                        added: None,
                        removed: None,
                        start_line: *start_line,
                    })
                    .collect(),
            ),
            added: 0,
            removed: 0,
            notice: None,
            error: None,
            highlighting: false,
        }
    }

    #[test]
    fn preserves_headers_and_file_boundaries_and_clamps_past_eof() {
        let snapshot = snapshot(30, &[("first", Some(10)), ("last", Some(20))]);
        for line in [0, 9, 10, 19, 20, 29, 99] {
            assert_eq!(
                DiffPosition::capture(&snapshot, line).restore(&snapshot),
                line.min(29)
            );
        }
    }

    #[test]
    fn follows_the_file_when_its_size_and_order_change() {
        let before = snapshot(32, &[("first", Some(10)), ("last", Some(21))]);
        let position = DiffPosition::capture(&before, 15);
        let after = snapshot(41, &[("last", Some(10)), ("first", Some(20))]);
        assert_eq!(position.restore(&after), 30);
        let removed = snapshot(31, &[("last", Some(10))]);
        assert_eq!(position.restore(&removed), 15);
    }

    #[test]
    fn empty_single_line_and_unavailable_file_offsets_are_supported() {
        for snapshot in [
            snapshot(0, &[]),
            snapshot(1, &[]),
            snapshot(1, &[("binary", None)]),
        ] {
            assert_eq!(DiffPosition::capture(&snapshot, 0).restore(&snapshot), 0);
        }
        let before = snapshot(
            30,
            &[("first", Some(10)), ("binary", None), ("last", Some(20))],
        );
        let position = DiffPosition::capture(&before, 19);
        assert_eq!(position.restore(&before), 19);
        let after = snapshot(40, &[("first", Some(20)), ("binary", None)]);
        assert_eq!(position.restore(&after), 29);
    }
}
