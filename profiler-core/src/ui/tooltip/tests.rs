use super::*;

fn chars(s: &str) -> usize {
    s.chars().count()
}

fn card_detail() -> RowDetail {
    use StatTone::*;
    let stat = |label: &str, value: &str, tone: StatTone| StatLine {
        label: label.to_owned(),
        value: value.to_owned(),
        tone,
    };
    RowDetail {
        title: "\"STRIKE\" x12".to_owned(),
        stats: vec![
            stat(
                "dmg (12345 unblk)",
                "12345",
                Direct(Section::Damage, SourceKind::Card),
            ),
            stat("direct", "12345", Direct(Section::Damage, SourceKind::Card)),
            stat("indirect", "12345", Attributed),
            stat("mod", "0", Modifier),
            stat(
                "block (100 eff)",
                "120",
                Direct(Section::Defense, SourceKind::Card),
            ),
            stat("blk mod", "10", Modifier),
            stat("weak", "30", MitigateDebuff),
            stat("buff", "0", MitigateBuff),
            stat("str", "5", MitigateStr),
            stat("self dmg", "12", SelfDamage),
            stat("forge", "2", Neutral),
        ],
    }
}

/// Every tone equals the bar's color, so a palette rework can never
/// leave the tip stale.
#[test]
fn tone_colors_match_the_chart_segments() {
    use SourceKind as K;
    use StatTone::*;
    use m::Section as S;

    use crate::ui::ui_model as m;
    for (tone, slot, section, kind) in [
        (
            Direct(S::Damage, K::Card),
            m::Segment::Direct,
            S::Damage,
            K::Card,
        ),
        (
            Direct(S::Damage, K::Relic),
            m::Segment::Direct,
            S::Damage,
            K::Relic,
        ),
        (
            Direct(S::Defense, K::Card),
            m::Segment::Direct,
            S::Defense,
            K::Card,
        ),
        (
            Direct(S::Defense, K::Osty),
            m::Segment::Direct,
            S::Defense,
            K::Osty,
        ),
        (Attributed, m::Segment::Attributed, S::Damage, K::Card),
        (Modifier, m::Segment::Modifier, S::Damage, K::Card),
        (
            MitigateDebuff,
            m::Segment::MitigateDebuff,
            S::Defense,
            K::Card,
        ),
        (MitigateBuff, m::Segment::MitigateBuff, S::Defense, K::Card),
        (MitigateStr, m::Segment::MitigateStr, S::Defense, K::Card),
        (SelfDamage, m::Segment::SelfDamage, S::Defense, K::Card),
    ] {
        assert_eq!(
            tone_color(tone),
            palette::slot_color(slot, section, kind),
            "{tone:?} must equal the bar's color"
        );
    }
    // The Osty-absorb exception flows through.
    assert_eq!(
        tone_color(StatTone::Direct(Section::Defense, K::Osty)),
        palette::COL_OSTY
    );
    assert_eq!(tone_color(StatTone::Neutral), palette::COL_CREAM);
}

#[test]
fn shape_emits_one_two_column_line_per_stat() {
    let detail = card_detail();
    let lines = shape(&detail, 64);
    assert_eq!(lines.len(), 1 + detail.stats.len());
    assert!(lines[0].title);
    assert_eq!(lines[0].color, palette::COL_GOLD);
    assert_eq!(lines[0].text, "\"STRIKE\" x12");
    for (line, stat) in lines[1..].iter().zip(detail.stats.iter()) {
        assert!(!line.title);
        assert_eq!(line.text, stat.label, "the label is the left column");
        assert_eq!(
            line.color,
            palette::COL_CREAM,
            "labels are cream; the value carries the semantic color"
        );
        let value = line.value.as_ref().expect("a two-column stat line");
        assert_eq!(value.text, stat.value);
        assert_eq!(value.color, tone_color(stat.tone));
        assert!(chars(&line.text) <= LABEL_BUDGET, "label over budget");
        assert!(chars(&value.text) <= VALUE_BUDGET, "value over budget");
    }
}

#[test]
fn shape_hard_breaks_overlong_title_slugs() {
    let id = "A".repeat(64);
    let detail = RowDetail {
        title: format!("\"{id}\" x3"),
        stats: vec![StatLine {
            label: "forge".to_owned(),
            value: "1".to_owned(),
            tone: StatTone::Neutral,
        }],
    };
    let lines = shape(&detail, 64);
    assert!(lines.len() > 4, "the slug spans several lines: {lines:?}");
    for line in &lines {
        let budget = if line.title {
            TITLE_BUDGET
        } else {
            LABEL_BUDGET
        };
        assert!(chars(&line.text) <= budget, "over budget: {:?}", line.text);
    }
    let title: String = lines
        .iter()
        .filter(|l| l.title)
        .map(|l| l.text.replace(' ', ""))
        .collect();
    assert_eq!(title, format!("\"{id}\"x3"));
}

#[test]
fn shape_wraps_over_budget_stats_as_single_color_lines() {
    let detail = RowDetail {
        title: "\"GREED\" x1".to_owned(),
        stats: vec![StatLine {
            label: "dmg (999999999999 unblk)".to_owned(),
            value: "999999999999".to_owned(),
            tone: StatTone::Direct(Section::Damage, SourceKind::Card),
        }],
    };
    let lines = shape(&detail, 64);
    assert_eq!(lines[0].text, "\"GREED\" x1");
    let wrapped = &lines[1..];
    assert!(wrapped.len() > 1, "the stat wraps: {lines:?}");
    for line in wrapped {
        assert!(line.value.is_none(), "backstop lines carry no column");
        assert_eq!(
            line.color,
            palette::slot_color(Segment::Direct, Section::Damage, SourceKind::Card)
        );
        assert!(chars(&line.text) <= BODY_BUDGET, "over budget: {line:?}");
    }
    let joined: String = wrapped.iter().map(|l| l.text.replace(' ', "")).collect();
    assert_eq!(joined, "dmg(999999999999unblk)999999999999");
}

#[test]
fn shape_truncates_to_max_lines_with_the_ellipsis_marker() {
    let lines = shape(&card_detail(), 4);
    assert_eq!(lines.len(), 4);
    assert!(lines[0].title);
    assert_eq!(lines[3].text, crate::ui::chart_layout::TRUNCATION_MARK);
    assert!(
        panel_replay::kreon_covers(&lines[3].text),
        "the marker renders in the native faces"
    );
    assert!(!lines[3].title);
    assert!(lines[3].value.is_none());
}

#[test]
fn shape_handles_the_terse_self_damage_detail() {
    let detail = RowDetail {
        title: "\"CRIMSON_MANTLE\"".to_owned(),
        stats: vec![StatLine {
            label: "self dmg".to_owned(),
            value: "3".to_owned(),
            tone: StatTone::SelfDamage,
        }],
    };
    let lines = shape(&detail, 64);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].title);
    let value = lines[1].value.as_ref().expect("two-column");
    assert_eq!(value.text, "3");
    assert_eq!(value.color, palette::COL_SELF);
}

#[test]
fn row_detail_texts_cover_everything_the_tip_draws() {
    let detail = card_detail();
    let texts: Vec<&str> = detail.texts().collect();
    assert_eq!(texts.len(), 1 + 2 * detail.stats.len());
    assert_eq!(texts[0], detail.title);
    assert!(RowDetail::default().is_empty());
    assert!(!detail.is_empty());
}

fn plate() -> Rect2 {
    Rect2::new(Vector2::new(660.0, 200.0), Vector2::new(600.0, 400.0))
}

fn viewport() -> Vector2 {
    Vector2::new(1920.0, 1080.0)
}

fn tip_size() -> Vector2 {
    Vector2::new(TIP_WIDTH, 200.0)
}

#[test]
fn place_defaults_to_the_plate_right_at_the_row_y() {
    let tip = place(viewport(), plate(), 300.0, tip_size(), None);
    assert_eq!(tip.position.x, 1260.0);
    assert_eq!(tip.position.y, 300.0);
}

#[test]
fn place_flips_left_past_the_threshold() {
    // Plate right edge at 1500 > 1440: the tip anchors left.
    let plate = Rect2::new(Vector2::new(800.0, 200.0), Vector2::new(700.0, 400.0));
    let tip = place(viewport(), plate, 300.0, tip_size(), None);
    assert_eq!(tip.position.x, 800.0 - TIP_WIDTH);
    assert_eq!(tip.position.y, 300.0);
    let plate = Rect2::new(Vector2::new(700.0, 200.0), Vector2::new(700.0, 400.0));
    let tip = place(viewport(), plate, 300.0, tip_size(), None);
    assert_eq!(tip.position.x, 1400.0);
}

#[test]
fn place_clamps_vertically_into_the_plate_band() {
    let top = place(viewport(), plate(), -100.0, tip_size(), None);
    assert_eq!(top.position.y, 200.0 + TIP_PAD);
    let bottom = place(viewport(), plate(), 10_000.0, tip_size(), None);
    assert_eq!(bottom.position.y, 600.0 - 200.0 - TIP_PAD);
}

#[test]
fn place_floors_on_a_tiny_viewport() {
    let small = Vector2::new(300.0, 200.0);
    let plate = Rect2::new(Vector2::new(0.0, 48.0), Vector2::new(300.0, 104.0));
    let tip = place(small, plate, 60.0, Vector2::new(TIP_WIDTH, 96.0), None);
    assert_eq!(tip.position.x, 0.0);
    assert!(tip.position.y >= plate.position.y + TIP_PAD);
}

#[test]
fn place_never_leaves_the_viewport() {
    let viewports = [
        Vector2::new(1920.0, 1080.0),
        Vector2::new(1280.0, 720.0),
        Vector2::new(800.0, 600.0),
        Vector2::new(400.0, 300.0),
    ];
    for v in viewports {
        for px in [0.0, v.x * 0.2, v.x * 0.5, v.x * 0.9] {
            let plate = Rect2::new(
                Vector2::new(px, 48.0),
                Vector2::new(600.0_f32.min(v.x), (v.y - 96.0).max(96.0)),
            );
            for row_y in [0.0, 48.0, v.y * 0.5, v.y, v.y * 2.0] {
                let tip = place(v, plate, row_y, tip_size(), None);
                assert!(tip.position.x >= 0.0, "x offscreen: {tip:?} in {v:?}");
                if v.x >= TIP_WIDTH {
                    assert!(
                        tip.position.x + tip.size.x <= v.x,
                        "right edge offscreen: {tip:?} in {v:?}"
                    );
                }
                if tip.size.y <= plate.size.y - 2.0 * TIP_PAD {
                    assert!(tip.position.y >= plate.position.y + TIP_PAD);
                    assert!(
                        tip.position.y + tip.size.y <= plate.position.y + plate.size.y - TIP_PAD
                    );
                }
            }
        }
    }
}

#[test]
fn placed_tips_satisfy_the_frame_band_contract() {
    for row_y in [-50.0, 200.0, 400.0, 900.0] {
        let tip = place(viewport(), plate(), row_y, tip_size(), None);
        let (control, _) = frame(plate(), None, Some(tip));
        assert_eq!(control.size.y, plate().size.y);
    }
}

#[test]
fn place_legend_shares_the_main_plates_top_edge() {
    let size = Vector2::new(183.0, 284.0);
    let legend = place_legend(viewport(), plate(), size);
    assert_eq!(legend.position, Vector2::new(1260.0, 200.0));
    // The plate (660..1260) sits past 75% of a 1280-wide viewport.
    let narrow = Vector2::new(1280.0, 720.0);
    let legend = place_legend(narrow, plate(), size);
    assert_eq!(legend.position, Vector2::new(660.0 - 183.0, 200.0));
    let tiny = Vector2::new(700.0, 500.0);
    let leftish = Rect2::new(Vector2::new(100.0, 48.0), Vector2::new(500.0, 300.0));
    let legend = place_legend(tiny, leftish, size);
    assert_eq!(legend.position.x, 0.0);
    assert_eq!(legend.position.y, 48.0, "the top line still shares");
    let short = Rect2::new(Vector2::new(660.0, 200.0), Vector2::new(600.0, 200.0));
    let legend = place_legend(viewport(), short, size);
    assert_eq!(legend.position.y, short.position.y);
}

fn tall_plate() -> Rect2 {
    Rect2::new(Vector2::new(660.0, 140.0), Vector2::new(600.0, 800.0))
}

fn tall_legend() -> Rect2 {
    Rect2::new(Vector2::new(1260.0, 140.0), Vector2::new(183.0, 284.0))
}

#[test]
fn place_stacks_the_tip_under_the_legend() {
    let legend = tall_legend();
    let floor = 140.0 + 284.0 + STACK_GAP; // 429
    let tip = place(viewport(), tall_plate(), 148.0, tip_size(), Some(legend));
    assert_eq!(tip.position.y, floor);
    let tip = place(
        viewport(),
        tall_plate(),
        floor - 1.0,
        tip_size(),
        Some(legend),
    );
    assert_eq!(tip.position.y, floor);
    let tip = place(
        viewport(),
        tall_plate(),
        floor + 1.0,
        tip_size(),
        Some(legend),
    );
    assert_eq!(tip.position.y, floor + 1.0);
    let tip = place(viewport(), tall_plate(), 10_000.0, tip_size(), Some(legend));
    assert_eq!(tip.position.y, 140.0 + 800.0 - 200.0 - TIP_PAD);
    assert_eq!(
        place(viewport(), tall_plate(), 300.0, tip_size(), None)
            .position
            .y,
        300.0
    );
    assert_eq!(
        place(viewport(), tall_plate(), 0.0, tip_size(), None)
            .position
            .y,
        140.0 + TIP_PAD
    );
}

#[test]
fn place_stacks_the_tip_on_the_flipped_side_too() {
    // The plate's right edge passes 75% of a 1280-wide viewport.
    let narrow = Vector2::new(1280.0, 900.0);
    let plate = Rect2::new(Vector2::new(400.0, 140.0), Vector2::new(600.0, 700.0));
    let legend = place_legend(narrow, plate, Vector2::new(183.0, 284.0));
    assert_eq!(legend.position.x, 400.0 - 183.0, "the legend flipped");
    let floor = 140.0 + 284.0 + STACK_GAP;
    let tip = place(narrow, plate, 148.0, tip_size(), Some(legend));
    assert_eq!(tip.position.x, 400.0 - TIP_WIDTH, "the tip flipped too");
    assert_eq!(tip.position.y, floor, "the floor applies on the left");
}

#[test]
fn place_falls_back_to_row_anchored_when_the_stack_cannot_fit() {
    // The legend's floor (200 + 284 + 5 = 489) is past the band's hi
    // (392), so the stack cannot fit.
    let legend = Rect2::new(Vector2::new(1260.0, 200.0), Vector2::new(183.0, 284.0));
    let tip = place(viewport(), plate(), 300.0, tip_size(), Some(legend));
    assert_eq!(tip.position.y, 300.0, "row-anchored, over the legend");
    let tip = place(viewport(), plate(), 0.0, tip_size(), Some(legend));
    assert_eq!(
        tip.position.y,
        200.0 + TIP_PAD,
        "the band clamp, not the floor"
    );
}

#[test]
fn frame_widens_the_control_sideways_only() {
    let plate = plate();
    let (control, origin_x) = frame(plate, None, None);
    assert_eq!(control, plate);
    assert_eq!(origin_x, 0.0);
    let tip = Rect2::new(Vector2::new(1260.0, 300.0), tip_size());
    let (control, origin_x) = frame(plate, None, Some(tip));
    assert_eq!(control.position, plate.position);
    assert_eq!(control.size.x, plate.size.x + TIP_WIDTH);
    assert_eq!(control.size.y, plate.size.y, "never a vertical extension");
    assert_eq!(origin_x, 0.0);
    let tip = Rect2::new(Vector2::new(300.0, 300.0), tip_size());
    let (control, origin_x) = frame(plate, None, Some(tip));
    assert_eq!(control.position.x, 300.0);
    assert_eq!(control.size.x, plate.size.x + TIP_WIDTH);
    assert_eq!(origin_x, 660.0 - 300.0);
}

#[test]
fn frame_unions_the_legend_and_the_tip() {
    let plate = plate();
    let legend = Rect2::new(Vector2::new(1260.0, 200.0), Vector2::new(183.0, 284.0));
    let tip = Rect2::new(Vector2::new(1260.0, 400.0), tip_size());
    let (control, origin_x) = frame(plate, Some(legend), Some(tip));
    assert_eq!(control.position, plate.position);
    assert_eq!(control.size.x, plate.size.x + TIP_WIDTH);
    assert_eq!(control.size.y, plate.size.y);
    assert_eq!(origin_x, 0.0);
    let left = Rect2::new(Vector2::new(477.0, 200.0), Vector2::new(183.0, 284.0));
    let (control, origin_x) = frame(plate, Some(left), None);
    assert_eq!(control.position.x, 477.0);
    assert_eq!(control.size.x, plate.size.x + 183.0);
    assert_eq!(origin_x, 660.0 - 477.0);
    let short = Rect2::new(Vector2::new(660.0, 200.0), Vector2::new(600.0, 200.0));
    let legend = Rect2::new(Vector2::new(1260.0, 200.0), Vector2::new(183.0, 284.0));
    let (control, origin_x) = frame(short, Some(legend), None);
    assert_eq!(control.position, short.position);
    assert_eq!(control.size.y, 200.0 + 284.0 - 200.0);
    assert_eq!(origin_x, 0.0);
}

#[test]
fn max_tip_lines_scales_with_the_box_and_floors() {
    assert_eq!(max_tip_lines(96.0), 1);
    assert_eq!(max_tip_lines(0.0), 1);
    let full = max_tip_lines(984.0);
    assert!(full >= 30, "a full-height box fits a full detail: {full}");
    let box_h = 300.0;
    assert!(tip_height(max_tip_lines(box_h)) <= box_h - 2.0 * TIP_PAD);
}
