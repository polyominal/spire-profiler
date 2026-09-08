//! Each test builds a chart input and asserts the emitted commands; the
//! goldens pin the full dump.

use super::*;
use crate::test_util::{cmd_texts as texts, test_row};

/// The tab/rows/meta core of a BuildInput with default chrome; a
/// test overlays its tweaks via `..build_input(..)`.
fn build_input(tab: UiTab, rows: &[UiRow], meta: UiMeta) -> BuildInput<'_> {
    BuildInput {
        tab,
        rows,
        meta,
        ..BuildInput::default()
    }
}

#[test]
fn scaled_texture_rects_keep_their_original_center() {
    let tex = TextureCmd {
        x: 100.0,
        y: 40.0,
        w: 64.0,
        h: 64.0,
        icon: theme::IconId::Character(0),
    };

    assert_eq!(
        tex.scaled_rect(1.0),
        Rect2::new(Vector2::new(100.0, 40.0), Vector2::new(64.0, 64.0))
    );
    assert_eq!(
        tex.scaled_rect(1.1),
        Rect2::new(Vector2::new(96.8, 36.8), Vector2::new(70.4, 70.4))
    );
    assert_eq!(
        tex.scaled_rect(0.95),
        Rect2::new(Vector2::new(101.6, 41.6), Vector2::new(60.8, 60.8))
    );
}

#[test]
fn segment_offsets_per_mille_to_pixels_uncapped_tail() {
    let segs = [500, 250, 0, 0, 0, 0, 0];
    let px = segment_offsets(&segs, 300.0);
    assert_eq!(px[0].x, 0.0);
    assert_eq!(px[0].w, 150.0);
    assert_eq!(px[1].x, 150.0);
    assert_eq!(px[1].w, 75.0);
    assert_eq!(px[2].w, 0.0);

    // Sum > 1000 overflows the bar instead of clamping (by design).
    let over = [800, 400, 0, 0, 0, 0, 0];
    let px_over = segment_offsets(&over, 300.0);
    assert_eq!(px_over[0].w, 240.0);
    assert_eq!(px_over[1].x, 240.0);
    assert_eq!(px_over[1].w, 120.0);

    // Truncation, not rounding: 1/3 of 300 = 99.9 -> 99.
    let third = [333, 0, 0, 0, 0, 0, 0];
    assert_eq!(segment_offsets(&third, 300.0)[0].w, 99.0);
}

fn with_encounter(mut meta: UiMeta, enc: &str) -> UiMeta {
    meta.encounter[..enc.len()].copy_from_slice(enc.as_bytes());
    meta.encounter_len = enc.len() as u8;
    meta
}

fn combat_tab_layout() -> Layout {
    combat_tab_layout_mode(true, false, 0.0)
}

fn combat_tab_layout_mode(flat_chrome: bool, tab_sprites: bool, right_gutter: f32) -> Layout {
    let rows = [
        test_row(
            Section::Damage,
            SourceKind::Card,
            0,
            "STRIKE",
            2,
            20,
            487,
            [769, 0, 0, 0, 0, 0, 0],
        ),
        test_row(
            Section::Defense,
            SourceKind::Card,
            0,
            "CRIMSON_MANTLE",
            1,
            10,
            1000,
            [1000, 0, 0, 0, 0, 0, 0],
        ),
        test_row(
            Section::Defense,
            SourceKind::Card,
            ui_model::ROW_FLAG_SELF,
            "CRIMSON_MANTLE",
            1,
            -3,
            0,
            [0, 0, 0, 0, 0, 0, 428],
        ),
    ];
    let meta = with_encounter(
        UiMeta {
            turns: 2,
            dps_x10: 100,
            ..UiMeta::default()
        },
        "BYGONE_EFFIGY",
    );
    build(BuildInput {
        flat_chrome,
        tab_sprites,
        right_gutter,
        ..build_input(UiTab::Combat, &rows, meta)
    })
}

#[test]
fn layout_sections_row_order_self_rendering_hit_tests() {
    let l = combat_tab_layout();
    assert!(!l.cmds.is_empty());
    assert_eq!(l.row_hits.len(), 3);

    let y0 = l.row_hits[0].y0;
    assert_eq!(row_at(&l.row_hits, y0 + 1.0), Some(0));
    assert_eq!(row_at(&l.row_hits, l.row_hits[1].y0 + 1.0), Some(1));
    assert_eq!(row_at(&l.row_hits, l.height - 1.0), None);

    let mut found_self_bar = false;
    let mut found_self_label = false;
    for cmd in &l.cmds {
        match cmd {
            Cmd::Rect(r) => {
                if r.color[0] == COL_SELF[0] && r.color[1] == COL_SELF[1] && r.w > 0.0 {
                    found_self_bar = true;
                }
            }
            Cmd::Text(t) => {
                if t.text.contains("self damage") {
                    found_self_label = true;
                }
            }
            Cmd::Texture(_) => {}
        }
    }
    assert!(found_self_bar);
    assert!(found_self_label);

    assert_eq!(l.tab_hits.len(), 2);
    assert_eq!(
        tab_at(&l, l.tab_hits[1].x0 + 1.0, l.tab_hits[1].y0 + 1.0),
        Some(UiTab::Run)
    );
    assert_eq!(tab_at(&l, 2.0, 2.0), None);
}

#[test]
fn tab_strip_sprites_and_text_fallback_modes() {
    let sprite = combat_tab_layout_mode(true, true, 0.0);
    let text = combat_tab_layout_mode(true, false, 0.0);
    let textures: Vec<&TextureCmd> = sprite
        .header_cmds
        .iter()
        .filter_map(|cmd| match cmd {
            Cmd::Texture(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(textures.len(), 3, "two plates + the active stroke");
    assert_eq!(textures[0].icon, theme::IconId::TabPlate);
    assert_eq!(textures[1].icon, theme::IconId::TabStroke);
    assert_eq!(textures[2].icon, theme::IconId::TabPlate);
    assert_eq!(textures[0].w, TAB_W);
    assert_eq!(textures[0].h, TABS_H);
    assert_eq!(textures[0].x, textures[1].x);
    assert_eq!(textures[0].y, textures[1].y);
    assert!(
        !text
            .header_cmds
            .iter()
            .any(|cmd| matches!(cmd, Cmd::Texture(_))),
        "the text fallback draws no sprites"
    );
    assert!(
        text.header_cmds
            .iter()
            .any(|cmd| matches!(cmd, Cmd::Rect(r) if r.color == COL_GOLD && r.w < TAB_W)),
        "the text fallback marks the active tab with the underline"
    );
    assert!(
        !sprite
            .header_cmds
            .iter()
            .any(|cmd| matches!(cmd, Cmd::Rect(r) if r.color == COL_GOLD && r.w < TAB_W)),
        "the sprite mode's active marker is the stroke, not the underline"
    );
    for (a, b) in ["This Combat", "Run Summary"].map(|label| {
        let find = |l: &Layout| {
            l.header_cmds
                .iter()
                .find_map(|cmd| match cmd {
                    Cmd::Text(t) if t.text == label => Some(t.clone()),
                    _ => None,
                })
                .expect("the tab label renders")
        };
        (find(&sprite), find(&text))
    }) {
        assert_eq!(a, b, "the label command is mode-independent");
        assert_eq!(a.align, TextAlign::Center(TAB_W));
        assert_eq!(a.size, theme::SIZE_HEADER);
    }
    assert_eq!(sprite.tab_hits, text.tab_hits);
}

#[test]
fn layout_value_share_formatting_and_self_row_semantics() {
    let rows = [
        test_row(
            Section::Damage,
            SourceKind::Card,
            0,
            "STRIKE",
            2,
            20,
            487,
            [1000, 0, 0, 0, 0, 0, 0],
        ),
        test_row(
            Section::Defense,
            SourceKind::Card,
            ui_model::ROW_FLAG_SELF,
            "OFFERING",
            1,
            -6,
            0,
            [0, 0, 0, 0, 0, 0, 500],
        ),
    ];
    let meta = UiMeta::default();
    let l = build(build_input(UiTab::Combat, &rows, meta));

    let mut found_share = false;
    let mut found_self_value = false;
    for t in texts(&l.cmds) {
        if t == "20  (48.7%)" {
            found_share = true;
        }
        if t == "-6" {
            found_self_value = true;
        }
        assert!(!t.contains("-6  ("), "self row got a share suffix: {t:?}");
    }
    assert!(found_share);
    assert!(found_self_value);
}

#[test]
fn layout_solo_self_row_shows_the_card_name_and_plays() {
    let rows = [test_row(
        Section::Defense,
        SourceKind::Card,
        ui_model::ROW_FLAG_SELF | ui_model::ROW_FLAG_SELF_SOLO,
        "BLOODLETTING",
        3,
        -9,
        0,
        [0, 0, 0, 0, 0, 0, 1000],
    )];
    let l = build(build_input(UiTab::Combat, &rows, UiMeta::default()));
    let all: Vec<&str> = texts(&l.cmds).collect();
    assert!(
        all.iter().any(|t| t.contains("BLOODLETTING")),
        "solo self row must show the card name: {all:?}"
    );
    assert!(
        !all.iter().any(|t| t.contains("self damage")),
        "solo self row must not use the hanging label: {all:?}"
    );
    assert!(all.contains(&"x3"), "plays shown: {all:?}");
    assert!(
        all.contains(&"-9"),
        "red raw cost, no share suffix: {all:?}"
    );
    assert!(!all.iter().any(|t| t.contains("-9  (")));
}

#[test]
fn layout_run_tab_renders_the_dps_dash_without_turns() {
    let rows = [test_row(
        Section::Damage,
        SourceKind::Card,
        0,
        "STRIKE",
        1,
        5,
        1000,
        [1000, 0, 0, 0, 0, 0, 0],
    )];
    let meta = UiMeta {
        combats: 3,
        ..UiMeta::default()
    };
    let l = build(build_input(UiTab::Run, &rows, meta));

    let dps_dash = texts(&l.header_cmds).any(|t| t.contains("DPS —"));
    assert!(dps_dash);
    assert!(texts(&l.header_cmds).any(|t| t == "DPS — · 3 combats"));
}

#[test]
fn meta_block_formats_the_stats_line_per_tab() {
    let meta = with_encounter(
        UiMeta {
            turns: 4,
            plays: 14,
            dps_x10: 405,
            damage_taken: 27,
            combats: 2,
            ..UiMeta::default()
        },
        "DEVOTED_SCULPTOR_WEAK",
    );
    assert_eq!(
        meta_line(UiTab::Combat, &meta),
        "DPS 40.5 · 4 turns · 14 plays · took 27"
    );
    assert_eq!(
        meta_line(UiTab::Run, &meta),
        "DPS 40.5 · 4 turns · 2 combats"
    );
    let no_dps = UiMeta { turns: 0, ..meta };
    assert_eq!(meta_line(UiTab::Combat, &no_dps), "DPS — · 14 plays");
    assert_eq!(meta_line(UiTab::Run, &no_dps), "DPS — · 2 combats");
}

#[test]
fn encounter_line_emits_untruncated_and_clipped() {
    let rows = [test_row(
        Section::Damage,
        SourceKind::Card,
        0,
        "STRIKE",
        1,
        9,
        1000,
        [1000, 0, 0, 0, 0, 0, 0],
    )];
    let build_with = |enc: &str, flat_chrome: bool| {
        let meta = with_encounter(
            UiMeta {
                turns: 2,
                dps_x10: 100,
                ..UiMeta::default()
            },
            enc,
        );
        build(BuildInput {
            flat_chrome,
            ..build_input(UiTab::Combat, &rows, meta)
        })
    };
    // A free fn, not a closure: the return borrows the layout alone,
    // which a closure's lifetime elision cannot express.
    fn find_line<'a>(l: &'a Layout, prefix: &str) -> &'a TextCmd {
        l.header_cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Text(t) if t.text.starts_with(prefix) => Some(t),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the {prefix} line renders"))
    }
    for flat_chrome in [true, false] {
        let l = build_with("BATTLEWORN_DUMMY_EVENT_V1_ENCOUNTER", flat_chrome);
        let enc_line = find_line(&l, "Vs. ");
        assert_eq!(enc_line.text, "Vs. BATTLEWORN_DUMMY_EVENT_V1_ENCOUNTER");
        assert_eq!(enc_line.role, TextRole::Body, "a name, not a number");
        let stats = find_line(&l, "DPS ");
        assert_eq!(stats.role, TextRole::Title, "the number idiom");
        assert_eq!(stats.y, enc_line.y + META_H, "stats under the encounter");
        for line in [enc_line, stats] {
            assert_eq!(line.x, l.content.x);
            assert_eq!(line.align, TextAlign::LeftClipped(l.content.w));
        }
        let l = build_with("", flat_chrome);
        assert!(
            !texts(&l.header_cmds).any(|t| t.starts_with("Vs. ")),
            "no bare Vs. line"
        );
        assert!(texts(&l.header_cmds).any(|t| t.starts_with("DPS ")));
    }
}

#[test]
fn encounter_line_fits_the_longest_real_slug() {
    // fontTools-measured Kreon Regular hmtx advances at 24px plus the
    // 1px/char tracking.
    const HEAD: f32 = 38.8; // "Vs. "
    const SLUG_CHAR: f32 = 15.24; // the uppercase-slug mean + tracking
    const CONTENT_W: f32 = 713.0; // the plate chrome's content width
    const LONGEST_REAL_SLUG: f32 = 35.0; // BATTLEWORN_DUMMY_EVENT_V1_ENCOUNTER
    let line = HEAD + LONGEST_REAL_SLUG * SLUG_CHAR;
    assert!(line <= CONTENT_W, "the longest real slug fits: {line}");
}

#[test]
fn truncate_marked_swaps_the_last_char_for_the_marker() {
    assert_eq!(truncate_marked("STRIKE", 16), "STRIKE");
    assert_eq!(truncate_marked("ABCDEFGHIJKLMNOP", 16), "ABCDEFGHIJKLMNOP");
    assert_eq!(truncate_marked("ABCDEFGHIJKLMNOPQ", 16), "ABCDEFGHIJKLMNO…");
    let wide = format!("{}É", "A".repeat(17));
    assert_eq!(truncate_marked(&wide, 16), "AAAAAAAAAAAAAAA…");
    assert!(truncate_marked(&wide, 16).chars().count() == 16);
}

#[test]
fn title_band_never_renders_a_player_filter_label() {
    let rows = [test_row(
        Section::Damage,
        SourceKind::Card,
        0,
        "STRIKE",
        1,
        9,
        1000,
        [1000, 0, 0, 0, 0, 0, 0],
    )];
    for tab in [UiTab::Combat, UiTab::Run] {
        let l = build(build_input(tab, &rows, UiMeta::default()));
        assert!(
            !texts(&l.header_cmds).any(|t| t == "All" || t == "P1" || t == "P2"),
            "the avatar row carries the filter state, never a text label"
        );
    }
}

fn avatar_facts() -> Vec<AvatarFact> {
    vec![
        AvatarFact {
            slot: 0,
            loaded: true,
            path: "res://images/ui/top_panel/character_icon_ironclad.png".to_owned(),
        },
        AvatarFact {
            slot: 1,
            loaded: true,
            path: "res://images/ui/top_panel/character_icon_silent.png".to_owned(),
        },
    ]
}

fn avatar_fact(slot: u8, loaded: bool) -> AvatarFact {
    AvatarFact {
        slot,
        loaded,
        path: format!("res://images/ui/top_panel/character_icon_{slot}.png"),
    }
}

fn combat_build(avatars: &[AvatarFact]) -> Layout {
    let rows = [test_row(
        Section::Damage,
        SourceKind::Card,
        0,
        "STRIKE",
        1,
        9,
        1000,
        [1000, 0, 0, 0, 0, 0, 0],
    )];
    build(BuildInput {
        avatars,
        ..build_input(UiTab::Combat, &rows, UiMeta::default())
    })
}

fn icons_of(l: &Layout) -> Vec<&TextureCmd> {
    l.header_cmds
        .iter()
        .filter_map(|cmd| match cmd {
            Cmd::Texture(t) => Some(t),
            _ => None,
        })
        .collect()
}

/// Without the avatar row the header flow is title + meta (the tabs
/// live in the strip band, already inside `content.top`).
fn bare_header_bottom(content: theme::ContentBox) -> f32 {
    content.top + HEADER_H + META_H
}

#[test]
fn avatar_row_emits_textures_hits_and_paths_on_the_combat_tab() {
    let l = combat_build(&avatar_facts());
    let icons = icons_of(&l);
    assert_eq!(icons.len(), 2, "two loaded avatars draw");
    assert_eq!(icons[0].icon, theme::IconId::Character(0));
    assert_eq!(icons[0].w, icons[0].h, "the portrait art is square");
    assert_eq!(icons[0].w, AVATAR_H);
    assert_eq!(icons[1].x, icons[0].x + AVATAR_H + AVATAR_GAP);
    assert_eq!(
        l.portrait_paths,
        avatar_facts()
            .iter()
            .map(|a| a.path.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(l.avatar_hits.len(), 2);
    assert_eq!(
        l.avatar_hits[0],
        AvatarHit {
            x0: l.content.x,
            y0: l.content.top + HEADER_H,
            x1: l.content.x + AVATAR_H,
            y1: l.content.top + HEADER_H + AVATAR_H,
            slot: 0,
        }
    );
    assert_eq!(l.avatar_hits[1].slot, 1);
    // The row pins with the chrome: body starts past the avatar row.
    assert_eq!(l.header_bottom, bare_header_bottom(l.content) + AVATAR_H);
}

#[test]
fn avatar_at_maps_presses_inside_the_boxes_only() {
    let l = combat_build(&avatar_facts());
    let hit = l.avatar_hits[1];
    assert_eq!(
        avatar_at(&l.avatar_hits, hit.x0 + 1.0, hit.y0 + 1.0),
        Some(1)
    );
    assert_eq!(avatar_at(&l.avatar_hits, hit.x1, hit.y0 + 1.0), None);
    assert_eq!(
        avatar_at(
            &l.avatar_hits,
            l.avatar_hits[0].x0 + 1.0,
            l.avatar_hits[0].y1
        ),
        None
    );
    assert_eq!(avatar_at(&[], 5.0, 5.0), None);
}

#[test]
fn avatar_row_renders_on_the_run_tab_too() {
    let rows = [test_row(
        Section::Damage,
        SourceKind::Card,
        0,
        "STRIKE",
        1,
        9,
        1000,
        [1000, 0, 0, 0, 0, 0, 0],
    )];
    let l = build(BuildInput {
        avatars: &avatar_facts(),
        ..build_input(UiTab::Run, &rows, UiMeta::default())
    });
    assert_eq!(icons_of(&l).len(), 2, "the run tab carries the row");
    assert_eq!(l.avatar_hits.len(), 2);
    assert_eq!(
        l.portrait_paths,
        avatar_facts()
            .iter()
            .map(|a| a.path.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(l.avatar_hits[0].slot, 0);
    assert_eq!(l.avatar_hits[1].slot, 1);
    assert_eq!(l.header_bottom, bare_header_bottom(l.content) + AVATAR_H);
}

#[test]
fn avatar_row_skips_unloaded_avatars_but_records_their_paths() {
    let facts = vec![avatar_fact(0, false), avatar_fact(1, true)];
    let l = combat_build(&facts);
    assert_eq!(
        l.portrait_paths.len(),
        2,
        "paths record wanted, loaded or not"
    );
    let icons = icons_of(&l);
    assert_eq!(icons.len(), 1, "only the loaded avatar draws");
    assert_eq!(
        icons[0].icon,
        theme::IconId::Character(1),
        "index stays roster-relative"
    );
    assert_eq!(l.avatar_hits.len(), 1);
    assert_eq!(l.avatar_hits[0].slot, 1);
}

#[test]
fn avatar_row_collapses_when_nothing_draws() {
    let l = combat_build(&[avatar_fact(0, false)]);
    assert!(l.avatar_hits.is_empty());
    assert_eq!(l.portrait_paths.len(), 1, "the wanted path still records");
    assert_eq!(l.header_bottom, bare_header_bottom(l.content));

    let l = combat_build(&[]);
    assert!(l.avatar_hits.is_empty() && l.portrait_paths.is_empty());
}

#[test]
fn chrome_less_build_emits_only_the_sections() {
    let rows = [test_row(
        Section::Damage,
        SourceKind::Card,
        0,
        "STRIKE",
        1,
        9,
        1000,
        [1000, 0, 0, 0, 0, 0, 0],
    )];
    let meta = UiMeta {
        turns: 3,
        combats: 1,
        dps_x10: 30,
        ..UiMeta::default()
    };
    let l = build(BuildInput {
        skip_chrome: true,
        ..build_input(UiTab::Run, &rows, meta)
    });
    assert!(l.tab_hits.is_empty(), "no tab strip without chrome");
    assert!(
        l.header_cmds.is_empty() && l.header_bottom == 0.0,
        "the chrome-less build is all body"
    );
    let t: Vec<&str> = texts(&l.cmds).collect();
    assert!(!t.contains(&"Contribution"), "no title band without chrome");
    assert!(!t.contains(&"This Combat"), "no tab labels without chrome");
    assert!(
        !t.iter().any(|s| s.starts_with("DPS ")),
        "no meta line: the splicing panel pins it"
    );
    assert!(t.contains(&"Damage"));
    assert!(t.contains(&"STRIKE"));
    // Content starts at y 0, so a caller splicing at its own offset
    // translates by exactly that offset.
    let first_y = l
        .cmds
        .iter()
        .find_map(|cmd| match cmd {
            Cmd::Rect(r) => Some(r.y),
            _ => None,
        })
        .expect("the section band renders first");
    assert_eq!(first_y, 0.0);
    assert!(l.height > 0.0);
}

#[test]
fn layout_emits_every_row_without_height_cap() {
    let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
    for (i, row) in rows.iter_mut().enumerate() {
        let section = if i % 2 == 0 {
            Section::Damage
        } else {
            Section::Defense
        };
        let name = format!("CARD{i}");
        *row = test_row(
            section,
            SourceKind::Card,
            0,
            &name,
            1,
            10,
            10,
            [500, 0, 0, 0, 0, 0, 0],
        );
    }
    let l = build(build_input(UiTab::Combat, &rows, UiMeta::default()));

    assert_eq!(l.row_hits.len(), ui_model::MAX_UI_ROWS);
    for hit in &l.row_hits {
        assert_eq!(row_at(&l.row_hits, hit.y0 + 1.0), Some(hit.flat_index));
    }

    assert!(!texts(&l.cmds).any(|t| t.contains("truncated")));

    // The height is asserted region by region (not the sum) so a
    // geometry tweak names the part it regressed; the fixture's meta
    // carries no encounter, so the block is one stats line. The strip
    // band sits above the plate but counts into the Control height.
    let sections = 2.0 * SECTION_HEADER_H + 2.0 * 128.0 * ROW_H + 2.0 * SECTION_GAP;
    assert_eq!(sections, 8_292.0);
    let chrome_and_headers =
        theme::FLAT_PAD + TABS_H + STRIP_GAP + HEADER_H + META_H + 4.0 + theme::FLAT_PAD;
    assert_eq!(chrome_and_headers, 204.0);
    assert_eq!(l.height, sections + chrome_and_headers);
}

#[test]
fn layout_reflows_to_the_build_width() {
    let rows = [test_row(
        Section::Damage,
        SourceKind::Card,
        0,
        "STRIKE",
        1,
        9,
        1000,
        [1000, 0, 0, 0, 0, 0, 0],
    )];
    let track_w = |width: f32| {
        let l = build(BuildInput {
            width,
            ..build_input(UiTab::Combat, &rows, UiMeta::default())
        });
        assert_eq!(l.width, width, "the layout reports the build width");
        l.cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Rect(r) if r.color == COL_TRACK => Some(r.w),
                _ => None,
            })
            .expect("every row emits a bar track")
    };
    // Designed 240px bar in flat chrome; plate insets leave 197px.
    assert_eq!(track_w(PANEL_WIDTH), 240.0);
    assert_eq!(track_w(PANEL_WIDTH + 200.0), 440.0);
    assert_eq!(track_w(PANEL_WIDTH - 150.0), 90.0);
    assert_eq!(track_w(100.0), MIN_BAR_W);
}

#[test]
fn worst_case_command_count_fits_the_cap() {
    // Full complement per row (relic kind, so the prefix run emits too).
    let mut rows = [UiRow::default(); ui_model::MAX_UI_ROWS];
    for (i, row) in rows.iter_mut().enumerate() {
        let section = if i % 2 == 0 {
            Section::Damage
        } else {
            Section::Defense
        };
        *row = test_row(
            section,
            SourceKind::Relic,
            0,
            &format!("CARD{i}"),
            1,
            10,
            10,
            [1000; 7],
        );
    }
    let l = build(BuildInput {
        tab: UiTab::Combat,
        rows: &rows,
        meta: UiMeta::default(),
        footer: &"footer\n".repeat(64),
        hover_row: Some(0),
        skip_chrome: false,
        avatars: &[],
        flat_chrome: true,
        // The sprite tabs add three texture commands over the text
        // tabs; the cap must see them.
        tab_sprites: true,
        width: PANEL_WIDTH,
        right_gutter: 0.0,
    });
    assert!(
        l.cmds.len() < MAX_CMDS,
        "the cap must fit the true worst case ({} cmds)",
        l.cmds.len()
    );
    assert_eq!(l.row_hits.len(), ui_model::MAX_UI_ROWS);
}

#[test]
fn insert_borders_splices_only_under_the_cap() {
    let border = || {
        Cmd::Rect(RectCmd {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            color: COL_TRACK,
        })
    };
    let mut full: Vec<Cmd> = (0..MAX_CMDS).map(|_| border()).collect();
    insert_borders(&mut full, "chart", 100.0, 100.0);
    assert_eq!(full.len(), MAX_CMDS, "at the cap the borders are dropped");

    let mut empty = Vec::new();
    insert_borders(&mut empty, "chart", 100.0, 100.0);
    assert_eq!(empty.len(), 4, "under the cap the four edges splice in");
}

#[test]
fn section_band_and_underline_span_exactly_the_content_width() {
    for (flat_chrome, tab_sprites, gutter) in
        [(true, false, 0.0), (false, false, 0.0), (false, true, 32.0)]
    {
        let l = combat_tab_layout_mode(flat_chrome, tab_sprites, gutter);
        let rows_w = l.content.w;
        // The gold rect spanning the rows region (text tabs' underline
        // is one tab wide).
        let underline = l
            .cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Rect(r) if r.color == COL_GOLD && r.w == rows_w => Some(r),
                _ => None,
            })
            .expect("the damage section underline renders");
        assert_eq!(underline.x, l.content.x, "flat_chrome={flat_chrome}");
        let band = l
            .cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Rect(r) if r.color == COL_HEADER_BG => Some(r),
                _ => None,
            })
            .expect("the section header band renders");
        assert_eq!(band.x, l.content.x, "flat_chrome={flat_chrome}");
        assert_eq!(band.w, rows_w, "flat_chrome={flat_chrome}");
    }
}

#[test]
fn section_header_underline_sits_below_the_title_inside_the_band() {
    // The 24px title's descent reaches ~7px under the baseline; the
    // emitted commands must agree with the compile-time pins.
    let l = combat_tab_layout();
    let rows_w = l.content.w;
    let underline = l
        .cmds
        .iter()
        .find_map(|cmd| match cmd {
            Cmd::Rect(r) if r.color == COL_GOLD && r.w == rows_w => Some(r),
            _ => None,
        })
        .expect("the damage underline renders");
    let title = l
        .cmds
        .iter()
        .find_map(|cmd| match cmd {
            Cmd::Text(t) if t.text == "Damage" => Some(t),
            _ => None,
        })
        .expect("the damage title renders");
    assert!(underline.y >= title.y + 7.0, "underline crosses the glyphs");
    let band = l
        .cmds
        .iter()
        .find_map(|cmd| match cmd {
            Cmd::Rect(r) if r.color == COL_HEADER_BG => Some(r),
            _ => None,
        })
        .expect("the header band renders");
    assert!(title.y >= band.y && title.y <= band.y + band.h);
    assert!(underline.y + underline.h <= band.y + band.h);
}

#[test]
fn the_legend_is_gone_from_the_command_lists() {
    let l = combat_tab_layout();
    for label in ["direct", "str down", "self dmg"] {
        assert!(
            !texts(&l.cmds)
                .chain(texts(&l.header_cmds))
                .any(|t| t == label),
            "the key renders on its own plate, not in the panel: {label}"
        );
    }
    assert_eq!(
        l.height,
        12.0 + TABS_H + STRIP_GAP + HEADER_H + 2.0 * META_H + 196.0 + 4.0 + 12.0
    );
}

#[test]
fn kind_prefix_renders_as_a_separate_colored_run() {
    let rows = [
        test_row(
            Section::Damage,
            SourceKind::Relic,
            0,
            "CRIMSON_MANTLE",
            1,
            10,
            1000,
            [1000, 0, 0, 0, 0, 0, 0],
        ),
        test_row(
            Section::Damage,
            SourceKind::Card,
            0,
            "STRIKE",
            1,
            10,
            1000,
            [1000, 0, 0, 0, 0, 0, 0],
        ),
    ];
    let l = build(build_input(UiTab::Combat, &rows, UiMeta::default()));
    let find = |text: &str| {
        l.cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Text(t) if t.text == text => Some(t),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{text:?} renders"))
    };
    let name_x = l.content.x + 4.0;
    let prefix = find("[R] ");
    assert_eq!(prefix.x, name_x);
    assert_eq!(prefix.color, COL_GOLD, "the relic marker is gold");
    let name = find("CRIMSON_MANTLE");
    assert_eq!(name.x, name_x + PREFIX_ADVANCE);
    assert_eq!(name.color, COL_CREAM);
    assert_eq!(prefix.y, name.y, "the two runs share the baseline");
    assert_eq!(find("STRIKE").x, name_x);
}

#[test]
fn every_command_lies_inside_the_content_box() {
    for (flat_chrome, tab_sprites, gutter) in
        [(true, false, 0.0), (false, false, 0.0), (false, true, 32.0)]
    {
        let l = combat_tab_layout_mode(flat_chrome, tab_sprites, gutter);
        let chrome: Vec<Cmd> = if flat_chrome {
            crate::ui::panel_common::border_rects(l.width, l.height).to_vec()
        } else {
            Vec::new()
        };
        crate::test_util::assert_layout_bounds(
            &l.header_cmds,
            &l.cmds,
            l.content,
            l.header_bottom,
            l.height,
            l.strip_h,
            &chrome,
        );
    }
}

#[test]
fn header_carries_the_title_tabs_and_meta_body_starts_at_the_sections() {
    for flat_chrome in [true, false] {
        let l = combat_tab_layout_mode(flat_chrome, false, 0.0);
        let header: Vec<&str> = texts(&l.header_cmds).collect();
        assert!(header.contains(&"Contribution"), "flat={flat_chrome}");
        assert!(header.contains(&"This Combat"), "flat={flat_chrome}");
        assert!(header.contains(&"Run Summary"), "flat={flat_chrome}");
        assert!(
            header.iter().any(|t| t.starts_with("Vs. ")),
            "the encounter line pins, flat={flat_chrome}"
        );
        assert!(
            header.iter().any(|t| t.starts_with("DPS ")),
            "the stats line pins, flat={flat_chrome}"
        );
        assert!(
            !texts(&l.cmds)
                .any(|t| t == "Contribution" || t.starts_with("Vs. ") || t.starts_with("DPS ")),
            "header content must not scroll, flat={flat_chrome}"
        );
        assert_eq!(
            l.header_bottom,
            l.content.top + HEADER_H + 2.0 * META_H,
            "the tabs moved out of the header flow, flat={flat_chrome}"
        );
        // The tab strip itself floats in the band above the plate.
        assert_eq!(l.strip_h, TABS_H + STRIP_GAP, "flat={flat_chrome}");
        assert!(
            l.tab_hits
                .iter()
                .all(|hit| hit.y1 <= TABS_H && hit.y0 >= 0.0),
            "the tab boxes live entirely in the strip band, flat={flat_chrome}"
        );
        let first = l
            .cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::Text(t) => Some(t),
                _ => None,
            })
            .expect("a section title renders");
        assert_eq!(first.text, "Damage");
        assert_eq!(
            first.y,
            l.header_bottom + SECTION_TITLE_Y,
            "flat={flat_chrome}"
        );
    }
}

fn golden_fixture(flat_chrome: bool) -> Layout {
    let rows = [
        test_row(
            Section::Damage,
            SourceKind::Card,
            0,
            "STRIKE",
            2,
            20,
            487,
            [769, 231, 0, 0, 0, 0, 0],
        ),
        test_row(
            Section::Damage,
            SourceKind::Relic,
            0,
            "SEVER_SOUL",
            1,
            12,
            292,
            [1000, 0, 0, 0, 0, 0, 0],
        ),
        test_row(
            Section::Defense,
            SourceKind::Card,
            0,
            "DEFEND",
            3,
            15,
            1000,
            [1000, 0, 0, 0, 0, 0, 0],
        ),
        test_row(
            Section::Defense,
            SourceKind::Card,
            ui_model::ROW_FLAG_SELF,
            "OFFERING",
            1,
            -6,
            0,
            [0, 0, 0, 0, 0, 0, 1000],
        ),
    ];
    let meta = with_encounter(
        UiMeta {
            turns: 2,
            plays: 7,
            dps_x10: 160,
            damage_taken: 6,
            ..UiMeta::default()
        },
        "BYGONE_EFFIGY",
    );
    build(BuildInput {
        footer: "Total 32 damage in 2 turns",
        hover_row: Some(0),
        flat_chrome,
        tab_sprites: !flat_chrome,
        ..build_input(UiTab::Combat, &rows, meta)
    })
}

#[test]
fn right_side_columns_align_to_their_zone_right_edges() {
    let l = golden_fixture(false);
    let cmd = l
        .cmds
        .iter()
        .find_map(|cmd| match cmd {
            Cmd::Text(t) if t.text == "20  (48.7%)" => Some(t),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the value column renders"));
    let TextAlign::Right(w) = cmd.align else {
        panic!("the value column right-aligns")
    };
    assert_eq!(cmd.x + w, l.content.right(), "value column");
}

/// The dump splits header from body, so a wrong-zone command reviews.
#[test]
fn golden_chart_commands_flat_chrome() {
    let l = golden_fixture(true);
    insta::assert_snapshot!(crate::test_util::dump_layout(&l.header_cmds, &l.cmds));
}

#[test]
fn golden_chart_commands_plate_chrome() {
    let l = golden_fixture(false);
    insta::assert_snapshot!(crate::test_util::dump_layout(&l.header_cmds, &l.cmds));
}
