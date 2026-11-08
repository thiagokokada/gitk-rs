//! Parsed commit filters shared by the worker and search feedback UI.
use crate::git::Commit;
use anyhow::{Result, ensure};
use std::{fmt, str::FromStr};

/// Supported qualifiers and their user-facing names share one inventory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Qualifier {
    Author,
    Committer,
    Message,
    Hash,
    Path,
    Before,
    After,
    Merge,
}

impl Qualifier {
    pub const ALL: [Self; 8] = [
        Self::Author,
        Self::Committer,
        Self::Message,
        Self::Hash,
        Self::Path,
        Self::Before,
        Self::After,
        Self::Merge,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Author => "author",
            Self::Committer => "committer",
            Self::Message => "message",
            Self::Hash => "hash",
            Self::Path => "path",
            Self::Before => "before",
            Self::After => "after",
            Self::Merge => "merge",
        }
    }

    pub fn syntax(self) -> String {
        let value = match self {
            Self::Before | Self::After => "YYYY-MM-DD",
            Self::Merge => "true|false",
            Self::Author | Self::Committer | Self::Message | Self::Hash | Self::Path => "text",
        };
        format!("{self}:{value}")
    }

    pub const fn help_description(self) -> &'static str {
        match self {
            Self::Author => "Author name or email; Regex enables patterns such as Alice|Bob",
            Self::Committer => "Committer name or email; Regex enables patterns such as Alice|Bob",
            Self::Message => "Commit message substring; with Regex, use ^fix to match its start",
            Self::Hash => "Commit hash substring; with Regex, use ^a1b2 to match its start",
            Self::Path => "Changed path substring or regex; merges compare with their first parent",
            Self::Before => "Author's recorded calendar day strictly before this date",
            Self::After => "Author's recorded calendar day strictly after this date",
            Self::Merge => "Include merges with true; exclude merges with false",
        }
    }

    fn validate(self, value: &str) -> Result<()> {
        ensure!(!value.is_empty(), "{self}: needs a value");
        match self {
            Self::Before | Self::After => {
                ensure!(valid_date(value), "{self}: use a valid YYYY-MM-DD date")
            }
            Self::Merge => ensure!(
                matches!(value, "true" | "false"),
                "{self}: expects true or false"
            ),
            Self::Author | Self::Committer | Self::Message | Self::Hash | Self::Path => (),
        }
        Ok(())
    }
}

impl fmt::Display for Qualifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Qualifier {
    type Err = anyhow::Error;

    fn from_str(name: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|qualifier| qualifier.name().eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Unknown filter {}: ({})",
                    name.to_ascii_lowercase(),
                    Self::ALL.map(Self::name).join(", ")
                )
            })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Query {
    terms: Vec<(Qualifier, String)>,
    text: String,
    pub path: Option<String>,
    pub filters: Vec<String>,
}

impl Query {
    pub fn parse(input: &str) -> Result<Self> {
        Self::parse_with_mode(input, false)
    }

    pub fn parse_with_mode(input: &str, regex: bool) -> Result<Self> {
        ensure!(
            input.len() <= 8192,
            "filter is too long (maximum 8192 bytes)"
        );
        let mut tokens = Vec::new();
        let mut token = String::new();
        let mut quoted = false;
        let mut whole_quoted = false;
        let mut escaped = false;
        for ch in input.chars() {
            if escaped {
                if ch != '"' {
                    token.push('\\');
                }
                token.push(ch);
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => {
                    if token.is_empty() && !quoted {
                        whole_quoted = true;
                    }
                    quoted = !quoted;
                }
                ch if ch.is_whitespace() && !quoted => {
                    if !token.is_empty() {
                        tokens.push((std::mem::take(&mut token), whole_quoted));
                    }
                    whole_quoted = false;
                }
                _ => token.push(ch),
            }
        }
        if escaped {
            token.push('\\');
        }
        ensure!(!quoted, "Close the quoted search value with a double quote");
        if !token.is_empty() {
            tokens.push((token, whole_quoted));
        }
        let mut result = Self::default();
        let mut text = Vec::new();
        for (token, whole_quoted) in tokens {
            let qualifier = if whole_quoted {
                None
            } else {
                token.split_once(':').filter(|(key, _)| {
                    !regex
                        || key
                            .bytes()
                            .all(|byte| byte.is_ascii_alphabetic() || byte == b'-')
                })
            };
            let (key, value) = qualifier.unwrap_or(("", &token));
            if key.is_empty() {
                text.push(value.to_owned());
                continue;
            }
            let qualifier: Qualifier = key.parse()?;
            qualifier.validate(value)?;
            if qualifier == Qualifier::Path {
                ensure!(
                    result.path.is_none(),
                    "Use one {qualifier}: filter per search"
                );
                result.path = Some(value.to_owned());
            } else {
                result.terms.push((qualifier, value.to_owned()));
            }
            result.filters.push(format!("{qualifier}:{value}"));
        }
        result.text = text.join(" ");
        Ok(result)
    }

    /// Compile once on the history worker, never in a UI render path.
    pub fn compile(self, regex: bool) -> Result<CompiledQuery> {
        let pattern_count = self
            .terms
            .iter()
            .filter(|(key, _)| {
                !matches!(key, Qualifier::Before | Qualifier::After | Qualifier::Merge)
            })
            .count()
            + usize::from(self.path.is_some())
            + usize::from(!self.text.is_empty());
        ensure!(
            !regex || pattern_count <= 16,
            "Regex search supports at most 16 text/path patterns"
        );
        let mut predicates = Vec::new();
        for (qualifier, value) in self.terms {
            let pattern = |value: String| {
                Matcher::new(value, regex).map_err(|error| anyhow::anyhow!("{qualifier}: {error}"))
            };
            predicates.push(match qualifier {
                Qualifier::Author => Predicate::Author(pattern(value)?),
                Qualifier::Committer => Predicate::Committer(pattern(value)?),
                Qualifier::Message => Predicate::Message(pattern(value)?),
                Qualifier::Hash => Predicate::Hash(pattern(value)?),
                Qualifier::Before => Predicate::Before(value),
                Qualifier::After => Predicate::After(value),
                Qualifier::Merge => Predicate::Merge(value == "true"),
                Qualifier::Path => continue, // Paths use the separate diff stream.
            });
        }
        Ok(CompiledQuery {
            predicates,
            text: if self.text.is_empty() {
                None
            } else {
                Some(Matcher::new(self.text, regex)?)
            },
            path: self
                .path
                .map(|value| {
                    Matcher::new(value, regex)
                        .map_err(|error| anyhow::anyhow!("{}: {error}", Qualifier::Path))
                })
                .transpose()?,
        })
    }
}

#[derive(Clone, Debug)]
pub enum Matcher {
    Literal(String),
    Regex(regex::Regex),
}

impl Matcher {
    fn new(value: String, regex: bool) -> Result<Self> {
        if !regex {
            return Ok(Self::Literal(value.to_lowercase()));
        }
        let compiled = regex::RegexBuilder::new(&value)
            .case_insensitive(true)
            .size_limit(1024 * 1024)
            .dfa_size_limit(1024 * 1024)
            .nest_limit(64)
            .build()
            .map_err(|error| anyhow::anyhow!("Invalid regular expression: {error}"))?;
        Ok(Self::Regex(compiled))
    }

    pub fn is_match(&self, text: &str) -> bool {
        match self {
            Self::Literal(value) => text.to_lowercase().contains(value),
            Self::Regex(regex) => regex.is_match(text),
        }
    }
}

#[derive(Clone, Debug)]
enum Predicate {
    Author(Matcher),
    Committer(Matcher),
    Message(Matcher),
    Hash(Matcher),
    Before(String),
    After(String),
    Merge(bool),
}

#[derive(Clone, Debug, Default)]
pub struct CompiledQuery {
    predicates: Vec<Predicate>,
    text: Option<Matcher>,
    pub path: Option<Matcher>,
}

impl CompiledQuery {
    /// Paths are resolved separately by the cancellable diff stream.
    pub fn matches(&self, commit: &Commit) -> bool {
        self.predicates.iter().all(|predicate| match predicate {
            Predicate::Author(value) => {
                value.is_match(&commit.author.name) || value.is_match(&commit.author.email)
            }
            Predicate::Committer(value) => {
                value.is_match(&commit.committer.name) || value.is_match(&commit.committer.email)
            }
            Predicate::Message(value) => value.is_match(&commit.message),
            Predicate::Hash(value) => value.is_match(&commit.id.to_hex()),
            Predicate::Before(value) => commit
                .author
                .time
                .get(..10)
                .is_some_and(|day| day < value.as_str()),
            Predicate::After(value) => commit
                .author
                .time
                .get(..10)
                .is_some_and(|day| day > value.as_str()),
            Predicate::Merge(value) => (commit.parents.len() > 1) == *value,
        }) && self
            .text
            .as_ref()
            .is_none_or(|pattern| pattern.is_match(&commit.searchable_text()))
    }
}

fn valid_date(value: &str) -> bool {
    let b = value.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || b.iter()
            .enumerate()
            .any(|(i, b)| i != 4 && i != 7 && !b.is_ascii_digit())
    {
        return false;
    }
    let year: u32 = value[..4].parse().unwrap();
    let month: usize = value[5..7].parse().unwrap();
    let day: u32 = value[8..].parse().unwrap();
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    year > 0 && (1..=12).contains(&month) && day > 0 && day <= days[month - 1]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample_commit(message: &str) -> Commit {
        use crate::git::{CommitId, Signature};
        Commit {
            id: CommitId::parse(&"1".repeat(40)).unwrap(),
            parents: Box::new([]),
            author: Signature {
                name: "Alice Smith".into(),
                email: "alice@example.test".into(),
                time: "2026-01-02T00:30:00+05:00".into(),
            },
            committer: Signature {
                name: "Bob Jones".into(),
                email: "bob@example.test".into(),
                time: "2026-01-03T00:30:00+05:00".into(),
            },
            message: message.into(),
        }
    }

    #[test]
    fn regex_is_opt_in_and_preserves_escapes_case_flags_and_dates() {
        let commit = sample_commit("fix all bugs\nTICKET-42\nsay \"hello\"");
        for (input, regex, expected) in [
            (r"message:fix.*bugs", false, false),
            (r"message:fix.*bugs", true, true),
            (
                r"author:^(Alice|Bob) committer:^Bob hash:^1{40}$",
                true,
                true,
            ),
            (r"author:^Bob", true, false),
            (r"message:fix.*TICKET", true, false),
            (r"message:(?s)fix.*TICKET", true, true),
            (r"message:(?-i)FIX", true, false),
            (r"(?-i)Alice", true, true),
            (r"(?:fix|feat)", true, true),
            (
                r"before:2026-01-03 after:2026-01-01 merge:false",
                true,
                true,
            ),
            (r"before:2026-01-02", true, false),
            (r#"author:"Alice Smith|Bob Jones""#, true, true),
            (r#"message:"say \"hello\"""#, false, true),
            (r#"message:"say \"hello\"""#, true, true),
        ] {
            let query = Query::parse_with_mode(input, regex)
                .unwrap()
                .compile(regex)
                .unwrap();
            assert_eq!(query.matches(&commit), expected, "{input} regex={regex}");
        }
        let whitespace = sample_commit("   ");
        for (pattern, expected) in [(r"message:\S+", false), (r"message:\s+", true)] {
            assert_eq!(
                Query::parse_with_mode(pattern, true)
                    .unwrap()
                    .compile(true)
                    .unwrap()
                    .matches(&whitespace),
                expected
            );
        }
        assert!(
            Query::parse(r#""foo:bar""#)
                .unwrap()
                .compile(false)
                .unwrap()
                .matches(&sample_commit("foo:bar"))
        );
        assert!(
            Query::parse("\"\" author:Alice")
                .unwrap()
                .compile(false)
                .unwrap()
                .matches(&commit)
        );
    }

    #[test]
    fn invalid_or_excessive_regexes_do_not_affect_literal_mode() {
        for input in [
            r"author:[",
            r"path:(?=foo)",
            r"message:(a)\1",
            r"message:a{1000000}",
        ] {
            let error = Query::parse_with_mode(input, true)
                .unwrap()
                .compile(true)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("Invalid regular expression"),
                "{input}: {error}"
            );
            assert!(
                Query::parse(input).unwrap().compile(false).is_ok(),
                "{input}"
            );
        }
        let many = ["author:Alice"; 17].join(" ");
        assert!(Query::parse(&many).unwrap().compile(true).is_err());
        assert!(Query::parse(&many).unwrap().compile(false).is_ok());
        for regex in [false, true] {
            assert!(
                Query::parse_with_mode("abc:", regex)
                    .unwrap_err()
                    .to_string()
                    .starts_with("Unknown filter abc:")
            );
            assert!(Query::parse_with_mode("before:.*", regex).is_err());
        }
    }

    #[test]
    fn advertised_qualifiers_accept_case_insensitive_names_and_canonicalize_badges() {
        for qualifier in Qualifier::ALL {
            let value = match qualifier {
                Qualifier::Before | Qualifier::After => "2026-01-01",
                Qualifier::Merge => "false",
                Qualifier::Author
                | Qualifier::Committer
                | Qualifier::Message
                | Qualifier::Hash
                | Qualifier::Path => "Alice",
            };
            let query = Query::parse(&format!(
                "{}:{value}",
                qualifier.name().to_ascii_uppercase()
            ))
            .unwrap();
            assert_eq!(query.filters, [format!("{qualifier}:{value}")]);
        }
    }

    #[test]
    fn unknown_filters_are_reported_before_missing_values() {
        for input in ["abc:", "abc:value", "ABC:"] {
            let error = Query::parse(input).unwrap_err().to_string();
            assert!(error.starts_with("Unknown filter abc:"), "{input}: {error}");
        }
        for key in Qualifier::ALL {
            let error = Query::parse(&format!("{key}:")).unwrap_err().to_string();
            assert_eq!(error, format!("{key}: needs a value"));
        }
    }

    #[test]
    fn validates_and_explains_filters() {
        let query =
            Query::parse("fix author:\"Alice Smith\" path:ci.yml after:2024-02-29 merge:false")
                .unwrap();
        assert_eq!(query.path.as_deref(), Some("ci.yml"));
        assert_eq!(query.filters.len(), 4);
        for bad in [
            "author:",
            "before:2025-02-29",
            "after:2026-13-01",
            "before:2026-00-01",
            "after:0000-01-01",
            "merge:yes",
            "from:abc",
            "path:a path:b",
            "author:\"abc",
        ] {
            assert!(Query::parse(bad).is_err(), "{bad}");
        }
        assert!(Query::parse("before:2000-02-29").is_ok());
    }
}
