//! `value`: one parameter, large: its name, its value as the plug-in writes it, and a
//! thin bar under it.
//!
//! Inputs (all optional):
//!
//! | input | |
//! |---|---|
//! | `name` | the parameter's name, above |
//! | `text` | the value as written ("1.20 kHz", "-6.0 dB"); a trailing unit is drawn smaller |
//! | `value` | 0..1: the bar under the number, from the left |
//! | `bipolar` | the bar grows from the middle |
//! | `value_mod` | 0..1: the modulated position, a mark on the bar |
//!
//! Touch focus on `value` lights the bar; the number is never dimmed.

use mlua::Table;

use super::{Align, Canvas, flag, num, text};
use crate::screen::{Picture, Style};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Value {
    pub name: String,
    pub text: String,
    pub value: Option<f32>,
    pub bipolar: bool,
    pub value_mod: Option<f32>,
}

const UNITS: [&str; 6] = ["Hz", "kHz", "dB", "ms", "s", "%"];

/// "1.20 kHz" as ("1.20", Some("kHz")); "Off" as ("Off", None).
pub fn split_unit(s: &str) -> (&str, Option<&str>) {
    match s.rsplit_once(' ') {
        Some((n, u)) if UNITS.contains(&u) && !n.is_empty() => (n.trim_end(), Some(u)),
        _ => (s, None),
    }
}

/// The number's style (stepped down from a large size until the number and unit fit
/// `avail` pixels), the unit's style, and their widths.
fn fit(big: Style, label: Style, number: &str, unit: Option<&str>, avail: i32) -> (Style, Style, i32, i32) {
    let mut size = 40.0f32;
    loop {
        let n = Style { size, ..big };
        let u = Style { size: (size * 0.5).max(10.0), colour: label.colour, ..label };
        let nw = Picture::width(&n, number);
        let uw = unit.map_or(0, |s| Picture::width(&u, s));
        let total = nw + if unit.is_some() { 4 + uw } else { 0 };
        if total <= avail || size <= 10.0 {
            return (n, u, nw, uw);
        }
        size -= 1.0;
    }
}

impl Value {
    pub fn from_lua(t: &Table) -> mlua::Result<Value> {
        Ok(Value {
            name: text(t, "name").unwrap_or_default(),
            text: text(t, "text").unwrap_or_default(),
            value: num(t, "value"),
            bipolar: flag(t, "bipolar"),
            value_mod: num(t, "value_mod"),
        })
    }

    pub fn draw(&self, c: &mut Canvas) {
        let look = *c.look;
        let (w, h) = (c.r.w, c.r.h);
        let avail = w - 16;
        let (number, unit) = split_unit(&self.text);
        let (ns, us, nw, uw) = fit(look.big, look.label, number, unit, avail);
        let nh = (ns.size * 1.3).ceil() as i32;
        let bar = self.value.is_some();
        let name_h = if self.name.is_empty() { 0 } else { 16 };
        let total = name_h + nh + if bar { 14 } else { 0 };
        let mut y = ((h - total) / 2).max(1);
        if name_h > 0 {
            c.text(8, y, avail, name_h, &self.name, look.label, look.weak, Align::Center);
            y += name_h;
        }
        let tw = nw + unit.map_or(0, |_| 4 + uw);
        let x = (w - tw) / 2;
        c.text(x, y, nw + 2, nh, number, ns, look.text, Align::Left);
        if let Some(u) = unit {
            // (Sits on the number's baseline: a little below its middle.)
            let uh = (us.size * 1.3).ceil() as i32;
            let uy = y + nh - uh - (ns.size * 0.12) as i32;
            c.text(x + nw + 4, uy, uw + 2, uh, u, us, look.weak, Align::Left);
        }
        if let Some(v) = self.value {
            let lit = c.lit("value") || c.lit("name");
            let (bx, bw, bh) = (12, w - 24, 4);
            let by = y + nh + 6;
            c.fill(bx, by, bw, bh, look.grid);
            let at = |t: f32| (bx as f32 + t.clamp(0.0, 1.0) * bw as f32).round() as i32;
            let (a, b) = if self.bipolar { (at(0.5).min(at(v)), at(0.5).max(at(v))) } else { (bx, at(v)) };
            if b > a {
                c.fill(a, by, b - a, bh, if lit { look.accent } else { look.muted });
            }
            if self.bipolar {
                c.fill(at(0.5), by - 1, 1, bh + 2, look.separator);
            }
            if let Some(m) = self.value_mod {
                let mx = at(m).clamp(bx, bx + bw - 2);
                c.fill(mx, by - 2, 2, bh + 4, look.marker);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::golden;

    #[test]
    fn the_unit_is_split_off() {
        assert_eq!(split_unit("1.20 kHz"), ("1.20", Some("kHz")));
        assert_eq!(split_unit("-6.0 dB"), ("-6.0", Some("dB")));
        assert_eq!(split_unit("Off"), ("Off", None));
        assert_eq!(split_unit("Pre Delay"), ("Pre Delay", None));
    }

    #[test]
    fn long_text_fits_at_120() {
        let look = crate::widget::Look::fallback();
        let (n, u) = split_unit("-120.0 dB");
        let (ns, _, nw, uw) = fit(look.big, look.label, n, u, 120 - 16);
        assert!(nw + 4 + uw <= 104, "{nw} + {uw} at size {}", ns.size);
        assert!(ns.size >= 10.0);
    }

    fn freq() -> Value {
        Value { name: "Cutoff".into(), text: "1.20 kHz".into(), value: Some(0.6), value_mod: Some(0.75), ..Default::default() }
    }

    #[test]
    fn value_frequency() {
        let v = freq();
        golden::check("value-frequency", 1, None, |cv| v.draw(cv));
    }

    #[test]
    fn value_bipolar() {
        let v = Value { name: "Gain".into(), text: "-120.0 dB".into(), value: Some(0.25), bipolar: true, ..Default::default() };
        golden::check("value-bipolar", 1, None, |cv| v.draw(cv));
    }

    #[test]
    fn value_text_only() {
        let v = Value { text: "Off".into(), ..Default::default() };
        golden::check("value-text", 1, None, |cv| v.draw(cv));
    }

    #[test]
    fn touching_the_value_lights_the_bar() {
        let v = freq();
        golden::check("value-focus", 1, Some("value"), |cv| v.draw(cv));
        assert!(golden::focus_changes(240, "value", |cv| v.draw(cv)));
    }

    #[test]
    fn scripts_draw_a_value() {
        let mut p = crate::screen::Picture::new([0; 3]);
        let lua = mlua::Lua::new();
        let t: Table = lua.load("return { name = 'Cutoff', text = '1.20 kHz', value = 0.5, bipolar = true }").eval().unwrap();
        let look = crate::widget::Look::fallback();
        crate::widget::draw_lua("value", &mut p, crate::widget::Rect { x: 0, y: 0, w: 120, h: 104 }, &t, &look).unwrap();
        assert!(p.ops.len() > 3);
    }
}
