//! The scripts' display widgets as a [`deckard_nwl`] tree, to draw as NI's hardware
//! does.

use crate::pattern_view::{self, View};
use crate::state::{Prop, State, Widgets};
use deckard_nwl::draw::{Dir, Element};

/// Widget `id` and what's under it; None if it's hidden or inactive. Under it,
/// inactive widgets (`setActive(false)`) are left out, and hidden ones
/// (`setVisible(false)`) kept, marked hidden: NWL leaves their space empty. Native
/// widgets get drawn what they show (the pattern editors, as `view` says).
pub fn element(st: &State, id: usize, view: View) -> Option<Element> {
    build(&st.widgets, id, Some((st, view))).filter(|e| !e.hidden)
}

/// Widget `id` and its ancestors, root first, each without its children: what style
/// sheet rules match on.
pub fn chain(w: &Widgets, id: usize) -> Vec<Element> {
    let mut out = Vec::new();
    let mut at = Some(id);
    while let Some(i) = at {
        out.push(build_one(w, i, None, false).unwrap_or_default());
        at = w.get(i).and_then(|x| x.parent);
    }
    out.reverse();
    out
}

fn int(p: &Prop) -> i32 {
    match p {
        Prop::Int(i) => *i as i32,
        Prop::Num(n) => n.round() as i32,
        _ => 0,
    }
}

fn build(w: &Widgets, id: usize, native: Option<(&State, View)>) -> Option<Element> {
    build_one(w, id, native, true)
}

/// Widget `id` as an element, with what's under it if `deep`.
fn build_one(w: &Widgets, id: usize, native: Option<(&State, View)>, deep: bool) -> Option<Element> {
    let x = w.get(id)?;
    let p = |k: &str| x.props.get(k);
    if matches!(p("Active"), Some(Prop::Bool(false))) {
        return None;
    }
    let str_of = |k: &str| match p(k) {
        Some(Prop::Str(s)) => Some(s.clone()),
        _ => None,
    };
    let class = str_of("Class").unwrap_or_default();
    // Labels and buttons take their text from `style(text, class)` until `setText`.
    let text = str_of("Text").or_else(|| str_of("style").filter(|s| *s != class && !s.is_empty()));
    let index_of = |k: &str| match p(k) {
        Some(Prop::Widget(c)) => x.children.iter().position(|ch| ch == c),
        _ => None,
    };
    let attrs = x
        .props
        .iter()
        .filter_map(|(k, v)| {
            Some((
                k.strip_prefix("attr:")?.to_string(),
                match v {
                    Prop::Str(s) => s.clone(),
                    _ => String::new(),
                },
            ))
        })
        .collect();
    let mut children = Vec::new();
    let mut flex = None;
    let mut top = None;
    for (i, &c) in x.children.iter().enumerate().filter(|_| deep) {
        if let Some(e) = build(w, c, native) {
            if index_of("Flex") == Some(i) {
                flex = Some(children.len());
            }
            // (`setTop(widget)`, or `setTopIndex(i)`: the Settings page's tabs.)
            if index_of("Top") == Some(i) || matches!(p("TopIndex"), Some(Prop::Int(n)) if *n == i as i64) {
                top = Some(children.len());
            }
            children.push(e);
        }
    }
    let mut e = Element {
        kind: x.kind.clone(),
        name: x.name.clone(),
        class,
        attrs,
        hidden: matches!(p("Visible"), Some(Prop::Bool(false))),
        selected: matches!(p("Selected"), Some(Prop::Bool(true))),
        enabled: !matches!(p("Enabled"), Some(Prop::Bool(false))),
        text,
        dir: match p("Align") {
            Some(Prop::Int(1)) => Dir::Down,
            Some(Prop::Int(2)) => Dir::Left,
            Some(Prop::Int(3)) => Dir::Up,
            _ => Dir::Right,
        },
        flex,
        top,
        palette_index: match p("PaletteColorIndex") {
            Some(Prop::Int(i)) if *i >= 0 => Some(*i as u32),
            _ => None,
        },
        // setNoAlign(true): out of its parent's row, at its corner unless placed.
        pos: match (p("PosX"), p("PosY"), p("NoAlign")) {
            (Some(x), Some(y), _) => Some((int(x), int(y))),
            (_, _, Some(Prop::Bool(true))) => Some((0, 0)),
            _ => None,
        },
        width: p("Width").map(int),
        height: p("Height").map(int),
        scroll: None,
        shapes: Vec::new(),
        picture: str_of("PictureUrl"),
        // A transparent colour (the scripts' "none", as an empty slot's palette gives) is
        // no colour: the style sheet's background shows.
        bg: match p("BGColor") {
            Some(Prop::Int(c)) => Some((*c as u32).to_be_bytes()).filter(|c| c[3] > 0),
            _ => None,
        },
        children,
    };
    // A browser result: its text and selection are its label's (as the style sheet has
    // them, `ResultListItemWidget Label#m_pTextLabel`).
    if e.kind == "ResultListItemWidget"
        && let Some(label) = e.children.iter_mut().find(|c| c.name == "m_pTextLabel")
    {
        if label.text.is_none() {
            label.text = e.text.take();
        }
        label.selected = e.selected;
    }
    // A Mixer strip: drawn whole by the host (its parts are what the script set).
    if e.kind == "MixerBar" && deep {
        e.shapes = crate::mixer_view::bar(w, id);
        e.children.clear();
        e.width = e.width.or(Some(120));
        e.height = e.height.or(Some(crate::mixer_view::height(w, id)));
    }
    // The Song view's arranger: the overview is the one marked for the hardware screen.
    if e.kind == "ArrangerPanel"
        && let Some((st, _)) = native
    {
        e.shapes = crate::arranger_view::shapes(st, matches!(p("HWScreen"), Some(Prop::Bool(true))));
    }
    // The Sampling page's wave displays (the overview marked `setHWOverview()`) and
    // zone map.
    if let Some((st, _)) = native {
        let overview = x.props.contains_key("HWOverview");
        match e.kind.as_str() {
            "SampleOwnerWaveEditor" => e.shapes = crate::sample_view::wave(st, overview, false),
            "SliceWaveEditor" => e.shapes = crate::sample_view::wave(st, overview, true),
            "ZoneMapEditor" => e.shapes = crate::sample_view::zone_map(st),
            _ => {}
        }
    }
    // The Sampling page's input meter.
    if e.kind == "MasterLevelMeter" {
        e.shapes = crate::record_meter::shapes(w, id);
    }
    // The piano roll's keys beside it.
    if e.kind == "PianorollKeyboardWidget"
        && let Some((st, view)) = native
    {
        e.shapes = pattern_view::keyboard(st, view);
    }
    // The pattern editor: the overview is the one given an overview source.
    if e.kind == "EventPatternEditor"
        && let Some((st, view)) = native
    {
        // The Step page's PROBABILITY button: probabilities instead of velocities.
        let view = View { probability: matches!(p("HWStepDisplayProbability"), Some(Prop::Bool(true))), ..view };
        e.shapes = pattern_view::shapes(st, matches!(p("OverviewSource"), Some(Prop::Widget(_))), view);
    }
    // A list's scrollbar: a grip over the items shown, hidden while they all fit.
    if e.kind.starts_with("Vector<") {
        let num = |k: &str| p(k).map(int).unwrap_or(0);
        let (total, rows, offset) = (num("ScrollTotal"), num("ScrollRows"), num("ItemOffset"));
        if let Some(sb) = e.children.iter_mut().find(|c| c.kind == "Scrollbar") {
            if total > rows && rows > 0 {
                sb.scroll = Some((offset as f32 / total as f32, rows as f32 / total as f32));
                sb.children.insert(0, Element { kind: "Grip".into(), name: "thumb".into(), enabled: true, ..Default::default() });
            } else {
                sb.hidden = true;
            }
        }
    }
    // A text edit draws its text in a content area of its own, styled as
    // `MultilineTextEditContentArea` with the edit's class, state and color.
    if e.kind == "MultilineTextEdit" {
        let area = Element {
            kind: "MultilineTextEditContentArea".into(),
            name: String::new(),
            text: e.text.take(),
            children: Vec::new(),
            ..e.clone()
        };
        e.flex = Some(e.children.len());
        e.children.push(area);
    }
    Some(e)
}
