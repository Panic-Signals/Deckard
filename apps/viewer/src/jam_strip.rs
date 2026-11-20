//! The Jam's on-screen strip: the focused OSO page the surface host stored
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

/// A slot's size and the gap after it.
const SLOT: Vec2 = Vec2::new(52.0, 44.0);
const GAP: f32 = 4.0;
/// The band above the slots carrying the group headers.
const HEADER_BAND: f32 = 12.0;
/// The corner brackets' arm length, in points.
const BRACKET: f32 = 7.0;

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

/// The strip's size for a page of `slots` slots: its slots and the gaps between
/// them, plus the header band.
pub fn strip_size(slots: usize) -> Vec2 {
    Vec2::new(slots as f32 * (SLOT.x + GAP) - GAP, SLOT.y + HEADER_BAND)
}

/// Where the page's `slot` sits, in points from the strip's top-left corner.
pub fn slot_rect(slot: usize) -> egui::Rect {
    let at = Vec2::new(slot as f32 * (SLOT.x + GAP), HEADER_BAND);
    egui::Rect::from_min_size(egui::Pos2::new(at.x, at.y), SLOT)
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

/// The slots: one cell per published slot, the section header over each, the focused slot
/// underlined, the editing slot in corner brackets. The mouse does nothing here — the strip
/// belongs to the encoder — so this allocates hover sense and never reads a click.
///
/// This is the whole of what the strip draws: the title row that used to sit above it lives in
/// the window's own bar (`jam_window`), where the reference keeps it. A docked row is gone, so
/// there is no second way to draw the same page.
pub fn slots(ui: &mut egui::Ui, slots: usize, rows: &[Row]) {
    let width = strip_size(slots).x;
    let (rect, _hover) = ui.allocate_exact_size(strip_size(slots), egui::Sense::hover());
    let origin = rect.min.to_vec2();
    // Each header covers its group: consecutive slots sharing a header text. The
    // reference draws it as one label over the band, aligned to the group's
    // first slot.
    let runs = header_runs(rows.iter().map(|r| (r.slot, r.header.as_deref())));
    for (start, len, header) in &runs {
        let mut band = slot_rect(*start);
        band.max.x = slot_rect(start + len - 1).max.x;
        band.min.y = 0.0;
        band.max.y = HEADER_BAND;
        let mut band = band.translate(origin);
        // A label may run past its group into the space to its right, as the reference
        // does: clipped to its own band, a one-slot group cut PAD LAYOUT to PAD LAYOU.
        let next = runs.iter().find(|(s, ..)| *s > start + len - 1).map_or(origin.x + width, |(s, ..)| slot_rect(*s).min.x + origin.x);
        band.max.x = next.min(origin.x + width).max(band.max.x) - 2.0;
        ui.painter().with_clip_rect(band).text(
            band.left_top() + Vec2::new(4.0, 1.0),
            egui::Align2::LEFT_TOP,
            header,
            egui::FontId::proportional(9.0),
            Color32::from_gray(170),
        );
    }
    for row in rows {
        let cell = slot_rect(row.slot).translate(origin);
        let fill = if row.editing {
            Color32::from_rgb(96, 74, 28)
        } else if row.focused {
            Color32::from_gray(70)
        } else {
            Color32::from_gray(45)
        };
        ui.painter().rect_filled(cell, 3.0, fill);
        // Value above, its display name below — the reference's two rows in the cell.
        ui.painter().with_clip_rect(cell).text(
            cell.left_top() + Vec2::new(4.0, 5.0),
            egui::Align2::LEFT_TOP,
            &row.value,
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
        // Long values clip at the slot: the strip never invents a shorter name.
        ui.painter().with_clip_rect(cell).text(
            cell.left_bottom() + Vec2::new(4.0, -4.0),
            egui::Align2::LEFT_BOTTOM,
            row.sub.as_deref().unwrap_or(""),
            egui::FontId::proportional(9.0),
            Color32::from_gray(170),
        );
        if row.focused {
            ui.painter().line_segment([cell.left_bottom(), cell.right_bottom()], egui::Stroke::new(2.0, Color32::WHITE));
        }
        if row.editing {
            brackets(ui, cell);
        }
    }
}

/// The editing mark: corner brackets around the slot being changed (the
/// hardware-confirmed mark; a filled background alone is not it).
fn brackets(ui: &egui::Ui, cell: egui::Rect) {
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
    fn the_strip_is_one_row_of_equal_slots() {
        assert_eq!(strip_size(9).x, 9.0 * (SLOT.x + GAP) - GAP);
        assert_eq!(strip_size(2).y, SLOT.y + HEADER_BAND);
        let (s0, s1) = (slot_rect(0), slot_rect(1));
        assert!(s0.min.x < s1.min.x && s0.min.y == s1.min.y, "one row, left to right");
        assert_eq!(s0.size(), s1.size(), "all slots the same size");
        assert_eq!(strip_size(9).x, slot_rect(8).max.x, "the strip is exactly its slots");
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
