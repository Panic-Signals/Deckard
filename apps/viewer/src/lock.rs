//! Lock in the app (docs/superpowers/plans/2026-10-22-phase6-lock.md): LOCK takes or
//! updates the focused snapshot and, pressed again, recalls it; Extended Lock's pads take
//! a snapshot in a free slot or recall one; UPDATE, DELETE, the banks; recalls morph
//! when Morph is on: Travel over Time bars from now, Target arriving at the next Grid
//! boundary of the playing song. Continuous parameters glide; the rest switch on
//! arrival.

use crate::App;
use deckard_doc::{LockEntry, Target};
use deckard_model::PPQ;
use eframe::egui;
use mxprj::decode::Value;

/// Travel's lengths, in bars (`LOCK_TIMES`).
const TIME_BARS: [f64; 6] = [0.5, 1.0, 2.0, 4.0, 8.0, 16.0];

/// A recall on its way.
pub struct LockMorph {
    slot: usize,
    /// The continuous parameters: where each was and where it goes.
    glide: Vec<(LockEntry, f64)>,
    start: f64,
    end: f64,
}

/// Seconds since the app started (the morphs' clock).
pub fn clock() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}

impl App {
    fn lock_state(&self) -> Option<deckard_model::Lock> {
        Some(self.doc.as_ref()?.borrow().project().lock.clone())
    }

    fn lock_do(&mut self, what: &str, r: Result<(), String>) {
        if let Err(e) = r {
            self.status = format!("can't {what}: {e}");
        }
    }

    /// Lock as the controller's own layer shows it.
    pub fn lock_view(&self) -> deckard_surface::LockView {
        let l = self.lock_state().unwrap_or_default();
        deckard_surface::LockView {
            active: self.lock_active,
            slots: l.slots,
            focus: l.focus,
            bank: l.focus_bank,
            banks: l.banks,
            morph: l.morph,
            mode: l.mode,
            time: l.time,
            grid: l.grid,
        }
    }

    /// LOCK: takes a snapshot (updating the focused one, else into the first free slot)
    /// and lights; pressed again, recalls it and goes out.
    pub fn lock_button(&mut self) {
        let (Some(doc), Some(l)) = (self.doc.clone(), self.lock_state()) else { return };
        if self.lock_active {
            if let Some(slot) = l.focus {
                self.lock_recall(slot);
            }
            self.lock_active = false;
            return;
        }
        let Some(slot) = l.focus.or_else(|| (0..64).find(|s| !l.slots.contains(s))) else {
            self.status = "every Lock slot is taken".into();
            return;
        };
        let r = doc.borrow_mut().take_snapshot(slot);
        self.lock_do("take a snapshot", r);
        self.lock_active = true;
    }

    /// Extended Lock's pad `pad` (0–15) of the focused bank: recalls its snapshot (and
    /// focuses it), or takes one there.
    pub fn lock_pad(&mut self, pad: usize) {
        let (Some(doc), Some(l)) = (self.doc.clone(), self.lock_state()) else { return };
        let slot = l.focus_bank * 16 + pad.min(15);
        if l.slots.contains(&slot) {
            let r = doc.borrow_mut().focus_snapshot(slot);
            self.lock_do("focus the snapshot", r);
            self.lock_recall(slot);
        } else {
            let r = doc.borrow_mut().take_snapshot(slot);
            self.lock_do("take a snapshot", r);
        }
    }

    /// MIDI asked for the snapshot in `slot` (an empty slot: nothing).
    pub fn lock_midi_recall(&mut self, slot: usize) {
        let (Some(doc), Some(l)) = (self.doc.clone(), self.lock_state()) else { return };
        if l.slots.contains(&slot) {
            let r = doc.borrow_mut().focus_snapshot(slot);
            self.lock_do("focus the snapshot", r);
            self.lock_recall(slot);
        }
    }

    /// UPDATE: the focused snapshot taken again.
    pub fn lock_update(&mut self) {
        let (Some(doc), Some(l)) = (self.doc.clone(), self.lock_state()) else { return };
        if let Some(slot) = l.focus {
            let r = doc.borrow_mut().take_snapshot(slot);
            self.lock_do("update the snapshot", r);
        }
    }

    /// DELETE: the focused snapshot (or the one in `slot`).
    pub fn lock_delete(&mut self) {
        let Some(slot) = self.lock_state().and_then(|l| l.focus) else { return };
        self.lock_delete_slot(slot);
    }

    pub fn lock_delete_slot(&mut self, slot: usize) {
        let Some(doc) = self.doc.clone() else { return };
        let r = doc.borrow_mut().delete_snapshot(slot);
        self.lock_do("delete the snapshot", r);
    }

    /// ◀ / ▶: the previous or next bank; ▶ past the last adds one (up to four).
    pub fn lock_bank(&mut self, next: bool) {
        let Some(l) = self.lock_state() else { return };
        if next && l.focus_bank + 1 >= l.banks {
            if l.banks < 4 {
                self.lock_setting("banks", (l.banks + 1) as f64);
                self.lock_setting("focus_bank", l.banks as f64);
            }
        } else if next {
            self.lock_setting("focus_bank", (l.focus_bank + 1) as f64);
        } else if l.focus_bank > 0 {
            self.lock_setting("focus_bank", (l.focus_bank - 1) as f64);
        }
    }

    /// A Lock setting (`morph`, `morph_mode`, `morph_time`, `morph_grid`, `focus_bank`,
    /// `banks`).
    pub fn lock_setting(&mut self, field: &str, v: f64) {
        let Some(doc) = self.doc.clone() else { return };
        let r = doc.borrow_mut().set("Lock", Target::Song, &format!("parameter_snapshots/{field}"), Value::Float(v));
        self.lock_do("change Lock", r);
    }

    /// Recall the snapshot in `slot` now (morphing if Morph is on).
    pub fn lock_recall(&mut self, slot: usize) {
        let tick = self.playhead().map(|h| h.position);
        self.lock_recall_at(slot, clock(), tick);
    }

    /// Recall the snapshot in `slot` at `now` (seconds), the song at `tick` if playing.
    pub fn lock_recall_at(&mut self, slot: usize, now: f64, tick: Option<f64>) {
        let (Some(doc), Some(l)) = (self.doc.clone(), self.lock_state()) else { return };
        self.lock_morph = None;
        if !l.morph {
            let r = doc.borrow_mut().recall_snapshot(slot);
            self.lock_do("recall the snapshot", r);
            return;
        }
        let d = doc.borrow();
        let p = d.project();
        let bar = p.bar_length() as f64;
        let secs = |ticks: f64| ticks / PPQ as f64 * 60.0 / p.tempo.max(1.0);
        let duration = if l.mode == 0 {
            secs(TIME_BARS[l.time.min(5)] * bar)
        } else {
            let grid = [PPQ as f64 / 2.0, PPQ as f64, 2.0 * PPQ as f64, bar][l.grid.min(3)];
            match tick {
                Some(t) => secs(((t / grid).floor() + 1.0) * grid - t),
                None => secs(grid),
            }
        };
        let glide = d
            .snapshot_entries(slot)
            .into_iter()
            .filter_map(|e| match d.number_at(&e.path) {
                Some((from, true)) => Some((e, from)),
                _ => None,
            })
            .collect();
        drop(d);
        self.lock_morph = Some(LockMorph { slot, glide, start: now, end: now + duration });
    }

    /// Move a morph on to `now` (seconds); arrived, the snapshot is recalled whole.
    pub fn lock_tick(&mut self, now: f64, _tick: Option<f64>) {
        let (Some(doc), Some(m)) = (self.doc.clone(), self.lock_morph.as_ref()) else { return };
        let t = ((now - m.start) / (m.end - m.start).max(1e-6)).clamp(0.0, 1.0);
        if t >= 1.0 {
            let slot = m.slot;
            self.lock_morph = None;
            let r = doc.borrow_mut().recall_snapshot(slot);
            self.lock_do("recall the snapshot", r);
            return;
        }
        let step: Vec<LockEntry> =
            m.glide.iter().map(|(e, from)| LockEntry { path: e.path.clone(), value: from + (e.value - from) * t }).collect();
        let r = doc.borrow_mut().morph_step(&step);
        self.lock_do("morph", r);
    }
}

/// What the panel asked for.
enum Do {
    Lock,
    Pad(usize),
    Update,
    Delete,
    DeleteSlot(usize),
    Bank(bool),
    Set(&'static str, f64),
}

impl App {
    /// The Lock panel (View ▸ Lock): LOCK, UPDATE, DELETE, the bank's 16 pads (as the
    /// MK3's: pad 1 bottom left; a pad with a snapshot shows its number, the focused one
    /// lit), the banks and the morph settings.
    pub fn lock_window(&mut self, ctx: &egui::Context) {
        let Some(l) = self.lock_state() else { return };
        let mut open = true;
        let mut todo = Vec::new();
        let active = self.lock_active;
        egui::Window::new("Lock").open(&mut open).resizable(false).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.selectable_label(active, "LOCK").on_hover_text("Take a snapshot; again: recall it").clicked() {
                    todo.push(Do::Lock);
                }
                let focus = l.focus.is_some();
                if ui.add_enabled(focus, egui::Button::new("UPDATE")).on_hover_text("Take the focused snapshot again").clicked() {
                    todo.push(Do::Update);
                }
                if ui.add_enabled(focus, egui::Button::new("DELETE")).clicked() {
                    todo.push(Do::Delete);
                }
                ui.separator();
                if ui.add_enabled(l.focus_bank > 0, egui::Button::new("◀")).clicked() {
                    todo.push(Do::Bank(false));
                }
                ui.label(format!("Bank {} / {}", l.focus_bank + 1, l.banks));
                if ui.add_enabled(l.focus_bank < 3, egui::Button::new("▶")).on_hover_text("Past the last: a new bank").clicked() {
                    todo.push(Do::Bank(true));
                }
            });
            egui::Grid::new("lock-pads").spacing([4.0, 4.0]).show(ui, |ui| {
                for row in (0..4).rev() {
                    for col in 0..4 {
                        let pad = row * 4 + col;
                        let slot = l.focus_bank * 16 + pad;
                        let taken = l.slots.contains(&slot);
                        let text = if taken { format!("{}", slot + 1) } else { String::new() };
                        let b = egui::Button::new(text).min_size(egui::vec2(44.0, 36.0)).selected(l.focus == Some(slot));
                        let b = if taken { b } else { b.fill(egui::Color32::from_gray(30)) };
                        let resp = ui.add(b);
                        let name = format!("Lock pad {}", pad + 1);
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
                        let resp = resp.on_hover_text(if taken { "Recall (right-click: delete)" } else { "Take a snapshot here" });
                        if resp.clicked() {
                            todo.push(Do::Pad(pad));
                        }
                        if taken && resp.secondary_clicked() {
                            todo.push(Do::DeleteSlot(slot));
                        }
                    }
                    ui.end_row();
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                let mut morph = l.morph;
                if ui.checkbox(&mut morph, "Morph").on_hover_text("Recalls glide to the snapshot").changed() {
                    todo.push(Do::Set("morph", f64::from(u8::from(morph))));
                }
                ui.add_enabled_ui(l.morph, |ui| {
                    let mut choose = |ui: &mut egui::Ui, id: &str, field: &'static str, list: &[&str], at: usize| {
                        egui::ComboBox::from_id_salt(id).selected_text(list[at.min(list.len() - 1)]).show_ui(ui, |ui| {
                            for (i, name) in list.iter().enumerate() {
                                if ui.selectable_label(i == at, *name).clicked() {
                                    todo.push(Do::Set(field, i as f64));
                                }
                            }
                        });
                    };
                    choose(ui, "lock-mode", "morph_mode", &deckard_model::LOCK_MODES, l.mode);
                    if l.mode == 0 {
                        choose(ui, "lock-time", "morph_time", &deckard_model::LOCK_TIMES, l.time);
                    } else {
                        choose(ui, "lock-grid", "morph_grid", &deckard_model::LOCK_GRIDS, l.grid);
                    }
                });
            });
        });
        self.lock_panel = open;
        for d in todo {
            match d {
                Do::Lock => self.lock_button(),
                Do::Pad(p) => self.lock_pad(p),
                Do::Update => self.lock_update(),
                Do::Delete => self.lock_delete(),
                Do::DeleteSlot(s) => self.lock_delete_slot(s),
                Do::Bank(next) => self.lock_bank(next),
                Do::Set(f, v) => self.lock_setting(f, v),
            }
        }
    }
}

impl App {
    /// Edit ▸ MIDI Change: Lock's section (Maschine's dialog also has Scene and Section
    /// changes): which MIDI recalls snapshots (notes 0–63 or programs 1–64), from which
    /// input and channel. Stored in the project.
    pub fn midi_change_window(&mut self, ctx: &egui::Context) {
        let (Some(doc), Some(l)) = (self.doc.clone(), self.lock_state()) else { return };
        let ports: Vec<String> = self.midi_ports().0.clone();
        let mut open = true;
        let mut set: Vec<(&'static str, Value)> = Vec::new();
        egui::Window::new("MIDI Change").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            ui.heading("Lock");
            egui::Grid::new("midi-change-lock").num_columns(2).show(ui, |ui| {
                ui.label("Trigger");
                let shown = deckard_model::MIDI_TRIGGERS[l.midi_trigger.min(2)];
                let r = egui::ComboBox::from_id_salt("lock-trigger").selected_text(shown).show_ui(ui, |ui| {
                    for (i, name) in deckard_model::MIDI_TRIGGERS.iter().enumerate() {
                        if ui.selectable_label(i == l.midi_trigger, *name).clicked() {
                            set.push(("trigger", Value::Int(i as i64)));
                        }
                    }
                });
                r.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Lock trigger"));
                ui.end_row();
                ui.add_enabled_ui(l.midi_trigger != 0, |ui| {
                    ui.label("Source");
                });
                ui.add_enabled_ui(l.midi_trigger != 0, |ui| {
                    egui::ComboBox::from_id_salt("lock-source").selected_text(&l.midi_source).show_ui(ui, |ui| {
                        for name in std::iter::once("None".to_string()).chain(ports.iter().cloned()) {
                            if ui.selectable_label(name == l.midi_source, &name).clicked() {
                                set.push(("input", Value::Str(name.clone())));
                            }
                        }
                    });
                });
                ui.end_row();
                ui.add_enabled_ui(l.midi_trigger != 0, |ui| {
                    ui.label("Channel");
                });
                ui.add_enabled_ui(l.midi_trigger != 0, |ui| {
                    egui::ComboBox::from_id_salt("lock-channel").selected_text(l.midi_channel.to_string()).show_ui(ui, |ui| {
                        for ch in 1..=16 {
                            if ui.selectable_label(ch == l.midi_channel, ch.to_string()).clicked() {
                                set.push(("midi_channel", Value::Int(ch)));
                            }
                        }
                    });
                });
                ui.end_row();
            });
            ui.label(egui::RichText::new("Notes 0–63 (C-2 up) or programs 1–64 recall Lock slots 1–64.").weak());
        });
        self.midi_change = open;
        for (field, v) in set {
            let r = match (field, &v) {
                ("trigger", Value::Int(t)) => doc.borrow_mut().set_lock_midi_trigger(*t as usize),
                _ => doc.borrow_mut().set("MIDI Change", Target::Song, &format!("snapshot_recall/{field}"), v),
            };
            self.lock_do("change MIDI Change", r);
        }
    }
}
