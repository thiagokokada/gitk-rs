//! Temporary, line-addressable diffs. Git/IO/highlighting live on a worker; no
//! commit-history database or persistent parsing cache is created.
use crate::{
    git::{Cancellation, Commit, GitStream, command_path},
    syntax::Span,
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::OsString,
    fs::File,
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    ops::Range,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, atomic::Ordering},
    thread::{self, JoinHandle},
};

mod highlight;
#[cfg(test)]
mod highlight_tests;
pub mod options;
use highlight::HighlightCache;

pub mod selection;
pub use selection::{TextPoint, TextSelection};
const MAX_LINES: usize = 1_000_000;
const MAX_DIFF_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_DISPLAY_BYTES: usize = 16 * 1024;
const MAX_COPY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Metadata,
    Header,
    Hunk,
    Context,
    Added,
    Removed,
}

impl LineKind {
    pub fn code(self) -> bool {
        matches!(self, Self::Context | Self::Added | Self::Removed)
    }
}

#[derive(Clone, Debug)]
pub struct FileSection {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub added: Option<u64>,
    pub removed: Option<u64>,
    pub start_line: Option<usize>,
}

#[derive(Clone)]
pub enum DiffSource {
    Commit(Arc<Commit>),
    Local { staged: bool },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionId(pub u64);

#[derive(Clone)]
pub struct DiffSelection {
    pub options: DiffOptions,
    pub id: SelectionId,
    pub repository: PathBuf,
    pub source: DiffSource,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffVersion {
    #[default]
    Diff,
    Old,
    New,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WordDiff {
    #[default]
    Lines,
    Markup,
    Color,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DiffOptions {
    pub version: DiffVersion,
    pub context: u32,
    pub ignore_space: bool,
    pub words: WordDiff,
}
impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            version: DiffVersion::Diff,
            context: 3,
            ignore_space: false,
            words: WordDiff::Lines,
        }
    }
}
impl DiffOptions {
    fn args(self, numstat: bool) -> Vec<OsString> {
        let mut args = Vec::new();
        if !numstat {
            args.push(format!("--unified={}", self.context).into());
        }
        if self.ignore_space {
            args.push("-w".into());
        }
        if !numstat && self.words != WordDiff::Lines {
            args.push("--word-diff=porcelain".into());
        }
        args
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffSettings {
    pub syntax: bool,
    pub dark: bool,
}

#[derive(Clone, Debug)]
pub struct DiffPage {
    pub selection: SelectionId,
    pub id: u64,
    pub range: Range<usize>,
    pub settings: DiffSettings,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub text: String,
    pub kind: LineKind,
    pub spans: Vec<Span>,
    pub original_len: usize,
}

#[derive(Clone, Debug)]
pub struct DiffSnapshot {
    pub selection: SelectionId,
    pub request: u64,
    pub start: usize,
    pub lines: Vec<DiffLine>,
    pub count: usize,
    pub last_line_len: usize,
    pub max_width: usize,
    pub files: Arc<Vec<FileSection>>,
    pub added: u64,
    pub removed: u64,
    pub notice: Option<String>,
    pub error: Option<String>,
    pub highlighting: bool,
}

#[derive(Clone, Copy)]
pub struct CopyRequest {
    pub selection: SelectionId,
    pub range: TextSelection,
    pub strip_markers: bool,
}
pub struct CopyResult {
    pub selection: SelectionId,
    pub result: Result<String, String>,
}

#[derive(Clone, Copy)]
struct LineIndex {
    offset: u64,
    len: u32,
    kind: LineKind,
    file: Option<usize>,
    hunk: usize,
}

pub struct DiffDocument {
    reader: BufReader<File>,
    position: u64,
    index: Vec<LineIndex>,
    files: Arc<Vec<FileSection>>,
    notice: Option<String>,
    max_width: usize,
    added: u64,
    removed: u64,
    highlight_cache: Option<HighlightCache>,
    old_version: bool,
    word_markup: bool,
    word_spans: std::collections::HashMap<usize, Vec<(Range<usize>, bool)>>,
}

impl DiffDocument {
    pub fn load(selection: &DiffSelection, cancellation: &Cancellation) -> Result<Self> {
        // Git's untracked enumeration is relative to its working directory.
        // Resolve once so a repository opened through a subdirectory still shows
        // all files and uses repository-relative paths throughout this document.
        let mut rooted = selection.clone();
        rooted.repository = command_path(
            &selection.repository,
            &["rev-parse", "--show-toplevel"],
            cancellation,
        )?;
        let selection = &rooted;
        let commands = DiffCommands::new(selection, cancellation)?;
        let mut files = load_files(selection, &commands, cancellation)?;
        let (added, removed) = files.iter().fold((0_u64, 0_u64), |(added, removed), file| {
            (
                added.saturating_add(file.added.unwrap_or(0)),
                removed.saturating_add(file.removed.unwrap_or(0)),
            )
        });
        let mut output =
            BufWriter::new(tempfile::tempfile().context("create temporary diff document")?);
        let mut index = Vec::new();
        let mut offset = 0;
        let mut max_width = 0;
        let header = match &selection.source {
            DiffSource::Commit(commit) => commit_header(commit),
            DiffSource::Local { staged } => format!(
                "{}\n\n",
                if *staged {
                    "Local changes staged for commit"
                } else {
                    "Local unstaged changes"
                }
            ),
        };
        for line in header.split_terminator('\n') {
            max_width = max_width.max(display_width_bound(line));
            write_line(
                &mut output,
                &mut index,
                &mut offset,
                line,
                LineKind::Metadata,
                None,
                0,
            )?;
        }
        let mut file = None;
        let mut hunk = 0;
        let mut in_hunk = false;
        let mut notice = None;
        let mut word_spans = std::collections::HashMap::new();
        let mut words = String::new();
        let mut spans = Vec::new();
        for command in 0..commands.len() {
            let args = commands.args(command, false);
            let mut stream = GitStream::spawn(
                &selection.repository,
                &args,
                cancellation.clone(),
                commands.allowed(command),
            )?;
            loop {
                if cancellation.load(Ordering::Acquire) {
                    bail!("diff operation cancelled");
                }
                let record = match stream.record(b'\n') {
                    Ok(Some(record)) => record,
                    Ok(None) => break,
                    Err(error) => {
                        notice = Some(format!("Diff preview stopped: {error:#}"));
                        break;
                    }
                };
                let line = String::from_utf8_lossy(&record.bytes);
                if line.starts_with("diff --git ") {
                    file = Some(file.map_or(0, |index| index + 1));
                    if let Some(section) = file.and_then(|file_index| files.get_mut(file_index)) {
                        section.start_line = Some(index.len());
                    }
                    hunk = 0;
                    in_hunk = false;
                }
                if selection.options.words != WordDiff::Lines
                    && in_hunk
                    && !line.starts_with("@@")
                    && !line.starts_with("diff --git ")
                {
                    if line == "~" {
                        ensure!(
                            index.len() < MAX_LINES
                                && offset + words.len() as u64 + 2 <= MAX_DIFF_BYTES,
                            "word diff exceeds preview limit"
                        );
                        let mut text = String::from(" ");
                        text.push_str(&words);
                        max_width = max_width.max(display_width_bound(&text));
                        word_spans.insert(index.len(), std::mem::take(&mut spans));
                        write_line(
                            &mut output,
                            &mut index,
                            &mut offset,
                            &text,
                            LineKind::Context,
                            file,
                            hunk,
                        )?;
                        words.clear();
                        continue;
                    }
                    if let Some(prefix @ (b' ' | b'+' | b'-')) = line.as_bytes().first().copied() {
                        let hidden = (prefix == b'+'
                            && selection.options.version == DiffVersion::Old)
                            || (prefix == b'-' && selection.options.version == DiffVersion::New);
                        if !hidden {
                            let start = words.len() + 1;
                            let markup = selection.options.words == WordDiff::Markup;
                            if markup && prefix != b' ' {
                                words.push_str(if prefix == b'+' { "{+" } else { "[-" });
                            }
                            words.push_str(&line[1..]);
                            if markup && prefix != b' ' {
                                words.push_str(if prefix == b'+' { "+}" } else { "-]" });
                            }
                            if prefix != b' ' {
                                spans.push((start..words.len() + 1, prefix == b'+'));
                            }
                        }
                        ensure!(
                            words.len() as u64 <= MAX_DIFF_BYTES,
                            "word diff line exceeds preview limit"
                        );
                        continue;
                    }
                }
                let kind = if line.starts_with("@@") {
                    in_hunk = true;
                    hunk = index.len();
                    LineKind::Hunk
                } else if in_hunk {
                    match line.as_bytes().first() {
                        Some(b'+') => LineKind::Added,
                        Some(b'-') => LineKind::Removed,
                        Some(b' ') => LineKind::Context,
                        _ => LineKind::Header,
                    }
                } else {
                    LineKind::Header
                };
                if (kind == LineKind::Added && selection.options.version == DiffVersion::Old)
                    || (kind == LineKind::Removed && selection.options.version == DiffVersion::New)
                {
                    continue;
                }
                if index.len() >= MAX_LINES || offset + line.len() as u64 + 1 > MAX_DIFF_BYTES {
                    notice =
                        Some("Diff preview is limited to 256 MiB or one million lines.".into());
                    break;
                }
                max_width = max_width.max(display_width_bound(&line));
                write_line(
                    &mut output,
                    &mut index,
                    &mut offset,
                    &line,
                    kind,
                    file,
                    hunk,
                )?;
            }
            drop(stream);
            if notice.is_some() {
                break;
            }
        }
        if index.len() == header.lines().count() {
            write_line(
                &mut output,
                &mut index,
                &mut offset,
                "No file-level changes.",
                LineKind::Metadata,
                None,
                0,
            )?;
        }
        for section in &mut files {
            if section.start_line.is_some_and(|line| line >= index.len()) {
                section.start_line = None;
            }
        }
        let mut file = output.into_inner()?;
        file.seek(SeekFrom::Start(0))?;
        Ok(Self {
            reader: BufReader::new(file),
            position: 0,
            index,
            files: Arc::new(files),
            notice,
            max_width,
            added,
            removed,
            highlight_cache: None,
            word_spans,
            word_markup: selection.options.words == WordDiff::Markup,
            old_version: selection.options.version == DiffVersion::Old,
        })
    }

    fn line(&mut self, line: usize) -> Result<String> {
        let location = self.index.get(line).context("diff line is out of range")?;
        if self.position != location.offset {
            self.reader.seek(SeekFrom::Start(location.offset))?;
        }
        let mut bytes = vec![0; location.len as usize];
        self.reader.read_exact(&mut bytes)?;
        let mut newline = [0];
        self.reader.read_exact(&mut newline)?;
        self.position = location.offset + location.len as u64 + 1;
        Ok(String::from_utf8(bytes)?)
    }

    fn page(&mut self, request: &DiffPage) -> Result<DiffSnapshot> {
        let start = request.range.start.min(self.index.len());
        let end = request.range.end.min(start + 256).min(self.index.len());
        let mut lines = Vec::new();
        let mut notice = self.notice.clone();
        for index in start..end {
            let mut text = self.line(index)?;
            let original_len = text.len();
            if text.len() > MAX_DISPLAY_BYTES {
                let boundary = text.floor_char_boundary(MAX_DISPLAY_BYTES);
                text.truncate(boundary);
                notice = Some(
                    "Long lines are shortened for display; copying reads the original text.".into(),
                );
            }
            lines.push(DiffLine {
                text: text.clone(),
                kind: self.index[index].kind,
                spans: self
                    .word_spans
                    .get(&index)
                    .into_iter()
                    .flatten()
                    .filter_map(|(range, added)| {
                        let end = range.end.min(text.len());
                        (range.start < end).then_some(Span {
                            range: range.start..end,
                            color: if *added {
                                if request.settings.dark {
                                    0x80e090
                                } else {
                                    0x008000
                                }
                            } else if request.settings.dark {
                                0xff9090
                            } else {
                                0xc00000
                            },
                        })
                    })
                    .collect(),
                original_len,
            });
        }
        Ok(DiffSnapshot {
            selection: request.selection,
            request: request.id,
            start,
            lines,
            count: self.index.len(),
            last_line_len: self.index.last().map_or(0, |line| line.len as usize),
            max_width: self.max_width,
            files: self.files.clone(),
            added: self.added,
            removed: self.removed,
            notice,
            error: None,
            highlighting: request.settings.syntax,
        })
    }

    fn highlight(
        &mut self,
        snapshot: &mut DiffSnapshot,
        settings: DiffSettings,
        obsolete: impl Fn() -> bool,
    ) -> Result<bool> {
        let mut cache = self.highlight_cache.take().unwrap_or_default();
        let overlays: Vec<_> = snapshot
            .lines
            .iter()
            .map(|line| line.spans.clone())
            .collect();
        let result = cache.apply(self, snapshot, settings, obsolete);
        if matches!(result, Ok(true)) {
            for (line, words) in snapshot.lines.iter_mut().zip(overlays) {
                for word in words {
                    let mut combined = Vec::new();
                    for span in line.spans.drain(..) {
                        if span.range.end <= word.range.start || span.range.start >= word.range.end
                        {
                            combined.push(span);
                        } else {
                            if span.range.start < word.range.start {
                                combined.push(Span {
                                    range: span.range.start..word.range.start,
                                    color: span.color,
                                });
                            }
                            if span.range.end > word.range.end {
                                combined.push(Span {
                                    range: word.range.end..span.range.end,
                                    color: span.color,
                                });
                            }
                        }
                    }
                    combined.push(word);
                    combined.sort_by_key(|span| span.range.start);
                    line.spans = combined;
                }
            }
        }
        cache.limit_cursor();
        self.highlight_cache = Some(cache);
        result
    }

    fn copy(&mut self, request: CopyRequest, cancellation: &Cancellation) -> Result<String> {
        let (start, end) = request.range.ordered();
        let mut result = String::new();
        for line in start.line..=end.line.min(self.index.len().saturating_sub(1)) {
            ensure!(!cancellation.load(Ordering::Acquire), "copy cancelled");
            let text = self.line(line)?;
            let mut from = if line == start.line {
                start.byte.min(text.len())
            } else {
                0
            };
            let to = if line == end.line {
                end.byte.min(text.len())
            } else {
                text.len()
            };
            from = text.floor_char_boundary(from);
            let to = text.floor_char_boundary(to);
            if request.strip_markers && self.index[line].kind.code() && from == 0 && to > 0 {
                from = 1;
            }
            ensure!(
                result.len() + to.saturating_sub(from) < MAX_COPY_BYTES,
                "selection exceeds the 16 MiB clipboard limit"
            );
            if to > from {
                result.push_str(&text[from..to]);
            }
            if line < end.line {
                result.push('\n');
            }
        }
        Ok(result)
    }
}

fn display_width_bound(line: &str) -> usize {
    let prefix = &line[..line.floor_char_boundary(MAX_DISPLAY_BYTES)];
    // Tabs expand to at most four cells. Counting UTF-8 bytes for other text
    // deliberately overestimates width rather than hiding the end of a line.
    prefix.len() + 3 * prefix.bytes().filter(|byte| *byte == b'\t').count()
}

fn write_line(
    output: &mut impl Write,
    index: &mut Vec<LineIndex>,
    offset: &mut u64,
    line: &str,
    kind: LineKind,
    file: Option<usize>,
    hunk: usize,
) -> Result<()> {
    output.write_all(line.as_bytes())?;
    output.write_all(b"\n")?;
    index.push(LineIndex {
        offset: *offset,
        len: line.len() as u32,
        kind,
        file,
        hunk,
    });
    *offset += line.len() as u64 + 1;
    Ok(())
}

pub fn commit_header(commit: &Commit) -> String {
    let mut header = format!(
        "commit {}\nAuthor: {} <{}>  {}\nCommitter: {} <{}>  {}\n\n",
        commit.id,
        commit.author.name,
        commit.author.email,
        commit.author.time,
        commit.committer.name,
        commit.committer.email,
        commit.committer.time
    );
    for line in commit.message.trim_end_matches('\n').lines() {
        header.push_str("    ");
        header.push_str(line);
        header.push('\n');
    }
    header.push('\n');
    header
}

fn diff_args(source: &DiffSource, numstat: bool) -> Vec<OsString> {
    let mut args: Vec<OsString> = match source {
        DiffSource::Commit(commit) if commit.parents.is_empty() => {
            vec!["show".into(), "--format=".into()]
        }
        _ => vec!["diff".into()],
    };
    args.extend(["--no-color", "--no-ext-diff", "--no-textconv"].map(OsString::from));
    if numstat {
        args.extend(["--numstat", "-z"].map(OsString::from));
    }
    match source {
        DiffSource::Commit(commit) => {
            if let Some(parent) = commit.parents.first() {
                args.push(parent.to_hex().into());
            }
            args.push(commit.id.to_hex().into());
        }
        DiffSource::Local { staged: true } => args.push("--cached".into()),
        DiffSource::Local { staged: false } => args.push("--ours".into()),
    }
    args
}

struct DiffCommands {
    source: DiffSource,
    untracked: Vec<PathBuf>,
    options: DiffOptions,
}
impl DiffCommands {
    fn new(selection: &DiffSelection, cancellation: &Cancellation) -> Result<Self> {
        let mut untracked = Vec::new();
        if matches!(selection.source, DiffSource::Local { staged: false }) {
            let mut stream = GitStream::spawn(
                &selection.repository,
                &["ls-files", "--others", "--exclude-standard", "-z"],
                cancellation.clone(),
                &[0],
            )?;
            let mut bytes = 0;
            while let Some(record) = stream.record(0)? {
                bytes += record.bytes.len() + std::mem::size_of::<PathBuf>();
                ensure!(bytes <= 16 * 1024 * 1024, "untracked paths exceed 16 MiB");
                let path = path_from_bytes(record.bytes)?;
                ensure!(
                    path.is_relative()
                        && !path
                            .components()
                            .any(|c| matches!(c, std::path::Component::ParentDir)),
                    "invalid untracked path"
                );
                untracked.push(path);
            }
        }
        Ok(Self {
            source: selection.source.clone(),
            options: selection.options,
            untracked,
        })
    }
    fn len(&self) -> usize {
        self.untracked.len() + 1
    }
    fn allowed(&self, command: usize) -> &'static [i32] {
        if command == 0 { &[0] } else { &[0, 1] }
    }
    fn args(&self, command: usize, numstat: bool) -> Vec<OsString> {
        if command == 0 {
            let mut args = diff_args(&self.source, numstat);
            args.extend(self.options.args(numstat));
            return args;
        }
        let mut args = vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-index",
            "--src-prefix=a/",
            "--dst-prefix=b/",
        ]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
        if numstat {
            args.extend(["--numstat", "-z"].map(OsString::from));
        }
        args.extend(self.options.args(numstat));
        args.extend([
            OsString::from("--"),
            OsString::from("/dev/null"),
            self.untracked[command - 1].as_os_str().to_owned(),
        ]);
        args
    }
}

fn load_files(
    selection: &DiffSelection,
    commands: &DiffCommands,
    cancellation: &Cancellation,
) -> Result<Vec<FileSection>> {
    let mut files = Vec::new();
    let mut bytes = 0;
    for command in 0..commands.len() {
        let mut stream = GitStream::spawn(
            &selection.repository,
            &commands.args(command, true),
            cancellation.clone(),
            commands.allowed(command),
        )?;
        while let Some(record) = stream.record(0)? {
            if record.bytes.iter().all(|b| matches!(b, b'\r' | b'\n')) {
                continue;
            }
            bytes += record.bytes.len();
            ensure!(
                bytes < 16 * 1024 * 1024,
                "diff file metadata exceeds 16 MiB"
            );
            let mut parts = record.bytes.splitn(3, |b| *b == b'\t');
            let added = std::str::from_utf8(parts.next().context("missing numstat additions")?)?
                .trim()
                .parse()
                .ok();
            let removed = std::str::from_utf8(parts.next().context("missing numstat removals")?)?
                .parse()
                .ok();
            let path = parts.next().context("missing numstat filename")?;
            let (path, old_path) = if path.is_empty() {
                let old = stream.record(0)?.context("missing rename source")?;
                let new = stream.record(0)?.context("missing rename destination")?;
                bytes += old.bytes.len() + new.bytes.len();
                (
                    path_from_bytes(new.bytes)?,
                    Some(path_from_bytes(old.bytes)?),
                )
            } else {
                (path_from_bytes(path.to_vec())?, None)
            };
            // Git emits a zero-count unmerged entry followed by its actual patch
            // numstat. Keep one navigation entry in the patch's file order.
            if let Some(previous) = files
                .last_mut()
                .filter(|f: &&mut FileSection| f.path == path && f.old_path == old_path)
            {
                previous.added = added;
                previous.removed = removed;
            } else {
                files.push(FileSection {
                    path,
                    old_path,
                    added,
                    removed,
                    start_line: None,
                });
            }
        }
    }
    Ok(files)
}

pub(crate) fn path_from_bytes(bytes: Vec<u8>) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(OsString::from_vec(bytes).into())
    }
    #[cfg(not(unix))]
    {
        Ok(String::from_utf8(bytes)?.into())
    }
}

struct DiffMailbox {
    state: options::State,
    save_state: bool,
    selection: Option<DiffSelection>,
    page: Option<DiffPage>,
    copy: Option<CopyRequest>,
    snapshot: Option<DiffSnapshot>,
    copied: Option<CopyResult>,
    current: SelectionId,
    cancellation: Cancellation,
    clear: bool,
    stopped: bool,
}

struct DiffShared {
    options_path: Option<PathBuf>,
    initial_state: options::State,
    mailbox: Mutex<DiffMailbox>,
    changed: Condvar,
}
#[derive(Clone)]
pub struct DiffHandle {
    shared: Arc<DiffShared>,
}
pub struct DiffEngine {
    handle: DiffHandle,
    worker: Option<JoinHandle<()>>,
}

impl DiffEngine {
    pub fn spawn() -> Self {
        Self::with_options_file(None)
    }

    /// Load startup state before opening the UI; subsequent writes run on the worker.
    pub fn with_options_file(path: Option<PathBuf>) -> Self {
        let initial_state = path.as_ref().map_or_else(options::State::default, |path| {
            options::load_state_from(path).unwrap_or_else(|error| {
                eprintln!("gitk-rs: load state {}: {error:#}", path.display());
                options::State::default()
            })
        });
        let shared = Arc::new(DiffShared {
            options_path: path,
            initial_state,
            mailbox: Mutex::new(DiffMailbox {
                state: initial_state,
                save_state: false,
                selection: None,
                page: None,
                copy: None,
                snapshot: None,
                copied: None,
                current: SelectionId(0),
                cancellation: Cancellation::default(),
                clear: false,
                stopped: false,
            }),
            changed: Condvar::new(),
        });
        let worker_shared = shared.clone();
        let worker = thread::spawn(move || diff_worker(worker_shared));
        Self {
            handle: DiffHandle { shared },
            worker: Some(worker),
        }
    }

    pub fn handle(&self) -> DiffHandle {
        self.handle.clone()
    }
}

impl Drop for DiffEngine {
    fn drop(&mut self) {
        {
            let mut mailbox = self.handle.shared.mailbox.lock().unwrap();
            mailbox.stopped = true;
            mailbox.cancellation.store(true, Ordering::Release);
        }
        self.handle.shared.changed.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl DiffHandle {
    pub fn initial_options(&self) -> DiffOptions {
        self.shared.initial_state.diff
    }
    pub fn save_options(&self, options: DiffOptions) {
        if self.shared.options_path.is_none() {
            return;
        }
        let mut mailbox = self.shared.mailbox.lock().unwrap();
        mailbox.state.diff = options;
        mailbox.save_state = true;
        drop(mailbox);
        self.shared.changed.notify_one();
    }

    pub fn initial_search(&self) -> options::Search {
        self.shared.initial_state.search
    }

    pub fn save_search(&self, search: options::Search) {
        if self.shared.options_path.is_none() {
            return;
        }
        let mut mailbox = self.shared.mailbox.lock().unwrap();
        mailbox.state.search = search;
        mailbox.save_state = true;
        drop(mailbox);
        self.shared.changed.notify_one();
    }

    pub fn initial_layout(&self) -> options::Layout {
        self.shared.initial_state.layout
    }
    pub fn save_layout(&self, change: options::LayoutChange) {
        if self.shared.options_path.is_none() {
            return;
        }
        let mut mailbox = self.shared.mailbox.lock().unwrap();
        mailbox.state.layout.apply(change);
        mailbox.save_state = true;
        drop(mailbox);
        self.shared.changed.notify_one();
    }

    pub fn select(&self, selection: DiffSelection, page: DiffPage) {
        self.clear(selection.id);
        {
            let mut mailbox = self.shared.mailbox.lock().unwrap();
            mailbox.selection = Some(selection);
            mailbox.page = Some(page);
        }
        self.shared.changed.notify_one();
    }

    pub fn clear(&self, selection: SelectionId) {
        let old = {
            let mut mailbox = self.shared.mailbox.lock().unwrap();
            mailbox.cancellation.store(true, Ordering::Release);
            mailbox.cancellation = Cancellation::default();
            mailbox.current = selection;
            mailbox.clear = true;
            (
                mailbox.selection.take(),
                mailbox.page.take(),
                mailbox.copy.take(),
                mailbox.snapshot.take(),
                mailbox.copied.take(),
            )
        };
        drop(old);
        self.shared.changed.notify_one();
    }

    pub fn page(&self, page: DiffPage) {
        {
            let mut mailbox = self.shared.mailbox.lock().unwrap();
            if page.selection != mailbox.current {
                return;
            }
            mailbox.page = Some(page);
        }
        self.shared.changed.notify_one();
    }

    pub fn copy(&self, copy: CopyRequest) {
        {
            let mut mailbox = self.shared.mailbox.lock().unwrap();
            if copy.selection != mailbox.current {
                return;
            }
            mailbox.copy = Some(copy);
        }
        self.shared.changed.notify_one();
    }

    pub fn take_snapshot(&self) -> Option<DiffSnapshot> {
        self.shared.mailbox.lock().unwrap().snapshot.take()
    }
    pub fn take_copy(&self) -> Option<CopyResult> {
        self.shared.mailbox.lock().unwrap().copied.take()
    }
}

fn publish_diff(shared: &DiffShared, snapshot: DiffSnapshot) {
    let old = {
        let mut mailbox = shared.mailbox.lock().unwrap();
        if mailbox.current != snapshot.selection || mailbox.stopped {
            return;
        }
        mailbox.snapshot.replace(snapshot)
    };
    drop(old);
}

fn diff_error(page: &DiffPage, error: String) -> DiffSnapshot {
    DiffSnapshot {
        selection: page.selection,
        request: page.id,
        start: 0,
        lines: Vec::new(),
        count: 0,
        last_line_len: 0,
        max_width: 0,
        files: Arc::new(Vec::new()),
        added: 0,
        removed: 0,
        notice: None,
        error: Some(error),
        highlighting: false,
    }
}

fn diff_worker(shared: Arc<DiffShared>) {
    let mut document: Option<DiffDocument> = None;
    let mut failure: Option<String> = None;
    loop {
        let (selection, page, copy, cancellation, clear, options, stopped) = {
            let mut mailbox = shared.mailbox.lock().unwrap();
            while mailbox.selection.is_none()
                && mailbox.page.is_none()
                && mailbox.copy.is_none()
                && !mailbox.save_state
                && !mailbox.clear
                && !mailbox.stopped
            {
                mailbox = shared.changed.wait(mailbox).unwrap();
            }
            let clear = std::mem::take(&mut mailbox.clear);
            (
                mailbox.selection.take(),
                mailbox.page.take(),
                mailbox.copy.take(),
                mailbox.cancellation.clone(),
                clear,
                std::mem::take(&mut mailbox.save_state).then_some(mailbox.state),
                mailbox.stopped,
            )
        };
        if let Some(options) = options
            && let Some(path) = &shared.options_path
            && let Err(error) = options::save_state_to(options, path)
        {
            eprintln!("gitk-rs: save state {}: {error:#}", path.display());
        }
        if stopped {
            break;
        }
        if clear {
            document = None;
            failure = None;
        }
        if let Some(selection) = selection {
            match DiffDocument::load(&selection, &cancellation) {
                Ok(loaded) => document = Some(loaded),
                Err(error) => failure = Some(format!("{error:#}")),
            }
        }
        if let Some(copy) = copy
            && let Some(document) = &mut document
        {
            let result = document
                .copy(copy, &cancellation)
                .map_err(|error| format!("{error:#}"));
            let mut mailbox = shared.mailbox.lock().unwrap();
            if mailbox.current == copy.selection {
                mailbox.copied = Some(CopyResult {
                    selection: copy.selection,
                    result,
                });
            }
        }
        let Some(page) = page else {
            continue;
        };
        if cancellation.load(Ordering::Acquire) {
            continue;
        }
        if let Some(error) = &failure {
            publish_diff(&shared, diff_error(&page, error.clone()));
            continue;
        }
        let Some(document) = &mut document else {
            continue;
        };
        let mut snapshot = match document.page(&page) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                publish_diff(&shared, diff_error(&page, format!("{error:#}")));
                continue;
            }
        };
        publish_diff(&shared, snapshot.clone());
        if page.settings.syntax {
            let complete = document.highlight(&mut snapshot, page.settings, || {
                if cancellation.load(Ordering::Acquire) {
                    return true;
                }
                let mailbox = shared.mailbox.lock().unwrap();
                mailbox.stopped || mailbox.current != page.selection || mailbox.page.is_some()
            });
            match complete {
                Ok(true) => publish_diff(&shared, snapshot),
                Ok(false) => {}
                Err(error) => {
                    snapshot.highlighting = false;
                    snapshot.notice = Some(format!("Syntax highlighting unavailable: {error:#}"));
                    publish_diff(&shared, snapshot);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{git::Repository, test_support::Fixture};
    use std::{
        fs,
        time::{Duration, Instant},
    };

    fn cancel() -> Cancellation {
        Cancellation::default()
    }
    pub(super) fn selected(fixture: &Fixture, id: u64) -> DiffSelection {
        let cancellation = cancel();
        let repository = Repository::open(fixture.root(), &cancellation).unwrap();
        let commit = repository
            .log(cancellation)
            .unwrap()
            .next_commit()
            .unwrap()
            .unwrap();
        DiffSelection {
            options: Default::default(),
            id: SelectionId(id),
            repository: fixture.root().into(),
            source: DiffSource::Commit(Arc::new(commit)),
        }
    }
    pub(super) fn page(selection: u64) -> DiffPage {
        DiffPage {
            selection: SelectionId(selection),
            id: 1,
            range: 0..256,
            settings: DiffSettings {
                syntax: true,
                dark: true,
            },
        }
    }

    #[test]
    fn gitk_diff_options_filter_versions_context_whitespace_and_words() {
        let fixture = Fixture::new();
        fs::write(
            fixture.root().join("file.txt"),
            "first\nhello old world\nlast\n",
        )
        .unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", "old"]);
        fs::write(
            fixture.root().join("file.txt"),
            "first\nhello new world\nlast\n",
        )
        .unwrap();
        fixture.git(&["commit", "-qam", "new"]);
        let mut selection = selected(&fixture, 1);
        for version in [DiffVersion::Diff, DiffVersion::Old, DiffVersion::New] {
            selection.options.version = version;
            selection.options.context = 0;
            let mut document = DiffDocument::load(&selection, &cancel()).unwrap();
            let snapshot = document.page(&page(1)).unwrap();
            assert_eq!(
                snapshot.lines.iter().any(|l| l.text == "-hello old world"),
                version != DiffVersion::New
            );
            assert_eq!(
                snapshot.lines.iter().any(|l| l.text == "+hello new world"),
                version != DiffVersion::Old
            );
            assert!(!snapshot.lines.iter().any(|l| l.text == " first"));
            for words in [WordDiff::Markup, WordDiff::Color] {
                selection.options.words = words;
                let mut document = DiffDocument::load(&selection, &cancel()).unwrap();
                let snapshot = document.page(&page(1)).unwrap();
                let line = snapshot
                    .lines
                    .iter()
                    .find(|l| l.text.contains("hello"))
                    .unwrap();
                assert_eq!(line.text.contains("old"), version != DiffVersion::New);
                assert_eq!(line.text.contains("new"), version != DiffVersion::Old);
                assert_eq!(
                    line.text.contains("[-"),
                    words == WordDiff::Markup && version != DiffVersion::New
                );
                assert!(!line.spans.is_empty());
            }
            selection.options.words = WordDiff::Lines;
        }
        fs::write(
            fixture.root().join("file.txt"),
            "first\nhello   new world\nlast\n",
        )
        .unwrap();
        selection.source = DiffSource::Local { staged: false };
        selection.options = DiffOptions {
            ignore_space: true,
            ..Default::default()
        };
        let document = DiffDocument::load(&selection, &cancel()).unwrap();
        assert!(document.files.is_empty());
    }

    #[test]
    fn root_diff_and_copy_across_unloaded_lines_preserve_tabs_and_unicode() {
        let fixture = Fixture::new();
        let source = (0..400)
            .map(|i| format!("\tlet λ{i} = {i};\n"))
            .collect::<String>();
        fs::write(fixture.root().join("main.rs"), &source).unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", "root"]);
        let cancellation = cancel();
        let mut document = DiffDocument::load(&selected(&fixture, 1), &cancellation).unwrap();
        assert_eq!(document.files.len(), 1);
        assert_eq!(document.files[0].path, PathBuf::from("main.rs"));
        assert_eq!(document.files[0].added, Some(400));
        let first = document
            .index
            .iter()
            .position(|line| line.kind == LineKind::Added)
            .unwrap();
        let last = document.index.len() - 1;
        let mut snapshot = document.page(&page(1)).unwrap();
        document
            .highlight(&mut snapshot, page(1).settings, || false)
            .unwrap();
        assert!(snapshot.lines.iter().any(|line| line.spans.len() > 1));
        assert_eq!(snapshot.lines.len(), 256);
        let selection = TextSelection {
            anchor: TextPoint {
                line: first,
                byte: 0,
            },
            cursor: TextPoint {
                line: last,
                byte: document.index[last].len as usize,
            },
        };
        let copied = document
            .copy(
                CopyRequest {
                    selection: SelectionId(1),
                    range: selection,
                    strip_markers: true,
                },
                &cancellation,
            )
            .unwrap();
        assert_eq!(copied, source.trim_end_matches('\n'));
        let copied = document
            .copy(
                CopyRequest {
                    selection: SelectionId(1),
                    range: selection,
                    strip_markers: false,
                },
                &cancellation,
            )
            .unwrap();
        assert!(copied.starts_with("+\tlet λ0"));
        assert!(copied.contains("\n+\tlet λ399"));
    }

    #[test]
    fn rename_binary_unusual_paths_and_removed_header_like_code() {
        let fixture = Fixture::new();
        fs::write(
            fixture.root().join("old\tλ.lua"),
            "-- comment\n-- unchanged one\n-- unchanged two\n-- unchanged three\n--- header-like code\nreturn 1\n",
        )
        .unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", "first"]);
        fixture.git(&["mv", "old\tλ.lua", "new\nλ.lua"]);
        fs::write(
            fixture.root().join("new\nλ.lua"),
            "-- comment\n-- unchanged one\n-- unchanged two\n-- unchanged three\nreturn 2\n",
        )
        .unwrap();
        fs::write(fixture.root().join("binary.dat"), [0, 255, 0, 1]).unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", "rename and binary"]);
        let mut document = DiffDocument::load(&selected(&fixture, 1), &cancel()).unwrap();
        assert_eq!(document.files.len(), 2);
        assert!(
            document
                .files
                .iter()
                .any(|file| file.path == std::path::Path::new("binary.dat") && file.added.is_none())
        );
        assert!(
            document
                .files
                .iter()
                .any(|file| file.path == std::path::Path::new("new\nλ.lua"))
        );
        let mut found = false;
        for i in 0..document.index.len() {
            if document.line(i).unwrap() == "---- header-like code" {
                assert_eq!(document.index[i].kind, LineKind::Removed);
                found = true;
            }
        }
        assert!(found);
        assert!(
            document
                .files
                .iter()
                .any(|file| file.old_path.as_deref() == Some(std::path::Path::new("old\tλ.lua")))
        );
    }

    #[test]
    fn merges_use_first_parent_and_long_lines_are_copied_in_full() {
        let fixture = Fixture::new();
        fixture.commit("base");
        fixture.git(&["checkout", "-qb", "side"]);
        fs::write(
            fixture.root().join("side.nix"),
            format!("{}\n", "λ".repeat(10000)),
        )
        .unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", "side"]);
        fixture.git(&["checkout", "-q", "main"]);
        fixture.commit("main");
        fixture.git(&["merge", "--no-ff", "-qm", "merge", "side"]);
        let cancellation = cancel();
        let mut document = DiffDocument::load(&selected(&fixture, 1), &cancellation).unwrap();
        assert_eq!(document.files.len(), 1);
        assert_eq!(document.files[0].path, PathBuf::from("side.nix"));
        let snapshot = document.page(&page(1)).unwrap();
        let (line, text) = snapshot
            .lines
            .iter()
            .enumerate()
            .find(|(_, l)| l.kind == LineKind::Added)
            .unwrap();
        assert!(text.text.len() <= MAX_DISPLAY_BYTES);
        assert_eq!(text.original_len, 20001);
        assert!(snapshot.notice.is_some());
        let copy = document
            .copy(
                CopyRequest {
                    selection: SelectionId(1),
                    range: TextSelection {
                        anchor: TextPoint { line, byte: 0 },
                        cursor: TextPoint { line, byte: 20001 },
                    },
                    strip_markers: true,
                },
                &cancellation,
            )
            .unwrap();
        assert_eq!(copy, "λ".repeat(10000));
    }

    #[test]
    fn local_diffs_include_untracked_files_and_unborn_staged_changes() {
        let fixture = Fixture::new();
        std::fs::write(
            fixture.root().join("untracked\nλ.nix"),
            "{ answer = 42; }\n",
        )
        .unwrap();
        let selection = DiffSelection {
            options: Default::default(),
            id: SelectionId(1),
            repository: fixture.root().into(),
            source: DiffSource::Local { staged: false },
        };
        let document = DiffDocument::load(&selection, &cancel()).unwrap();
        assert_eq!(document.files.len(), 1);
        assert_eq!(
            document.files[0].path,
            std::path::Path::new("untracked\nλ.nix")
        );
        assert_eq!(document.files[0].added, Some(1));
        assert!(document.files[0].start_line.is_some());
        fixture.git(&["add", "."]);
        let selection = DiffSelection {
            source: DiffSource::Local { staged: true },
            ..selection
        };
        let document = DiffDocument::load(&selection, &cancel()).unwrap();
        assert_eq!(document.files.len(), 1);
        assert_eq!(document.files[0].added, Some(1));
    }

    #[test]
    fn conflicted_worktree_has_one_file_entry_and_marker_lines() {
        let fixture = Fixture::new();
        fixture.commit("base");
        fixture.git(&["checkout", "-qb", "side"]);
        fixture.commit("side");
        fixture.git(&["checkout", "-q", "main"]);
        fixture.commit("main");
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(fixture.root())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "merge",
                "--no-commit",
                "side",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let selection = DiffSelection {
            options: Default::default(),
            id: SelectionId(1),
            repository: fixture.root().into(),
            source: DiffSource::Local { staged: false },
        };
        let mut document = DiffDocument::load(&selection, &cancel()).unwrap();
        assert_eq!(document.files.len(), 1);
        assert!(document.files[0].start_line.is_some());
        let snapshot = document.page(&page(1)).unwrap();
        assert!(
            snapshot
                .lines
                .iter()
                .any(|line| line.kind == LineKind::Added && line.text.contains("<<<<<<<"))
        );
    }

    #[test]
    fn horizontal_extent_includes_expanded_tabs_without_retaining_full_long_lines() {
        let line = format!("{}let λ = 42;", "\t".repeat(128));
        assert!(display_width_bound(&line) >= selection::DisplayText::new(&line).text.len());
        let long = "\t".repeat(MAX_DISPLAY_BYTES * 2);
        assert_eq!(display_width_bound(&long), MAX_DISPLAY_BYTES * 4);
    }

    #[test]
    fn worker_cancels_and_rejects_previous_selection_results() {
        let fixture = Fixture::new();
        fixture.commit("first");
        let engine = DiffEngine::spawn();
        let handle = engine.handle();
        handle.select(selected(&fixture, 1), page(1));
        handle.clear(SelectionId(2));
        handle.page(page(1));
        assert!(handle.take_snapshot().is_none());
        handle.select(selected(&fixture, 3), page(3));
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(snapshot) = handle.take_snapshot() {
                assert_eq!(snapshot.selection, SelectionId(3));
                assert!(snapshot.error.is_none());
                if !snapshot.highlighting {
                    break;
                }
            }
            assert!(Instant::now() < until, "diff worker timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }
}
