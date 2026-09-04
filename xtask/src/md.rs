//! The markdown docs are wrapped by comrak so casual edits reflow
//! mechanically: `fmt-md` rewrites them, and `fmt-md --check` (part of
//! `smoke`) fails on drift.

use std::fs;

use anyhow::{Context, Result, bail};
use comrak::{Options, markdown_to_commonmark};

use crate::workspace_root;

const DOCS: &[&str] = &[
    "README.md",
    "AGENTS.md",
    "docs/build.md",
    "docs/verify.md",
    "docs/gdextension.md",
    "docs/game.md",
];

const WRAP_WIDTH: usize = 80;

fn formatted(source: &str) -> String {
    let mut options = Options::default();
    options.render.width = WRAP_WIDTH;
    markdown_to_commonmark(source, &options)
}

pub fn fmt_md(check: bool) -> Result<()> {
    let read_formatted = |doc: &str| -> Result<(String, String)> {
        let path = workspace_root().join(doc);
        let source =
            fs::read_to_string(&path).with_context(|| format!("while attempting to read {doc}"))?;
        let output = formatted(&source);
        Ok((source, output))
    };
    if check {
        let mut drift = false;
        for doc in DOCS {
            let (source, output) = read_formatted(doc)?;
            if source != output {
                eprintln!(
                    "fmt-md: ERROR: {doc} is not wrapped to {WRAP_WIDTH} columns; run \
                     `cargo xtask fmt-md`"
                );
                drift = true;
            }
        }
        if drift {
            bail!("markdown docs drift from the formatter");
        }
    } else {
        for doc in DOCS {
            let (_, output) = read_formatted(doc)?;
            fs::write(workspace_root().join(doc), output)
                .with_context(|| format!("while attempting to write {doc}"))?;
            println!("{doc}: formatted");
        }
    }
    Ok(())
}
