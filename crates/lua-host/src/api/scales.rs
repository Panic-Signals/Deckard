//! The scale engine behind Keyboard and Chords modes (`NI.DATA.getScaleEngine`): the
//! scales and chord lists, the notes on the pads and how they are spelled.
//!
//! The tables (`scale_tables.rs`) are Maschine's own, read from its ScaleEngine and
//! ScaleHelper (docs/scales.md). Notes are named with MIDI note 60 as C3.

use super::scale_tables as t;
use super::{methods, object};
use crate::state::{ObjRef, Prop, ScaleState, State};
use mlua::{Lua, MultiValue, Table, Value};

/// "C3" for MIDI note 60, with sharps (the chromatic scale's spelling).
pub fn note_name(note: i64) -> String {
    format!("{}{}", t::NOTES_SHARP[note.rem_euclid(12) as usize], note.div_euclid(12) - 2)
}

/// A scale's index in the tables: bank * 15 + type.
fn scale_index(s: &ScaleState) -> usize {
    (s.bank.clamp(0, 7) * 15 + s.scale.clamp(0, 14)) as usize
}

/// The scale banks (Main, Modes, Jazz, …), as Maschine lists them.
pub fn banks() -> &'static [&'static str; 8] {
    &t::BANKS
}

/// The scales of bank `bank`, by their full names.
pub fn scale_names(bank: i64) -> &'static [&'static str; 15] {
    &t::SCALES[bank.clamp(0, 7) as usize]
}

/// Whether `note` is in scale `scale` of bank `bank` played from `root` (every note is
/// in Main's Chromatic).
pub fn in_scale(bank: i64, scale: i64, root: i64, note: i64) -> bool {
    let index = (bank.clamp(0, 7) * 15 + scale.clamp(0, 14)) as usize;
    t::SNAP[index][(note - root).rem_euclid(12) as usize] == 0
}

/// The semitones above the root that are in a scale.
fn scale_steps(index: usize) -> Vec<i64> {
    (0..12).filter(|&k| t::SNAP[index][k] == 0).map(|k| k as i64).collect()
}

/// The note pad `pad` (0..16, pad 1 bottom left) plays: the scale's notes in order up
/// from the root, an octave higher each time round; 128 above the MIDI range.
pub fn pad_note(s: &ScaleState, root: i64, pad: i64) -> i64 {
    let steps = scale_steps(scale_index(s));
    let n = steps.len() as i64;
    let note = root + 12 * pad.div_euclid(n) + steps[pad.rem_euclid(n) as usize];
    if (0..=127).contains(&note) { note } else { 128 }
}

/// A note's name as Maschine spells it for the scale (`ScaleHelper::getNoteName`):
/// sharps in the chromatic scale, the root from the mixed list, the other notes by
/// their degree of the root's major scale. Notes outside the scale have no name.
pub fn scale_note_name(s: &ScaleState, root: i64, note: i64) -> String {
    deckard_model::scales::note_name(scale_index(s), root, note)
}

/// Chord set `set`'s chord on pad `index` (0..12) in the key of `root`, e.g. "Ami",
/// "Fadd9", "C/B" (`ScaleHelper::getChordSetChordName`).
pub fn chord_set_name(set: i64, root: i64, index: i64, short: bool) -> String {
    let (Ok(set), Ok(i)) = (usize::try_from(set), usize::try_from(index)) else { return String::new() };
    if set >= 16 || i >= 12 {
        return String::new();
    }
    let key = root.rem_euclid(12) as u8;
    // Major sets spell sharp keys with sharps, minor sets theirs; Major 8's second and
    // fourth chords always.
    let sharp =
        if set < 8 { t::SHARP_KEYS_MAJOR.contains(&key) } else { t::SHARP_KEYS_MINOR.contains(&key) } || (set == 7 && matches!(i, 1 | 3));
    let names = if sharp { &t::NOTES_SHARP } else { &t::NOTES_FLAT };
    let mut name = names[(t::CHORD_SET_ROOTS[set][i] as usize + key as usize) % 12].to_string();
    let kind = if short { t::CHORD_SET_TYPES_SHORT[set][i] } else { t::CHORD_SET_TYPES[set][i] };
    // "Bb mi", not "Bbmi".
    if name.ends_with('b') && kind.chars().next().is_some_and(|c| c != '/' && !c.is_ascii_digit()) {
        name.push(' ');
    }
    let bass = if kind.ends_with('/') { names[(t::CHORD_SET_BASS[set][i] as usize + key as usize) % 12] } else { "" };
    format!("{name}{kind}{bass}")
}

/// Chords mode's chords on the pads, as Maschine's scale engine voices them (docs/
/// scales.md, "Chord voicings"): the Harmonizer's on Keyboard mode's notes `keys`, a chord
/// set's on pads 1–12 from `root`; 128 fills the rest. `auto` is Auto's state; `after`
/// gets what each pad's hit would make of it. None outside the Harmonizer and the chord
/// sets (no user chords here yet).
pub fn pad_chords(
    s: &ScaleState,
    root: i64,
    keys: &[i64; 16],
    auto: &deckard_model::chords::Auto,
    after: &mut [Option<deckard_model::chords::Auto>; 16],
) -> Option<[[i64; 16]; 16]> {
    let scale = scale_index(s);
    let kind = match s.chord_mode {
        1 if scale == 0 => s.chord_chromatic,
        1 => s.chord_scale,
        2 => s.chord_set,
        _ => return None,
    };
    let chords = deckard_model::chords::Chords {
        mode: s.chord_mode,
        scale,
        root,
        kind: kind.max(0) as usize,
        chromatic: s.chord_chromatic.max(0) as usize,
        position: s.position.clamp(0, 18) as usize,
        user: &[],
    };
    let mut out = [[128; 16]; 16];
    for (i, pad) in out.iter_mut().enumerate() {
        let note = if s.chord_mode == 1 {
            keys[i]
        } else if i < 12 {
            root + i as i64
        } else {
            128
        };
        after[i] = None;
        if !(0..=127).contains(&note) {
            continue;
        }
        let mut state = auto.clone();
        for (slot, n) in pad.iter_mut().zip(chords.play(note, &mut state)) {
            *slot = n;
        }
        after[i] = Some(state);
    }
    Some(out)
}

/// The root note's name: sharps for the chromatic scale, else the mixed spelling.
fn root_note_name(s: &ScaleState, note: i64) -> String {
    deckard_model::scales::root_name(scale_index(s), note)
}

/// Parameters whose values are MIDI notes.
pub fn is_note(owner: &ObjRef, name: &str) -> bool {
    matches!((owner, name), (ObjRef::Group(_), "RootNote") | (ObjRef::Sound(..), "BaseKey"))
}

/// Parameters the touched-knob list can show (`MaschineParameter.TAG_ENUM`).
pub fn is_enum(owner: &ObjRef, name: &str) -> bool {
    matches!(owner, ObjRef::ScaleEngine) && list(&ScaleState::default(), name).is_some()
}

/// Defaults for the notes the scale engine starts from.
pub fn default(owner: &ObjRef, name: &str) -> Option<Prop> {
    Some(match (owner, name) {
        (ObjRef::Group(_), "RootNote") | (ObjRef::Sound(..), "BaseKey") => Prop::Int(60),
        (ObjRef::Group(_), "KeyboardModeEnabled") => Prop::Bool(false),
        _ => return None,
    })
}

/// The kept chord type the Chords page's TYPE knob shows for the current mode and scale.
fn chord_type(s: &mut ScaleState) -> &mut i64 {
    match s.screen_mode {
        1 => &mut s.chord_set,
        2 => &mut s.user_set,
        _ if scale_index(s) == 0 => &mut s.chord_chromatic,
        _ => &mut s.chord_scale,
    }
}

/// A scale engine setting's full names (as listed when its knob is touched) and short
/// names (its value under the knob).
fn list(s: &ScaleState, name: &str) -> Option<(&'static [&'static str], &'static [&'static str])> {
    let b = s.bank.clamp(0, 7) as usize;
    Some(match name {
        "ScaleBank" => (&t::BANKS, &t::BANKS),
        "Scale" => (&t::SCALES[b], &t::SCALES_SHORT[b]),
        "ChordMode" => (&t::CHORD_MODES, &t::CHORD_MODES_SHORT),
        "ChordScreenChordMode" => (&t::CHORD_SCREEN_MODES, &t::CHORD_SCREEN_MODES_SHORT),
        "CurrentChordType" => match s.screen_mode {
            1 => (&t::CHORD_SETS, &t::CHORD_SETS_SHORT),
            2 => (&t::USER_CHORD_SETS, &t::USER_CHORD_SETS),
            _ if scale_index(s) == 0 => (&t::CHORDS_CHROMATIC, &t::CHORDS_CHROMATIC_SHORT),
            _ => (&t::CHORDS_SCALE, &t::CHORDS_SCALE),
        },
        "ChordPosition" => (&t::CHORD_POSITIONS, &t::CHORD_POSITIONS),
        _ => return None,
    })
}

/// The scale engine's settings, read from the state.
pub fn live(st: &State, owner: &ObjRef, name: &str) -> Option<Prop> {
    if !matches!(owner, ObjRef::ScaleEngine) {
        return None;
    }
    let mut s = st.scale;
    let v = match name {
        "ScaleBank" => s.bank,
        "Scale" => s.scale,
        "ChordMode" => s.chord_mode,
        "ChordScreenChordMode" => s.screen_mode,
        "CurrentChordType" => *chord_type(&mut s),
        "ChordPosition" => s.position,
        _ => return None,
    };
    Some(Prop::Int(v))
}

/// Sets a scale engine setting, kept in range. A new bank starts at its first scale,
/// and the Chords page's mode and the chord mode follow each other (while chords are on).
pub fn write(st: &mut State, owner: &ObjRef, name: &str, v: &Prop) -> bool {
    if !matches!(owner, ObjRef::ScaleEngine) {
        return false;
    }
    let Some(max) = max(st, owner, name) else { return false };
    let v = match v {
        Prop::Int(i) => *i,
        Prop::Num(n) => *n as i64,
        Prop::Bool(b) => *b as i64,
        _ => return true,
    }
    .clamp(0, max);
    let s = &mut st.scale;
    match name {
        "ScaleBank" => {
            if s.bank != v {
                s.scale = 0;
            }
            s.bank = v;
        }
        "Scale" => s.scale = v,
        "ChordMode" => {
            s.chord_mode = v;
            if v > 0 {
                s.screen_mode = v - 1;
            }
        }
        "ChordScreenChordMode" => {
            s.screen_mode = v;
            if s.chord_mode > 0 {
                s.chord_mode = v + 1;
            }
        }
        "CurrentChordType" => *chord_type(s) = v,
        "ChordPosition" => s.position = v,
        _ => return false,
    }
    true
}

/// How a setting's value shows under its knob, where it has a name.
pub fn value_string(st: &State, owner: &ObjRef, name: &str, v: &Prop) -> Option<String> {
    let Prop::Int(i) = v else { return None };
    if is_note(owner, name) {
        return Some(note_name(*i));
    }
    match (owner, name) {
        (ObjRef::ScaleEngine, _) => list(&st.scale, name).and_then(|(_, short)| short.get(*i as usize)).map(|s| s.to_string()),
        _ => None,
    }
}

/// Item `i` of a setting's list, as the touched-knob list shows it.
pub fn list_string(st: &State, owner: &ObjRef, name: &str, i: i64) -> Option<String> {
    if !matches!(owner, ObjRef::ScaleEngine) {
        return None;
    }
    list(&st.scale, name).and_then(|(full, _)| full.get(usize::try_from(i).ok()?)).map(|s| s.to_string())
}

/// The largest value a note or scale setting takes.
pub fn max(st: &State, owner: &ObjRef, name: &str) -> Option<i64> {
    if is_note(owner, name) {
        return Some(127);
    }
    if !matches!(owner, ObjRef::ScaleEngine) {
        return None;
    }
    list(&st.scale, name).map(|(full, _)| full.len() as i64 - 1)
}

/// The focused group's root note.
pub fn root_note(lua: &Lua) -> i64 {
    let f: mlua::Result<i64> =
        lua.load("local g = NI.DATA.StateHelper.getFocusGroup(App); return g and g:getRootNoteParameter():getValue() or 60").eval();
    f.unwrap_or(60)
}

fn scale_state(lua: &Lua) -> ScaleState {
    super::state(lua).borrow().scale
}

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let data = methods(lua, impl_t, "NI.DATA")?;
    data.set("getScaleEngine", lua.create_function(|lua, _: MultiValue| object(lua, ObjRef::ScaleEngine))?)?;
    let se = methods(lua, impl_t, "ScaleEngine")?;
    se.set("getRootNoteName", lua.create_function(|lua, (_, n): (Value, i64)| Ok(root_note_name(&scale_state(lua), n)))?)?;
    // getNoteName(root, note): the note's name, spelled for the root's scale.
    se.set("getNoteName", lua.create_function(|lua, (_, root, n): (Value, i64, i64)| Ok(scale_note_name(&scale_state(lua), root, n)))?)?;
    se.set("getChordModeIsChordSet", lua.create_function(|lua, _: MultiValue| Ok(scale_state(lua).chord_mode == 2))?)?;
    se.set("getChordModeIsUserChord", lua.create_function(|lua, _: MultiValue| Ok(scale_state(lua).chord_mode == 3))?)?;
    // No user chords yet.
    for name in ["userChordContainsAnyNotes", "selectedUserChordContainsNote"] {
        se.set(name, lua.create_function(|_, _: MultiValue| Ok(false))?)?;
    }
    se.set("selectedUserChordIndex", lua.create_function(|_, _: MultiValue| Ok(0))?)?;
    se.set("getUserChordName", lua.create_function(|_, _: MultiValue| Ok(""))?)?;
    // getChordSetChordName(root, pad index, short): the chord on a pad in the chosen set.
    se.set(
        "getChordSetChordName",
        lua.create_function(|lua, (_, root, index, short): (Value, i64, i64, Option<bool>)| {
            Ok(chord_set_name(scale_state(lua).chord_set, root, index, short.unwrap_or(false)))
        })?,
    )?;
    // Keyboard mode's pads: the scale up from the group's root note, pad 1 bottom left.
    let song = methods(lua, impl_t, "Song")?;
    song.set("getNoteForPad", lua.create_function(|lua, (_, pad): (Value, i64)| Ok(pad_note(&scale_state(lua), root_note(lua), pad)))?)?;
    let algos = methods(lua, impl_t, "NI.DATA.GroupAlgorithms")?;
    algos
        .set("canTransposeRootNote", lua.create_function(|lua, (_, inc): (Value, i64)| Ok((0..=127).contains(&(root_note(lua) + inc))))?)?;
    let helper = methods(lua, impl_t, "NI.UTILS.ScaleHelper")?;
    helper.set("getNoteNameChromatic", lua.create_function(|_, n: i64| Ok(note_name(n)))?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale(bank: i64, scale: i64) -> ScaleState {
        ScaleState { bank, scale, ..Default::default() }
    }

    #[test]
    fn notes_are_named_with_middle_c_as_c3() {
        assert_eq!(note_name(60), "C3");
        assert_eq!(note_name(61), "C#3");
        assert_eq!(note_name(0), "C-2");
        assert_eq!(note_name(127), "G8");
    }

    #[test]
    fn pads_follow_the_scale() {
        // Main/Minor from C3, as Maschine's pads showed: C3 D3 Eb3 F3 G3 Ab3 Bb3 C4 …
        let minor = scale(0, 2);
        let pads: Vec<String> = (0..16).map(|p| scale_note_name(&minor, 60, pad_note(&minor, 60, p))).collect();
        assert_eq!(pads, ["C3", "D3", "Eb3", "F3", "G3", "Ab3", "Bb3", "C4", "D4", "Eb4", "F4", "G4", "Ab4", "Bb4", "C5", "D5"]);
        // Chromatic: a semitone a pad, sharps.
        let chrom = ScaleState::default();
        assert_eq!(pad_note(&chrom, 60, 15), 75);
        assert_eq!(scale_note_name(&chrom, 60, 63), "D#3");
        // Major pentatonic: five notes an octave.
        let pent = scale(0, 4);
        assert_eq!((0..6).map(|p| pad_note(&pent, 60, p)).collect::<Vec<_>>(), [60, 62, 64, 67, 69, 72]);
        assert_eq!(pad_note(&chrom, 120, 15), 128);
    }

    #[test]
    fn notes_are_spelled_for_the_root() {
        // F major has B flat; E major four sharps.
        let major = scale(0, 1);
        assert_eq!(scale_note_name(&major, 65, 70), "Bb3");
        assert_eq!(scale_note_name(&major, 64, 68), "G#3");
        // Notes outside the scale have no name.
        assert_eq!(scale_note_name(&major, 60, 61), "");
    }

    #[test]
    fn chords_mode_plays_chords() {
        let keys: [i64; 16] = std::array::from_fn(|i| pad_note(&scale(0, 1), 60, i as i64));
        let (auto, mut after) = (Default::default(), Default::default());
        let off = ScaleState { chord_mode: 0, ..scale(0, 1) };
        assert!(pad_chords(&off, 60, &keys, &auto, &mut after).is_none());
        // C major, 1-3-5: D minor on pad 2.
        let harm = ScaleState { chord_mode: 1, ..scale(0, 1) };
        assert_eq!(pad_chords(&harm, 60, &keys, &auto, &mut after).unwrap()[1][..4], [62, 65, 69, 128]);
        assert!(after[1].is_some(), "a hit moves Auto on");
        // Minor 3 at position 1: the bass stays, the rest inverts.
        let set = ScaleState { chord_mode: 2, chord_set: 10, position: 10, ..scale(0, 1) };
        assert_eq!(pad_chords(&set, 60, &keys, &auto, &mut after).unwrap()[0][..5], [48, 63, 67, 72, 128]);
    }

    #[test]
    fn chord_sets_name_their_chords() {
        // Major 1 in C, as Maschine's pads showed.
        let names: Vec<String> = (0..12).map(|i| chord_set_name(0, 60, i, true)).collect();
        assert_eq!(names, ["C", "Emi", "F", "G", "Ami", "Esus4", "Gadd9", "Dmi", "Fadd9", "F6", "Gsus4", "G"]);
        // Major 2's slash chords take their bass note.
        assert_eq!(chord_set_name(1, 60, 1, true), "C/B");
        // Flat keys spell flats, and a flat stays apart from a lettered type.
        assert_eq!(chord_set_name(0, 70, 0, true), "Bb");
        assert_eq!(chord_set_name(0, 70, 8, true), "Eb add9");
    }

    #[test]
    fn a_new_bank_starts_at_its_first_scale() {
        let mut st = State::default();
        write(&mut st, &ObjRef::ScaleEngine, "Scale", &Prop::Int(9));
        write(&mut st, &ObjRef::ScaleEngine, "ScaleBank", &Prop::Int(2));
        assert_eq!((st.scale.bank, st.scale.scale), (2, 0));
        assert_eq!(value_string(&st, &ObjRef::ScaleEngine, "Scale", &Prop::Int(0)).as_deref(), Some("Lyd b7"));
        assert_eq!(list_string(&st, &ObjRef::ScaleEngine, "Scale", 0).as_deref(), Some("Lydian b7"));
        write(&mut st, &ObjRef::ScaleEngine, "Scale", &Prop::Int(99));
        assert_eq!(st.scale.scale, 14);
    }

    #[test]
    fn the_chord_type_list_follows_the_mode() {
        let mut st = State::default();
        let shown = |st: &State| {
            value_string(st, &ObjRef::ScaleEngine, "CurrentChordType", &live(st, &ObjRef::ScaleEngine, "CurrentChordType").unwrap())
        };
        assert_eq!(shown(&st).as_deref(), Some("Major"));
        write(&mut st, &ObjRef::ScaleEngine, "Scale", &Prop::Int(2));
        assert_eq!(shown(&st).as_deref(), Some("1-3-5"));
        write(&mut st, &ObjRef::ScaleEngine, "ChordScreenChordMode", &Prop::Int(1));
        assert_eq!(shown(&st).as_deref(), Some("Maj 1"));
        write(&mut st, &ObjRef::ScaleEngine, "ChordScreenChordMode", &Prop::Int(2));
        assert_eq!(shown(&st).as_deref(), Some("Set 1"));
    }
}
