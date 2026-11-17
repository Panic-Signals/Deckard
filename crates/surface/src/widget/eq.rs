//! `eq`: not drawn yet (see the plug-in mappings spec's "Widgets" table).

use mlua::Table;

use super::Canvas;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Eq {}

impl Eq {
    pub fn from_lua(_t: &Table) -> mlua::Result<Eq> {
        Ok(Eq::default())
    }

    pub fn draw(&self, _c: &mut Canvas) {}
}
