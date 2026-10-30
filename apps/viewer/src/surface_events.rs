//! The Events page of DECKARD's own controller layer (EVENTS held): its edits of the
//! focused pattern's notes and of the selection, as the scripts' Events page makes them
//! (`deckard_lua_host`'s `events.rs`).

use crate::App;
use crate::layout::{self, Act};
use deckard_model::Note;

/// The finest step notes move and grow by (a 128th triplet's worth: Maschine's
/// smallest snap).
const MIN_SNAP: i64 = deckard_model::PPQ / 32;

impl App {
    /// The focused pattern's notes and where it plays (start, end).
    fn event_notes(&self) -> Option<(Vec<Note>, i64, i64)> {
        let d = self.doc.as_ref()?.borrow();
        let p = d.project();
        let pat = p.groups.get(self.group)?.patterns.get(self.pattern)?;
        let len = if pat.length > 0 { pat.length } else { p.bar_length() };
        Some((pat.notes.clone(), pat.start, pat.start + len))
    }

    /// The selected notes still in the focused pattern.
    fn selected_events(&self, all: &[Note]) -> Vec<Note> {
        self.selected.iter().filter(|n| all.contains(n)).copied().collect()
    }

    /// The selection, or with none what the displays show: in Keyboard mode the
    /// focused sound's notes, else the group's.
    fn shown_events(&self, all: &[Note]) -> Vec<Note> {
        let sel = self.selected_events(all);
        if !sel.is_empty() {
            return sel;
        }
        all.iter().filter(|n| !self.piano_roll || n.sound == self.sound).copied().collect()
    }

    /// Edit `notes` with `f` (given the pattern's start and end) as one undo step; the
    /// selected ones stay selected.
    fn edit_events(&mut self, label: &'static str, notes: Vec<Note>, f: impl Fn(Note, i64, i64) -> Note) {
        let Some((_, start, end)) = self.event_notes() else { return };
        let (remove, add): (Vec<Note>, Vec<Note>) = notes.into_iter().map(|n| (n, f(n, start, end))).filter(|(a, b)| a != b).unzip();
        if !remove.is_empty() {
            layout::apply_acts(self, vec![Act::EditNotes { g: self.group, p: self.pattern, label, remove, add }]);
        }
    }

    fn delete_events(&mut self, label: &'static str, remove: Vec<Note>) {
        if !remove.is_empty() {
            layout::apply_acts(self, vec![Act::EditNotes { g: self.group, p: self.pattern, label, remove, add: Vec::new() }]);
        }
    }

    /// Notes in or out of the selection: all of `mine` selected already, they leave it;
    /// else they join it.
    fn toggle_events(&mut self, all: &[Note], mine: Vec<Note>) {
        let mut sel = self.selected_events(all);
        if !mine.is_empty() && mine.iter().all(|n| sel.contains(n)) {
            sel.retain(|n| !mine.contains(n));
        } else {
            sel.extend(mine.into_iter().filter(|n| !sel.contains(n)).collect::<Vec<_>>());
        }
        self.selected = sel;
        self.selected_in = (self.group, self.pattern);
    }

    fn select_events(&mut self, sel: Vec<Note>) {
        self.selected = sel;
        self.selected_in = (self.group, self.pattern);
    }

    /// One of the Events page's edits ([`deckard_surface::Action::Events`]).
    pub(crate) fn surface_events(&mut self, what: &str, n: i64) {
        let Some((all, ..)) = self.event_notes() else { return };
        let sel = self.selected_events(&all);
        let own: Vec<Note> = all.iter().filter(|x| x.sound == self.sound).copied().collect();
        let nudge = self.nudge.ticks(self.grid);
        let around = move |x: Note, by: i64, a: i64, b: i64| Note { position: a + (x.position + by - a).rem_euclid((b - a).max(1)), ..x };
        match what {
            "nudge" | "nudge_fine" | "nudge_shown" | "nudge_shown_fine" => {
                let by = n * if what.ends_with("fine") { MIN_SNAP } else { nudge };
                let notes = if what.starts_with("nudge_shown") { self.shown_events(&all) } else { sel };
                self.edit_events("Nudge", notes, |x, a, b| around(x, by, a, b));
            }
            "pitch" => self.edit_events("Pitch", sel, |x, _, _| Note { pitch: (x.pitch + n).clamp(0, 127), ..x }),
            "transpose" => {
                let notes = self.shown_events(&all);
                self.edit_events("Transpose", notes, |x, _, _| Note { pitch: (x.pitch + n).clamp(0, 127), ..x });
            }
            "velocity" => self.edit_events("Velocity", sel, |x, _, _| Note { velocity: (x.velocity + n).clamp(1, 127), ..x }),
            "probability" => self.edit_events("Probability", sel, |x, _, _| Note { probability: (x.probability + n).clamp(0, 100), ..x }),
            "velocity_to" => self.edit_events("Velocity", sel, |x, _, _| Note { velocity: n.clamp(1, 127), ..x }),
            "probability_to" => self.edit_events("Probability", sel, |x, _, _| Note { probability: n.clamp(0, 100), ..x }),
            "length" | "length_fine" => {
                let step = if what == "length_fine" { MIN_SNAP } else { self.grid.max(MIN_SNAP) };
                // (No shorter than a step.)
                self.edit_events("Length", sel, |x, _, _| Note { length: (x.length + n * step).max(step), ..x });
            }
            "quantize" | "quantize_half" => {
                let half = what == "quantize_half";
                let grid = self.grid.max(1);
                let notes = self.shown_events(&all);
                self.edit_events("Quantize", notes, |x, a, _| {
                    let rel = x.position - a;
                    let target = (rel + grid / 2).div_euclid(grid) * grid;
                    Note { position: a + if half { rel + (target - rel) / 2 } else { target }, ..x }
                });
            }
            // (SHIFT + pad 9: the selection, or what the displays show.)
            "clear" => {
                let gone = self.shown_events(&all);
                self.delete_events("Clear", gone);
            }
            "clear_all" => {
                let gone = if n == 1 { own } else { all };
                self.delete_events("Clear", gone);
            }
            "select_all" => self.select_events(all),
            "select_none" => self.select_events(Vec::new()),
            "select_sound" => {
                let s = n.max(0) as usize;
                let mine: Vec<Note> = all.iter().filter(|x| x.sound == s).copied().collect();
                self.toggle_events(&all, mine);
                self.sound = s;
            }
            "select_pitch" => {
                let mine: Vec<Note> = own.into_iter().filter(|x| x.pitch == n).collect();
                self.toggle_events(&all, mine);
            }
            "erase_pitch" => self.delete_events("Erase", own.into_iter().filter(|x| x.pitch == n).collect()),
            "select_step" => {
                // The focused sound's note after the last selected one (before the
                // first), alone; with none selected, its first (last).
                let mut own = own;
                own.sort_by_key(|x| (x.position, x.pitch));
                let at: Vec<usize> = own.iter().enumerate().filter(|(_, x)| sel.contains(x)).map(|(i, _)| i).collect();
                let to = match (n > 0, at.first(), at.last()) {
                    (true, _, Some(&i)) => Some(i + 1),
                    (true, ..) => Some(0),
                    (false, Some(&i), _) => i.checked_sub(1),
                    (false, ..) => own.len().checked_sub(1),
                };
                if let Some(x) = to.and_then(|i| own.get(i)) {
                    self.select_events(vec![*x]);
                }
            }
            _ => {}
        }
    }

    /// SHIFT + a pad's edit ([`deckard_surface::Action::Edit`]).
    pub(crate) fn surface_edit(&mut self, what: &str) {
        let Some(doc) = self.doc.clone() else { return };
        let done = match what {
            "undo" => doc.borrow_mut().undo(),
            "redo" => doc.borrow_mut().redo(),
            "undo_step" => doc.borrow_mut().undo_step(),
            "redo_step" => doc.borrow_mut().redo_step(),
            "copy" => {
                self.copy_notes(false);
                true
            }
            "paste" => {
                self.paste_notes(crate::clipboard::PasteAt::Keys);
                true
            }
            "clear_modulation" => {
                // (The focused sound's, as the scripts' Clear Auto; a touch still open
                // would write its movement back.)
                self.end_mod_take();
                let r = doc.borrow_mut().clear_pattern_modulation(deckard_doc::Chain::Sound(self.group, self.sound), self.pattern);
                if let Err(e) = r {
                    self.status = format!("can't remove the modulation: {e}");
                }
                true
            }
            _ => true,
        };
        if !done {
            self.status = format!("nothing to {}", if what.starts_with("undo") { "undo" } else { "redo" });
        }
    }

    /// The focused sound's notes from `from` to `to` with pitches `low` to `high` become
    /// the selection ([`deckard_surface::Action::EventsRange`]).
    pub(crate) fn surface_events_range(&mut self, from: i64, to: i64, low: i64, high: i64) {
        let Some((all, ..)) = self.event_notes() else { return };
        let sel = all
            .into_iter()
            .filter(|x| x.sound == self.sound && x.position >= from && x.position <= to && x.pitch >= low && x.pitch <= high)
            .collect();
        self.select_events(sel);
    }

    /// Step mode's pad ([`deckard_surface::Action::StepToggle`]): a note of the focused
    /// sound at its base key in the step at `at` (`length` long), selected, or with notes
    /// starting there already, those out. With no pattern, a new one first.
    pub(crate) fn surface_step(&mut self, at: i64, length: i64, velocity: i64) {
        if self.event_notes().is_none() {
            layout::apply_acts(self, vec![Act::NewPattern(self.scene, self.group)]);
        }
        let Some((all, ..)) = self.event_notes() else { return };
        let there: Vec<Note> =
            all.iter().filter(|n| n.sound == self.sound && n.position >= at && n.position < at + length).copied().collect();
        if !there.is_empty() {
            self.delete_events("Remove Step", there);
            return;
        }
        let Some(pitch) =
            self.doc.as_ref().and_then(|d| Some(d.borrow().project().groups.get(self.group)?.sounds.get(self.sound)?.channel.base_key))
        else {
            return;
        };
        let n = Note { expr: None, sound: self.sound, position: at, length, pitch, velocity, probability: 100 };
        layout::apply_acts(
            self,
            vec![Act::EditNotes { g: self.group, p: self.pattern, label: "Add Step", remove: Vec::new(), add: vec![n] }],
        );
        self.select_events(vec![n]);
    }
}
