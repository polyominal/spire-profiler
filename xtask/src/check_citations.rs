//! The citation gate: in-house text names methods, never `file:line`
//! positions — game line numbers move between builds and silently rot.
//! Vendored upstream trees are exempt; every other UTF-8 file is scanned.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::workspace_root;

/// Trees that hold upstream text the project does not author.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "dist",
    "dotnet-sdk",
    "zig-sdk",
    "tmp",
    "tools",
    "vendor",
];

/// Extensions a citation may carry; matches the source types we write.
const CITATION_EXTENSIONS: &[&str] = &[
    ".c", ".cs", ".h", ".json", ".md", ".rs", ".sh", ".snap", ".toml", ".txt",
];

pub fn run() -> Result<()> {
    let mut files = Vec::new();
    collect_text(workspace_root(), &mut files)?;
    let mut hits = Vec::new();
    for (path, content) in &files {
        for (index, line) in content.lines().enumerate() {
            if let Some(column) = citation_column(line) {
                hits.push((path, index + 1, column + 1, line));
            }
        }
    }
    if hits.is_empty() {
        println!("no file:line citations");
        return Ok(());
    }
    for (path, line, column, text) in &hits {
        eprintln!(
            "check-citations: ERROR: {}:{line}:{column}: {text}",
            path.display()
        );
    }
    bail!(
        "{} file:line citation(s): name the method and game version instead",
        hits.len()
    );
}

fn collect_text(dir: &Path, out: &mut Vec<(PathBuf, String)>) -> Result<()> {
    let entries =
        fs::read_dir(dir).with_context(|| format!("while attempting to list {}", dir.display()))?;
    for entry in entries {
        let entry = entry
            .with_context(|| format!("while attempting to read an entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if !SKIP_DIRS.contains(&name.to_string_lossy().as_ref()) {
                collect_text(&path, out)?;
            }
        } else if let Ok(content) = fs::read_to_string(&path) {
            out.push((path, content));
        }
    }
    Ok(())
}

/// The 0-based column of the first `file:line` citation, if any.
fn citation_column(line: &str) -> Option<usize> {
    for (colon, _) in line.match_indices(':') {
        let after = &line[colon + 1..];
        let digits = after.bytes().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 {
            continue;
        }
        if after.as_bytes().get(digits) == Some(&b'-') {
            let range_digits = after[digits + 1..]
                .bytes()
                .take_while(|b| b.is_ascii_digit())
                .count();
            if range_digits == 0 {
                continue;
            }
        }
        let token_start = line[..colon]
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/')))
            .map_or(0, |i| i + 1);
        let token = &line[token_start..colon];
        if CITATION_EXTENSIONS.iter().any(|ext| token.ends_with(ext)) {
            return Some(token_start);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::citation_column;

    #[test]
    fn catches_simple_line_and_range_citations() {
        let simple = "// {}:297".replace("{}", "PotionModel.cs");
        assert_eq!(citation_column(&simple), Some(3));
        let ranged = "/// `{}:212-227`".replace("{}", "NRunHistory.cs");
        assert_eq!(citation_column(&ranged), Some(5));
    }

    #[test]
    fn ignores_methods_urls_times_and_resource_paths() {
        for line in [
            "// CombatManager.StartTurn",
            "https://github.com/dotnet/install-scripts/issues",
            "12:30",
            "res://themes/kreon_bold_glyph_space_two.tres",
            "C:\\Program Files (x86)",
        ] {
            assert_eq!(citation_column(line), None, "{line}");
        }
    }
}
