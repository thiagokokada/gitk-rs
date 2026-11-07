use super::{
    DiffDocument, DiffSnapshot, SelectionId,
    highlight::HighlightCache,
    tests::{page, selected},
};
use crate::{git::Cancellation, test_support::Fixture};
use std::{cell::Cell, fs};

fn document() -> (Fixture, DiffDocument) {
    let fixture = Fixture::new();
    for (word, number) in [("before", 1), ("after", 2)] {
        let mut source = format!("/* {word} λ\n");
        for line in 0..400 {
            source.push_str(&format!("{word} comment {line} λ\n"));
        }
        source.push_str(&format!(
            "*/\nlet text = r#\"λ\n/* still a string */\n\"#;\nlet number = {number};\n"
        ));
        fs::write(fixture.root().join("one.rs"), source).unwrap();
        fs::write(
            fixture.root().join("two.json"),
            format!("{{\n  \"value\": {number}\n}}\n"),
        )
        .unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", word]);
    }
    let document = DiffDocument::load(&selected(&fixture, 1), &Cancellation::default()).unwrap();
    (fixture, document)
}

// A fresh parser replays the hunk; the warmed path must produce identical tokens.
fn assert_fresh_page(document: &mut DiffDocument, start: usize, dark: bool) -> DiffSnapshot {
    assert_fresh_range(document, start..start + 128, dark)
}

fn assert_fresh_range(
    document: &mut DiffDocument,
    range: std::ops::Range<usize>,
    dark: bool,
) -> DiffSnapshot {
    let cache = document.highlight_cache.take();
    let mut request = page(1);
    request.range = range;
    request.settings.dark = dark;
    let mut expected = document.page(&request).unwrap();
    assert!(
        document
            .highlight(&mut expected, request.settings, || false)
            .unwrap()
    );
    document.highlight_cache = cache;
    let mut actual = document.page(&request).unwrap();
    assert!(
        document
            .highlight(&mut actual, request.settings, || false)
            .unwrap()
    );
    for (actual, expected) in actual.lines.iter().zip(&expected.lines) {
        assert_eq!(actual.text, expected.text);
        assert_eq!(actual.kind, expected.kind);
        assert_eq!(actual.spans, expected.spans);
        for span in &actual.spans {
            assert!(actual.text.is_char_boundary(span.range.start));
            assert!(actual.text.is_char_boundary(span.range.end));
        }
    }
    actual
}

#[test]
fn consecutive_and_reverse_pages_preserve_multiline_state_without_replaying_prefixes() {
    let (_fixture, mut document) = document();
    assert_fresh_page(&mut document, 100, true);
    let first = document.highlight_cache.as_ref().unwrap().parsed_lines;
    assert_fresh_page(&mut document, 228, true);
    let next = document.highlight_cache.as_ref().unwrap().parsed_lines;
    assert_eq!(next - first, 128);
    assert_fresh_page(&mut document, 100, true);
    assert_eq!(
        document.highlight_cache.as_ref().unwrap().parsed_lines,
        next
    );
    assert_fresh_page(&mut document, 350, true);
    assert_fresh_page(&mut document, 550, true);
    assert_fresh_page(&mut document, 750, true);
}

#[test]
fn theme_changes_and_file_boundaries_cannot_reuse_old_tokens() {
    let (_fixture, mut document) = document();
    let dark = assert_fresh_page(&mut document, 200, true);
    let light = assert_fresh_page(&mut document, 200, false);
    assert_ne!(
        dark.lines
            .iter()
            .map(|line| &line.spans)
            .collect::<Vec<_>>(),
        light
            .lines
            .iter()
            .map(|line| &line.spans)
            .collect::<Vec<_>>()
    );
    let second_file = document
        .index
        .iter()
        .position(|line| line.file == Some(1))
        .unwrap();
    assert_fresh_page(&mut document, second_file.saturating_sub(30), false);
    assert_fresh_page(&mut document, second_file, false);
    assert_fresh_page(&mut document, 250, true);
}

#[test]
fn eviction_and_disabled_cache_keep_complete_page_tokens() {
    for budget in [0, 8192] {
        let (_fixture, mut document) = document();
        document.highlight_cache = Some(HighlightCache::with_budget(budget));
        for start in [100, 228, 356, 100, 550, 420] {
            assert_fresh_page(&mut document, start, true);
            assert!(document.highlight_cache.as_ref().unwrap().retained_bytes() <= budget);
        }
    }
}

#[test]
fn obsolete_pages_leave_a_valid_cursor_for_the_next_request() {
    let (_fixture, mut document) = document();
    let mut request = page(1);
    request.range = 400..528;
    let mut snapshot = document.page(&request).unwrap();
    let checks = Cell::new(0);
    assert!(
        !document
            .highlight(&mut snapshot, request.settings, || {
                checks.set(checks.get() + 1);
                checks.get() >= 3
            })
            .unwrap()
    );
    assert_fresh_page(&mut document, 400, true);
    assert_fresh_page(&mut document, 528, true);
    assert_fresh_page(&mut document, 300, false);
    assert_eq!(snapshot.selection, SelectionId(1));
}

#[test]
fn very_large_hunks_release_the_cursor_and_keep_recent_tokens() {
    let fixture = Fixture::new();
    let source = format!("/*\n{}*/\n", format!("{}\n", "λ".repeat(6000)).repeat(100));
    fs::write(fixture.root().join("large.rs"), source).unwrap();
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "-qm", "large comment"]);
    let mut document =
        DiffDocument::load(&selected(&fixture, 1), &Cancellation::default()).unwrap();
    let mut request = page(1);
    let mut snapshot = document.page(&request).unwrap();
    assert!(
        document
            .highlight(&mut snapshot, request.settings, || false)
            .unwrap()
    );
    let cache = document.highlight_cache.as_ref().unwrap();
    assert_eq!(cache.cursor_input_bytes(), 0);
    let parsed = cache.parsed_lines;
    request.range = 30..60;
    let mut reverse = document.page(&request).unwrap();
    assert!(
        document
            .highlight(&mut reverse, request.settings, || false)
            .unwrap()
    );
    assert_eq!(
        document.highlight_cache.as_ref().unwrap().parsed_lines,
        parsed
    );
    assert!(reverse.lines.iter().all(|line| !line.spans.is_empty()));
}

#[test]
#[ignore = "large syntax fixture; run explicitly to profile forward/reverse paging"]
fn large_hunk_paging_parses_the_prefix_once() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root().join("large.rs"),
        "let value = \"λ\";\n".repeat(25000),
    )
    .unwrap();
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "-qm", "large syntax fixture"]);
    let mut document =
        DiffDocument::load(&selected(&fixture, 1), &Cancellation::default()).unwrap();
    let mut request = page(1);
    let mut warmup = document.page(&request).unwrap();
    document
        .highlight(&mut warmup, request.settings, || false)
        .unwrap();
    let started = std::time::Instant::now();
    for start in [
        20000, 20256, 20512, 20768, 21024, 21280, 21536, 21792, 21536, 21280, 21024, 20768,
    ] {
        request.range = start..start + 256;
        let mut snapshot = document.page(&request).unwrap();
        assert!(
            document
                .highlight(&mut snapshot, request.settings, || false)
                .unwrap()
        );
        assert!(snapshot.lines.iter().all(|line| !line.spans.is_empty()));
    }
    let cache = document.highlight_cache.as_ref().unwrap();
    assert!(cache.parsed_lines < 23000);
    assert!(cache.retained_bytes() <= 4 * 1024 * 1024);
    println!(
        "pages=12 elapsed_ms={} parsed_lines={} token_cache_bytes={}",
        started.elapsed().as_millis(),
        cache.parsed_lines,
        cache.retained_bytes()
    );
}

#[test]
fn separate_hunks_in_one_file_reset_their_parser_state() {
    let fixture = Fixture::new();
    let mut lines = (0..200)
        .map(|i| format!("let value_{i} = {i};\n"))
        .collect::<Vec<_>>();
    fs::write(fixture.root().join("hunks.rs"), lines.concat()).unwrap();
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "-qm", "base hunks"]);
    lines[10] = "let first = \"λ\";\n".into();
    lines[150] = "let second = \"λ\";\n".into();
    fs::write(fixture.root().join("hunks.rs"), lines.concat()).unwrap();
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "-qm", "two hunks"]);
    let mut document =
        DiffDocument::load(&selected(&fixture, 1), &Cancellation::default()).unwrap();
    let hunks = document
        .index
        .iter()
        .enumerate()
        .filter_map(|(i, line)| (line.kind == super::LineKind::Hunk).then_some(i))
        .collect::<Vec<_>>();
    assert_eq!(hunks.len(), 2);
    for start in [hunks[0], hunks[0] + 3, hunks[1], hunks[1] + 3, hunks[0]] {
        assert_fresh_range(&mut document, start..start + 4, true);
    }
}

#[test]
fn cursor_input_budget_counts_blank_line_separators() {
    let fixture = Fixture::new();
    fs::write(fixture.root().join("blank.rs"), "\n".repeat(300)).unwrap();
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "-qm", "blank syntax lines"]);
    let mut document =
        DiffDocument::load(&selected(&fixture, 1), &Cancellation::default()).unwrap();
    let request = page(1);
    let mut snapshot = document.page(&request).unwrap();
    document
        .highlight(&mut snapshot, request.settings, || false)
        .unwrap();
    // Each diff marker replaces the newline appended by CodeHighlighter::line.
    let expected = snapshot
        .lines
        .iter()
        .filter(|line| line.kind.code())
        .map(|line| line.text.len())
        .sum::<usize>();
    assert!(expected > 0);
    assert_eq!(
        document
            .highlight_cache
            .as_ref()
            .unwrap()
            .cursor_input_bytes(),
        expected
    );
}

#[test]
fn word_modes_preserve_syntax_colors_and_overlay_changes_across_pages_and_themes() {
    use super::{DiffVersion, WordDiff};
    let fixture = Fixture::new();
    for value in ["old", "new"] {
        fs::write(
            fixture.root().join("file.rs"),
            format!("fn main() {{\n    let text = \"{value}\";\n    let number = 42;\n}}\n"),
        )
        .unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", value]);
    }
    for words in [WordDiff::Markup, WordDiff::Color] {
        for version in [DiffVersion::Diff, DiffVersion::Old, DiffVersion::New] {
            let mut selection = selected(&fixture, 1);
            selection.options.words = words;
            selection.options.version = version;
            let mut document = DiffDocument::load(&selection, &Cancellation::default()).unwrap();
            for dark in [true, false, true] {
                let snapshot = assert_fresh_range(&mut document, 0..256, dark);
                let context = snapshot
                    .lines
                    .iter()
                    .find(|line| line.text.contains("let number"))
                    .unwrap();
                assert!(
                    context.spans.len() > 1,
                    "unchanged code must retain syntax colors"
                );
                let changed = snapshot
                    .lines
                    .iter()
                    .find(|line| line.text.contains("let text"))
                    .unwrap();
                let keyword = changed.text.find("let").unwrap();
                assert!(
                    changed
                        .spans
                        .iter()
                        .any(|span| span.range.contains(&keyword))
                );
                for word in ["old", "new"] {
                    if let Some(start) = changed.text.find(word) {
                        let color = match (word, dark) {
                            ("old", true) => 0xff9090,
                            ("old", false) => 0xc00000,
                            (_, true) => 0x80e090,
                            (_, false) => 0x008000,
                        };
                        assert!(
                            changed
                                .spans
                                .iter()
                                .any(|span| span.range.contains(&start) && span.color == color)
                        );
                    }
                }
                let mut request = page(1);
                request.range = snapshot
                    .lines
                    .iter()
                    .position(|line| line.text.contains("let number"))
                    .unwrap()..256;
                request.settings.dark = dark;
                assert_fresh_range(&mut document, request.range, dark);
            }
        }
    }
}

#[test]
fn word_syntax_tracks_multiline_state_on_the_selected_side() {
    use super::{DiffVersion, WordDiff};
    use crate::syntax::CodeHighlighter;
    let fixture = Fixture::new();
    for (label, source) in [
        ("old", "/* comment\nlet value = 42;\n*/\n"),
        ("new", "// comment\nlet value = 42;\n// end\n"),
    ] {
        fs::write(fixture.root().join("file.rs"), source).unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", label]);
    }
    for words in [WordDiff::Lines, WordDiff::Markup, WordDiff::Color] {
        for version in [DiffVersion::Diff, DiffVersion::Old, DiffVersion::New] {
            let mut selection = selected(&fixture, 1);
            selection.options.words = words;
            selection.options.version = version;
            let mut document = DiffDocument::load(&selection, &Cancellation::default()).unwrap();
            let snapshot = assert_fresh_range(&mut document, 0..256, true);
            let line = snapshot
                .lines
                .iter()
                .find(|line| line.text.contains("let value"))
                .unwrap();
            let opening = if version == DiffVersion::Old {
                "/* comment"
            } else {
                "// comment"
            };
            let mut parser = CodeHighlighter::new(std::path::Path::new("file.rs"), opening, true);
            parser.line(opening).unwrap();
            let mut expected = parser.line("let value = 42;").unwrap();
            for span in &mut expected {
                span.range.start += 1;
                span.range.end += 1;
            }
            assert_eq!(line.spans, expected, "{words:?} {version:?}");
        }
    }
}
