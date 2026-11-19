//! `choices`: a knob that picks one of a few values (a filter type, a waveform, a
//! switch), as the list of them with the one it's on filled in. Drawn where such a knob
//! has no picture over it, in place of a dial; up to [`MAX`] values (a longer list
//! would be too small to read: such a knob gets nothing, its cell shows the value).
//!
//! The list is left-aligned with the knob's name and value in the cell below, and
//! centred in the box's height.
//!
//! Inputs:
//!
//! | input | |
//! |---|---|
//! | `items` | the values' names, in order |
//! | `active` | the one the knob is on (1-based, as Lua counts) |
//!
//! The values it isn't on stay dim, touched or not: only the filled one stands out.
//! The box is filled with the screen's background first, so a list drawn over a
//! picture (a plug-in's, on the right screen) stays readable.

use mlua::Table;

use super::{Align, Canvas, mix, num};

/// The most values listed.
pub const MAX: usize = 6;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choices {
    pub items: Vec<String>,
    /// 0-based.
    pub active: Option<usize>,
}

/// Each row's height: roomy for a few values, tighter for six.
fn row_h(n: usize, h: f32) -> f32 {
    (h / n.max(1) as f32).clamp(11.0, 18.0)
}

impl Choices {
    pub fn from_lua(t: &Table) -> mlua::Result<Choices> {
        let items: Vec<String> = match t.raw_get::<Option<Table>>("items")? {
            Some(list) => list.sequence_values::<String>().filter_map(Result::ok).collect(),
            None => Vec::new(),
        };
        let active = num(t, "active").and_then(|a| (a as usize).checked_sub(1)).filter(|a| *a < items.len());
        Ok(Choices { items, active })
    }

    pub fn draw(&self, c: &mut Canvas) {
        let n = self.items.len();
        if n == 0 || n > MAX {
            return;
        }
        let look = *c.look;
        c.fill(0, 0, c.r.w, c.r.h, look.background);
        let rh = row_h(n, c.h() - 8.0);
        let top = ((c.h() - rh * n as f32) / 2.0).round();
        let mut st = look.label;
        st.size = if n > 4 { 11.0 } else { 12.0 };
        // (The cell's text starts 11 px in; the fill reaches a little left of it.)
        let (x, w) = (5, c.r.w - 10);
        for (i, item) in self.items.iter().enumerate() {
            let y = (top + rh * i as f32).round() as i32;
            let on = self.active == Some(i);
            if on {
                c.fill(x, y + 1, w, rh as i32 - 2, c.ink(look.accent, &["value"]));
            }
            let ink = if on { look.on_colour } else { mix(look.background, look.text, 0.6) };
            c.text(x + 6, y, w - 8, rh as i32, item, st, ink, Align::Left);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::golden;

    fn list(items: &[&str], active: usize) -> Choices {
        Choices { items: items.iter().map(|s| s.to_string()).collect(), active: Some(active) }
    }

    #[test]
    fn choices_two() {
        let c = list(&["Off", "On"], 1);
        golden::check("choices-two", 1, None, |cv| c.draw(cv));
    }

    #[test]
    fn choices_four() {
        let c = list(&["BP", "LP1", "LP2", "MM"], 3);
        golden::check("choices-four", 1, None, |cv| c.draw(cv));
        // Touching the knob doesn't brighten the values it isn't on.
        assert!(!golden::focus_changes(120, "value", |cv| c.draw(cv)));
    }

    #[test]
    fn choices_six() {
        let c = list(&["Triangle", "Tri/Saw", "Saw", "Pulse", "Narrow", "Very Nrw"], 2);
        golden::check("choices-six", 1, None, |cv| c.draw(cv));
    }

    #[test]
    fn too_many_draws_nothing() {
        let items: Vec<String> = (0..9).map(|i| format!("Mode {i}")).collect();
        let c = Choices { items, active: Some(2) };
        let look = golden::look("maschine");
        let px = golden::render(&look, 120, None, |cv| c.draw(cv));
        assert!(px.iter().all(|p| p[..3] == look.background.rgb), "nothing drawn");
    }

    #[test]
    fn scripts_draw_choices() {
        let lua = mlua::Lua::new();
        let t: Table = lua.load("return { items = { 'A', 'B', 'C' }, active = 2 }").eval().unwrap();
        let c = Choices::from_lua(&t).unwrap();
        assert_eq!(c.active, Some(1));
        let mut p = crate::screen::Picture::new([0; 3]);
        let look = crate::widget::Look::fallback();
        crate::widget::draw_lua("choices", &mut p, crate::widget::Rect { x: 0, y: 0, w: 120, h: 94 }, &t, &look).unwrap();
        assert!(p.texts.iter().any(|t| t == "B"));
    }
}
