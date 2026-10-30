//! `NI.UTILS`: small helpers and engine constants.

use super::methods;
use mlua::{Lua, Table};

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let u = methods(lua, impl_t, "NI.UTILS")?;
    u.set("TICKS_PER_QUARTER", deckard_model::PPQ)?;
    // The smallest snap (SHIFT on the edit knobs): DECKARD's guess, a 128th note.
    u.set("CONST_MIN_SNAP", super::events::MIN_SNAP)?;
    u.set("CONST_SCENES_PER_BANK", 16)?;
    // getManagedResource(kind, name, key, _): artwork and short names Maschine keeps for
    // its library products; DECKARD has none ("" — the scripts fall back to the name).
    u.set("getManagedResource", lua.create_function(|_, _: mlua::MultiValue| Ok(""))?)?;
    // Interned style/attribute names; plain strings work the same for our widgets.
    u.set("Symbol", lua.create_function(|_, s: String| Ok(s))?)?;
    u.set("removeStringParensAndWhitespace", lua.create_function(|_, s: String| Ok(remove_parens_and_whitespace(&s)))?)?;
    u.set("getUserHomeDir", lua.create_function(|_, ()| Ok(std::env::var("HOME").unwrap_or_default()))?)?;
    Ok(())
}

/// "Bass Synth (Legacy)" → "BassSynth": what `removeStringParensAndWhitespace` does.
pub fn remove_parens_and_whitespace(s: &str) -> String {
    let mut depth = 0;
    s.chars()
        .filter(|&c| {
            match c {
                '(' => depth += 1,
                ')' => depth = (depth - 1).max(0),
                _ => return depth == 0 && !c.is_whitespace(),
            }
            false
        })
        .collect()
}
