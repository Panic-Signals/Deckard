//! Typing a value: a double-click on a knob, slider or fader opens a box over it (Enter keeps,
//! Esc or clicking away cancels); ⌘-click (Ctrl-click) sets its default.
//!
//! A value is typed as it's shown ("-6", "1.2 kHz", "C3", "Sine"). What a knob shows is worked
//! out by its page from what's stored, so the stored value is found by asking the page what it
//! would show across the knob's travel, which holds whatever the knob's curve is.

use super::{Act, Edit, SwingField};
use deckard_doc::Target;
use eframe::egui::{self, Rect, Response, Ui};

/// What a box sets.
#[derive(Clone, Copy)]
pub enum What {
    Edit(Edit),
    Tempo,
    /// A send's level: target, send.
    Aux(Target, usize),
    /// An end of the sample's selection (the end, else the start): the ends now, both in,
    /// and the sample's last frame.
    Selection(bool, i64, i64, i64),
}

impl What {
    fn reset(self) -> Act {
        match self {
            What::Edit(e) => e.reset(),
            What::Tempo => Act::SetTempo(120.0),
            What::Aux(t, k) => Act::SetAux(t, k, 0.75),
            What::Selection(..) => Act::Sampling(super::sampling::SamplingAct::Select(None)),
        }
    }

    /// On/off switches have nothing to type.
    fn typed(self) -> bool {
        !matches!(self, What::Edit(Edit::Toggle(..) | Edit::Route(_)))
    }
}

#[derive(Clone)]
struct Open {
    at: egui::Id,
    text: String,
    fresh: bool,
}

fn open_id() -> egui::Id {
    egui::Id::new("value-entry")
}

/// A double-click on `resp` types its value, ⌘-click sets its default, and while it's being
/// typed the box sits over `resp`. `shown`: the value as it's shown now.
pub fn gestures(ui: &Ui, resp: &Response, what: What, shown: &str, acts: &mut Vec<Act>) {
    if resp.double_clicked() {
        if what.typed() {
            ui.data_mut(|d| d.insert_temp(open_id(), Open { at: resp.id, text: shown.trim().to_string(), fresh: true }));
        } else {
            acts.push(what.reset());
        }
    } else if resp.clicked() && ui.input(|i| i.modifiers.command) {
        acts.push(what.reset());
    }
    let Some(mut open) = ui.data(|d| d.get_temp::<Open>(open_id())).filter(|o| o.at == resp.id) else { return };
    let (r, fresh) = (box_over(resp.rect), open.fresh);
    let out = egui::Area::new(resp.id.with("value-entry")).order(egui::Order::Foreground).fixed_pos(r.min).show(ui.ctx(), |ui| {
        egui::TextEdit::singleline(&mut open.text).desired_width(r.width()).horizontal_align(egui::Align::Center).show(ui)
    });
    let mut edit = out.inner;
    if fresh {
        edit.response.request_focus();
        let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(open.text.chars().count()));
        edit.state.cursor.set_char_range(Some(all));
        edit.state.store(ui.ctx(), edit.response.id);
        open.fresh = false;
    }
    if edit.response.lost_focus() || (!fresh && !edit.response.has_focus()) {
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            acts.push(Act::TypeValue(what, open.text.clone(), shown.to_string()));
        }
        ui.data_mut(|d| d.remove::<Open>(open_id()));
    } else {
        ui.data_mut(|d| d.insert_temp(open_id(), open));
    }
}

fn box_over(r: Rect) -> Rect {
    Rect::from_center_size(r.center(), egui::vec2(r.width().max(72.0), 20.0))
}

/// A number as typed or shown, with what its unit scales it by (`None`: no unit).
fn number(s: &str) -> Option<(f64, Option<f64>)> {
    let s = s.trim().replace(',', ".").replace('\u{2212}', "-");
    let end = s
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || (i == 0 && (c == '-' || c == '+'))))
        .map_or(s.len(), |(i, _)| i);
    let v: f64 = s[..end].parse().ok()?;
    let unit = s[end..].trim().to_ascii_lowercase();
    let scale = match unit.as_str() {
        "khz" | "k" => Some(1000.0),
        "ms" => Some(0.001),
        "hz" | "s" | "sec" | "db" | "%" => Some(1.0),
        _ => None,
    };
    Some((v, scale))
}

fn same(a: &str, b: &str) -> bool {
    let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
    squeeze(a) == squeeze(b)
}

/// Where on a knob's travel (0..1) it shows `typed`, of what it would show along it. A plain
/// number takes the unit `shown` is in. `None`: nothing there reads like it.
fn along(typed: &str, shown: &str, text_at: impl Fn(f32) -> Option<String>) -> Option<f32> {
    const STEPS: usize = 1000;
    let texts: Vec<(f32, String)> = (0..=STEPS)
        .filter_map(|n| {
            let a = n as f32 / STEPS as f32;
            text_at(a).map(|t| (a, t))
        })
        .collect();
    // Named (a choice, a note): the middle of where it says so.
    let named: Vec<f32> = texts.iter().filter(|(_, t)| same(t, typed)).map(|(a, _)| *a).collect();
    if let Some(a) = named.get(named.len() / 2) {
        return Some(*a);
    }
    let (v, unit) = number(typed)?;
    let scale = unit.or_else(|| number(shown).and_then(|(_, u)| u)).unwrap_or(1.0);
    let want = v * scale;
    let read = |t: &str| number(t).map(|(x, u)| x * u.unwrap_or(1.0));
    let best = texts.iter().filter_map(|(_, t)| read(t).map(|x| (x - want).abs())).fold(f64::INFINITY, f64::min);
    if !best.is_finite() {
        return None;
    }
    // (Shown rounded, many places read alike: the middle of them.)
    let near: Vec<f32> = texts.iter().filter(|(_, t)| read(t).is_some_and(|x| (x - want).abs() <= best + 1e-9)).map(|(a, _)| *a).collect();
    near.get(near.len() / 2).copied()
}

/// What the knob of `field` on `t`'s pages would show with `stored` there.
fn page_text(app: &crate::App, t: Target, field: &str, stored: f64) -> Option<String> {
    let doc = app.doc.as_ref()?.borrow();
    let p = doc.project();
    let matches = |k: &super::Knob| match k.3 {
        Some(Edit::Param(_, f, ..) | Edit::Log(_, f, ..) | Edit::Int(_, f, ..) | Edit::Choice(_, f, ..)) => f == field,
        _ => false,
    };
    if let Target::Sampler(g, s) = t {
        let mut smp = p.groups.get(g)?.sounds.get(s)?.sampler.clone()?;
        smp.set(field, stored);
        return (0..crate::sampler_ui::PAGES.len())
            .find_map(|n| crate::sampler_ui::page(&smp, g, s, n).slots.into_iter().flatten().find(&matches).map(|k| k.1));
    }
    let mut slot = match t {
        Target::Module(g, s, i) => p.groups.get(g)?.sounds.get(s)?.slots.get(i)?.clone(),
        Target::ChainModule(c, i) => doc.chain_slots(c)?.get(i)?.clone(),
        _ => return None,
    };
    match slot.params.iter_mut().find(|(k, _)| k == field) {
        Some(x) => x.1 = stored,
        None => slot.params.push((field.to_string(), stored)),
    }
    let pages = crate::module_ui::pages(&slot.module)?.len();
    (0..pages).find_map(|n| crate::module_ui::page_for(&slot, t, n).slots.into_iter().flatten().find(&matches).map(|k| k.1))
}

/// The edit that sets `what` to `typed`; `None` if it can't be read as a value for it.
pub fn resolve(app: &crate::App, what: What, typed: &str, shown: &str) -> Option<Act> {
    let plain = || number(typed).map(|(v, _)| v);
    let edit = match what {
        What::Tempo => return plain().filter(|v| *v > 0.0).map(|v| Act::SetTempo((v * 100.0).round() / 100.0)),
        What::Aux(t, k) => return level(typed).map(|v| Act::SetAux(t, k, v)),
        What::Selection(end, a, b, last) => {
            let v = plain()?.round() as i64;
            let (a, b) = if end { (a, v.clamp(a + 1, last)) } else { (v.clamp(0, b - 1), b) };
            return Some(Act::Sampling(super::sampling::SamplingAct::Select(Some((a.max(0) as usize, b as usize + 1)))));
        }
        What::Edit(e) => e,
    };
    let of_percent = || {
        let (v, unit) = number(typed)?;
        Some(if unit.is_some() || v > 1.0 || shown.trim_end().ends_with('%') { v / 100.0 } else { v })
    };
    match edit {
        Edit::Level(t) => level(typed).map(|v| Act::SetLevel(t, v)),
        Edit::Pan(t) => pan(typed).map(|v| Act::SetPan(t, v)),
        // (To the tenth the header shows, where a knob's turn takes whole percents.)
        Edit::Swing(t, swing, SwingField::Amount) => {
            plain().map(|v| Act::SetSwing(t, deckard_model::Swing { amount: ((v * 10.0).round() / 10.0).clamp(0.0, 100.0), ..swing }))
        }
        Edit::Swing(_, _, SwingField::Cycle) => plain().map(|v| edit.to_act((v / 7.0).clamp(0.0, 1.0) as f32)),
        Edit::Swing(..) | Edit::Toggle(..) | Edit::Route(_) => None,
        Edit::Param(t, f, ..) | Edit::Log(t, f, ..) | Edit::Int(t, f, ..) | Edit::Choice(t, f, ..) => {
            let stored = |a: f32| match edit.to_act(a) {
                Act::SetParam(_, _, v) => v,
                _ => 0.0,
            };
            if page_text(app, t, f, stored(0.0)).is_some() {
                return along(typed, shown, |a| page_text(app, t, f, stored(a))).map(|a| edit.to_act(a));
            }
            // No page says how it reads: in its own units, or named.
            match edit {
                Edit::Choice(_, _, options, ..) => {
                    let at = options.iter().position(|o| same(o, typed))?;
                    Some(Act::SetParam(t, f, at as f64))
                }
                Edit::Param(_, _, lo, hi, _) | Edit::Log(_, _, lo, hi, _) | Edit::Int(_, _, lo, hi, _) => {
                    let v = plain()?;
                    let v = if hi <= 1.0 && shown.trim_end().ends_with('%') { v / 100.0 } else { v };
                    let v = v.clamp(lo.min(hi), lo.max(hi));
                    Some(Act::SetParam(t, f, if matches!(edit, Edit::Int(..)) { v.round() } else { v }))
                }
                _ => None,
            }
        }
        // A plug-in's knob runs 0..1, however the plug-in reads it: typed as a percentage.
        Edit::Plugin(..) | Edit::PluginChoice(..) | Edit::PluginIn(..) => of_percent().map(|v| edit.to_act(v.clamp(0.0, 1.0) as f32)),
    }
}

/// A level typed in dB ("off" or "inf": silent).
fn level(typed: &str) -> Option<f64> {
    let t = typed.trim().to_ascii_lowercase();
    if t == "off" || t.contains("inf") || t.contains('\u{221e}') {
        return Some(0.0);
    }
    number(typed).map(|(db, _)| deckard_model::level_from_db(db))
}

/// A pan typed as −100..100, or "L30", "R30", "C".
fn pan(typed: &str) -> Option<f64> {
    let t = typed.trim().to_ascii_lowercase();
    if t == "c" || t == "center" || t == "centre" {
        return Some(0.0);
    }
    let v = if let Some(x) = t.strip_prefix('l').or_else(|| t.strip_suffix('l')) {
        -number(x)?.0
    } else if let Some(x) = t.strip_prefix('r').or_else(|| t.strip_suffix('r')) {
        number(x)?.0
    } else {
        number(&t)?.0
    };
    Some((v / 100.0).clamp(-1.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_units() {
        assert_eq!(number("-6.0 dB"), Some((-6.0, Some(1.0))));
        assert_eq!(number("1.2 kHz"), Some((1.2, Some(1000.0))));
        assert_eq!(number("20 ms"), Some((20.0, Some(0.001))));
        assert_eq!(number("+3"), Some((3.0, None)));
        assert_eq!(number("Sine"), None);
    }

    /// On any curve, a plain number takes the unit shown; a unit typed wins.
    #[test]
    fn found_along_a_curve() {
        let hz = |a: f32| {
            let f = 20.0 * 1000f32.powf(a);
            Some(if f >= 1000.0 { format!("{:.2} kHz", f / 1000.0) } else { format!("{f:.0} Hz") })
        };
        let read = |a: f32| 20.0 * 1000f32.powf(a);
        let a = along("440", "100 Hz", hz).unwrap();
        assert!((read(a) - 440.0).abs() < 3.0, "{}", read(a));
        let a = along("1.5", "2.00 kHz", hz).unwrap();
        assert!((read(a) - 1500.0).abs() < 8.0, "{}", read(a));
        let a = along("1.5 kHz", "100 Hz", hz).unwrap();
        assert!((read(a) - 1500.0).abs() < 8.0, "{}", read(a));
        // Past the end: the end.
        assert_eq!(along("90 kHz", "100 Hz", hz), Some(1.0));
        assert_eq!(along("nonsense", "100 Hz", hz), None);
    }

    #[test]
    fn names_are_found() {
        let names = |a: f32| Some(if a < 0.5 { "Sine" } else { "Saw" }.to_string());
        assert!(along("saw", "Sine", names).unwrap() >= 0.5);
        assert!(along("SINE", "Saw", names).unwrap() < 0.5);
    }

    #[test]
    fn levels_and_pans() {
        assert_eq!(level("off"), Some(0.0));
        assert!((deckard_model::level_db(level("-6").unwrap()) + 6.0).abs() < 1e-6);
        assert_eq!(pan("C"), Some(0.0));
        assert_eq!(pan("L50"), Some(-0.5));
        assert_eq!(pan("25"), Some(0.25));
        assert_eq!(pan("R200"), Some(1.0));
        assert_eq!(pan("x"), None);
    }
}
