//! Test-only helpers shared by the crate's test modules and the
//! integration tests (the latter link with the `test-support` feature).
//! Filesystem fixtures are freshly reserved under the gitignored tmp/unique/
//! and retained for inspection (delete freely when it grows).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::{fmt, fs, io};

use crate::data::persistence::{bind_log_path, event_log};
use crate::engine::object::TextAlign;
use crate::source_kind::SourceKind;
use crate::ui::chart_layout::Cmd;
use crate::ui::theme::ContentBox;
use crate::ui::ui_model::{SEG_COUNT, Section, UiRow};
use crate::{fail, marker, warn};

/// A fresh empty directory, exclusively reserved even after PID reuse.
pub fn unique_dir(label: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("profiler-core is a workspace member")
        .join("tmp/unique");
    reserve_dir(&root, label, &COUNTER).expect("fixture directory must be freshly reserved")
}

fn reserve_dir(root: &Path, label: &str, counter: &AtomicU32) -> io::Result<PathBuf> {
    let prefix = root.join(label);
    let parent = prefix
        .parent()
        .expect("fixture labels have a workspace root");
    fs::create_dir_all(parent).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("creating fixture parent {}: {err}", parent.display()),
        )
    })?;
    loop {
        let n = counter.fetch_add(1, Ordering::Relaxed);
        let dir = root.join(format!("{label}-{}-{n}", std::process::id()));
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(io::Error::new(
                    err.kind(),
                    format!("reserving fixture {}: {err}", dir.display()),
                ));
            }
        }
    }
}

/// The combat store under `runs_dir` as (run id, combat id) pairs, in
/// combat-id order; a non-numeric run dir or a file without an
/// `<id>.json` name is skipped.
pub fn combat_ids(runs_dir: &Path) -> Vec<(u32, u32)> {
    let mut ids: Vec<(u32, u32)> = Vec::new();
    for run in fs::read_dir(runs_dir).expect("runs dir exists").flatten() {
        let run_dir = run.path();
        if !run_dir.is_dir() {
            continue;
        }
        let Ok(run_id) = run_dir
            .file_name()
            .expect("run dir has a name")
            .to_string_lossy()
            .parse::<u32>()
        else {
            continue;
        };
        for entry in fs::read_dir(&run_dir).expect("run dir exists").flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            let Ok(id) = stem.parse::<u32>() else {
                continue;
            };
            ids.push((run_id, id));
        }
    }
    ids.sort_unstable_by_key(|&(_, id)| id);
    ids
}

pub fn emit_allocation_probe(literal: &str, integer: i64, path: &Path, err: &io::Error) {
    fail!("{literal}");
    warn!("integer {integer}");
    marker!("path {}", path.display());
    fail!(
        "io {}: {} (os error {})",
        literal,
        err.kind(),
        err.raw_os_error().unwrap_or(-1)
    );
}

pub fn emit_event_log_probe(args: fmt::Arguments<'_>) {
    event_log!("{}", args);
}

pub fn bind_event_log_probe(path: &Path) {
    bind_log_path(path);
}

/// Runs the pure combat-finalization staging step without persistence I/O.
pub fn finish_combat_for_allocation_probe(
    combat_seq: u64,
) -> Option<crate::data::events::FinishedCombat> {
    crate::data::state::STATE
        .with(|cell| cell.borrow_mut().finished_combat(combat_seq))
        .then(crate::data::events::FinishedCombat::take)
        .flatten()
}

pub fn text<const N: usize>(value: &str) -> crate::data::state::Text<N> {
    value
        .try_into()
        .expect("fixture text must fit the supported input policy")
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
    // The floating strip starts at the Control's top edge; without a
    // strip the header band begins at the plate's inner top.
    let header_lo = if strip_h > 0.0 { 0.0 } else { content.top };
    for cmd in header {
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

pub fn combat_epoch() -> u64 {
    crate::data::state::STATE.with(|cell| {
        u64::from(
            cell.borrow()
                .current
                .as_ref()
                .expect("fixture combat exists")
                .seq,
        )
    })
}

pub struct SourceFixture {
    epoch: u64,
    instance: u64,
    id: String,
    slot: i32,
    generation: i32,
    role: i32,
    segment: i32,
    shares: Vec<(u64, u64)>,
}

impl SourceFixture {
    fn identity() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .expect("fixture identities fit the u64 counter")
    }

    pub fn card(id: &str, slot: i32) -> Self {
        Self::capture(id, slot, Self::identity(), 1, 0, 0, 1, 0)
    }

    pub fn generated(id: &str, slot: i32, instance: u64) -> Self {
        Self::capture(id, slot, instance, 1, 0, 1, 1, 0)
    }

    pub fn relic(id: &str, slot: i32) -> Self {
        Self::capture(id, slot, 0, 4, 1, 0, 3, 0)
    }

    pub fn power(instance: u64) -> Self {
        Self::capture("", 4, instance, 2, 2, 0, 2, 1)
    }

    #[allow(clippy::too_many_arguments)]
    fn capture(
        id: &str,
        slot: i32,
        instance: u64,
        capture: i32,
        kind: i32,
        generation: i32,
        role: i32,
        segment: i32,
    ) -> Self {
        use crate::data::events;
        let epoch = combat_epoch();
        let transfer = events::source_capture(epoch, capture, instance, id, kind, slot, generation);
        assert_ne!(transfer, 0);
        let count = events::source_count(transfer);
        assert!(count > 0);
        let shares = (0..count)
            .map(|index| {
                (
                    events::source_destination(transfer, index),
                    events::source_weight(transfer, index),
                )
            })
            .collect();
        assert_eq!(events::source_transfer_release(transfer), 1);
        Self {
            epoch,
            instance,
            id: id.to_owned(),
            slot,
            generation,
            role,
            segment,
            shares,
        }
    }

    pub fn with_transfer<T>(&self, operation: impl FnOnce(u64) -> T) -> T {
        use crate::data::events;
        struct Lease(u64);
        impl Drop for Lease {
            fn drop(&mut self) {
                events::source_transfer_release(self.0);
            }
        }
        let transfer = events::source_transfer_begin(self.epoch);
        assert_ne!(transfer, 0);
        let lease = Lease(transfer);
        for (destination, weight) in &self.shares {
            assert_eq!(
                events::source_transfer_add(transfer, *destination, *weight),
                1
            );
        }
        assert_eq!(events::source_transfer_seal(transfer), 1);
        let result = operation(transfer);
        drop(lease);
        result
    }

    pub fn play(&self) -> u64 {
        self.with_transfer(|source| {
            crate::data::events::card_play_started(
                self.epoch,
                Self::identity(),
                self.instance,
                &self.id,
                self.slot,
                0,
                1,
                self.generation,
                source,
            )
        })
    }

    pub fn finish(&self, play: u64) {
        assert_eq!(crate::data::events::card_play_finished(play), 1);
    }

    pub fn hit(&self, total: i32, blocked: i32, kind: i32, receiver: i32) {
        use crate::data::events;
        let calculation = self.with_transfer(|source| {
            events::damage_calculation_begin(self.epoch, source, self.role, self.segment, 999)
        });
        assert_ne!(calculation, 0);
        assert_eq!(
            events::damage_result_append(
                calculation,
                total,
                total - blocked,
                blocked,
                kind,
                receiver,
                0
            ),
            1
        );
        assert_eq!(events::damage_calculation_commit(calculation), 1);
    }

    pub fn deal(&self, total: i32, blocked: i32) {
        self.hit(total, blocked, 0, 4);
    }

    pub fn block(&self, amount: i32, receiver: i32) {
        assert_eq!(
            self.with_transfer(|source| crate::data::events::block_gained(
                self.epoch, amount, source, receiver
            )),
            1
        );
    }

    pub fn forge(&self, amount: i32) {
        assert_eq!(
            self.with_transfer(|source| crate::data::events::forge(self.epoch, source, amount)),
            1
        );
    }

    pub fn generate(&self, instance: u64) {
        assert_eq!(
            self.with_transfer(|source| crate::data::events::card_generated(
                self.epoch, instance, source, self.role
            )),
            1
        );
    }

    pub fn contribution(&self, calculation: u64, amount: i32) {
        assert_eq!(
            self.with_transfer(|source| crate::data::events::damage_modifier_contribution(
                calculation,
                source,
                amount
            )),
            1
        );
    }
}

pub fn fail_lifecycle_initialization() {
    crate::data::state::FAIL_LIFECYCLE_INITIALIZATION.with(|fail| fail.set(true));
}

pub fn lifecycle_storage_snapshot() -> [Option<(usize, usize)>; 9] {
    crate::data::state::STATE.with(|cell| {
        let state = cell.borrow();
        [
            Some((
                state.current.storage.cards.as_ptr() as usize,
                state.current.storage.cards.capacity(),
            )),
            Some((
                state.current.storage.players.as_ptr() as usize,
                state.current.storage.players.capacity(),
            )),
            Some((
                state.run_ctx.storage.players.as_ptr() as usize,
                state.run_ctx.storage.players.capacity(),
            )),
            Some((
                state.run_cards.as_ptr() as usize,
                state.run_cards.capacity(),
            )),
            Some((
                state.per_player.as_ptr() as usize,
                state.per_player.capacity(),
            )),
            state
                .finish_combat
                .as_ref()
                .map(|c| (c.cards.as_ptr() as usize, c.cards.capacity())),
            state
                .finish_combat
                .as_ref()
                .map(|c| (c.players.as_ptr() as usize, c.players.capacity())),
            state.finish_run.as_ref().map(|r| {
                (
                    r.context.players.as_ptr() as usize,
                    r.context.players.capacity(),
                )
            }),
            state
                .self_test_leases
                .as_ref()
                .map(|l| (l.as_ptr() as usize, l.capacity())),
        ]
    })
}

pub fn replace_run_for_allocation_probe(
    run: &crate::data::state::RunSnapshot,
    players: &[crate::data::state::RunPlayer],
) -> bool {
    crate::data::state::STATE.with(|cell| cell.borrow_mut().replace_run(run, players))
}

pub fn finish_run_for_allocation_probe(
    outcome: crate::data::state::RunOutcome,
) -> Option<crate::data::events::FinishedRun> {
    crate::data::events::FinishedRun::take(outcome)
}

pub fn snapshot_allocation_fixture() -> impl FnMut() {
    crate::data::state::State::snapshot_allocation_fixture()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occupied_candidates_keep_their_contents() {
        let root = unique_dir("fixture-occupied");
        let parent = root.join("nested");
        fs::create_dir(&parent).expect("the isolated fixture parent is writable");
        let stale = parent.join(format!("reused-{}-0", std::process::id()));
        fs::create_dir(&stale).expect("the first candidate is unoccupied");
        fs::write(stale.join("sentinel"), b"stale run-43").expect("the stale fixture is writable");
        let occupied_file = parent.join(format!("reused-{}-1", std::process::id()));
        fs::write(&occupied_file, b"unrelated file").expect("the next candidate is writable");

        let fresh = reserve_dir(&root, "nested/reused", &AtomicU32::new(0))
            .expect("occupied candidates must be skipped");
        assert_eq!(
            fresh,
            parent.join(format!("reused-{}-2", std::process::id()))
        );
        assert!(
            fs::read_dir(&fresh)
                .expect("the fixture exists")
                .next()
                .is_none()
        );
        assert_eq!(
            fs::read(stale.join("sentinel")).expect("the stale sentinel must survive"),
            b"stale run-43"
        );
        assert_eq!(
            fs::read(&occupied_file).expect("the occupied file must survive"),
            b"unrelated file"
        );
    }

    #[test]
    fn concurrent_same_label_reservations_keep_their_sentinels() {
        let root = unique_dir("fixture-concurrent");
        let dirs = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|id| {
                    let root = &root;
                    scope.spawn(move || {
                        // Independent counters force contention on the same candidates.
                        let dir = reserve_dir(root, "same-label", &AtomicU32::new(0))
                            .expect("each caller must reserve its own fixture");
                        fs::write(dir.join("sentinel"), id.to_string())
                            .expect("the reserved fixture is writable");
                        (dir, id)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .expect("fixture setup must succeed in every thread")
                })
                .collect::<Vec<_>>()
        });
        for (dir, id) in dirs {
            assert_eq!(
                fs::read_to_string(dir.join("sentinel"))
                    .expect("each caller's sentinel must survive"),
                id.to_string()
            );
        }
    }

    #[test]
    fn creation_errors_fail_without_retrying_or_removing_contents() {
        let root = unique_dir("fixture-errors");
        let blocked = root.join("blocked");
        fs::write(&blocked, b"unrelated file").expect("the isolated fixture is writable");
        let counter = AtomicU32::new(0);
        let error = reserve_dir(&blocked, "nested/fixture", &counter)
            .expect_err("a file cannot contain a fixture parent");
        assert!(error.to_string().contains(&blocked.display().to_string()));
        assert_eq!(counter.load(Ordering::Relaxed), 0);

        let error = reserve_dir(&root, "invalid\0", &counter)
            .expect_err("a NUL cannot be part of a directory name");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains(&root.display().to_string()));
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        assert_eq!(
            fs::read(&blocked).expect("setup errors must preserve existing files"),
            b"unrelated file"
        );
    }
}
