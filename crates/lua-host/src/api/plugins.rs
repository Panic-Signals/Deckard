//! The controller's plug-in browser (the Module page, PLUG-IN then the 4-D encoder): the
//! plug-ins the app can load (`State::plugins`) as NI's scripts ask for them, built-in
//! modules through `NI.DATA.ModuleFactory` and external plug-ins through
//! `NI.DATA.PluginHelper` (by format and vendor, `NI.HOSTING.PluginFilter` bits), and
//! loading one into the focused slot or after it ([`ControlRequest::Load`], carried out by
//! the app).
//!
//! A built-in module's id is [`ID_BASE`] + its entry, so `createModuleOrLoadDefault`
//! finds it again; a plug-in descriptor is its entry.

use super::{methods, object, object_ref, state};
use crate::state::{ControlRequest, ObjRef, PluginEntry, PluginFormat, State};
use mlua::{Lua, MultiValue, Table, Value};

/// Built-in modules' ids start here (clear of `NI.DATA.ModuleInfo`'s own ids).
pub(crate) const ID_BASE: i64 = 10_000;

/// Maschine's name for its own vendor in the browser.
const VENDOR_NI: &str = "Native Instruments";

/// What a `PluginFilter` lets through.
#[derive(Clone, Copy)]
struct Filter {
    bits: i64,
}

impl Filter {
    fn new(lua: &Lua, bits: i64) -> (Filter, Bits) {
        (Filter { bits }, Bits::read(lua))
    }
}

/// The `NI.HOSTING.PluginFilter` bits.
struct Bits {
    not_synth: i64,
    not_effect: i64,
    third_party: i64,
    not_third_party: i64,
    not_au: i64,
    not_vst3: i64,
    not_vst2: i64,
}

impl Bits {
    fn read(lua: &Lua) -> Bits {
        let get = |name: &str| -> i64 { lua.load(format!("return NI.HOSTING.PluginFilter.{name} or 0")).eval().unwrap_or(0) };
        Bits {
            not_synth: get("SKIP_NOT_SYNTH_PLUGINS"),
            not_effect: get("SKIP_NOT_EFFECT_PLUGINS"),
            third_party: get("SKIP_THIRD_PARTY_PLUGINS"),
            not_third_party: get("SKIP_NOT_THIRD_PARTY_PLUGINS"),
            not_au: get("SKIP_NOT_AUv2_PLUGINS"),
            not_vst3: get("SKIP_NOT_VST3_PLUGINS"),
            not_vst2: get("SKIP_NOT_VST2_PLUGINS"),
        }
    }
}

fn passes(e: &PluginEntry, f: Filter, b: &Bits) -> bool {
    let on = |bit: i64| bit != 0 && f.bits & bit != 0;
    let ni = e.vendor == VENDOR_NI;
    e.format != PluginFormat::Internal
        && !(on(b.not_synth) && !e.instrument)
        && !(on(b.not_effect) && e.instrument)
        && !(on(b.third_party) && !ni)
        && !(on(b.not_third_party) && ni)
        && !(on(b.not_au) && e.format != PluginFormat::AudioUnit)
        && !(on(b.not_vst3) && e.format != PluginFormat::Vst3)
        && !(on(b.not_vst2) && e.format != PluginFormat::Vst2)
}

/// The external plug-ins a filter (and vendor filter) lets through, by entry, in order.
fn external(st: &State, lua: &Lua, filter: i64, vendor_filter: i64, vendor: Option<&str>) -> Vec<usize> {
    let (f, b) = Filter::new(lua, filter | vendor_filter);
    st.plugins.iter().enumerate().filter(|(_, e)| passes(e, f, &b) && vendor.is_none_or(|v| e.vendor == v)).map(|(i, _)| i).collect()
}

/// The built-in instruments (or effects), by entry.
fn internal(st: &State, instrument: bool) -> Vec<usize> {
    st.plugins
        .iter()
        .enumerate()
        .filter(|(_, e)| e.format == PluginFormat::Internal && e.instrument == instrument)
        .map(|(i, _)| i)
        .collect()
}

/// A Lua vector of `n` items (`size()` is all the scripts ask of these).
fn sized(lua: &Lua, items: Vec<Value>) -> mlua::Result<Value> {
    let list = lua.create_sequence_from(items)?;
    let host: Table = lua.globals().get("__host")?;
    host.get::<mlua::Function>("vector")?.call(list)
}

fn int(args: &MultiValue, i: usize) -> i64 {
    match args.get(i) {
        Some(Value::Integer(n)) => *n,
        Some(Value::Number(n)) => *n as i64,
        _ => 0,
    }
}

fn string(args: &MultiValue, i: usize) -> String {
    match args.get(i) {
        Some(Value::String(s)) => s.to_string_lossy(),
        _ => String::new(),
    }
}

/// The entry behind a descriptor or module-info argument.
fn entry_arg(lua: &Lua, args: &MultiValue) -> Option<usize> {
    args.iter().find_map(|v| match object_ref(lua, v) {
        Some(ObjRef::PluginEntry(i)) if i != usize::MAX => Some(i),
        _ => None,
    })
}

/// The slot argument's index, if one was given (replace it), else none (insert).
fn slot_arg(lua: &Lua, args: &MultiValue) -> Option<usize> {
    args.iter().find_map(|v| match object_ref(lua, v) {
        Some(ObjRef::Slot(i)) => Some(i),
        _ => None,
    })
}

fn load(lua: &Lua, entry: usize, replace: Option<usize>) {
    state(lua).borrow_mut().control_requests.push(ControlRequest::Load { entry, replace });
}

/// A built-in module's or plug-in entry's id (`ModuleInfo:getId()`).
pub(crate) fn entry_id(lua: &Lua, i: usize) -> i64 {
    if i == usize::MAX {
        return lua.load("return NI.DATA.ModuleInfo.ID_NONE").eval().unwrap_or(47);
    }
    let st = state(lua);
    let st = st.borrow();
    match st.plugins.get(i) {
        Some(e) if e.format == PluginFormat::Internal => ID_BASE + i as i64,
        Some(_) => lua.load("return NI.DATA.ModuleInfo.ID_PLUGINHOST").eval().unwrap_or(38),
        None => -1,
    }
}

/// An entry's name.
pub(crate) fn entry_name(lua: &Lua, i: usize) -> String {
    state(lua).borrow().plugins.get(i).map_or(String::new(), |e| e.name.clone())
}

/// An entry's type (`NI.DATA.ModuleInfo.TYPE_INSTRUMENT` or `TYPE_EFFECT`).
pub(crate) fn entry_type(lua: &Lua, i: usize) -> i64 {
    let instrument = state(lua).borrow().plugins.get(i).is_some_and(|e| e.instrument);
    type_of(lua, instrument)
}

pub(crate) fn type_of(lua: &Lua, instrument: bool) -> i64 {
    let name = if instrument { "TYPE_INSTRUMENT" } else { "TYPE_EFFECT" };
    lua.load(format!("return NI.DATA.ModuleInfo.{name}")).eval().unwrap_or(i64::from(!instrument))
}

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    // Built-in modules.
    let factory = methods(lua, impl_t, "NI.DATA.ModuleFactory")?;
    for (name, instrument) in [("getInstrumentInfos", true), ("getEffectInfos", false)] {
        factory.set(
            name,
            lua.create_function(move |lua, _: MultiValue| {
                let ids = internal(&state(lua).borrow(), instrument);
                let items = ids.into_iter().map(|i| object(lua, ObjRef::PluginEntry(i))).collect::<mlua::Result<Vec<_>>>()?;
                sized(lua, items)
            })?,
        )?;
    }
    for (name, instrument) in [("getInstrumentInfoAt", true), ("getEffectInfoAt", false)] {
        factory.set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                let at = int(&args, 0).max(0) as usize;
                let entry = internal(&state(lua).borrow(), instrument).get(at).copied().unwrap_or(usize::MAX);
                object(lua, ObjRef::PluginEntry(entry))
            })?,
        )?;
    }
    // The plug-in list changed (the browser rebuilds its lists).
    let ws = methods(lua, impl_t, "NI.DATA.WORKSPACE")?;
    ws.set("getPluginPreferencesChanged", lua.create_function(|lua, _: MultiValue| Ok(state(lua).borrow().plugins_changed))?)?;
    // createModuleOrLoadDefault(App, ModuleId, Chain, Slot, Fresh): a built-in module
    // into the slot given, else after the focused one.
    let chain = methods(lua, impl_t, "NI.DATA.ChainAccess")?;
    chain.set(
        "createModuleOrLoadDefault",
        lua.create_function(|lua, args: MultiValue| {
            let id = int(&args, 1);
            let entry = (id - ID_BASE).max(0) as usize;
            let known = state(lua).borrow().plugins.get(entry).is_some_and(|e| e.format == PluginFormat::Internal);
            if id >= ID_BASE && known {
                load(lua, entry, slot_arg(lua, &args));
            }
            Ok(())
        })?,
    )?;

    // External plug-ins.
    let ph = methods(lua, impl_t, "NI.DATA.PluginHelper")?;
    ph.set(
        "getValidPluginFormats",
        lua.create_function(|lua, _: MultiValue| {
            let formats: Vec<PluginFormat> = {
                let st = state(lua);
                let st = st.borrow();
                [PluginFormat::Internal, PluginFormat::AudioUnit, PluginFormat::Vst3, PluginFormat::Vst2]
                    .into_iter()
                    .filter(|f| *f == PluginFormat::Internal || st.plugins.iter().any(|e| e.format == *f))
                    .collect()
            };
            let items = formats.into_iter().map(|f| lua.create_string(f.label()).map(Value::String)).collect::<mlua::Result<Vec<_>>>()?;
            sized(lua, items)
        })?,
    )?;
    // getValidPluginDescriptorsSize(Filter, VendorFilter), getValidPluginDescriptor(Index,
    // Filter, VendorFilter), getValidPluginDescriptorName(App, ThirdParty, Index, Filter,
    // VendorFilter): Maschine's own vendor's (or the third parties').
    ph.set(
        "getValidPluginDescriptorsSize",
        lua.create_function(|lua, args: MultiValue| Ok(external(&state(lua).borrow(), lua, int(&args, 0), int(&args, 1), None).len()))?,
    )?;
    ph.set(
        "getValidPluginDescriptor",
        lua.create_function(|lua, args: MultiValue| {
            let found = external(&state(lua).borrow(), lua, int(&args, 1), int(&args, 2), None).get(int(&args, 0).max(0) as usize).copied();
            found.map_or(Ok(Value::Nil), |i| object(lua, ObjRef::PluginEntry(i)))
        })?,
    )?;
    ph.set(
        "getValidPluginDescriptorName",
        lua.create_function(|lua, args: MultiValue| {
            let found = external(&state(lua).borrow(), lua, int(&args, 3), int(&args, 4), None).get(int(&args, 2).max(0) as usize).copied();
            Ok(found.map_or("(none)".to_string(), |i| entry_name(lua, i)))
        })?,
    )?;
    // getVendors(Filter): the third-party makers, in order.
    ph.set(
        "getVendors",
        lua.create_function(|lua, args: MultiValue| {
            let third: i64 = lua.load("return NI.HOSTING.PluginFilter.SKIP_NOT_THIRD_PARTY_PLUGINS").eval().unwrap_or(256);
            let mut vendors: Vec<String> = {
                let st = state(lua);
                let st = st.borrow();
                external(&st, lua, int(&args, 0), third, None).into_iter().map(|i| st.plugins[i].vendor.clone()).collect()
            };
            vendors.sort_by_key(|v| v.to_lowercase());
            vendors.dedup();
            let items = vendors.into_iter().map(|v| lua.create_string(v).map(Value::String)).collect::<mlua::Result<Vec<_>>>()?;
            sized(lua, items)
        })?,
    )?;
    // …ByVendorSize(Vendor, Filter), …ByVendor(Vendor, Filter, Index), …ByVendorName(…).
    ph.set(
        "getValidPluginDescriptorsByVendorSize",
        lua.create_function(|lua, args: MultiValue| {
            let vendor = string(&args, 0);
            Ok(external(&state(lua).borrow(), lua, int(&args, 1), 0, Some(&vendor)).len())
        })?,
    )?;
    ph.set(
        "getValidPluginDescriptorByVendor",
        lua.create_function(|lua, args: MultiValue| {
            let vendor = string(&args, 0);
            let found = external(&state(lua).borrow(), lua, int(&args, 1), 0, Some(&vendor)).get(int(&args, 2).max(0) as usize).copied();
            found.map_or(Ok(Value::Nil), |i| object(lua, ObjRef::PluginEntry(i)))
        })?,
    )?;
    ph.set(
        "getValidPluginDescriptorByVendorName",
        lua.create_function(|lua, args: MultiValue| {
            let vendor = string(&args, 0);
            let found = external(&state(lua).borrow(), lua, int(&args, 1), 0, Some(&vendor)).get(int(&args, 2).max(0) as usize).copied();
            Ok(found.map_or("(none)".to_string(), |i| entry_name(lua, i)))
        })?,
    )?;
    // createPluginModule(App, Descriptor, Chain, Slot, _, Fresh).
    ph.set(
        "createPluginModule",
        lua.create_function(|lua, args: MultiValue| {
            if let Some(entry) = entry_arg(lua, &args) {
                load(lua, entry, slot_arg(lua, &args));
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

/// After the scripts load: the built-in plug-ins' vendor is DECKARD ("DK"), where
/// Maschine's scripts name their own ("NI", "Native Instruments" on the knob's touch
/// list). External plug-ins keep their makers' names.
pub(crate) const OVERRIDES: &str = r#"
if ModuleHelper and ModuleHelper.getVendorDisplayName then
    local display = ModuleHelper.getVendorDisplayName
    local function builtin(v)
        return v == ModuleHelper.VENDOR_NI and ModuleHelper.getSelectedFormat() == ModuleHelper.INTERNAL
    end
    ModuleHelper.getVendorDisplayName = function(v)
        if builtin(v) then return "DK" end
        return display(v)
    end
    if ModulePageStudio and ModulePageStudio.showPluginLoaderControllers then
        local show = ModulePageStudio.showPluginLoaderControllers
        ModulePageStudio.showPluginLoaderControllers = function(self, ...)
            show(self, ...)
            if builtin(ModuleHelper.getCurrentVendor()) then
                self.Controller.CapacitiveList:assignListToCap(3, { "DK" })
            end
        end
    end
end
"#;
