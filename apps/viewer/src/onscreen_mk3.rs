//! The on-screen MK3's core: pointer contact in, [`deckard_mk3::Event`]s out, no window.
//!
//! `Panel` turns pointer contact into the same events the MK3's drivers emit, so every consumer
//! downstream (`App::mk3_events`, `Pads`, the scripts) reads the panel and the hardware alike.
//! It holds no painting and no egui context: which finger is where (finger id to control), the
//! per-finger gesture state, and a queue of emitted events. The painter (Task 7) forwards pointer
//! contact; the tests beside this module pin the gestures with no window.
//!
//! Gesture map, per control kind:
//! - pad: down emits `PadOn`, moves inside it emit `PadPressure` from the finger's vertical place
//!   in the pad (top light, bottom full), up emits `PadOff`;
//! - button: down emits `Button(name, true)`, up emits `Button(name, false)`;
//! - knob: vertical/horizontal drags turn past a dead zone, emitting `Knob(index, inc)`;
//! - jog: the same drag emits `Wheel` clicks;
//! - strip: down/move/up emit `Touch(slot, 0|1|2, pos)` with the pointer's horizontal place;
//! - screen: held for bookkeeping, emits nothing.
//!
//! Every event names the control the finger landed on, not where it has moved to since.

use crate::mk3_layout::{self, Control, Kind};
use deckard_mk3::Event;
use eframe::egui;
use std::collections::HashMap;

/// The presented surface's size: landscape it is the table itself (`WIDTH` x `HEIGHT`);
/// portrait it is the whole surface turned 90° clockwise for a vertical monitor, so the
/// axes flip (a 1280 x 800 surface becomes 800 x 1280). Rotation is a transform of the
/// table's rects, never a re-layout: the painter and the hit test share it.
pub fn surface_size(portrait: bool) -> egui::Vec2 {
    if portrait { egui::vec2(mk3_layout::HEIGHT, mk3_layout::WIDTH) } else { egui::vec2(mk3_layout::WIDTH, mk3_layout::HEIGHT) }
}

/// A point on the presented surface back to landscape table space: the hit test.
/// Landscape maps nothing; portrait undoes the clockwise turn (`to_portrait`).
pub fn to_landscape(p: egui::Pos2, portrait: bool) -> egui::Pos2 {
    if portrait { egui::pos2(p.y, mk3_layout::HEIGHT - p.x) } else { p }
}

/// A landscape point onto the presented surface: the painter's image of it.
/// Landscape maps nothing; portrait turns it 90° clockwise.
pub fn to_portrait(p: egui::Pos2, portrait: bool) -> egui::Pos2 {
    if portrait { egui::pos2(mk3_layout::HEIGHT - p.y, p.x) } else { p }
}

/// A landscape rect's placement on the presented surface: the painter.
/// Landscape places nothing; portrait turns the whole surface, corners and all.
pub fn to_surface(rect: egui::Rect, portrait: bool) -> egui::Rect {
    if portrait {
        egui::Rect::from_min_max(
            egui::pos2(mk3_layout::HEIGHT - rect.max.y, rect.min.x),
            egui::pos2(mk3_layout::HEIGHT - rect.min.y, rect.max.x),
        )
    } else {
        rect
    }
}
/// Velocity a pad press reports when the caller has no pressure: full enough to sound, inside 0..=1.
const FIXED_VELOCITY: f64 = 0.8;
/// Drag swallowed before a knob or jog starts turning: small jitters are not turns.
const DEAD_ZONE_PX: f32 = 3.0;
/// Pixels of drag past the dead zone for one knob step or jog click.
const PIXELS_PER_STEP: f32 = 6.0;
/// A fraction of a turn one knob step reports: a 300 px drag is one full turn, the same order as
/// the hardware's reports (1000 units to the turn, so a brisk flick is tens of units).
const KNOB_STEP: f64 = 0.02;
/// How far a pad's pressure must move before it reports: one pixel is ~0.013, so one pixel never reports.
const PAD_PRESSURE_EPS: f64 = 0.05;

/// Which finger is where, the per-finger gesture state, and the queue of emitted events.
#[derive(Debug, Default)]
pub struct Panel {
    down: HashMap<usize, &'static Control>,
    drags: Vec<Drag>,
    out: Vec<Event>,
}

/// One held finger's gesture state: everything `pointer_move` and `pointer_up` need without
/// re-hitting the table, so a gesture always reports the control it started on.
#[derive(Debug)]
struct Drag {
    finger: usize,
    state: DragState,
}

/// The gesture state per control kind. Pads and the strip carry their rect so moves stay in the
/// down control's coordinates; knobs and the jog carry their drag accumulator.
#[derive(Debug)]
enum DragState {
    Pad { pad: usize, rect: egui::Rect, last: f64 },
    Button,
    Knob { index: usize, last: egui::Pos2, acc: f32, sent: i32 },
    Jog { last: egui::Pos2, acc: f32, sent: i32 },
    Strip { slot: usize, rect: egui::Rect, pos: f64 },
    Idle,
}

/// The pad number of a `padN` id, if it names one.
fn pad_number(id: &str) -> Option<usize> {
    id.strip_prefix("pad")?.parse::<usize>().ok().filter(|&n| (1..=16).contains(&n))
}

/// The knob index of a `knobN` id (knob 1 is index 0), if it names one.
fn knob_index(id: &str) -> Option<usize> {
    id.strip_prefix("knob")?.parse::<usize>().ok().filter(|&n| (1..=8).contains(&n)).map(|n| n - 1)
}

/// Steps of drag the accumulator has earned: nothing inside the dead zone, then one per
/// [`PIXELS_PER_STEP`] px, signed by direction.
fn turn_steps(acc: f32) -> i32 {
    let over = acc.abs() - DEAD_ZONE_PX;
    if over < 0.0 { 0 } else { (over / PIXELS_PER_STEP) as i32 * acc.signum() as i32 }
}

/// The strip position of a pointer: the horizontal fraction across the strip, clamped to 0..1.
fn strip_pos(rect: egui::Rect, p: egui::Pos2) -> f64 {
    ((p.x - rect.min.x) / rect.width()).clamp(0.0, 1.0) as f64
}

/// The pad pressure of a pointer: the vertical fraction down the pad, clamped to 0..1.
fn pad_pressure(rect: egui::Rect, p: egui::Pos2) -> f64 {
    ((p.y - rect.min.y) / rect.height()).clamp(0.0, 1.0) as f64
}

impl Panel {
    /// A finger touched the panel: `at` decides the control, and the down event (if any) queues.
    /// A down for a finger that is already down releases the old gesture first, exactly as
    /// `pointer_up` would: paired down/up per finger is not required of callers.
    /// A down on no control, or on a strip with both finger slots held, records nothing.
    pub fn pointer_down(&mut self, finger: usize, p: egui::Pos2, pressure: Option<f32>) {
        let Some(control) = mk3_layout::at(p) else { return };
        if control.kind == Kind::Strip && self.strip_slot().is_none() {
            return;
        }
        // A down for a finger that never lifted releases its gesture first (see above):
        // the contact is still down somewhere, so the old control must not stick.
        self.release_drag(finger);
        self.down.insert(finger, control);
        let state = match control.kind {
            Kind::Pad => {
                match pad_number(control.id) {
                    Some(pad) => {
                        let velocity = pressure.map(f64::from).unwrap_or(FIXED_VELOCITY).clamp(0.0, 1.0);
                        self.out.push(Event::PadOn(pad, velocity));
                        DragState::Pad { pad, rect: control.rect, last: pad_pressure(control.rect, p) }
                    }
                    // Unreachable from the fixed table: loud in debug, a silent hold in release.
                    None => {
                        debug_assert!(false, "pad control with an unparseable id: {:?}", control.id);
                        DragState::Idle
                    }
                }
            }
            Kind::Button => {
                self.out.push(Event::Button(control.id, true));
                DragState::Button
            }
            Kind::Knob => {
                match knob_index(control.id) {
                    Some(index) => DragState::Knob { index, last: p, acc: 0.0, sent: 0 },
                    // Unreachable from the fixed table: loud in debug, a silent hold in release.
                    None => {
                        debug_assert!(false, "knob control with an unparseable id: {:?}", control.id);
                        DragState::Idle
                    }
                }
            }
            Kind::Jog => DragState::Jog { last: p, acc: 0.0, sent: 0 },
            Kind::Strip => {
                let slot = self.strip_slot().unwrap_or(0);
                let pos = strip_pos(control.rect, p);
                self.out.push(Event::Touch(slot, 0, pos));
                DragState::Strip { slot, rect: control.rect, pos }
            }
            Kind::Screen => DragState::Idle,
        };
        self.drags.push(Drag { finger, state });
    }

    /// A held finger moved: pads report pressure past [`PAD_PRESSURE_EPS`], knobs and the jog turn
    /// past the dead zone, the strip follows. Anything else, and any unknown finger, is silent.
    pub fn pointer_move(&mut self, finger: usize, p: egui::Pos2) {
        let (drags, out) = (&mut self.drags, &mut self.out);
        let Some(drag) = drags.iter_mut().find(|d| d.finger == finger) else { return };
        match &mut drag.state {
            DragState::Pad { pad, rect, last } => {
                let now = pad_pressure(*rect, p);
                if (now - *last).abs() >= PAD_PRESSURE_EPS {
                    *last = now;
                    out.push(Event::PadPressure(*pad, now));
                }
            }
            DragState::Knob { index, last, acc, sent } => {
                // Right and up both turn clockwise; the remainder stays in the accumulator.
                *acc += (p.x - last.x) - (p.y - last.y);
                *last = p;
                let steps = turn_steps(*acc) - *sent;
                if steps != 0 {
                    *sent += steps;
                    out.push(Event::Knob(*index, steps as f64 * KNOB_STEP));
                }
            }
            DragState::Jog { last, acc, sent } => {
                *acc += (p.x - last.x) - (p.y - last.y);
                *last = p;
                let steps = turn_steps(*acc) - *sent;
                if steps != 0 {
                    *sent += steps;
                    out.push(Event::Wheel(steps));
                }
            }
            DragState::Strip { slot, rect, pos } => {
                // Every move reports, even an in-place one: the brief pins per-move Touch(1, pos).
                *pos = strip_pos(*rect, p);
                out.push(Event::Touch(*slot, 1, *pos));
            }
            DragState::Button | DragState::Idle => {}
        }
    }

    /// A finger lifted: the held control's release queues (`PadOff`, `Button(name, false)`,
    /// `Touch(slot, 2, pos)`), and the finger is forgotten. An unknown finger is silent.
    pub fn pointer_up(&mut self, finger: usize) {
        self.release_drag(finger);
    }

    /// Release one finger's gesture, emitting what `pointer_up` emits for it, then forget the
    /// finger. A re-down calls this before starting the new gesture, so no control sticks when
    /// paired down/up per finger is not honoured. An unknown finger only clears the record.
    fn release_drag(&mut self, finger: usize) {
        let Some(at) = self.drags.iter().position(|d| d.finger == finger) else {
            self.down.remove(&finger);
            return;
        };
        let drag = self.drags.remove(at);
        let control = self.down.remove(&finger);
        self.emit_release(drag.state, control);
    }

    /// Queue one gesture's release: `PadOff`, `Button(name, false)`, or `Touch(slot, 2, pos)`.
    /// Knobs, the jog and screens hold nothing downstream, so they clear silently.
    fn emit_release(&mut self, state: DragState, control: Option<&'static Control>) {
        match state {
            DragState::Pad { pad, .. } => self.out.push(Event::PadOff(pad)),
            DragState::Button => {
                if let Some(control) = control {
                    self.out.push(Event::Button(control.id, false));
                }
            }
            DragState::Strip { slot, pos, .. } => self.out.push(Event::Touch(slot, 2, pos)),
            DragState::Knob { .. } | DragState::Jog { .. } | DragState::Idle => {}
        }
    }

    /// A mouse-wheel tick over the panel: steps clockwise, the jog's own event.
    pub fn wheel(&mut self, steps: i32) {
        if steps != 0 {
            self.out.push(Event::Wheel(steps));
        }
    }

    /// Let go of everything held, in a deterministic order: pads by number, then buttons in press
    /// order, then strip fingers by slot. Knobs, the jog and screens hold nothing downstream, so
    /// they clear silently. A client that disconnects must never leave a note or a light stuck.
    pub fn release_all(&mut self) {
        let mut pads: Vec<usize> = Vec::new();
        let mut strips: Vec<(usize, f64)> = Vec::new();
        for drag in &self.drags {
            match drag.state {
                DragState::Pad { pad, .. } => pads.push(pad),
                DragState::Strip { slot, pos, .. } => strips.push((slot, pos)),
                _ => {}
            }
        }
        pads.sort_unstable();
        strips.sort_by_key(|&(slot, _)| slot);
        for pad in pads {
            self.out.push(Event::PadOff(pad));
        }
        let drags = std::mem::take(&mut self.drags);
        for drag in drags {
            if matches!(drag.state, DragState::Button) {
                let control = self.down.get(&drag.finger).copied();
                self.emit_release(drag.state, control);
            }
        }
        for (slot, pos) in strips {
            self.out.push(Event::Touch(slot, 2, pos));
        }
        self.down.clear();
    }

    /// Drain and return the queued events; nothing else drains the queue.
    pub fn take(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.out)
    }

    /// The lowest free strip finger slot (the hardware tracks two), if one is free.
    fn strip_slot(&self) -> Option<usize> {
        (0..2).find(|slot| !self.drags.iter().any(|d| matches!(d.state, DragState::Strip { slot: s, .. } if s == *slot)))
    }
}
