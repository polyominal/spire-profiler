//! The documentation gate: `cargo doc` must be warning-free, and the
//! in-house Rust comment density must stay at or below the AGENTS.md
//! budget.
//!
//! The density metric is comment lines / (comment lines + code lines)
//! over the three in-house Rust roots. A line is a comment when its first
//! non-blank characters begin `//` (doc comments `///` and `//!`
//! included) or `/*`, or when it sits inside a `/* */` block; string
//! literal contents never start a comment, and a line where a block
//! closes and code follows, or that carries a trailing comment, is code.
//! Blank lines are not counted. Exceeding
//! the 15% budget is a warning, not a failure: the figure is a
//! crate-wide floor checked at checkpoints, not a per-file or per-commit
//! gate.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use xshell::{Shell, cmd};

/// The AGENTS.md budget: comment+code density, not code-only.
const DENSITY_LIMIT_PERCENT: f64 = 15.0;

/// Tests included.
const RUST_ROOTS: &[&str] = &["profiler-core/src", "profiler-core/tests", "xtask/src"];

/// Doc-only files hold preserved spec content, not slashable offenders.
const MIN_CODE_LINES: usize = 20;

/// Default length of the offender report when `--top` is absent.
const DEFAULT_TOP_OFFENDERS: usize = 10;

pub fn check_docs(shell: &Shell, top_offenders: Option<usize>) -> Result<()> {
    // A rustdoc warning is a doc bug; -D warnings turns every one (broken
    // links, HTML tags, unresolved names) into a hard failure.
    cmd!(shell, "cargo doc --workspace --no-deps")
        .env("RUSTDOCFLAGS", "-D warnings")
        .run()
        .context("while attempting to run the cargo doc gate")?;

    let mut files = Vec::new();
    for root in RUST_ROOTS {
        let root = crate::workspace_root().join(root);
        collect_files(&root, &mut files)?;
    }
    let comments: usize = files.iter().map(|file| file.comments).sum();
    let code: usize = files.iter().map(|file| file.code).sum();
    let total = comments + code;
    let density = if total == 0 {
        0.0
    } else {
        density_percent(comments, code)
    };
    println!(
        "in-house Rust density: {density:.1}% ({comments} comments / {total} comment+code lines)"
    );
    if density > DENSITY_LIMIT_PERCENT {
        eprintln!("warning: density exceeds the {DENSITY_LIMIT_PERCENT:.0}% budget");
    }
    report_offenders(&files, top_offenders.unwrap_or(DEFAULT_TOP_OFFENDERS));
    Ok(())
}

struct FileCount {
    path: PathBuf,
    comments: usize,
    code: usize,
}

fn collect_files(dir: &Path, out: &mut Vec<FileCount>) -> Result<()> {
    let entries =
        fs::read_dir(dir).with_context(|| format!("while attempting to list {}", dir.display()))?;
    for entry in entries {
        let entry = entry
            .with_context(|| format!("while attempting to read an entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("while attempting to read {}", path.display()))?;
            let (comments, code) = count_lines(&content);
            out.push(FileCount {
                path,
                comments,
                code,
            });
        }
    }
    Ok(())
}

fn density_percent(comments: usize, code: usize) -> f64 {
    let total = comments + code;
    debug_assert!(total > 0);
    100.0 * comments as f64 / total as f64
}

/// Biggest comment contributors first, so drift has a name.
fn report_offenders(files: &[FileCount], top_offenders: usize) {
    let mut files: Vec<&FileCount> = files
        .iter()
        .filter(|file| file.code >= MIN_CODE_LINES)
        .collect();
    files.sort_by_key(|file| std::cmp::Reverse(file.comments));
    for file in files.into_iter().take(top_offenders) {
        println!(
            "  {:>4} comments / {:>4} code lines  {:>5.1}%  {}",
            file.comments,
            file.code,
            density_percent(file.comments, file.code),
            file.path.display()
        );
    }
}

enum LineKind {
    Blank,
    Comment,
    Code,
}

enum StringState {
    Plain,
    Raw { hashes: usize },
}

#[derive(Default)]
struct LineScanner {
    in_block_comment: bool,
    string: Option<StringState>,
}

impl LineScanner {
    fn classify(&mut self, line: &str) -> LineKind {
        let line = line.trim();
        if line.is_empty() {
            return LineKind::Blank;
        }
        // String contents are code and hide every comment marker.
        if self.string.is_some() {
            self.consume_string(line);
            return LineKind::Code;
        }
        if self.in_block_comment {
            return self.consume_block(line);
        }
        if line.starts_with("//") {
            LineKind::Comment
        } else if let Some(rest) = line.strip_prefix("/*") {
            self.consume_block(rest)
        } else {
            self.scan_code(line);
            LineKind::Code
        }
    }

    /// The line is a comment unless the block closes and code follows.
    fn consume_block(&mut self, rest: &str) -> LineKind {
        match rest.find("*/") {
            None => {
                self.in_block_comment = true;
                LineKind::Comment
            }
            Some(close) => {
                self.in_block_comment = false;
                let after = &rest[close + 2..];
                if after.trim().is_empty() {
                    LineKind::Comment
                } else {
                    self.scan_code(after);
                    LineKind::Code
                }
            }
        }
    }

    fn consume_string(&mut self, line: &str) {
        let state = self.string.take().expect("a string is open here");
        match close_string(line, &state) {
            Some(after) => self.scan_code(after),
            None => self.string = Some(state),
        }
    }

    /// A trailing `//` needs no state.
    fn scan_code(&mut self, line: &str) {
        let mut rest = line;
        while !rest.is_empty() {
            if let Some(body) = rest.strip_prefix("/*") {
                match body.find("*/") {
                    None => {
                        self.in_block_comment = true;
                        return;
                    }
                    Some(close) => rest = &body[close + 2..],
                }
            } else if rest.starts_with('\'') {
                rest = skip_char_literal(rest);
            } else if let Some((state, after_open)) = open_string(rest) {
                match close_string(after_open, &state) {
                    None => {
                        self.string = Some(state);
                        return;
                    }
                    Some(after_close) => rest = after_close,
                }
            } else {
                rest = step_char(rest);
            }
        }
    }
}

fn count_lines(content: &str) -> (usize, usize) {
    let mut scanner = LineScanner::default();
    let mut comments = 0;
    let mut code = 0;
    for line in content.lines() {
        match scanner.classify(line) {
            LineKind::Blank => {}
            LineKind::Comment => comments += 1,
            LineKind::Code => code += 1,
        }
    }
    (comments, code)
}

/// Handles `"`, `b"`, `r"`, and `br#"..."#`-style raw strings.
fn open_string(s: &str) -> Option<(StringState, &str)> {
    let bytes = s.as_bytes();
    let mut start = 0;
    if bytes.first() == Some(&b'b') {
        start = 1;
    }
    if bytes.get(start) == Some(&b'"') {
        return Some((StringState::Plain, &s[start + 1..]));
    }
    if bytes.get(start) == Some(&b'r') {
        let hashes = bytes[start + 1..]
            .iter()
            .take_while(|&&byte| byte == b'#')
            .count();
        if bytes.get(start + 1 + hashes) == Some(&b'"') {
            return Some((StringState::Raw { hashes }, &s[start + 2 + hashes..]));
        }
    }
    None
}

fn close_string<'a>(s: &'a str, state: &StringState) -> Option<&'a str> {
    match state {
        StringState::Plain => {
            let bytes = s.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                match bytes[i] {
                    b'\\' => i += 2,
                    b'"' => return Some(&s[i + 1..]),
                    _ => i += 1,
                }
            }
            None
        }
        StringState::Raw { hashes } => {
            let close = format!("\"{}", "#".repeat(*hashes));
            s.find(&close).map(|i| &s[i + close.len()..])
        }
    }
}

/// A lifetime (`'a`) has no closing quote and only advances past the
/// apostrophe.
fn skip_char_literal(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.get(1) == Some(&b'\\') && bytes.get(3) == Some(&b'\'') {
        &s[4..]
    } else if bytes.get(2) == Some(&b'\'') {
        &s[3..]
    } else {
        &s[1..]
    }
}

/// Markers are ASCII, so multibyte chars are skipped whole.
fn step_char(s: &str) -> &str {
    let len = s.chars().next().map_or(0, char::len_utf8);
    &s[len..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_and_plain_comments_count_as_comments() {
        let src = "//! module doc\n/// item doc\n// plain\npub fn f() {}\n";
        assert_eq!(count_lines(src), (3, 1));
    }

    #[test]
    fn strings_hide_comment_markers() {
        let src = "let s = \"// not a comment\";\nlet r = r#\"/* also not */\"#;\n// real\n";
        assert_eq!(count_lines(src), (1, 2));
    }

    #[test]
    fn block_comments_span_lines() {
        let src = "/* start\nmiddle\n*/ after\n// tail\n";
        assert_eq!(count_lines(src), (3, 1));
    }

    #[test]
    fn trailing_comments_leave_the_line_code() {
        let src = "let x = 1; // trailing\n// whole\n";
        assert_eq!(count_lines(src), (1, 1));
    }

    #[test]
    fn char_literal_holding_a_quote_does_not_leak_state() {
        let src = "let c = '\"';\n// comment\n";
        assert_eq!(count_lines(src), (1, 1));
    }

    #[test]
    fn multi_line_raw_string_is_code() {
        let src = "let s = r#\"\n// inside a string\n\"#;\n";
        assert_eq!(count_lines(src), (0, 3));
    }
}
