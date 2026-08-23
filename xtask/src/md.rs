//! The markdown docs are wrapped by comrak so casual edits reflow
//! mechanically: `fmt-md` rewrites them, and `fmt-md --check` (part of
//! `smoke`) fails on drift.

use std::fs;

use anyhow::{Context, Result, bail};
use comrak::{Options, markdown_to_commonmark};

use crate::workspace_root;

const DOCS: &[&str] = &["README.md", "AGENTS.md", "docs/pitfalls.md"];

const WRAP_WIDTH: usize = 80;

fn formatted(source: &str) -> String {
    let mut options = Options::default();
    options.render.width = WRAP_WIDTH;
    markdown_to_commonmark(source, &options)
}

pub fn fmt_md(check: bool) -> Result<()> {
    let mut drift = false;
    for doc in DOCS {
        let path = workspace_root().join(doc);
        let source =
            fs::read_to_string(&path).with_context(|| format!("while attempting to read {doc}"))?;
        let output = formatted(&source);
        if check {
            if source != output {
                println!("{doc}: not wrapped to {WRAP_WIDTH} columns; run `cargo xtask fmt-md`");
                drift = true;
            }
        } else {
            fs::write(&path, output).with_context(|| format!("while attempting to write {doc}"))?;
            println!("{doc}: formatted");
        }
    }
    if drift {
        bail!("markdown docs drift from the formatter");
    }
    Ok(())
}
