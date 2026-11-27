//! Index edits use the exact displayed diff, never a worktree rewrite.
use super::*;
use std::ffi::OsString;

#[derive(Clone, Debug)]
pub enum IndexTarget {
    File(usize),
    Lines(Range<usize>),
    /// Any displayed row in the hunk, including its header.
    Hunk(usize),
}

pub(super) fn edit(
    document: &mut DiffDocument,
    selection: &DiffSelection,
    target: IndexTarget,
    cancel: &Cancellation,
) -> Result<()> {
    let DiffSource::Local { staged } = selection.source else {
        bail!("Only local changes can be staged or unstaged");
    };
    if let IndexTarget::File(index) = target {
        let file = document
            .files
            .get(index)
            .context("File is no longer available")?;
        let mut args: Vec<OsString> = if staged {
            let head = crate::git::run_bytes(
                &document.worktree_root,
                &["rev-parse", "--verify", "HEAD"],
                cancel,
                &[0, 128],
            )?;
            if head.is_empty() {
                vec!["rm".into(), "--cached".into(), "-f".into()]
            } else {
                vec!["reset".into(), "-q".into(), "HEAD".into()]
            }
        } else {
            vec!["add".into(), "--all".into()]
        };
        args.insert(0, "--literal-pathspecs".into());
        args.push("--".into());
        args.push(file.path.as_os_str().to_owned());
        if let Some(old) = &file.old_path {
            args.push(old.as_os_str().to_owned());
        }
        crate::git::run_bytes(&document.worktree_root, &args, cancel, &[0])?;
        return Ok(());
    }
    ensure!(
        selection.options.version == DiffVersion::Diff
            && selection.options.words == WordDiff::Lines
            && !selection.options.ignore_space,
        "Use Diff, Line diff, and disable Ignore space change to select lines"
    );
    ensure!(
        document.notice.is_none(),
        "Reload a complete diff before selecting lines"
    );
    // Reconcile external edits before mapping screen rows to patch lines.
    ensure!(
        crate::git::run_bytes(
            &document.worktree_root,
            &["ls-files", "--unmerged"],
            cancel,
            &[0]
        )?
        .is_empty(),
        "Resolve conflicts and stage whole files before selecting lines"
    );
    let mut fresh = DiffDocument::load(selection, cancel)?;
    ensure!(
        fresh.index.len() == document.index.len(),
        "Diff is out of date; reload and select the lines again"
    );
    ensure!(
        fresh.notice.is_none(),
        "Reload a complete diff before selecting lines"
    );
    for i in 0..document.index.len() {
        ensure!(!cancel.load(Ordering::Acquire), "Index edit cancelled");
        ensure!(
            fresh.line(i)? == document.line(i)?,
            "Diff is out of date; reload and select the lines again"
        );
    }
    let range = match target {
        IndexTarget::Lines(range) => range,
        IndexTarget::Hunk(row) => {
            let entry = document
                .index
                .get(row)
                .context("Hunk is no longer available")?;
            ensure!(
                entry.hunk > 0 && entry.file.is_some(),
                "Right-click inside a text hunk"
            );
            let start = entry.hunk;
            let file = entry.file;
            let end = start
                + document.index[start..]
                    .partition_point(|entry| entry.file == file && entry.hunk == start);
            start..end
        }
        IndexTarget::File(_) => unreachable!(),
    };
    let mut patch = String::new();
    for file_index in 0..document.files.len() {
        let Some(start) = document.files[file_index].start_line else {
            continue;
        };
        let end =
            start + document.index[start..].partition_point(|entry| entry.file == Some(file_index));
        let rows = start..end;
        if !rows.clone().any(|i| {
            range.contains(&i)
                && matches!(document.index[i].kind, LineKind::Added | LineKind::Removed)
        }) {
            continue;
        }
        ensure!(
            document.files[file_index].old_path.is_none(),
            "Stage or unstage renamed files as a whole"
        );
        let mut lines = Vec::new();
        let mut bytes = 0;
        for row in rows.clone() {
            let line = document.line(row)?;
            bytes += line.len() + 1;
            ensure!(
                bytes <= MAX_COPY_BYTES,
                "File patch exceeds 16 MiB; stage or unstage the file as a whole"
            );
            lines.push(line);
        }
        ensure!(
            !lines.iter().any(|s| s.contains('\u{fffd}')
                || ((s.starts_with("old mode ")
                    || s.starts_with("new mode ")
                    || s.starts_with("new file mode ")
                    || s.starts_with("deleted file mode ")
                    || s.starts_with("index "))
                    && (s.ends_with("120000") || s.ends_with("160000")))),
            "Stage or unstage this file as a whole"
        );
        let old = lines
            .iter()
            .find_map(|l| l.strip_prefix("--- "))
            .context("No text patch available")?;
        let new = lines
            .iter()
            .find_map(|l| l.strip_prefix("+++ "))
            .context("No text patch available")?;
        let (base, destination) = if staged { (new, old) } else { (old, new) };
        // A partial deletion leaves a tracked file; file actions remove it entirely.
        let all_changes = rows
            .clone()
            .filter(|i| matches!(document.index[*i].kind, LineKind::Added | LineKind::Removed))
            .all(|i| range.contains(&i));
        let destination = if destination == "/dev/null" && !all_changes {
            base.replacen("a/", "b/", 1)
        } else {
            destination.to_owned()
        };
        patch.push_str(&format!("--- {base}\n+++ {destination}\n"));
        let mut offset = 0isize;
        let mut at = 0;
        while at < lines.len() {
            if !lines[at].starts_with("@@ ") {
                at += 1;
                continue;
            }
            let fields: Vec<_> = lines[at].split_whitespace().collect();
            let field = fields[if staged { 2 } else { 1 }];
            let start: usize = field[1..]
                .split(',')
                .next()
                .context("Invalid hunk")?
                .parse()?;
            at += 1;
            let mut body = String::new();
            let (mut before, mut after) = (0usize, 0usize);
            let mut changed = false;
            let mut retained = false;
            while at < lines.len() && !lines[at].starts_with("@@ ") {
                let text = &lines[at];
                let marker = match text.as_bytes().first() {
                    Some(b'+') => {
                        if staged {
                            b'-'
                        } else {
                            b'+'
                        }
                    }
                    Some(b'-') => {
                        if staged {
                            b'+'
                        } else {
                            b'-'
                        }
                    }
                    Some(b' ') => b' ',
                    Some(b'\\') => {
                        if retained {
                            body.push_str(text);
                            body.push('\n');
                        }
                        at += 1;
                        continue;
                    }
                    _ => bail!("Unsupported patch; stage or unstage the file as a whole"),
                };
                let selected = range.contains(&(rows.start + at));
                let marker = match (marker, selected) {
                    (b'+', false) => None,
                    (b'-', false) => Some(b' '),
                    (marker, _) => Some(marker),
                };
                retained = marker.is_some();
                if let Some(marker) = marker {
                    before += usize::from(marker != b'+');
                    after += usize::from(marker != b'-');
                    changed |= marker != b' ';
                    body.push(marker as char);
                    body.push_str(&text[1..]);
                    body.push('\n');
                }
                at += 1;
            }
            if changed {
                let base_start = if before == 0 { start } else { start.max(1) };
                let new_start = (base_start as isize + offset + isize::from(before == 0)
                    - isize::from(after == 0))
                .max(0) as usize;
                patch.push_str(&format!(
                    "@@ -{base_start},{before} +{new_start},{after} @@\n{body}"
                ));
                offset += after as isize - before as isize;
            }
        }
        ensure!(
            patch.len() <= MAX_COPY_BYTES,
            "Selected patch exceeds 16 MiB"
        );
    }
    ensure!(!patch.is_empty(), "Select added or removed lines first");
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(patch.as_bytes())?;
    crate::git::run_bytes(
        &document.worktree_root,
        &[
            OsString::from("apply"),
            "--cached".into(),
            "--unidiff-zero".into(),
            "--whitespace=nowarn".into(),
            "--".into(),
            file.path().as_os_str().to_owned(),
        ],
        cancel,
        &[0],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Fixture;
    use std::fs;

    fn selection(fixture: &Fixture, staged: bool) -> DiffSelection {
        DiffSelection {
            repository: fixture.root().into(),
            id: SelectionId(1),
            source: DiffSource::Local { staged },
            options: DiffOptions::default(),
        }
    }

    fn lines(fixture: &Fixture, staged: bool, wanted: &[&str]) {
        let cancel = Cancellation::default();
        let selection = selection(fixture, staged);
        let mut document = DiffDocument::load(&selection, &cancel).unwrap();
        let rows: Vec<_> = (0..document.index.len())
            .filter(|i| wanted.contains(&document.line(*i).unwrap().as_str()))
            .collect();
        assert_eq!(rows.len(), wanted.len());
        let range = *rows.first().unwrap()..rows.last().unwrap() + 1;
        edit(
            &mut document,
            &selection,
            IndexTarget::Lines(range),
            &cancel,
        )
        .unwrap();
    }

    #[test]
    fn partial_replacement_round_trip_preserves_worktree_and_other_changes() {
        let fixture = Fixture::new();
        fixture.commit("base");
        fs::write(fixture.root().join("file.txt"), "one\ntwo\nthree\n").unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", "lines"]);
        let worktree = "ONE\ntwo\nTHREE\n";
        fs::write(fixture.root().join("file.txt"), worktree).unwrap();
        lines(&fixture, false, &["-one", "+ONE"]);
        assert_eq!(fixture.git(&["show", ":file.txt"]), "ONE\ntwo\nthree");
        lines(&fixture, true, &["-one", "+ONE"]);
        assert_eq!(fixture.git(&["show", ":file.txt"]), "one\ntwo\nthree");
        assert_eq!(
            fs::read_to_string(fixture.root().join("file.txt")).unwrap(),
            worktree
        );
    }

    #[test]
    fn added_and_removed_sides_can_be_selected_independently() {
        let fixture = Fixture::new();
        fixture.commit("old\n");
        fs::write(fixture.root().join("file.txt"), "new\n").unwrap();
        lines(&fixture, false, &["+new"]);
        assert_eq!(fixture.git(&["show", ":file.txt"]), "old\nnew");
        lines(&fixture, false, &["-old"]);
        assert_eq!(fixture.git(&["show", ":file.txt"]), "new");
        lines(&fixture, true, &["+new"]);
        assert_eq!(fixture.git(&["show", ":file.txt"]), "");
    }

    #[test]
    fn untracked_lines_and_unborn_file_actions_preserve_disk_contents() {
        let fixture = Fixture::new();
        let path = fixture.root().join("new file.txt");
        fs::write(&path, "first\nsecond\n").unwrap();
        lines(&fixture, false, &["+first"]);
        assert_eq!(fixture.git(&["show", ":new file.txt"]), "first");
        let cancel = Cancellation::default();
        let selected = selection(&fixture, true);
        let mut document = DiffDocument::load(&selected, &cancel).unwrap();
        edit(&mut document, &selected, IndexTarget::File(0), &cancel).unwrap();
        assert_eq!(fixture.git(&["ls-files"]), "");
        assert_eq!(fs::read_to_string(path).unwrap(), "first\nsecond\n");
    }

    #[test]
    fn no_newline_replacement_and_full_new_file_unstage() {
        let fixture = Fixture::new();
        fixture.commit("old");
        fs::write(fixture.root().join("file.txt"), "new").unwrap();
        lines(&fixture, false, &["-old", "+new"]);
        assert_eq!(fixture.git(&["show", ":file.txt"]), "new");
        lines(&fixture, true, &["-old", "+new"]);
        assert_eq!(fixture.git(&["show", ":file.txt"]), "old");
        fs::write(fixture.root().join("new.txt"), "first\nsecond\n").unwrap();
        lines(&fixture, false, &["+first", "+second"]);
        lines(&fixture, true, &["+first", "+second"]);
        assert!(!fixture.git(&["ls-files"]).contains("new.txt"));
    }

    #[test]
    fn selection_spanning_multiple_hunks_uses_index_coordinates() {
        let fixture = Fixture::new();
        let original: String = (0..50).map(|i| format!("line {i}\n")).collect();
        fixture.commit(&original);
        let changed = original
            .replace("line 2\n", "extra\nline 2\n")
            .replace("line 40\n", "changed\n");
        fs::write(fixture.root().join("file.txt"), &changed).unwrap();
        lines(&fixture, false, &["+extra", "-line 40", "+changed"]);
        assert_eq!(fixture.git(&["show", ":file.txt"]), changed.trim_end());
        lines(&fixture, true, &["+extra", "-line 40", "+changed"]);
        assert_eq!(fixture.git(&["show", ":file.txt"]), original.trim_end());
    }

    #[test]
    fn whole_file_actions_handle_literal_paths_and_binary_contents() {
        let fixture = Fixture::new();
        fixture.commit("base");
        fs::write(fixture.root().join("[ab].bin"), [0, 1, 2]).unwrap();
        fs::write(fixture.root().join("a.bin"), [0, 3]).unwrap();
        let cancel = Cancellation::default();
        let selected = selection(&fixture, false);
        let mut document = DiffDocument::load(&selected, &cancel).unwrap();
        let index = document
            .files
            .iter()
            .position(|f| f.path == std::path::Path::new("[ab].bin"))
            .unwrap();
        edit(&mut document, &selected, IndexTarget::File(index), &cancel).unwrap();
        assert!(
            fixture
                .git(&["diff", "--cached", "--name-only"])
                .contains("[ab].bin")
        );
        assert_eq!(fixture.git(&["ls-files", "a.bin"]), "");
        let selected = selection(&fixture, true);
        let mut document = DiffDocument::load(&selected, &cancel).unwrap();
        edit(&mut document, &selected, IndexTarget::File(0), &cancel).unwrap();
        assert_eq!(
            fs::read(fixture.root().join("[ab].bin")).unwrap(),
            [0, 1, 2]
        );
    }

    #[test]
    fn stale_lines_are_rejected() {
        let fixture = Fixture::new();
        fixture.commit("old\n");
        fs::write(fixture.root().join("file.txt"), "new\n").unwrap();
        let cancel = Cancellation::default();
        let selected = selection(&fixture, false);
        let mut document = DiffDocument::load(&selected, &cancel).unwrap();
        fs::write(fixture.root().join("file.txt"), "external\n").unwrap();
        assert!(
            edit(
                &mut document,
                &selected,
                IndexTarget::Lines(0..100),
                &cancel
            )
            .is_err()
        );
        assert_eq!(fixture.git(&["show", ":file.txt"]), "old");
    }
}
