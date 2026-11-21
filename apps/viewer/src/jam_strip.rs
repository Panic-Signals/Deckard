//! The Jam's on-screen overlay (the manual's OSO): the focused page the surface host stored
//! (`deckard_surface::OsoPage`, published by `maschine_jam` through `deckard.oso`)
//! rendered as the reference draws it — the group header in its own band spanning
//! its slots, the value, then the sub-label; the focused slot underlined, the
//! editing slot in corner brackets, `n/N` in the title.
//!
//! Pages live in the script; rendering lives here. This module never composes page
//! membership: it renders exactly the slots the published page holds, holes stay
//! columns, and an empty page renders nothing. The cursor and its edit flag are the
//! script's on every page (menu or tab): one owner, no second cursor to drift from it.
//! The script is never asked.
//!
//! Painting is the strip the software panel used to draw (`controller::overlay`),
//! generalized from its fixed eight slots to the published page's width.

use deckard_surface::OsoPage;
use eframe::egui::{self, Color32, Vec2};

/// The body's rows, from the reference's own proportions: the group header band, the value,
/// then the sub-label, with the padding the capture shows. A strip is a wide bar — its columns
/// divide whatever width the window has, they are not a fixed cell size.
const HEADER_H: f32 = 40.0;
const VALUE_H: f32 = 40.0;
const SUB_H: f32 = 40.0;
const BOTTOM_PAD: f32 = 24.0;
/// The body's whole height, and the strip's, for a window's inner size.
pub const BODY_H: f32 = HEADER_H + VALUE_H + SUB_H + BOTTOM_PAD;

/// The text's left inset inside a column, and the column separator's width.
const PAD_X: f32 = 14.0;
const SEPARATOR: f32 = 1.0;

/// The type, as the capture has it: the header small and quiet, the value the loudest thing
/// on the strip, the sub-label between them.
const FONT_HEADER: f32 = 11.5;
const FONT_VALUE: f32 = 21.0;
const FONT_SUB: f32 = 13.0;

/// The corner brackets' arm length, in points.
const BRACKET: f32 = 7.0;

/// Everything the strip is, drawn on, from the capture: a flat body, the bar's own shade.
const PAPER: Color32 = Color32::from_rgb(0x3d, 0x3d, 0x3d);
const INK: Color32 = Color32::from_rgb(0xd8, 0xd8, 0xd8);
const HEADER_INK: Color32 = Color32::from_rgb(0x9a, 0x9a, 0x9a);
const RULE: Color32 = Color32::from_rgb(0x5a, 0x5a, 0x5a);

/// The strip is **eight slots wide**, always — the reference's own page — whatever the published
/// page holds. A page of one slot fills the first column and leaves seven empty; nothing stretches
/// and nothing shrinks, so `Scale 2/2` sits in one column exactly as its 2/2 page does in Maschine.
pub const SLOTS: usize = 8;

/// The strip's width for an app window `main` points wide: most of it, so a column is the
/// reference's ~148 points at a 1500-wide app.
pub fn width_for(main: f32) -> f32 {
    (main * 0.78).clamp(880.0, 1180.0)
}

/// Where the page's `slot` sits, in points from the strip's top-left corner: one of the eight
/// equal columns. A slot past the eighth is off the strip and draws nothing.
pub fn slot_rect(slot: usize, width: f32) -> egui::Rect {
    let w = width / SLOTS as f32;
    egui::Rect::from_min_size(egui::pos2(slot as f32 * w, 0.0), egui::vec2(w, BODY_H))
}

/// One rendered slot: its position on the published page, the spanning header
/// over it, its value text, and whether the encoder is on it (`focused`) or
/// changing it (`editing`, exactly when the edit flag is set and the slot is
/// focused).
#[derive(Debug, Clone)]
pub struct Row {
    pub slot: usize,
    pub header: Option<String>,
    pub value: String,
    /// The third row: the transcribed sub-label (`Root Note`), where the page
    /// shows one. Absent means the row stays blank, never a guessed name.
    pub sub: Option<String>,
    pub focused: bool,
    pub editing: bool,
}

/// The header runs the strip draws: each group's header with the slots it spans
/// (`CHORD` over four slots, not in every cell). A slot without a header breaks
/// a run; a hole inside a group still sits under its header, so lengths come
/// from the slot numbers instead of splitting it.
fn header_runs<'a>(slots: impl Iterator<Item = (usize, Option<&'a str>)>) -> Vec<(usize, usize, String)> {
    let mut runs: Vec<(usize, usize, String)> = Vec::new();
    for (slot, header) in slots {
        let Some(header) = header else { continue };
        match runs.last_mut() {
            // Lengths come from the slot numbers, so a hole inside a group
            // still sits under its header instead of splitting it.
            Some((start, len, text)) if *text == header => *len = slot - *start + 1,
            _ => runs.push((slot, 1, header.to_owned())),
        }
    }
    runs
}

/// The strip's title: the page's title with its counter where the script reports
/// more than one page (`Arpeggiator 1/3`). One page carries no counter.
pub fn title(page: &OsoPage) -> String {
    match counter(page) {
        Some(c) => format!("{} {c}", name(page)),
        None => name(page),
    }
}

/// The page's own name, without its counter: what the window's bar carries at its left.
pub fn name(page: &OsoPage) -> String {
    page.title.clone()
}

/// The `n/N` counter, where the script reports more than one page. One page carries none,
/// so its title bar draws no counter at all.
pub fn counter(page: &OsoPage) -> Option<String> {
    (page.pages > 1).then(|| format!("{}/{}", page.page, page.pages))
}

/// The strip's rows for `page`: one per filled slot, the script's cursor underlined and
/// its edit flag bracketed. `None` when the page holds no slot to draw.
///
/// The cursor is drawn where the script published it, holes counted. A cursor on a hole
/// underlines nothing: no second opinion is computed here, because a renderer that
/// settles its own idea of the cursor is how the underline came to sit on a slot the
/// encoder was not on. (The keyboard tab settles its own cursor before publishing; a menu
/// cursor may rest on an unavailable item, and the status line names the same one.)
pub fn rows(page: &OsoPage) -> Option<Vec<Row>> {
    let focus = page.focus;
    let rows = page
        .slots
        .iter()
        .enumerate()
        .filter_map(|(slot, s)| {
            let s = s.as_ref()?;
            let focused = slot == focus;
            Some(Row {
                slot,
                header: s.group.clone(),
                value: s.value.clone(),
                sub: s.sub.clone(),
                focused,
                editing: focused && page.editing,
            })
        })
        .collect::<Vec<_>>();
    (!rows.is_empty()).then_some(rows)
}

/// The cursor's rule: one segment under the focused column's sub-label, and none under any
/// other. The reference marks the column the encoder is on and leaves the rest bare, so a rule
/// over every column would be a cue the page does not have.
fn cursor_rules(rows: &[Row], width: f32, origin: egui::Vec2) -> Vec<[egui::Pos2; 2]> {
    rows.iter()
        .filter(|row| row.focused)
        .map(|row| {
            let cell = slot_rect(row.slot, width).translate(origin);
            let y = cell.top() + HEADER_H + VALUE_H + SUB_H - 5.0;
            let half = (cell.width() / 2.0 - PAD_X).max(0.0);
            [egui::pos2(cell.center().x - half, y), egui::pos2(cell.center().x + half, y)]
        })
        .collect()
}

/// The strip's body: one column per published slot, the group header over its run, the value
/// and the sub-label under it, the focused slot's underline and the editing slot's brackets.
/// The mouse does nothing here — the strip belongs to the encoder — so this allocates hover
/// sense and never reads a click.
///
/// What it draws is the reference's own arrangement, not a row of buttons: a flat body, a thin
/// rule between columns, three lines of type inside each, and no cell fill of any kind. A hole
/// keeps its column and draws nothing in it; a column whose group header runs into the next one
/// is labelled once, over the run's first column.
pub fn slots(ui: &mut egui::Ui, rows: &[Row], width: f32) {
    let size = egui::vec2(width, BODY_H);
    let (rect, _hover) = ui.allocate_exact_size(size, egui::Sense::hover());
    let origin = rect.min.to_vec2();
    ui.painter().rect_filled(rect, 0.0, PAPER);
    // Eight columns, whether or not the page fills them: the rules are the strip's, not the page's.
    // An empty column is background and its rules, and nothing else.
    for at in 1..SLOTS {
        let x = rect.left() + at as f32 * (width / SLOTS as f32);
        ui.painter().vline(x, rect.y_range(), egui::Stroke::new(SEPARATOR, RULE));
    }
    // Each header covers its group: consecutive slots sharing a header text, drawn once over
    // the run's first column and free to run past its own column into the gap after it.
    let runs = header_runs(rows.iter().map(|r| (r.slot, r.header.as_deref())));
    for (start, len, header) in &runs {
        let mut band = slot_rect(*start, width).translate(origin);
        band.max.x = slot_rect(start + len - 1, width).max.x + origin.x;
        band.max.y = band.min.y + HEADER_H;
        let next = runs.iter().find(|(s, ..)| *s > start + len - 1).map_or(rect.right(), |(s, ..)| slot_rect(*s, width).left() + origin.x);
        band.max.x = next.min(rect.right()).max(band.max.x) - 2.0;
        ui.painter().with_clip_rect(band).text(
            band.left_top() + Vec2::new(PAD_X, 14.0),
            egui::Align2::LEFT_TOP,
            header,
            egui::FontId::proportional(FONT_HEADER),
            HEADER_INK,
        );
    }
    for row in rows {
        let cell = slot_rect(row.slot, width).translate(origin);
        // The value: the strip's loudest line, centred in its column as the reference has it.
        ui.painter().with_clip_rect(cell.shrink2(egui::vec2(PAD_X * 0.5, 0.0))).text(
            egui::pos2(cell.center().x, cell.top() + HEADER_H + 4.0),
            egui::Align2::CENTER_TOP,
            &row.value,
            egui::FontId::proportional(FONT_VALUE),
            INK,
        );
        // Its display name below, quiet: `Root Note`, `Value`. Blank where the page has none.
        let sub_at = egui::pos2(cell.center().x, cell.top() + HEADER_H + VALUE_H);
        ui.painter().with_clip_rect(cell).text(
            sub_at,
            egui::Align2::CENTER_TOP,
            row.sub.as_deref().unwrap_or(""),
            egui::FontId::proportional(FONT_SUB),
            INK,
        );
        if row.editing {
            brackets(ui, cell);
        }
    }
    for rule in cursor_rules(rows, width, origin) {
        ui.painter().line_segment(rule, egui::Stroke::new(2.5, Color32::WHITE));
    }
}

/// The editing mark: corner brackets around the column being changed, inset from its edges and
/// spanning the value and sub-label rows, as the reference's editing capture has them. (A filled
/// background alone is not it.)
fn brackets(ui: &egui::Ui, cell: egui::Rect) {
    let cell = egui::Rect::from_min_max(
        egui::pos2(cell.left() + 6.0, cell.top() + HEADER_H + 2.0),
        egui::pos2(cell.right() - 6.0, cell.top() + BODY_H - 6.0),
    );
    let stroke = egui::Stroke::new(2.0, Color32::WHITE);
    let p = ui.painter();
    let (l, r, t, b) = (cell.left(), cell.right(), cell.top(), cell.bottom());
    for (x0, x1) in [(l, l + BRACKET), (r - BRACKET, r)] {
        p.line_segment([egui::pos2(x0, t), egui::pos2(x1, t)], stroke);
        p.line_segment([egui::pos2(x0, b), egui::pos2(x1, b)], stroke);
    }
    for (y0, y1) in [(t, t + BRACKET), (b - BRACKET, b)] {
        p.line_segment([egui::pos2(l, y0), egui::pos2(l, y1)], stroke);
        p.line_segment([egui::pos2(r, y0), egui::pos2(r, y1)], stroke);
    }
}

impl crate::App {
    /// The page the strip draws, and its rows — or `None` when nothing shows (no device, no
    /// page yet, an empty page, or the desktop browser open, which always wins over the
    /// strip). One lookup, so the docked row and the window can never disagree about whether
    /// there is a strip.
    pub(crate) fn jam_shown(&self) -> Option<(OsoPage, Vec<Row>)> {
        if self.browser.open {
            return None;
        }
        let page = self.jam_device.as_ref()?.oso()?;
        let rows = rows(&page)?;
        Some((page, rows))
    }

    /// The strip to draw: the published page's title, slot count and rows. (The window draws
    /// the page itself through `jam_shown`; this triple is what the tests assert on.)
    #[cfg(test)]
    pub(crate) fn jam_strip(&self) -> Option<(String, usize, Vec<Row>)> {
        let (page, rows) = self.jam_shown()?;
        Some((title(&page), page.slots.len(), rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_surface::{OsoPage, OsoSlot};

    fn slot(value: &str, group: Option<&str>, sub: Option<&str>) -> Option<OsoSlot> {
        Some(OsoSlot { value: value.into(), sub: sub.map(str::to_owned), group: group.map(str::to_owned) })
    }

    fn page(title: &str, page: usize, pages: usize, slots: Vec<Option<OsoSlot>>) -> OsoPage {
        OsoPage { title: title.into(), page, pages, slots, focus: 0, editing: false }
    }

    /// The page the script publishes: its own cursor and edit flag.
    fn at(mut page: OsoPage, focus: usize, editing: bool) -> OsoPage {
        page.focus = focus;
        page.editing = editing;
        page
    }

    #[test]
    fn the_title_counts_pages_only_when_there_are_two() {
        assert_eq!(title(&page("Arpeggiator", 1, 3, vec![])), "Arpeggiator 1/3");
        assert_eq!(title(&page("Arpeggiator", 2, 3, vec![])), "Arpeggiator 2/3");
        assert_eq!(title(&page("Scale", 1, 1, vec![])), "Scale", "one page carries no counter");
    }

    /// The window's bar draws the page's name at its left and the counter at its right; the
    /// strip's body draws slots only. A one-page type carries no counter anywhere.
    #[test]
    fn the_title_bar_carries_the_counter_and_the_body_does_not() {
        let three = page("Arpeggiator", 2, 3, vec![]);
        assert_eq!(name(&three), "Arpeggiator");
        assert_eq!(counter(&three).as_deref(), Some("2/3"));
        assert_eq!(title(&three), "Arpeggiator 2/3", "the whole label is still one call");
        let one = page("Scale", 1, 1, vec![]);
        assert_eq!(name(&one), "Scale");
        assert_eq!(counter(&one), None, "one page draws no counter");
    }

    #[test]
    fn rows_carry_header_value_and_sub_with_the_focus_marked() {
        let p = at(
            page(
                "Scale",
                1,
                1,
                vec![
                    slot("C3", Some("SCALE"), Some("Root Note")),
                    None,
                    slot("Main", Some("SCALE"), Some("Bank")),
                    slot("Off", None, None),
                ],
            ),
            2,
            false,
        );
        let rows = rows(&p).expect("filled slots render");
        assert_eq!(rows.len(), 3, "the hole keeps its column but draws no cell");
        assert_eq!(rows.iter().map(|r| r.slot).collect::<Vec<_>>(), [0, 2, 3]);
        assert_eq!(rows[0].header.as_deref(), Some("SCALE"));
        assert_eq!(rows[0].value, "C3");
        assert_eq!(rows[0].sub.as_deref(), Some("Root Note"));
        assert_eq!(rows[2].header, None, "no header draws nothing, never a guess");
        assert_eq!(rows[2].sub, None, "no sub-label leaves the row blank");
        assert_eq!(rows.iter().filter(|r| r.focused).count(), 1);
        assert!(rows[1].focused, "focus is the page index, holes counted");
        assert!(rows.iter().all(|r| !r.editing));
    }

    #[test]
    fn the_rule_is_drawn_under_the_focused_column_only() {
        let page = || {
            page(
                "Scale",
                1,
                1,
                vec![slot("C3", None, Some("Root Note")), slot("Main", None, Some("Bank")), slot("Chrom", None, Some("Type"))],
            )
        };
        let first = cursor_rules(&rows(&at(page(), 1, false)).expect("rows"), 800.0, egui::Vec2::ZERO);
        assert_eq!(first.len(), 1, "one rule, under the column the encoder is on, and none anywhere else");
        let [from, to] = first[0];
        assert_eq!(from.y, to.y, "a horizontal rule");
        assert!(from.x > 100.0 && to.x < 200.0, "{from:?}..{to:?} stays in the second column of eight");
        assert!(((from.x + to.x) / 2.0 - 150.0).abs() < 0.01, "centred in its column");
        let moved = cursor_rules(&rows(&at(page(), 2, false)).expect("rows"), 800.0, egui::Vec2::ZERO);
        assert_eq!(moved.len(), 1, "still one, wherever the focus goes");
        assert!(moved[0][0].x > 200.0, "and the rule follows the focus");
    }

    #[test]
    fn the_edit_mark_is_the_scripts_and_marks_only_the_focused_slot() {
        let p = page("Notes", 1, 1, vec![slot("Guitar", None, Some("Mode")), slot("C3", Some("SCALE"), Some("Root Note"))]);
        assert!(rows(&p).expect("rows").iter().all(|r| !r.editing), "browsing marks nothing");
        let editing = at(p, 1, true);
        let rows = rows(&editing).expect("rows");
        assert_eq!(rows.iter().filter(|r| r.editing).count(), 1);
        assert!(rows.iter().find(|r| r.focused).expect("a focus").editing, "the bracket is the focused slot");
        assert!(rows[0].focused == false && !rows[0].editing, "and only that one");
    }

    #[test]
    fn an_empty_page_renders_nothing() {
        assert!(rows(&page("", 1, 1, vec![])).is_none(), "the browser's empty page hides the strip");
        assert!(rows(&page("Scale", 1, 1, vec![None, None])).is_none(), "holes alone are no strip");
    }

    #[test]
    fn a_header_spans_its_whole_group() {
        // Repeated group text on every filled slot, as the script publishes it.
        let spans = [(0, Some("SCALE")), (1, Some("SCALE")), (2, Some("SCALE")), (3, Some("CHORD")), (4, Some("CHORD"))];
        assert_eq!(
            header_runs(spans.into_iter()),
            vec![(0, 3, "SCALE".to_string()), (3, 2, "CHORD".to_string())],
            "each group spans its own slots"
        );
        // A hole inside a group does not split it: the band still covers the hole's column.
        let holed = [(0, Some("CHORD")), (2, Some("CHORD")), (3, Some("PAD"))];
        assert_eq!(
            header_runs(holed.into_iter()),
            vec![(0, 3, "CHORD".to_string()), (3, 1, "PAD".to_string())],
            "a gap in the filled rows keeps the group whole"
        );
        assert!(header_runs([(0, None), (1, None)].into_iter()).is_empty());
    }

    #[test]
    fn the_strip_is_eight_columns_wide_whatever_the_page_holds() {
        // The model the reference has: eight slots, always. Its Grid page fills four and leaves
        // four empty; nothing stretches to fill the bar and nothing shrinks.
        let w = width_for(1500.0);
        assert_eq!(w, 1170.0, "most of the app's width, clamped");
        let (a, b) = (slot_rect(0, w), slot_rect(1, w));
        assert_eq!(a.width(), w / SLOTS as f32);
        assert_eq!(a.height(), BODY_H);
        assert!(a.min.x < b.min.x && a.min.y == b.min.y, "one row, left to right");
        assert_eq!(a.size(), b.size(), "all columns the same size");
        assert_eq!(slot_rect(7, w).right(), w, "the eighth column ends at the strip's edge");
        assert!(BODY_H >= 80.0, "a bar tall enough for three lines of type");
    }

    #[test]
    fn a_page_of_one_slot_keeps_one_of_the_eight_columns() {
        // The fault this replaces: `Scale 2/2` holds one slot and it was drawn across the whole
        // bar — "Horizontal" floating mid-screen. The eight columns exist whether the page fills
        // them or not, so a lone item keeps its own and the seven after it stay empty.
        let w = width_for(1500.0);
        assert_eq!(SLOTS, 8);
        assert_eq!(slot_rect(0, w).width(), w / 8.0);
        assert!(slot_rect(0, w).width() < w / 4.0, "a single item is never most of the bar");
        assert!(slot_rect(0, w).right() < w, "and the empty columns are still the strip's");
    }

    #[test]
    fn the_underline_follows_the_published_cursor_wherever_it_moves() {
        // The failure this replaces: a viewer-side cursor kept its own idea of where the
        // encoder was, so the underline sat on slot 0 while the script was elsewhere.
        let slots = || vec![slot("a", Some("SCALE"), None), slot("b", Some("SCALE"), None), slot("c", Some("SCALE"), None)];
        for focus in 0..3 {
            let rows = rows(&at(page("Scale", 1, 1, slots()), focus, false)).expect("rows");
            assert_eq!(rows.iter().position(|r| r.focused), Some(focus), "focus {focus}");
            assert_eq!(rows.iter().filter(|r| r.focused).count(), 1);
        }
    }

    #[test]
    fn a_cursor_on_a_hole_underlines_nothing() {
        // The script publishes its cursor where it is. A menu cursor can rest on an
        // unavailable item, and the strip then underlines nothing rather than a slot the
        // encoder is not on: a renderer that settles its own idea of the cursor is exactly
        // the drift this replaced.
        let holed = at(page("Scale", 1, 1, vec![slot("a", None, None), None, slot("c", None, None)]), 1, false);
        let holed_rows = rows(&holed).expect("rows");
        assert_eq!(holed_rows.len(), 2, "the hole keeps its column");
        assert!(holed_rows.iter().all(|r| !r.focused), "no underline on a hole");
        // Past the end is off the page: nothing is underlined there either.
        let short = at(page("Scale", 1, 1, vec![slot("a", None, None), slot("b", None, None), None, None]), 7, false);
        let short_rows = rows(&short).expect("rows");
        assert!(short_rows.iter().all(|r| !r.focused), "a cursor past the end marks nothing");
    }

    #[test]
    fn a_page_whose_fill_varies_keeps_the_cursor_on_a_slot() {
        // The rig's fill spread: successive rebuilds hold 9, 8, 6, 5 and 4 filled slots,
        // and whichever slot the script is on must still be drawn underlined.
        for &n in &[9usize, 8, 6, 5, 4] {
            let mut slots = vec![slot("a", Some("G"), None); 9];
            for s in slots.iter_mut().skip(n) {
                *s = None;
            }
            for focus in 0..n {
                let rows = rows(&at(page("Scale", 1, 1, slots.clone()), focus, false)).expect("rows");
                assert!(rows.iter().any(|r| r.focused && r.slot == focus), "fill {n}, focus {focus}: the cursor stays underlined");
            }
        }
    }
}
