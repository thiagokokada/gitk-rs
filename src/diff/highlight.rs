//! A single forward parser cursor and a byte-bounded suffix of highlighted lines.
//! Recent reverse/overlapping pages use cached tokens; older jumps replay the hunk.
use super::{DiffDocument, DiffSettings, DiffSnapshot, LineKind};
use crate::syntax::{CodeHighlighter, Span};
use anyhow::Result;
use std::{collections::VecDeque, mem::size_of};

const CACHE_BYTES: usize = 4 * 1024 * 1024;
const CACHE_LINES: usize = 4096;
const CURSOR_INPUT_BYTES: usize = 1024 * 1024;

struct CachedLine {
    index: usize,
    spans: Box<[Span]>,
}

pub(super) struct HighlightCache {
    dark: Option<bool>,
    next: usize,
    file: Option<usize>,
    old: Option<CodeHighlighter>,
    new: Option<CodeHighlighter>,
    lines: VecDeque<CachedLine>,
    span_bytes: usize,
    budget: usize,
    parser_bytes: usize,
    #[cfg(test)]
    pub(super) parsed_lines: usize,
}

impl Default for HighlightCache {
    fn default() -> Self {
        Self::with_budget(CACHE_BYTES)
    }
}

impl HighlightCache {
    pub(super) fn with_budget(budget: usize) -> Self {
        let slots = (budget / (size_of::<CachedLine>() * 4)).min(CACHE_LINES);
        Self {
            dark: None,
            next: 0,
            file: None,
            old: None,
            new: None,
            lines: VecDeque::with_capacity(slots),
            span_bytes: 0,
            budget,
            parser_bytes: 0,
            #[cfg(test)]
            parsed_lines: 0,
        }
    }

    // Includes reserved ring storage, not just occupied entries. The opaque
    // Syntect cursor is one old/new parser pair, independent of cache length.
    pub(super) fn retained_bytes(&self) -> usize {
        self.lines.capacity() * size_of::<CachedLine>() + self.span_bytes
    }

    fn get(&self, index: usize) -> Option<&[Span]> {
        let offset = index.checked_sub(self.lines.front()?.index)?;
        let line = self.lines.get(offset)?;
        (line.index == index).then_some(&line.spans)
    }

    fn reset(&mut self, start: usize, dark: bool) {
        self.dark = Some(dark);
        self.next = start;
        self.file = None;
        self.old = None;
        self.new = None;
        self.lines.clear();
        self.span_bytes = 0;
        self.parser_bytes = 0;
    }

    // Syntect owns opaque, potentially nested parser allocations. Limit the
    // input behind a retained cursor as well as token storage. Larger hunks
    // remain correct, but resume by replaying when their recent tokens miss.
    pub(super) fn limit_cursor(&mut self) {
        if self.parser_bytes > CURSOR_INPUT_BYTES {
            self.old = None;
            self.new = None;
            self.file = None;
            self.next = usize::MAX;
            self.parser_bytes = 0;
        }
    }

    #[cfg(test)]
    pub(super) fn cursor_input_bytes(&self) -> usize {
        self.parser_bytes
    }

    fn insert(&mut self, index: usize, spans: Vec<Span>) {
        let bytes = spans.len() * size_of::<Span>();
        if self.lines.capacity() == 0
            || bytes + self.lines.capacity() * size_of::<CachedLine>() > self.budget
        {
            self.lines.clear();
            self.span_bytes = 0;
            return;
        }
        while self.lines.len() == self.lines.capacity()
            || self.retained_bytes() + bytes > self.budget
        {
            let evicted = self.lines.pop_front().unwrap();
            self.span_bytes -= evicted.spans.len() * size_of::<Span>();
        }
        self.lines.push_back(CachedLine {
            index,
            spans: spans.into_boxed_slice(),
        });
        self.span_bytes += bytes;
        debug_assert!(self.retained_bytes() <= self.budget);
    }

    pub(super) fn apply(
        &mut self,
        document: &mut DiffDocument,
        snapshot: &mut DiffSnapshot,
        settings: DiffSettings,
        obsolete: impl Fn() -> bool,
    ) -> Result<bool> {
        if snapshot.lines.is_empty() {
            snapshot.highlighting = false;
            return Ok(true);
        }
        let hunk = document.index[snapshot.start].hunk;
        let start = if hunk > 0 { hunk } else { snapshot.start };
        let end = snapshot.start + snapshot.lines.len();
        if self.dark != Some(settings.dark) {
            self.reset(start, settings.dark);
        }
        let cached = self.get(snapshot.start).is_some() && self.get(end - 1).is_some();
        if !cached
            && !(self.next >= start
                && self.next <= end
                && (self.next <= snapshot.start || self.get(snapshot.start).is_some()))
        {
            self.reset(start, settings.dark);
        }
        if obsolete() {
            return Ok(false);
        }
        // Copy overlap before insertions can evict it. A small configured cache
        // must not lose already-produced spans from the current page.
        for (offset, line) in snapshot.lines.iter_mut().enumerate() {
            if let Some(spans) = self.get(snapshot.start + offset) {
                line.spans = spans.to_vec();
            }
        }
        if !cached {
            for index in self.next..end {
                if index % 32 == 0 && obsolete() {
                    return Ok(false);
                }
                #[cfg(test)]
                {
                    self.parsed_lines += 1;
                }
                let spans = self.parse_line(document, index, settings.dark)?;
                self.next = index + 1;
                if index >= snapshot.start {
                    snapshot.lines[index - snapshot.start].spans = spans.clone();
                }
                self.insert(index, spans);
            }
        }
        snapshot.highlighting = false;
        Ok(true)
    }

    fn parse_line(
        &mut self,
        document: &mut DiffDocument,
        index: usize,
        dark: bool,
    ) -> Result<Vec<Span>> {
        let location = document.index[index];
        if location.kind == LineKind::Hunk || self.file != location.file {
            self.old = None;
            self.new = None;
            self.file = location.file;
            self.parser_bytes = 0;
        }
        if !location.kind.code() {
            return Ok(Vec::new());
        }
        let text = document.line(index)?;
        if text.len() > super::MAX_DISPLAY_BYTES {
            self.old = None;
            self.new = None;
            self.parser_bytes = 0;
            return Ok(Vec::new());
        }
        let code = &text[1..];
        self.parser_bytes = self.parser_bytes.saturating_add(code.len());
        let Some(file) = location.file.and_then(|file| document.files.get(file)) else {
            return Ok(Vec::new());
        };
        if self.old.is_none() {
            self.old = Some(CodeHighlighter::new(
                file.old_path.as_deref().unwrap_or(&file.path),
                code,
                dark,
            ));
            self.new = Some(CodeHighlighter::new(&file.path, code, dark));
        }
        let spans = match location.kind {
            LineKind::Added => self.new.as_mut().unwrap().line(code),
            LineKind::Removed => self.old.as_mut().unwrap().line(code),
            _ => {
                let _ = self.old.as_mut().unwrap().line(code);
                self.new.as_mut().unwrap().line(code)
            }
        };
        Ok(spans
            .unwrap_or_default()
            .into_iter()
            .map(|mut span| {
                span.range.start += 1;
                span.range.end += 1;
                span
            })
            .collect())
    }
}
