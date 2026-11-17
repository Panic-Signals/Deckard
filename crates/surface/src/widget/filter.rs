//! `filter`: not drawn yet (see the plug-in mappings spec's "Widgets" table).

use mlua::Table;

use super::Canvas;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {}

impl Filter {
    pub fn from_lua(_t: &Table) -> mlua::Result<Filter> {
        Ok(Filter::default())
    }

    pub fn draw(&self, _c: &mut Canvas) {}
}
