//! Test-only helpers shared by the crate's test modules and the
//! integration tests (the latter link with the `test-support` feature).
//! Everything is `pub`: under the feature build `cfg(test)` is off, so
//! `pub(crate)` items would be dead code.

use std::fs;
use std::path::{Path, PathBuf};

use crate::engine::object::TextAlign;
use crate::source_kind::SourceKind;
use crate::ui::chart_layout::Cmd;
use crate::ui::theme::ContentBox;
use crate::ui::ui_model::{SEG_COUNT, Section, UiRow};

/// A fresh dir under the gitignored tmp/ (wiped first, so a crashed run
/// cannot leak state).
pub fn scratch_dir(label: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("profiler-core is a workspace member")
        .join("tmp")
        .join(label);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Unique per process and call, so parallel runs never collide.
pub fn temp_dir(label: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("spire-profiler-{label}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// A synthetic UiRow; the name truncates to the fixed 64-byte field.
#[allow(clippy::too_many_arguments)] // fixture: the args mirror UiRow's flat field shape
pub fn test_row(
    section: Section,
    kind: SourceKind,
    flags: u8,
    name: &str,
    plays: u32,
    value: i64,
    share_x10: i32,
    segs: [u16; SEG_COUNT],
) -> UiRow {
    let name_len = name.len().min(64);
    let mut name_bytes = [0u8; 64];
    name_bytes[..name_len].copy_from_slice(&name.as_bytes()[..name_len]);
    UiRow {
        section,
        kind,
        player: 0,
        flags,
        name_len: name_len as u8,
        plays,
        value,
        share_x10,
        seg_milli: segs,
        name: name_bytes,
    }
}

pub fn cmd_texts(cmds: &[Cmd]) -> impl Iterator<Item = &str> {
    cmds.iter().filter_map(|cmd| match cmd {
        Cmd::Text(text) => Some(text.text.as_str()),
        Cmd::Rect(_) | Cmd::Texture(_) => None,
    })
}

/// Every command lies in its zone and inside the content box; `chrome`
/// lists the deliberate exceptions. `strip_h` is the floating tab
/// strip's band above the plate: header commands may sit there (the
/// content box's top already includes it).
pub fn assert_layout_bounds(
    header: &[Cmd],
    body: &[Cmd],
    content: ContentBox,
    header_bottom: f32,
    height: f32,
    strip_h: f32,
    chrome: &[Cmd],
) {
    for cmd in header {
        // The floating strip starts at the Control's top edge; without a
        // strip the header band begins at the plate's inner top.
        let header_lo = if strip_h > 0.0 { 0.0 } else { content.top };
        check_cmd(cmd, content, header_lo, header_bottom, chrome);
    }
    for cmd in body {
        check_cmd(cmd, content, header_bottom, height, chrome);
    }
}

fn check_cmd(cmd: &Cmd, content: ContentBox, y_lo: f32, y_hi: f32, chrome: &[Cmd]) {
    const EPS: f32 = 0.01;
    if chrome.contains(cmd) {
        return;
    }
    match cmd {
        Cmd::Rect(r) => {
            assert!(
                r.x >= content.x - EPS && r.x + r.w <= content.right() + EPS,
                "rect {r:?} escapes the content box {content:?}"
            );
            assert!(
                r.y >= y_lo - EPS && r.y + r.h <= y_hi + EPS,
                "rect {r:?} escapes its zone's y-band [{y_lo}, {y_hi}]"
            );
        }
        Cmd::Texture(t) => {
            assert!(
                t.x >= content.x - EPS && t.x + t.w <= content.right() + EPS,
                "texture {t:?} escapes the content box {content:?}"
            );
            assert!(
                t.y >= y_lo - EPS && t.y + t.h <= y_hi + EPS,
                "texture {t:?} escapes its zone's y-band [{y_lo}, {y_hi}]"
            );
        }
        Cmd::Text(t) => {
            assert!(
                t.x >= content.x - EPS && t.x <= content.right() + EPS,
                "text {t:?} origin escapes the content box {content:?}"
            );
            // The alignment box bounds where the glyphs end; unconstrained
            // left-aligned extents stay unverifiable without font metrics.
            if let TextAlign::Right(w) | TextAlign::Center(w) | TextAlign::LeftClipped(w) = t.align
            {
                assert!(
                    t.x + w <= content.right() + EPS,
                    "aligned text {t:?} box escapes the content box {content:?}"
                );
            }
            assert!(
                t.y >= y_lo - EPS && t.y <= y_hi + EPS,
                "text {t:?} baseline escapes its zone's y-band [{y_lo}, {y_hi}]"
            );
        }
    }
}

/// Stable line-per-command text for insta snapshots; a geometry change
/// reviews as a diff of this text.
pub fn dump_cmds(cmds: &[Cmd]) -> String {
    let mut out = String::new();
    for cmd in cmds {
        match cmd {
            Cmd::Rect(r) => {
                out.push_str(&format!(
                    "rect x={:.1} y={:.1} w={:.1} h={:.1} {}\n",
                    r.x,
                    r.y,
                    r.w,
                    r.h,
                    hex(r.color)
                ));
            }
            Cmd::Texture(t) => {
                out.push_str(&format!(
                    "icon x={:.1} y={:.1} w={:.1} h={:.1} {:?}\n",
                    t.x, t.y, t.w, t.h, t.icon
                ));
            }
            Cmd::Text(t) => {
                let align = match t.align {
                    TextAlign::Right(w) => format!(" align=right w={w:.1}"),
                    TextAlign::Center(w) => format!(" align=center w={w:.1}"),
                    TextAlign::LeftClipped(w) => format!(" align=leftclip w={w:.1}"),
                    TextAlign::Left => String::new(),
                };
                out.push_str(&format!(
                    "text x={:.1} y={:.1} size={} role={:?}{}{}{} {} {:?}\n",
                    t.x,
                    t.y,
                    t.size,
                    t.role,
                    if t.shadow { " shadow" } else { "" },
                    if t.outline { " outline" } else { "" },
                    align,
                    hex(t.color),
                    t.text
                ));
            }
        }
    }
    out
}

/// The pinned header, a marker line, then the scrolling body; a command
/// landing in the wrong zone reviews as a diff.
pub fn dump_layout(header: &[Cmd], body: &[Cmd]) -> String {
    format!(
        "-- header --\n{}-- body --\n{}",
        dump_cmds(header),
        dump_cmds(body)
    )
}

/// #RRGGBBAA for the snapshot dump.
fn hex(c: [f32; 4]) -> String {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        ch(c[0]),
        ch(c[1]),
        ch(c[2]),
        ch(c[3])
    )
}
