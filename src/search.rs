//! Parsed commit filters shared by the worker and search feedback UI.
use crate::git::Commit;
use anyhow::{Result, bail, ensure};

#[derive(Clone, Debug, Default)]
pub struct Query {
    terms: Vec<(String, String)>,
    pub path: Option<String>,
    pub filters: Vec<String>,
}

impl Query {
    pub fn parse(input: &str) -> Result<Self> {
        ensure!(
            input.len() <= 8192,
            "filter is too long (maximum 8192 bytes)"
        );
        let mut tokens = Vec::new();
        let mut token = String::new();
        let mut quoted = false;
        for ch in input.chars() {
            match ch {
                '"' => quoted = !quoted,
                ch if ch.is_whitespace() && !quoted => {
                    if !token.is_empty() {
                        tokens.push(std::mem::take(&mut token));
                    }
                }
                _ => token.push(ch),
            }
        }
        ensure!(!quoted, "Close the quoted search value with a double quote");
        if !token.is_empty() {
            tokens.push(token);
        }
        let mut result = Self::default();
        let mut text = Vec::new();
        for token in tokens {
            let (key, value) = token.split_once(':').unwrap_or(("", &token));
            let key = key.to_ascii_lowercase();
            if key.is_empty() {
                text.push(value.to_lowercase());
                continue;
            }
            match key.as_str() {
                "author" | "committer" | "message" | "hash" | "path" | "before" | "after"
                | "merge"
                    if value.is_empty() =>
                {
                    bail!("{key}: needs a value")
                }
                "path" => {
                    ensure!(result.path.is_none(), "Use one path: filter per search");
                    result.path = Some(value.to_owned());
                }
                "before" | "after" => {
                    ensure!(valid_date(value), "{key}: use a valid YYYY-MM-DD date")
                }
                "merge" => ensure!(
                    matches!(value, "true" | "false"),
                    "merge: expects true or false"
                ),
                "author" | "committer" | "message" | "hash" => (),
                _ => bail!(
                    "Unknown filter {key}: (author, committer, message, hash, path, before, after, merge)"
                ),
            }
            result.filters.push(format!("{key}:{value}"));
            if key != "path" {
                result.terms.push((key, value.to_lowercase()));
            }
        }
        if !text.is_empty() {
            result.terms.push((String::new(), text.join(" ")));
        }
        Ok(result)
    }

    /// Paths are resolved separately by one cancellable Git traversal.
    pub fn matches(&self, commit: &Commit) -> bool {
        self.terms.iter().all(|(key, value)| {
            match key.as_str() {
                "author" => {
                    contains(&commit.author.name, value) || contains(&commit.author.email, value)
                }
                "committer" => {
                    contains(&commit.committer.name, value)
                        || contains(&commit.committer.email, value)
                }
                "message" => contains(&commit.message, value),
                "hash" => commit.id.to_hex().contains(value),
                // ISO dates compare lexically. Use the author's recorded calendar day,
                // retaining its timezone rather than silently applying the local timezone.
                "before" => commit
                    .author
                    .time
                    .get(..10)
                    .is_some_and(|day| day < value.as_str()),
                "after" => commit
                    .author
                    .time
                    .get(..10)
                    .is_some_and(|day| day > value.as_str()),
                "merge" => (commit.parents.len() > 1) == (value == "true"),
                _ => commit.searchable_text().contains(value),
            }
        })
    }
}

fn contains(text: &str, needle: &str) -> bool {
    text.to_lowercase().contains(needle)
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
    #[test]
    fn unknown_filters_are_reported_before_missing_values() {
        for input in ["abc:", "abc:value", "ABC:"] {
            let error = Query::parse(input).unwrap_err().to_string();
            assert!(error.starts_with("Unknown filter abc:"), "{input}: {error}");
        }
        for key in [
            "author",
            "committer",
            "message",
            "hash",
            "path",
            "before",
            "after",
            "merge",
        ] {
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
