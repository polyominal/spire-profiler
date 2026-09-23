//! The citation gate: in-house text names methods, never `file:line`
//! positions: game line numbers move between builds and silently rot.
//! Build artifacts and tool caches are excluded; other UTF-8 files are scanned.

use std::fs;
use std::path::Path;

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
];

/// Extensions a citation may carry: the source types we write plus the
/// game's scene, resource, and script files.
const CITATION_EXTENSIONS: &[&str] = &[
    ".c", ".cs", ".gd", ".h", ".json", ".md", ".rs", ".sh", ".snap", ".toml", ".tres", ".tscn",
    ".txt",
];

pub fn run() -> Result<()> {
    let mut hits = Vec::new();
    collect_citations(workspace_root(), &mut hits)?;
    if hits.is_empty() {
        println!("no file:line citations");
        return Ok(());
    }
    for hit in &hits {
        eprintln!("check-citations: ERROR: {hit}");
    }
    bail!(
        "{} file:line citation(s): name the method and game version instead",
        hits.len()
    );
}

fn collect_citations(dir: &Path, out: &mut Vec<String>) -> Result<()> {
    let entries =
        fs::read_dir(dir).with_context(|| format!("while attempting to list {}", dir.display()))?;
    for entry in entries {
        let entry = entry
            .with_context(|| format!("while attempting to read an entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if !SKIP_DIRS.contains(&name.to_string_lossy().as_ref()) {
                collect_citations(&path, out)?;
            }
        } else if let Ok(content) = fs::read_to_string(&path) {
            for (index, line) in content.lines().enumerate() {
                if let Some(column) = citation_column(line) {
                    out.push(format!(
                        "{}:{}:{}: {line}",
                        path.display(),
                        index + 1,
                        column + 1
                    ));
                }
            }
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
    use super::*;

    #[test]
    fn traversal_keeps_first_hit_per_line_and_skips_non_authored_text() -> Result<()> {
        let scratch_root = workspace_root().join("tmp/xtask-citation-tests");
        fs::create_dir_all(&scratch_root)?;
        let mut serial = 0_u32;
        let scratch = loop {
            let candidate = scratch_root.join(format!("{}-{serial}", std::process::id()));
            match fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    serial = serial
                        .checked_add(1)
                        .context("test directory serial exhausted")?;
                }
                Err(error) => return Err(error.into()),
            }
        };
        let nested = scratch.join("nested");
        fs::create_dir(&nested)?;
        let first = "first.cs";
        let second = "second.rs";
        let third = "third.gd";
        let content = format!("plain\n// {first}:12 and {second}:99\né // {third}:20-24\r\n");
        let authored = nested.join("notes.bin");
        fs::write(&authored, &content)?;
        fs::write(scratch.join("clean.md"), "CombatManager.StartTurn\n")?;
        let mut invalid_utf8 = content.as_bytes().to_vec();
        invalid_utf8.push(0xff);
        fs::write(nested.join("invalid.rs"), invalid_utf8)?;
        for skipped in SKIP_DIRS {
            let path = nested.join(skipped);
            fs::create_dir(&path)?;
            fs::write(path.join("upstream.md"), &content)?;
        }
        let mut hits = Vec::new();
        collect_citations(&scratch, &mut hits)?;
        assert_eq!(
            hits,
            [
                format!("{}:2:4: // {first}:12 and {second}:99", authored.display()),
                format!("{}:3:7: é // {third}:20-24", authored.display()),
            ]
        );
        fs::remove_dir_all(scratch)?;
        Ok(())
    }

    #[test]
    fn catches_simple_line_and_range_citations() {
        let simple = "// {}:297".replace("{}", "PotionModel.cs");
        assert_eq!(citation_column(&simple), Some(3));
        let ranged = "/// `{}:212-227`".replace("{}", "NRunHistory.cs");
        assert_eq!(citation_column(&ranged), Some(5));
    }

    #[test]
    fn catches_scene_resource_and_script_citations() {
        // Built at runtime so this source holds no citable `name:line`.
        for (stem, suffix) in [
            ("foo.tscn", ":28"),
            ("enemy.gd", ":120-130"),
            ("ui_theme.tres", ":5"),
        ] {
            let line = format!("// {stem}{suffix}");
            assert!(citation_column(&line).is_some(), "{line}");
        }
    }

    #[test]
    fn ignores_methods_urls_times_and_resource_paths() {
        for line in [
            "// CombatManager.StartTurn",
            "https://github.com/dotnet/install-scripts/issues",
            "12:30",
            "res://themes/kreon_bold_glyph_space_two.tres",
            "res://ui/run.tscn",
            "res://scripts/enemy.gd",
            "C:\\Program Files (x86)",
        ] {
            assert_eq!(citation_column(line), None, "{line}");
        }
    }
}
