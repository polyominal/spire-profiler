//! The Rust source line classifier: comment, code, or blank, with
//! string and char literals hiding comment markers. Shared machinery
//! for the content gates.

pub(crate) enum LineKind {
    Blank,
    Comment,
    Code,
}

enum StringState {
    Plain,
    Raw { hashes: usize },
}

#[derive(Default)]
pub(crate) struct LineScanner {
    in_block_comment: bool,
    string: Option<StringState>,
}

impl LineScanner {
    pub(crate) fn classify(&mut self, line: &str) -> LineKind {
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

    fn scan_code(&mut self, line: &str) {
        let mut rest = line;
        while !rest.is_empty() {
            if rest.starts_with("//") {
                return;
            } else if let Some(body) = rest.strip_prefix("/*") {
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

pub(crate) fn count_lines(content: &str) -> (usize, usize) {
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
        for trailing in ["trailing", "\"", "r#\"", "/*", "*/"] {
            let src = format!("let x = 1; // {trailing}\n// whole\nlet y = 2;\n");
            assert_eq!(count_lines(&src), (1, 2), "trailing comment: {trailing}");
        }
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
