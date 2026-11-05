//! Offline syntax coverage from bat's full two-face bundle. Initialize only on
//! a worker (or an explicitly requested CLI inventory), never during rendering.
use anyhow::Result;
use std::{ops::Range, path::Path, sync::LazyLock};
use syntect::{
    easy::HighlightLines,
    highlighting::Theme,
    parsing::{SyntaxReference, SyntaxSet},
};
use two_face::theme::{EmbeddedLazyThemeSet, EmbeddedThemeName};

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);
static THEMES: LazyLock<EmbeddedLazyThemeSet> = LazyLock::new(two_face::theme::extra);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub color: u32,
}

pub fn detect(path: &Path, first_line: &str) -> &'static SyntaxReference {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let known = match filename.to_ascii_lowercase().as_str() {
        "dockerfile" | "containerfile" => Some("Dockerfile"),
        "makefile" | "gnumakefile" => Some("Makefile"),
        "cmakelists.txt" => Some("CMake"),
        _ => None,
    };
    known
        .and_then(|name| SYNTAXES.find_syntax_by_name(name))
        .or_else(|| SYNTAXES.find_syntax_by_extension(filename))
        .or_else(|| {
            path.extension()
                .and_then(|s| s.to_str())
                .and_then(|extension| SYNTAXES.find_syntax_by_extension(extension))
        })
        .or_else(|| SYNTAXES.find_syntax_by_first_line(first_line))
        .unwrap_or_else(|| SYNTAXES.find_syntax_plain_text())
}

pub fn theme(dark: bool) -> &'static Theme {
    THEMES.get(if dark {
        EmbeddedThemeName::Base16OceanDark
    } else {
        EmbeddedThemeName::InspiredGithub
    })
}

pub struct CodeHighlighter {
    inner: HighlightLines<'static>,
}

impl CodeHighlighter {
    pub fn new(path: &Path, first_line: &str, dark: bool) -> Self {
        Self {
            inner: HighlightLines::new(detect(path, first_line), theme(dark)),
        }
    }

    pub fn line(&mut self, code: &str) -> Result<Vec<Span>> {
        let input = format!("{code}\n");
        let mut position = 0;
        let mut spans = Vec::new();
        for (style, text) in self.inner.highlight_line(&input, &SYNTAXES)? {
            let end = (position + text.len()).min(code.len());
            if end > position && spans.len() < 512 {
                let c = style.foreground;
                spans.push(Span {
                    range: position..end,
                    color: (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32,
                });
            }
            position += text.len();
        }
        Ok(spans)
    }
}

pub fn inventory() -> String {
    let mut syntaxes: Vec<_> = SYNTAXES.syntaxes().iter().collect();
    syntaxes.sort_by_key(|syntax| syntax.name.to_lowercase());
    let mut output = format!("{} bundled syntaxes\n", syntaxes.len());
    for syntax in syntaxes {
        output.push_str(&format!(
            "{}\t{}\n",
            syntax.name,
            syntax.file_extensions.join(", ")
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broad_filename_detection_includes_required_and_additional_families() {
        assert!(SYNTAXES.syntaxes().len() > 100);
        for path in [
            "flake.nix",
            "main.rs",
            "main.go",
            "run.sh",
            "data.json",
            "Cargo.toml",
            "main.py",
            "main.js",
            "main.ts",
            "main.cpp",
            "Main.java",
            "index.html",
            "style.css",
            "ci.yaml",
            "query.sql",
            "README.md",
            "app.rb",
            "Dockerfile",
            "Makefile",
        ] {
            assert_ne!(detect(Path::new(path), "").name, "Plain Text", "{path}");
        }
    }

    #[test]
    fn representative_language_families_produce_colored_tokens() {
        for (path, code) in [
            ("flake.nix", "{ pkgs }: pkgs.hello"),
            ("main.rs", "fn main() { let n = 42; }"),
            ("main.go", "func main() { println(42) }"),
            ("run.sh", "echo \"hello $USER\""),
            ("data.json", "{\"answer\": 42}"),
            ("Cargo.toml", "name = \"example\""),
            ("main.py", "def hello(): return 42"),
            ("main.js", "const n = 42;"),
            ("main.ts", "const n: number = 42;"),
            ("main.cpp", "int main() { return 42; }"),
            ("Main.java", "public class Main {}"),
            ("index.html", "<p>hello</p>"),
            ("style.css", "p { color: red; }"),
            ("ci.yaml", "answer: 42"),
            ("query.sql", "SELECT name FROM users;"),
            ("README.md", "# Heading"),
            ("app.rb", "def hello; puts 42; end"),
        ] {
            let spans = CodeHighlighter::new(Path::new(path), code, true)
                .line(code)
                .unwrap();
            assert!(
                spans.iter().any(|span| span.color != 0xc0c5ce),
                "no colored tokens for {path}"
            );
        }
    }

    #[test]
    fn shebangs_work_for_extensionless_files_and_unknowns_remain_plain_text() {
        assert_ne!(
            detect(Path::new("script"), "#!/usr/bin/env python3").name,
            "Plain Text"
        );
        assert_eq!(detect(Path::new("unknown.unknown"), "").name, "Plain Text");
    }

    #[test]
    fn multiline_state_is_kept_and_unicode_spans_have_valid_boundaries() {
        let mut lexer = CodeHighlighter::new(Path::new("main.rs"), "", true);
        lexer.line("/* comment begins").unwrap();
        let comment = lexer.line("still a comment */ let λ = 42;").unwrap();
        assert!(comment.len() > 1);
        for span in comment {
            let text = "still a comment */ let λ = 42;";
            assert!(text.is_char_boundary(span.range.start));
            assert!(text.is_char_boundary(span.range.end));
        }
    }
}
