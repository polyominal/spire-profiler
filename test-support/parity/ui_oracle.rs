//! Injected into a clean c928c47 source export; no current implementation is linked.
use std::path::Path;

use serde_json::{Value, json};

use crate::data::records::PlayerRec;
use crate::data::run_history::{self, CombatView, PlayerRollup, RunSummaryView};
use crate::data::state::{
    CardStat, Combat, CombatResult, PlayerFilter, RunOutcome, STATE, State, StorePaths,
};
use crate::engine::math::{Rect2, Vector2};
use crate::engine::object::TextAlign;
use crate::source_kind::SourceKind;
use crate::ui::chart_layout::{self, AvatarFact, BuildInput, Cmd};
use crate::ui::ui_model::{UiMeta, UiRow, UiTab};
use crate::ui::{
    palette, panel_common, panel_replay, run_layout, scroll, snapshot, theme, tooltip,
};

const BASELINE: &str = "c928c477852e75ffcc35f8cd16c7ed03caad7c7f";

fn command(command: &Cmd) -> Value {
    match command {
        Cmd::Rect(r) => json!({"type":"rect","x":r.x,"y":r.y,"w":r.w,"h":r.h,"color":r.color}),
        Cmd::Text(t) => {
            let align = match t.align {
                TextAlign::Left => json!({"kind":"Left"}),
                TextAlign::LeftClipped(width) => json!({"kind":"LeftClipped","width":width}),
                TextAlign::Right(width) => json!({"kind":"Right","width":width}),
                TextAlign::Center(width) => json!({"kind":"Center","width":width}),
            };
            json!({"type":"text","x":t.x,"y":t.y,"size":t.size,"color":t.color,
                "role":format!("{:?}",t.role),"effect":format!("{:?}",t.effect),"align":align,"text":t.text})
        }
        Cmd::Texture(t) => {
            let (icon, slot) = match t.icon {
                theme::IconId::Character(slot) => ("Character", Some(slot)),
                theme::IconId::TabPlate => ("TabPlate", None),
                theme::IconId::TabStroke => ("TabStroke", None),
            };
            json!({"type":"texture","x":t.x,"y":t.y,"w":t.w,"h":t.h,"icon":icon,"slot":slot})
        }
    }
}
fn commands(commands: &[Cmd]) -> Vec<Value> {
    commands.iter().map(command).collect()
}
fn rect(r: Rect2) -> Value {
    json!({"x":r.position.x,"y":r.position.y,"w":r.size.x,"h":r.size.y})
}
fn content(c: theme::ContentBox) -> Value {
    json!({"x":c.x,"top":c.top,"w":c.w,"outer_bottom_pad":c.outer_bottom_pad})
}
fn row(r: &UiRow) -> Value {
    json!({"section":format!("{:?}",r.section),"kind":r.kind as u8,"player":r.player,"flags":r.flags,
        "id":r.name_str(),"name_len":r.name_len,"plays":r.plays,"value":r.value,"share_x10":r.share_x10,"seg_milli":r.seg_milli})
}
fn card(c: &CardStat) -> Value {
    json!({"player":c.player,"id":c.id,"kind":c.kind as u8,"plays":c.plays,"damage_dealt":c.damage_dealt,
        "damage_blocked":c.damage_blocked,"block_gained":c.block_gained,"block_effective":c.block_effective,
        "dmg_direct":c.dmg_direct,"dmg_attributed":c.dmg_attributed,"dmg_modifier":c.dmg_modifier,
        "blk_modifier":c.blk_modifier,"mitigate_debuff":c.mitigate_debuff,"mitigate_buff":c.mitigate_buff,
        "mitigate_str":c.mitigate_str,"self_damage":c.self_damage,"forge":c.forge})
}
fn meta(m: UiMeta) -> Value {
    json!({"turns":m.turns,"plays":m.plays,"combats":m.combats,"total_damage":m.total_damage,
        "damage_taken":m.damage_taken,"dps_x10":m.dps_x10,"encounter":m.encounter_str()})
}
fn tip_lines(detail: &tooltip::RowDetail, max_lines: usize) -> Vec<Value> {
    tooltip::shape(detail,max_lines).iter().map(|line| json!({"text":line.text,"title":line.title,
        "color":line.color,"value":line.value.as_ref().map(|value| json!({"text":value.text,"color":value.color}))})).collect()
}
fn details(rows: &[UiRow], cards: &[CardStat]) -> Vec<Value> {
    (0..rows.len()).map(|index| {
        let detail = snapshot::ui_row_detail_from_cards(rows,index,cards);
        json!({"index":index,"title":detail.title,"lines":tip_lines(&detail,64),"truncated_lines":tip_lines(&detail,3)})
    }).collect()
}
fn rows(cards: &[CardStat]) -> Vec<UiRow> {
    let mut output = vec![UiRow::default(); 256];
    let count = snapshot::ui_snapshot_rows_from(cards, &mut output);
    output.truncate(count);
    output
}
fn fixture_card(
    id: &str,
    player: u8,
    kind: SourceKind,
    damage: i64,
    defense: i64,
    self_damage: i64,
) -> CardStat {
    CardStat {
        id: id.into(),
        player,
        kind,
        plays: 2,
        damage_dealt: damage,
        dmg_direct: damage,
        block_gained: defense,
        block_effective: defense,
        self_damage,
        ..CardStat::default()
    }
}
fn mixed_cards() -> Vec<CardStat> {
    let mut cards = vec![
        fixture_card("STRIKE_IRONCLAD", 0, SourceKind::Card, 2, 0, 0),
        fixture_card("STRIKE_IRONCLAD", 1, SourceKind::Card, 1, 0, 0),
        fixture_card("ANCHOR", 0, SourceKind::Relic, 0, 12, 0),
        fixture_card("CRIMSON_MANTLE", 0, SourceKind::Power, 0, 100, 90),
        fixture_card("NET_ZERO", 1, SourceKind::Card, 0, 10, 10),
        fixture_card("NET_NEGATIVE", 1, SourceKind::Card, 0, 2, 8),
        fixture_card("SELF_ONLY", 0, SourceKind::Card, 0, 0, 5),
        fixture_card("FIRE_POTION", 1, SourceKind::Potion, 20, 0, 0),
        fixture_card("OSTY", 0, SourceKind::Osty, 9, 8, 0),
        fixture_card("UNKNOWN", 4, SourceKind::Unknown, 3, 0, 0),
        fixture_card("STRIKE_IRONCLAD", 0, SourceKind::Relic, 2, 0, 0),
        fixture_card(
            "LONG_IDENTIFIER_ABCDEFGHIJKLMNOPQRSTUVWXYZ_0123456789_ABCDEFGHIJKLMNOPQRSTUVWXYZ",
            2,
            SourceKind::Power,
            8,
            0,
            0,
        ),
        fixture_card("é界🙂".repeat(22).as_str(), 3, SourceKind::Card, 7, 0, 0),
        fixture_card("é".repeat(25).as_str(), 3, SourceKind::Card, 6, 0, 0),
        fixture_card("🙂".repeat(12).as_str(), 3, SourceKind::Card, 5, 0, 0),
    ];
    cards[0].damage_blocked = 1;
    cards[3].blk_modifier = 4;
    cards[3].mitigate_debuff = 3;
    cards[3].mitigate_buff = 2;
    cards[3].mitigate_str = 1;
    cards[7].plays = 0;
    cards[8].dmg_direct = 3;
    cards[8].dmg_attributed = 4;
    cards[8].dmg_modifier = 2;
    cards.push(CardStat {
        id: "FORGE_ONLY".into(),
        forge: 13,
        ..CardStat::default()
    });
    cards
}
fn render_context(
    width: f32,
    height: f32,
    header_bottom: f32,
    strip: f32,
    flat: bool,
    header: &[Cmd],
    body: &[Cmd],
    hits: &[chart_layout::RowHit],
    detail: &tooltip::RowDetail,
    show_legend: bool,
) -> Value {
    let viewport = Vector2::new(1920.0, 1080.0);
    let mut offset = 0.0;
    let (size, position) = panel_common::modal_box(Some(viewport), width, height, &mut offset);
    let position = position.expect("render viewport exists");
    let strip_vector = Vector2::new(0.0, strip);
    let plate = Rect2::new(position + strip_vector, size - strip_vector);
    let legend_plate = palette::legend_plate(!flat);
    let legend = show_legend.then(|| tooltip::place_legend(viewport, plate, legend_plate.size));
    let lines = panel_common::reshape_tip(detail, size.y);
    let tip = hits
        .iter()
        .find(|hit| hit.flat_index == 1)
        .filter(|_| !lines.is_empty())
        .map(|hit| {
            tooltip::place(
                viewport,
                plate,
                position.y + hit.y0 - offset,
                Vector2::new(360.0, tooltip::tip_height(lines.len())),
                legend,
            )
        });
    let (frame, origin_x) = tooltip::frame(plate, legend, tip);
    let frame = Rect2::new(frame.position - strip_vector, frame.size + strip_vector);
    let body_clip = panel_common::body_frame(origin_x, size, !flat, header_bottom);
    let body_clip = Rect2::new(body_clip.position + frame.position, body_clip.size);
    let band = panel_replay::body_band(size.y, !flat, header_bottom);
    let scrollbar = scroll::scrollbar_geom(size, !flat, band, height, offset);
    let mut legend_commands = vec![];
    if let Some(legend) = legend {
        palette::emit_legend(
            &mut legend_commands,
            legend.position.x + legend_plate.origin.x,
            legend.position.y + legend_plate.origin.y,
        );
    }
    let plan = match panel_replay::FontPlan::scan(header, body, detail) {
        panel_replay::FontPlan::Kreon => "Kreon",
        panel_replay::FontPlan::Fallback => "Fallback",
        panel_replay::FontPlan::NoText => "NoText",
    };
    let (shadow, plate_body) = theme::plate_rects(plate.size);
    json!({"viewport":[1920,1080],"flat_chrome":flat,"control":rect(frame),"plate":rect(plate),"origin_x":origin_x,"box_size":[size.x,size.y],
        "header_offset":[position.x,position.y],"body_offset":[position.x,position.y-offset],"body_clip":rect(body_clip),"scroll":offset,"font_plan":plan,
        "plate_shadow":rect(Rect2::new(shadow.position+plate.position,shadow.size)),"plate_body":rect(Rect2::new(plate_body.position+plate.position,plate_body.size)),
        "legend":legend.map(rect),"legend_commands":commands(&legend_commands),"tip":tip.map(rect),"tip_lines":tip_lines(detail,tooltip::max_tip_lines(size.y)),
        "scrollbar":scrollbar.map(|s|{let translated=|r:Rect2|rect(Rect2::new(r.position+position,r.size));json!({"track":translated(s.track),"body":translated(s.body),"cap_top":translated(s.cap_top),"cap_bottom":translated(s.cap_bottom),"grabber":translated(s.grabber)})})})
}
fn render_contract() -> Value {
    json!({"fonts":{"Title":theme::FONT_TITLE_PATH,"Body":theme::FONT_BODY_PATH},
        "plate":{"path":theme::PLATE_PATH,"region":theme::PLATE_REGION,"margins":theme::PLATE_MARGINS,"tiled":true,"shadow":theme::PLATE_SHADOW_MODULATE},
        "textures":{"TabPlate":{"path":theme::TAB_PLATE_PATH,"modulate":theme::TAB_PLATE_MODULATE},"TabStroke":{"path":theme::TAB_STROKE_PATH,"modulate":theme::TAB_STROKE_MODULATE},
            "ScrollCenter":theme::SCROLL_TRACK_CENTER_PATH,"ScrollEdge":theme::SCROLL_TRACK_EDGE_PATH,"ScrollTrain":theme::SCROLL_TRAIN_PATH},
        "scroll_track_modulate":theme::SCROLL_TRACK_MODULATE,"avatar_dim_modulate":theme::AVATAR_DIM_MODULATE,
        "panel_background":palette::COL_PANEL_BG,"panel_border":palette::COL_PANEL_BORDER,"backdrop":[0,0,0,0.8],
        "text_effects":{"Plain":[{"offset":[0,0],"color":"main"}],"Shadow":[{"offset":[3,2],"color":palette::COL_SHADOW},{"offset":[0,0],"color":"main"}],
            "Outline":[{"offset":[5,4],"color":palette::COL_HEADER_SHADOW},{"offset":[-1,-1],"color":palette::COL_HEADER_OUTLINE},{"offset":[1,-1],"color":palette::COL_HEADER_OUTLINE},
                {"offset":[-1,1],"color":palette::COL_HEADER_OUTLINE},{"offset":[1,1],"color":palette::COL_HEADER_OUTLINE},{"offset":[0,0],"color":"main"}]},
        "tooltip":{"size":theme::SIZE_TOOLTIP,"text_x":theme::PLATE_PAD_LEFT,"value_x":theme::PLATE_PAD_LEFT+170.0,"value_width":360.0-theme::PLATE_SHADOW_OFFSET-theme::PLATE_PAD_RIGHT-theme::PLATE_PAD_LEFT-170.0,
            "first_baseline":theme::PLATE_PAD_TOP+22.0,"line_height":26.0,"shadow_offset":[3,2],"shadow":palette::COL_TIP_SHADOW}})
}
fn layout_output(l: &chart_layout::Layout) -> Value {
    json!({"width":l.width,"height":l.height,"header_bottom":l.header_bottom,"strip_h":l.strip_h,"content":content(l.content),
        "header":commands(&l.header_cmds),"body":commands(&l.cmds),
        "row_hits":l.row_hits.iter().map(|h|json!({"y0":h.y0,"y1":h.y1,"flat_index":h.flat_index})).collect::<Vec<_>>(),
        "tab_hits":l.tab_hits.iter().map(|h|json!({"x0":h.x0,"y0":h.y0,"x1":h.x1,"y1":h.y1,"tab":format!("{:?}",h.tab)})).collect::<Vec<_>>(),
        "avatar_hits":l.avatar_hits.iter().map(|h|json!({"x0":h.x0,"y0":h.y0,"x1":h.x1,"y1":h.y1,"slot":h.slot})).collect::<Vec<_>>(),
        "portrait_paths":l.portrait_paths})
}
fn state_cases(cases: &mut Vec<Value>, cards: &[CardStat]) {
    for tab in UiTab::ALL {
        for player in [None, Some(0), Some(1), Some(4), Some(3)] {
            STATE.with(|state| {
                let mut state = state.borrow_mut();
                *state = State::default();
                state.store_paths = Some(StorePaths::new(Path::new("unused-parity-store")));
                state.player_filter = player.map_or(PlayerFilter::All, PlayerFilter::Player);
                state.current = Some(Combat {
                    cards: cards.to_vec(),
                    turns: 3,
                    plays: 27,
                    damage_received: 17,
                    block_total: 222,
                    potions_used: 2,
                    encounter_id: "BYGONE_EFFIGY".into(),
                    ..Combat::default()
                });
                state.run_cards = cards.to_vec();
                state.run_turns = 7;
                state.run_combats = 2;
            });
            let mut output = vec![UiRow::default(); 256];
            let count = snapshot::ui_snapshot_rows(tab, &mut output);
            output.truncate(count);
            cases.push(json!({"name":format!("state_{tab:?}_{player:?}"),"kind":"state",
                "input":{"tab":format!("{tab:?}"),"player":player,"cards":cards.iter().map(card).collect::<Vec<_>>(),
                    "turns":3,"plays":27,"damage_received":17,"block_total":222,"potions_used":2,"encounter":"BYGONE_EFFIGY","run_turns":7,"run_combats":2},
                "expected":{"rows":output.iter().map(row).collect::<Vec<_>>(),"meta":meta(snapshot::ui_snapshot_meta(tab)),"footer":snapshot::ui_footer_text(tab),"details":details(&output,cards)}}));
        }
    }
    STATE.with(|state| *state.borrow_mut() = State::default());
}
fn chart_cases(cases: &mut Vec<Value>, cards: &[CardStat]) {
    let cards = cards
        .iter()
        .filter(|card| card.id.is_ascii())
        .cloned()
        .collect::<Vec<_>>();
    let rows = rows(&cards);
    for tab in UiTab::ALL {
        for flat in [false, true] {
            for avatars in [false, true] {
                let facts = if avatars {
                    vec![
                        AvatarFact {
                            slot: 0,
                            loaded: true,
                            path: "res://images/ui/top_panel/character_icon_ironclad.png".into(),
                        },
                        AvatarFact {
                            slot: 1,
                            loaded: false,
                            path: "missing.png".into(),
                        },
                        AvatarFact {
                            slot: 2,
                            loaded: true,
                            path: "res://images/ui/top_panel/character_icon_silent.png".into(),
                        },
                    ]
                } else {
                    vec![]
                };
                let mut m = UiMeta {
                    turns: 3,
                    plays: 27,
                    combats: 2,
                    total_damage: 79,
                    damage_taken: 17,
                    dps_x10: 263,
                    ..UiMeta::default()
                };
                m.encounter[..13].copy_from_slice(b"BYGONE_EFFIGY");
                m.encounter_len = 13;
                let footer = "TOTAL 79 dmg | 17 taken | 222 block | pots 2 | forge 13\n";
                let input = BuildInput {
                    tab,
                    rows: &rows,
                    meta: m,
                    footer,
                    hover_row: Some(1),
                    avatars: &facts,
                    flat_chrome: flat,
                    tab_sprites: !flat,
                    width: 780.0,
                    right_gutter: 32.0,
                    ..BuildInput::default()
                };
                let output = chart_layout::build(input);
                let detail = snapshot::ui_row_detail_from_cards(&rows, 1, &cards);
                let render = render_context(
                    output.width,
                    output.height,
                    output.header_bottom,
                    output.strip_h,
                    flat,
                    &output.header_cmds,
                    &output.cmds,
                    &output.row_hits,
                    &detail,
                    true,
                );
                cases.push(json!({"name":format!("chart_{tab:?}_flat{flat}_avatars{avatars}"),"kind":"chart",
                    "input":{"tab":format!("{tab:?}"),"rows":rows.iter().map(row).collect::<Vec<_>>(),"meta":meta(m),"footer":footer,
                        "hover_row":1,"flat_chrome":flat,"tab_sprites":!flat,"width":780,"right_gutter":32,"skip_chrome":false,
                        "avatars":facts.iter().map(|a|json!({"slot":a.slot,"loaded":a.loaded,"path":a.path})).collect::<Vec<_>>()},"expected":layout_output(&output),"render":render}));
            }
        }
    }
    for (name, width, skip_chrome, tab_sprites) in [
        ("empty", 780.0, false, false),
        ("narrow", 610.0, false, true),
        ("body_only", 780.0, true, false),
    ] {
        let input = BuildInput {
            rows: if name == "empty" { &[] } else { &rows },
            width,
            skip_chrome,
            tab_sprites,
            ..BuildInput::default()
        };
        let output = chart_layout::build(input);
        cases.push(json!({"name":format!("chart_{name}"),"kind":"chart","input":{"tab":"Combat","rows":if name=="empty"{vec![]}else{rows.iter().map(row).collect()},
            "meta":meta(UiMeta::default()),"footer":"","hover_row":null,"flat_chrome":true,"tab_sprites":tab_sprites,"width":width,"right_gutter":0,"skip_chrome":skip_chrome,"avatars":[]},"expected":layout_output(&output)}));
    }
}
fn history_cases(cases: &mut Vec<Value>, cards: &[CardStat]) {
    let cards = cards
        .iter()
        .filter(|card| card.id.is_ascii())
        .cloned()
        .collect::<Vec<_>>();
    for (label, outcome, ascension, empty) in [
        ("defeat", Some(RunOutcome::Defeat), 7, false),
        ("victory", Some(RunOutcome::Victory), 0, false),
        ("abandoned", Some(RunOutcome::Abandoned), 20, false),
        ("unfinished", None, -1, false),
        ("missing", None, -1, true),
    ] {
        for (flat, player) in [(false, None), (true, Some(1)), (false, Some(3))] {
            run_history::clear();
            if let Some(slot) = player {
                run_history::toggle_run_filter(slot);
            }
            let view = RunSummaryView {
                run_id: 2,
                character: "IRONCLAD,SILENT".into(),
                ascension,
                game_mode: if ascension < 0 {
                    "".into()
                } else {
                    "Standard".into()
                },
                outcome,
                seed: "LONG_SEED_ABCDEFGHIJKLMNOPQRSTUVWXYZ_0123456789".into(),
                players: vec![
                    PlayerRec {
                        slot: 0,
                        character: "IRONCLAD".into(),
                    },
                    PlayerRec {
                        slot: 1,
                        character: "SILENT".into(),
                    },
                ]
                .into(),
                combats: vec![
                    CombatView {
                        seq: 1,
                        encounter: "A".into(),
                        result: CombatResult::Completed,
                        damage_dealt: 79,
                        damage_taken: 17,
                        turns: 3,
                    },
                    CombatView {
                        seq: 2,
                        encounter: "B".into(),
                        result: CombatResult::Defeat,
                        damage_dealt: 31,
                        damage_taken: 9,
                        turns: 4,
                    },
                ]
                .into(),
                rollup: cards.to_vec().into(),
                player_rollups: vec![PlayerRollup {
                    slot: 1,
                    character: "SILENT".into(),
                    cards: cards.iter().filter(|c| c.player == 1).cloned().collect(),
                }]
                .into(),
                ..RunSummaryView::default()
            };
            let facts = run_layout::HeaderFacts {
                portraits: vec![
                    run_layout::PortraitFact {
                        slot: 0,
                        loaded: !flat,
                        path: "res://images/ui/top_panel/character_icon_ironclad.png".into(),
                    },
                    run_layout::PortraitFact {
                        slot: 1,
                        loaded: !flat,
                        path: "res://images/ui/top_panel/character_icon_silent.png".into(),
                    },
                ]
                .into(),
            };
            let mut buffer = vec![UiRow::default(); 256];
            let output = run_layout::build_run_layout(
                if empty { None } else { Some(&view) },
                &facts,
                Some(1),
                &mut buffer,
                780.0,
                flat,
                32.0,
            );
            let filtered = run_history::filtered_rollup(&view);
            let output_rows = rows(filtered);
            let detail = if empty {
                tooltip::RowDetail::default()
            } else {
                snapshot::ui_row_detail_from_cards(&output_rows, 1, filtered)
            };
            let render = render_context(
                output.width,
                output.height,
                output.header_bottom,
                0.0,
                flat,
                &output.header_cmds,
                &output.cmds,
                &output.row_hits,
                &detail,
                output.has_chart,
            );
            cases.push(json!({"name":format!("history_{label}_flat{flat}_{player:?}"),"kind":"history",
                "input":{"missing":empty,"player":player,"ascension":ascension,"game_mode":view.game_mode,"outcome":label,"seed":view.seed,"character":view.character,
                    "cards":cards.iter().map(card).collect::<Vec<_>>(),"player_rollups":[{"slot":1,"cards":cards.iter().filter(|c|c.player==1).map(card).collect::<Vec<_>>()}],
                    "combats":[{"turns":3,"damage_taken":17},{"turns":4,"damage_taken":9}],"width":780,"flat_chrome":flat,"right_gutter":32,"hover_row":1,
                    "portraits":facts.portraits.iter().map(|p|json!({"slot":p.slot,"path":p.path,"loaded":p.loaded})).collect::<Vec<_>>()},
                "expected":{"width":output.width,"height":output.height,"header_bottom":output.header_bottom,"content":content(output.content),"has_chart":output.has_chart,"chart_rows":output.chart_rows,
                    "header":commands(&output.header_cmds),"body":commands(&output.cmds),"portrait_paths":output.portrait_paths,
                    "avatar_hits":output.avatar_hits.iter().map(|h|json!({"x0":h.x0,"y0":h.y0,"x1":h.x1,"y1":h.y1,"slot":h.slot})).collect::<Vec<_>>(),
                    "row_hits":output.row_hits.iter().map(|h|json!({"y0":h.y0,"y1":h.y1,"flat_index":h.flat_index})).collect::<Vec<_>>(),
                    "rows":if empty{vec![]}else{output_rows.iter().map(row).collect()},"details":if empty{vec![]}else{details(&output_rows,filtered)}},"render":render}));
        }
    }
    run_history::clear();
}
fn geometry_cases(cases: &mut Vec<Value>) {
    for (width, height, content_height, requested_scroll, plate) in [
        (1920.0, 1080.0, 400.0, 60.0, true),
        (1920.0, 1080.0, 1800.0, 90.0, true),
        (1280.0, 720.0, 1800.0, 99999.0, false),
        (800.0, 600.0, 600.0, 0.0, true),
        (640.0, 480.0, 1800.0, 300.0, true),
    ] {
        let viewport = Vector2::new(width, height);
        let mut offset = requested_scroll;
        let (size, position) =
            panel_common::modal_box(Some(viewport), 780.0, content_height, &mut offset);
        let position = position.expect("supplied viewport");
        let panel = Rect2::new(position, size);
        let band = panel_replay::body_band(size.y, plate, 240.0);
        let scrollbar = scroll::scrollbar_geom(size, plate, band, content_height, offset);
        let legend_plate = palette::legend_plate(plate);
        let legend = tooltip::place_legend(viewport, panel, legend_plate.size);
        let lines = tooltip::max_tip_lines(size.y).min(8);
        let tip_size = Vector2::new(360.0, tooltip::tip_height(lines));
        let tip = tooltip::place(viewport, panel, position.y + 280.0, tip_size, Some(legend));
        let (frame, origin_x) = tooltip::frame(panel, Some(legend), Some(tip));
        let mut legend_commands = vec![];
        palette::emit_legend(
            &mut legend_commands,
            legend_plate.origin.x,
            legend_plate.origin.y,
        );
        cases.push(json!({"name":format!("geometry_{width}_{height}_{content_height}_{plate}"),"kind":"geometry",
            "input":{"viewport":[width,height],"width":780,"content_height":content_height,"requested_scroll":requested_scroll,"plate":plate,"header_bottom":240,"tip_lines":lines,"row_offset":280},
            "expected":{"panel":rect(panel),"scroll":offset,"band":[band.0,band.1],"gutter":panel_common::scrollbar_gutter(content_height,size.y,true),
                "scrollbar":scrollbar.map(|s|json!({"track":rect(s.track),"body":rect(s.body),"cap_top":rect(s.cap_top),"cap_bottom":rect(s.cap_bottom),"grabber":rect(s.grabber)})),
                "legend":rect(legend),"tip":rect(tip),"frame":rect(frame),"origin_x":origin_x,"legend_commands":commands(&legend_commands)}}));
    }
    let mut input = vec![];
    let mut expected = vec![];
    for (button, pressed, pan) in [
        (4, true, 0.0),
        (5, true, 0.0),
        (4, false, 0.0),
        (0, false, 0.5),
        (0, false, -7.25),
        (9, true, 3.5),
    ] {
        input.push(json!({"button":button,"pressed":pressed,"pan":pan}));
        expected.push(json!(scroll::event_scroll_delta(button, pressed, pan)));
    }
    cases.push(
        json!({"name":"scroll_events","kind":"scroll_events","input":input,"expected":expected}),
    );
    let mut input = vec![];
    let mut expected = vec![];
    for active in [false, true] {
        for pressed in [false, true] {
            for previous in [false, true] {
                for track in [false, true] {
                    input.push(json!([active, pressed, previous, track]));
                    expected.push(json!(scroll::scrollbar_state_next(
                        active, pressed, previous, track
                    )));
                }
            }
        }
    }
    cases.push(json!({"name":"scroll_drag_state","kind":"scroll_drag_state","input":input,"expected":expected}));
    let mut input = vec![];
    let mut expected = vec![];
    for (offset, delta, box_h, content_h) in [
        (0.0, -60.0, 200.0, 400.0),
        (180.0, 60.0, 200.0, 400.0),
        (200.0, -60.0, 200.0, 400.0),
        (100.0, 60.0, 400.0, 300.0),
    ] {
        input.push(json!([offset, delta, box_h, content_h]));
        expected.push(json!(scroll::apply_scroll(offset, delta, box_h, content_h)));
    }
    cases.push(
        json!({"name":"scroll_clamps","kind":"scroll_clamps","input":input,"expected":expected}),
    );
}
fn hover_cases(cases: &mut Vec<Value>) {
    let hits = [
        chart_layout::RowHit {
            y0: 100.0,
            y1: 140.0,
            flat_index: 0,
        },
        chart_layout::RowHit {
            y0: 140.0,
            y1: 180.0,
            flat_index: 1,
        },
    ];
    let panel = Rect2::new(Vector2::new(50.0, 80.0), Vector2::new(780.0, 180.0));
    let band = (100.0, 168.0);
    let mut queries = vec![];
    let mut expected = vec![];
    for scroll in [0.0, 40.0, 80.0] {
        for (x, y) in [
            (49.9, 180.0),
            (50.0, 180.0),
            (829.9, 180.0),
            (830.0, 180.0),
            (100.0, 179.9),
            (100.0, 219.9),
            (100.0, 220.0),
            (100.0, 247.9),
            (100.0, 248.0),
            (100.0, 260.0),
        ] {
            let mouse = Vector2::new(x, y);
            queries.push(json!({"mouse":[x,y],"scroll":scroll}));
            expected.push(json!({"over":panel_common::over_panel(panel,mouse),"hover":panel_common::hover_row(&hits,panel,mouse,scroll,band)}));
        }
    }
    cases.push(json!({"name":"hover_boundaries","kind":"hover","input":{"panel":rect(panel),"band":[band.0,band.1],"hits":hits.iter().map(|h|json!({"y0":h.y0,"y1":h.y1,"flat_index":h.flat_index})).collect::<Vec<_>>(),"queries":queries},"expected":expected}));
}

#[test]
fn export_reference() {
    let destination =
        std::env::var_os("PARITY_OUTPUT").expect("PARITY_OUTPUT is the output JSON path");
    let mut cases = vec![];
    let mixed = mixed_cards();
    let capped = (0..140)
        .map(|index| {
            fixture_card(
                &format!("SOURCE_{index:03}"),
                (index % 2) as u8,
                SourceKind::Card,
                if index < 128 { 1 } else { 1000 },
                10,
                3,
            )
        })
        .collect::<Vec<_>>();
    for (name, cards) in [
        ("empty", vec![]),
        ("mixed", mixed.clone()),
        (
            "thirds",
            vec![
                fixture_card("TWO", 0, SourceKind::Card, 2, 0, 0),
                fixture_card("ONE", 0, SourceKind::Card, 1, 0, 0),
            ],
        ),
        ("caps", capped),
    ] {
        let output = rows(&cards);
        cases.push(json!({"name":format!("snapshot_{name}"),"kind":"snapshot","input":{"cards":cards.iter().map(card).collect::<Vec<_>>()},
            "expected":{"rows":output.iter().map(row).collect::<Vec<_>>(),"details":details(&output,&cards)}}));
    }
    state_cases(&mut cases, &mixed);
    chart_cases(&mut cases, &mixed);
    history_cases(&mut cases, &mixed);
    geometry_cases(&mut cases);
    hover_cases(&mut cases);
    let output =
        json!({"baseline":BASELINE,"schema":1,"render_contract":render_contract(),"cases":cases});
    std::fs::write(
        destination,
        serde_json::to_string_pretty(&output).expect("JSON values serialize") + "\n",
    )
    .expect("write parity reference");
}
