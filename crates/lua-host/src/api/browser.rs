//! BROWSE: NI's browse pages read Maschine's browser through `App:getDatabaseFrontend()`
//! (its model and controller) and change it through
//! `App:getBrowserControllerDeferredSearch()`. Here both are the app's library as it
//! hands it over ([`State::browser`], a [`crate::state::BrowserView`]); what the page asks
//! is left in [`State::browse_calls`]. When the view changes the host tells the
//! scripts (`onDB3ModelChanged`), as Maschine does after a search.
//!
//! The scripts' model: the file type (section), user mode (User/Factory), the bank
//! chain (slot 0 the products; DECKARD has no banks under them), three attributes
//! (Type, Sub-type, Character) and the result list.

use super::data::set_fn;
use super::{methods, object, state};
use crate::state::{BrowseCall, ObjRef, State};
use mlua::{Lua, MultiValue, Table, Value};

/// `NI.DB3` constants: the scripts only compare them with what this API returns.
pub const CONSTANTS: [(&str, i64); 16] = [
    ("MODEL_BROWSER", 1),
    ("MODEL_BANKCHAIN", 2),
    ("MODEL_ATTRIBUTES", 3),
    ("MODEL_RESULTS", 4),
    ("MODEL_PRODUCTS", 5),
    ("FILE_TYPE_PROJECT", 11),
    ("FILE_TYPE_GROUP", 12),
    ("FILE_TYPE_SOUND", 13),
    ("FILE_TYPE_INSTRUMENT", 14),
    ("FILE_TYPE_EFFECT", 15),
    ("FILE_TYPE_LOOP_SAMPLE", 16),
    ("FILE_TYPE_ONESHOT_SAMPLE", 17),
    ("BY_CATEGORY", 0),
    ("BY_VENDOR", 1),
    ("SOURCE_DEFAULT", 0),
    ("SOURCE_GUI", 1),
];

pub const MODEL_BANKCHAIN: i64 = 2;
pub const MODEL_ATTRIBUTES: i64 = 3;
pub const MODEL_RESULTS: i64 = 4;
pub const MODEL_PRODUCTS: i64 = 5;

/// The sections in `deckard_library::Kind::ALL`'s order, as the scripts name them.
const FILE_TYPES: [(i64, &str); 7] =
    [(11, "Project"), (12, "Group"), (13, "Sound"), (14, "Instrument"), (15, "Effect"), (16, "Loop"), (17, "One-shot")];

/// Maschine's name for "everything" in a product group.
const ALL_PRODUCTS: &str = "All...";
const NPOS: i64 = -1;

fn call(lua: &Lua, c: BrowseCall) {
    state(lua).borrow_mut().browse_calls.push(c);
}

fn with<T>(lua: &Lua, f: impl FnOnce(&State) -> T) -> T {
    f(&state(lua).borrow())
}

fn index(i: Option<usize>) -> i64 {
    i.map_or(NPOS, |i| i as i64)
}

/// A 0-based index from the scripts (-1: none).
fn opt(i: i64) -> Option<usize> {
    usize::try_from(i).ok()
}

fn attribute(st: &State, a: i64) -> Option<&(Vec<String>, Option<usize>)> {
    st.browser.attributes.get(usize::try_from(a).ok()?)
}

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let app = methods(lua, impl_t, "App")?;
    set_fn(lua, &app, "getDatabaseFrontend", |lua, _: MultiValue| object(lua, ObjRef::DatabaseFrontend))?;
    set_fn(lua, &app, "getBrowserControllerDeferredSearch", |lua, _: MultiValue| object(lua, ObjRef::DeferredSearch))?;
    let df = methods(lua, impl_t, "DatabaseFrontend")?;
    set_fn(lua, &df, "getBrowserController", |lua, _: MultiValue| object(lua, ObjRef::BrowserController))?;
    set_fn(lua, &df, "getBrowserModel", |lua, _: MultiValue| object(lua, ObjRef::BrowserModel))?;

    // The model.
    let m = methods(lua, impl_t, "BrowserModel")?;
    set_fn(lua, &m, "getSelectedFileType", |lua, _: MultiValue| Ok(with(lua, |st| FILE_TYPES.get(st.browser.kind).map_or(0, |f| f.0))))?;
    set_fn(lua, &m, "getFileTypeSelection", |lua, _: MultiValue| {
        Ok(with(lua, |st| FILE_TYPES.get(st.browser.kind).map_or("", |f| f.1).to_string()))
    })?;
    set_fn(lua, &m, "getVisibleFileTypeNames", |lua, _: MultiValue| lua.create_sequence_from(FILE_TYPES.iter().map(|f| f.1)))?;
    set_fn(lua, &m, "hasNextVisibleFileType", |lua, _: MultiValue| Ok(with(lua, |st| st.browser.kind + 1 < FILE_TYPES.len())))?;
    set_fn(lua, &m, "hasPreviousVisibleFileType", |lua, _: MultiValue| Ok(with(lua, |st| st.browser.kind > 0)))?;
    set_fn(lua, &m, "getUserMode", |lua, _: MultiValue| Ok(with(lua, |st| st.browser.user)))?;
    set_fn(lua, &m, "getProductGroupSelection", |_, _: MultiValue| Ok(ALL_PRODUCTS))?;
    set_fn(lua, &m, "getProductModel", |lua, _: MultiValue| object(lua, ObjRef::ProductModel))?;
    set_fn(lua, &m, "getBankChainModel", |lua, _: MultiValue| object(lua, ObjRef::BankChainModel))?;
    set_fn(lua, &m, "getAttributesModel", |lua, _: MultiValue| object(lua, ObjRef::AttributesModel))?;
    set_fn(lua, &m, "getResultListModel", |lua, _: MultiValue| object(lua, ObjRef::ResultListModel))?;
    set_fn(lua, &m, "getProductsSortOrder", |_, _: MultiValue| Ok(0))?;
    set_fn(lua, &m, "isFavoritesFilterEnabled", |lua, _: MultiValue| Ok(with(lua, |st| st.browser.favorites)))?;
    // "The file type was just set" (forces the info bar's redraw): it redraws anyway.
    set_fn(lua, &m, "isFileTypeSetStateFlag", |_, _: MultiValue| Ok(false))?;

    // Products: one group of them all.
    let p = methods(lua, impl_t, "ProductModel")?;
    set_fn(lua, &p, "areProductsGrouped", |_, _: MultiValue| Ok(false))?;
    set_fn(lua, &p, "getNumProductGroups", |_, _: MultiValue| Ok(1))?;
    set_fn(lua, &p, "getProductGroupIndexFromName", |_, _: MultiValue| Ok(0))?;
    set_fn(lua, &p, "getProductGroupNameFromIndex", |_, _: MultiValue| Ok(ALL_PRODUCTS))?;
    set_fn(lua, &p, "getProductGroupSize", |lua, _: MultiValue| Ok(with(lua, |st| st.browser.products.len() as i64)))?;
    set_fn(lua, &p, "getProductVendorizedNameByIndex", |lua, (_, i): (Value, i64)| {
        Ok(with(lua, |st| opt(i).and_then(|i| st.browser.products.get(i).cloned()).unwrap_or_default()))
    })?;

    // The bank chain: slot 0 the products.
    let bc = methods(lua, impl_t, "BankChainModel")?;
    set_fn(lua, &bc, "getSelectedSlotItem", |lua, (_, slot): (Value, i64)| {
        Ok(with(lua, |st| if slot == 0 { index(st.browser.product) } else { NPOS }))
    })?;
    set_fn(lua, &bc, "getSlotItemCount", |lua, (_, slot): (Value, i64)| {
        Ok(with(lua, |st| if slot == 0 { st.browser.products.len() as i64 } else { 0 }))
    })?;
    set_fn(lua, &bc, "getSlotItemName", |lua, (_, slot, i): (Value, i64, i64)| {
        Ok(with(lua, |st| if slot == 0 { opt(i).and_then(|i| st.browser.products.get(i).cloned()) } else { None }).unwrap_or_default())
    })?;

    // Type, Sub-type, Character.
    let at = methods(lua, impl_t, "AttributesModel")?;
    set_fn(lua, &at, "getActiveAttributeCount", |lua, (_, a): (Value, i64)| {
        Ok(with(lua, |st| attribute(st, a).map_or(0, |x| x.0.len() as i64)))
    })?;
    set_fn(lua, &at, "getActiveIndexOfFocusAttribute", |lua, (_, a): (Value, i64)| {
        Ok(with(lua, |st| attribute(st, a).map_or(NPOS, |x| index(x.1))))
    })?;
    set_fn(lua, &at, "getActiveAttributeName", |lua, (_, a, i): (Value, i64, i64)| {
        Ok(with(lua, |st| attribute(st, a).and_then(|x| opt(i).and_then(|i| x.0.get(i).cloned()))).unwrap_or_default())
    })?;
    set_fn(lua, &at, "getAttributeSelectionCount", |lua, (_, a): (Value, i64)| {
        Ok(with(lua, |st| attribute(st, a).map_or(0, |x| i64::from(x.1.is_some()))))
    })?;

    // Results.
    let r = methods(lua, impl_t, "ResultListModel")?;
    set_fn(lua, &r, "getItemCount", |lua, _: MultiValue| Ok(with(lua, |st| st.browser.results.len() as i64)))?;
    set_fn(lua, &r, "getFocusItem", |lua, _: MultiValue| {
        Ok(with(lua, |st| if st.browser.results.is_empty() { NPOS } else { st.browser.focus as i64 }))
    })?;
    set_fn(lua, &r, "getItemName", |lua, (_, i): (Value, i64)| {
        Ok(with(lua, |st| opt(i).and_then(|i| st.browser.results.get(i).map(|x| x.0.clone()))).unwrap_or_default())
    })?;
    set_fn(lua, &r, "isItemSelected", |lua, (_, i): (Value, i64)| Ok(with(lua, |st| opt(i) == Some(st.browser.focus))))?;
    set_fn(lua, &r, "isItemFavorite", |lua, (_, i): (Value, i64)| {
        Ok(with(lua, |st| opt(i).and_then(|i| st.browser.results.get(i)).is_some_and(|x| x.1)))
    })?;

    // What the page does.
    let c = methods(lua, impl_t, "BrowserController")?;
    set_fn(lua, &c, "hasJob", |lua, _: MultiValue| Ok(with(lua, |st| st.browser.busy)))?;
    set_fn(lua, &c, "changeResultListFocus", |lua, (_, by): (Value, i64)| {
        if by != 0 {
            call(lua, BrowseCall::Focus(by));
        }
        Ok(())
    })?;
    set_fn(lua, &c, "loadSelectedResultListItem", |lua, _: MultiValue| {
        call(lua, BrowseCall::Load);
        Ok(())
    })?;
    set_fn(lua, &c, "toggleFavoritesFilter", |lua, _: MultiValue| {
        call(lua, BrowseCall::ToggleFavorites);
        Ok(())
    })?;
    let s = methods(lua, impl_t, "DeferredSearch")?;
    set_fn(lua, &s, "setUserMode", |lua, (_, on): (Value, bool)| {
        call(lua, BrowseCall::User(on));
        Ok(())
    })?;
    set_fn(lua, &s, "selectPrevOrNextVisibleFileType", |lua, (_, next): (Value, bool)| {
        let k = with(lua, |st| st.browser.kind);
        let to = if next { (k + 1).min(FILE_TYPES.len() - 1) } else { k.saturating_sub(1) };
        if to != k {
            call(lua, BrowseCall::Kind(to));
        }
        Ok(())
    })?;
    set_fn(lua, &s, "setBankChainSelection", |lua, (_, slot, i): (Value, i64, i64)| {
        if slot == 0 {
            call(lua, BrowseCall::Product(opt(i)));
        }
        Ok(())
    })?;
    set_fn(lua, &s, "setAttributeSingleSelection", |lua, (_, a, i): (Value, i64, i64)| {
        if let Some(a) = opt(a).filter(|&a| a < 3) {
            call(lua, BrowseCall::Attribute(a, opt(i)));
        }
        Ok(())
    })?;
    set_fn(lua, &s, "setProductGroupSelection", |_, _: MultiValue| Ok(()))?;

    let fts = methods(lua, impl_t, "NI.DB3.filetypes")?;
    set_fn(lua, &fts, "getFileTypeName", |_, (ft,): (i64,)| Ok(FILE_TYPES.iter().find(|f| f.0 == ft).map_or("", |f| f.1)))?;

    // Prehear and Quick Browse.
    let pa = methods(lua, impl_t, "NI.DATA.SamplePrehearAccess")?;
    let is_sample = |lua: &Lua| with(lua, |st| st.browser.kind >= 5);
    set_fn(lua, &pa, "isPrehearTypeSelected", move |lua, _: MultiValue| Ok(is_sample(lua)))?;
    set_fn(lua, &pa, "shouldShowPrehearWidget", move |lua, _: MultiValue| Ok(is_sample(lua)))?;
    set_fn(lua, &pa, "playLastDatabaseBrowserSelection", |lua, _: MultiValue| {
        call(lua, BrowseCall::Prehear);
        Ok(())
    })?;
    let qb = methods(lua, impl_t, "NI.DATA.QuickBrowseAccess")?;
    set_fn(lua, &qb, "restoreQuickBrowse", |_, _: MultiValue| Ok(()))?;
    Ok(())
}
