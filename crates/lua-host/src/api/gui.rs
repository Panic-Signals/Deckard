//! `NI.GUI`: the hardware-display widget tree.
//!
//! `NI.GUI.insert<Kind>(parent, name)` creates a widget; every `setX(v)`/`style(v)` on
//! it is stored as property `X`, and `getX()`/`isX()` read it back. Renderers walk
//! [`crate::state::Widgets`].

use super::{methods, new_object, object_id, state};
use crate::state::{Prop, Widget};
use mlua::{Lua, MultiValue, Table, Value};

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let gui = methods(lua, impl_t, "NI.GUI")?;
    // How a bar lines up its children (`style(NI.GUI.ALIGN_WIDGET_DOWN, ...)`).
    for (i, name) in ["ALIGN_WIDGET_RIGHT", "ALIGN_WIDGET_DOWN", "ALIGN_WIDGET_LEFT", "ALIGN_WIDGET_UP"].iter().enumerate() {
        gui.raw_set(*name, i as i64)?;
    }
    // insertPage(stack, pageRoot, name): put an existing page's root bar on a stack.
    gui.set(
        "insertPage",
        lua.create_function(|lua, args: MultiValue| {
            let (Some(stack), Some(page)) = (args.front().and_then(object_id), args.get(1).and_then(object_id)) else {
                return Ok(());
            };
            let name = args.get(2).and_then(|v| v.to_string().ok()).unwrap_or_default();
            state(lua).borrow_mut().widgets.reparent(page, stack, &name);
            Ok(())
        })?,
    )?;
    let gui_meta = lua.create_table()?;
    gui_meta.set(
        "__index",
        lua.create_function(|lua, (_, key): (Table, String)| {
            let kind = key.strip_prefix("insert").or_else(|| key.strip_prefix("create"));
            let Some(kind) = kind.filter(|k| k.starts_with(char::is_uppercase)).map(str::to_string) else {
                return Ok(Value::Nil);
            };
            let create = key.starts_with("create");
            // insertLabelVector makes a `Vector<Label>`: a bar of item widgets the scripts
            // fill through connectVector (`getBar()`), and a scrollbar beside it.
            let kind = match kind.strip_suffix("Vector") {
                Some(item) if !item.is_empty() => format!("Vector<{item}>"),
                // The style sheet's name for the piano roll's keys.
                _ if kind == "PianorollKeyboard" => "PianorollKeyboardWidget".to_string(),
                // A bar showing a picture (the Control page's plug-in logo) is styled as
                // a bar.
                _ if kind == "PictureBar" => "Bar".to_string(),
                _ => kind,
            };
            // Kinds as the style sheet names them: a browser result
            // (`ResultListItemWidget`, its text in a label) and a row of three product
            // tiles (`ProductIconRow`), alone or in a vector.
            let styled = |k: &str| match k {
                "ResultListItem" => "ResultListItemWidget".to_string(),
                "ProductRow" => "ProductIconRow".to_string(),
                // Settings ▸ MIDI's Link status and phase meter.
                "LinkStatus" => "HWLinkStatus".to_string(),
                "LinkPhaseMeter" => "HWLinkPhaseMeter".to_string(),
                k => k.to_string(),
            };
            let kind = match kind.strip_prefix("Vector<").and_then(|k| k.strip_suffix('>')) {
                Some(item) => format!("Vector<{}>", styled(item)),
                None => styled(&kind),
            };
            let f = lua.create_function(move |lua, args: MultiValue| {
                // insertX(parent, name, ...) / createX(...) without a parent.
                let parent = if create { None } else { args.front().and_then(object_id) };
                // The name: the first string after the parent (some take App first, as
                // insertPianorollKeyboard(parent, App, name)).
                let name = args
                    .iter()
                    .skip(if create { 0 } else { 1 })
                    .find_map(|v| match v {
                        Value::String(s) => Some(s.to_string_lossy()),
                        _ => None,
                    })
                    .unwrap_or_default();
                let st = state(lua);
                let mut st = st.borrow_mut();
                let id = st.widgets.insert(parent, &kind, &name);
                // A plug-in chain's item (the Control page's tiles): a header with the
                // type icon and the name, and the framed picture under it.
                if kind == "PluginItem" {
                    st.widgets.set(id, "Align", Prop::Int(1));
                    let header = st.widgets.insert(Some(id), "Bar", "m_header");
                    let kind_icon = st.widgets.insert(Some(header), "Label", "m_type");
                    let name = st.widgets.insert(Some(header), "Label", "m_name");
                    let frame = st.widgets.insert(Some(id), "Label", "m_pictureFrame");
                    let picture = st.widgets.insert(Some(frame), "Bar", "m_picture");
                    for (k, w) in
                        [("Header", header), ("TypeLabel", kind_icon), ("NameLabel", name), ("PictureFrame", frame), ("Picture", picture)]
                    {
                        st.widgets.set(id, k, Prop::Widget(w));
                    }
                }
                super::sync::link_widget(&mut st, &kind, id);
                if kind == "ResultListItemWidget" {
                    let label = st.widgets.insert(Some(id), "Label", "m_pTextLabel");
                    st.widgets.set(id, "TextLabel", Prop::Widget(label));
                }
                // Three columns (`getBar(i)`), each the product's picture (`getIcon(i)`)
                // over its name (`getLabel(i)`).
                if kind == "ProductIconRow" {
                    for i in 0..3 {
                        let bar = st.widgets.insert(Some(id), "Bar", "");
                        st.widgets.set(bar, "Class", Prop::Str("ProductBar".into()));
                        st.widgets.set(bar, "Align", Prop::Int(1));
                        let icon = st.widgets.insert(Some(bar), "Bar", "");
                        st.widgets.set(icon, "Class", Prop::Str("ProductIcon".into()));
                        let label = st.widgets.insert(Some(bar), "Label", "");
                        st.widgets.set(label, "Class", Prop::Str("ProductLabel".into()));
                        for (k, w) in [("Bar", bar), ("Icon", icon), ("Label", label)] {
                            st.widgets.set(id, &format!("{k}{i}"), Prop::Widget(w));
                        }
                    }
                }
                if let Some(item) = kind.strip_prefix("Vector<").and_then(|k| k.strip_suffix('>')) {
                    st.widgets.set(id, "ItemKind", Prop::Str(item.to_string()));
                }
                if kind.starts_with("Vector<") {
                    let bar = st.widgets.insert(Some(id), "Bar", "widgets");
                    st.widgets.set(bar, "Align", Prop::Int(1));
                    let scrollbar = st.widgets.insert(Some(id), "Scrollbar", "");
                    st.widgets.set(id, "Bar", Prop::Widget(bar));
                    st.widgets.set(id, "Scrollbar", Prop::Widget(scrollbar));
                    st.widgets.set(id, "Flex", Prop::Widget(bar));
                }
                drop(st);
                new_object(lua, "Widget", id)
            })?;
            Ok(Value::Function(f))
        })?,
    )?;
    gui.set_metatable(Some(gui_meta))?;
    // connectVector(vector, size(), setup(item), load(item, index)): the vector shows
    // items offset .. offset + rows of size(), each a widget made once (setup) and
    // filled (load) whenever the scripts change.
    lua.load(VECTORS).exec()?;
    // __is_shown(widget, screen): on the active screen (root bar id `screen`) or the
    // overlay, with nothing above it hidden.
    lua.globals().set(
        "__is_shown",
        lua.create_function(|lua, (w, screen): (Value, Option<usize>)| {
            let Some(mut at) = object_id(&w) else { return Ok(false) };
            let st = state(lua);
            let st = st.borrow();
            let overlay = st.hw.display_roots.get("OverlayRoot").copied();
            loop {
                let Some(x) = st.widgets.get(at) else { return Ok(false) };
                if matches!(x.props.get("Visible"), Some(Prop::Bool(false))) || matches!(x.props.get("Active"), Some(Prop::Bool(false))) {
                    return Ok(false);
                }
                if Some(at) == screen || Some(at) == overlay {
                    return Ok(true);
                }
                match x.parent {
                    Some(p) => at = p,
                    None => return Ok(false),
                }
            }
        })?,
    )?;
    gui.set("connectVector", lua.globals().get::<mlua::Function>("__connect_vector")?)?;
    // insertIdeaSpace(parent, App, name): Maschine's native Ideas view widget, built from
    // the parts its style sheet (`Shared/HW/MaschineStudio/IdeaSpace.txt`) styles.
    lua.globals().set(
        "__group_progress",
        lua.create_function(|lua, group: usize| {
            Ok(state(lua).borrow().group_progress.get(&group).map_or((None, None), |&(bars, reached)| (Some(bars), Some(reached))))
        })?,
    )?;
    lua.load(IDEA_SPACE).exec()?;
    // NI.UTILS.PictureManager.getPictureOrLoadFromDisk(path, internal): a picture the
    // style sheet's look has (NI's, from the install, or DECKARD's own), with its size.
    let utils = methods(lua, impl_t, "NI.UTILS.PictureManager")?;
    utils.set(
        "getPictureOrLoadFromDisk",
        lua.create_function(|lua, (path, _): (Option<String>, Option<bool>)| {
            let Some(path) = path.filter(|p| !p.is_empty() && p != "no") else { return Ok(Value::Nil) };
            let Some(look) = lua.app_data_ref::<std::rc::Rc<deckard_nwl::draw::Look>>().map(|l| l.clone()) else { return Ok(Value::Nil) };
            let Some((w, h)) = look.picture_size(&path) else { return Ok(Value::Nil) };
            let t = lua.create_table()?;
            t.set("__picture", path)?;
            t.set("getWidth", lua.create_function(move |_, _: MultiValue| Ok(w))?)?;
            t.set("getHeight", lua.create_function(move |_, _: MultiValue| Ok(h))?)?;
            Ok(Value::Table(t))
        })?,
    )?;
    gui.set("insertIdeaSpace", lua.globals().get::<mlua::Function>("__insert_idea_space")?)?;

    let w = methods(lua, impl_t, "Widget")?;
    let w_meta = lua.create_table()?;
    w_meta.set(
        "__index",
        lua.create_function(|lua, (_, key): (Table, String)| {
            let f = if key == "style" {
                style(lua)?
            } else if let Some(f) = layout_method(lua, &key)? {
                f
            } else if key == "setAttribute" {
                // setAttribute(name, value): matched by style sheet selectors `[name=value]`.
                lua.create_function(|lua, (this, name, value): (Table, String, Value)| {
                    if let Some(id) = object_id(&Value::Table(this.clone())) {
                        let v = match &value {
                            Value::String(s) => s.to_string_lossy(),
                            Value::Boolean(b) => b.to_string(),
                            Value::Nil => String::new(),
                            v => v.to_string().unwrap_or_default(),
                        };
                        state(lua).borrow_mut().widgets.set(id, &format!("attr:{name}"), Prop::Str(v));
                    }
                    Ok(this)
                })?
            } else if key == "resetPeak" {
                // A Mixer meter's held peak, cleared (the strip shows another channel).
                lua.create_function(|lua, this: Table| {
                    if let Some(id) = object_id(&Value::Table(this.clone())) {
                        let st = state(lua);
                        let mut st = st.borrow_mut();
                        st.widgets.set(id, "Peak", Prop::Num(0.0));
                        st.widgets.set(id, "PeakHeld", Prop::Num(0.0));
                    }
                    Ok(this)
                })?
            } else if key == "getSlotIcon" {
                // A Mixer strip's plug-in icons: one part per slot (`SlotIcon<i>`).
                lua.create_function(|lua, (this, i): (Table, Option<i64>)| {
                    getter(lua, format!("SlotIcon{}", i.unwrap_or(0)), false)?.call::<Value>(this)
                })?
            } else if matches!(key.as_str(), "getBar" | "getLabel" | "getIcon") {
                // A product row's column parts by index; else the widget's own part.
                let part = key[3..].to_string();
                lua.create_function(move |lua, (this, i): (Table, Option<i64>)| {
                    let indexed = i.map(|i| format!("{part}{i}"));
                    let has = indexed.as_ref().is_some_and(|k| {
                        object_id(&Value::Table(this.clone()))
                            .and_then(|id| state(lua).borrow().widgets.get(id).map(|w| w.props.contains_key(k)))
                            .unwrap_or(false)
                    });
                    let prop = if has { indexed.unwrap_or_default() } else { part.clone() };
                    getter(lua, prop, false)?.call::<Value>(this)
                })?
            } else if key == "getAt" || key == "getWidgetAt" {
                lua.globals().get::<mlua::Function>("__vector_at")?
            } else if key == "getAttribute" {
                lua.create_function(|lua, (this, name): (Table, String)| {
                    let st = state(lua);
                    let st = st.borrow();
                    let w = object_id(&Value::Table(this)).and_then(|id| st.widgets.get(id));
                    Ok(match w.and_then(|w| w.props.get(&format!("attr:{name}"))) {
                        Some(Prop::Str(s)) => Some(s.clone()),
                        _ => None,
                    })
                })?
            } else if let Some(p) = key.strip_prefix("set").filter(|p| p.starts_with(char::is_uppercase)) {
                setter(lua, p.to_string())?
            } else if let Some(p) = key.strip_prefix("get").or_else(|| key.strip_prefix("is")) {
                getter(lua, p.to_string(), key.starts_with("is"))?
            } else {
                return Ok(Value::Nil);
            };
            Ok(Value::Function(f))
        })?,
    )?;
    w.set_metatable(Some(w_meta))?;
    Ok(())
}

const VECTORS: &str = r#"
__vectors = {}
-- getAt/getWidgetAt(index): a vector's item for its entry `index` (from 0), if it's
-- shown.
function __vector_at(vector, index)
    local id = rawget(vector, "__id")
    for _, v in ipairs(__vectors) do
        if rawget(v.vector, "__id") == id then
            local offset = v.vector:getItemOffset() or 0
            return v.items[index - offset + 1]
        end
    end
end
function __connect_vector(vector, size, setup, load)
    __vectors[#__vectors + 1] = { vector = vector, size = size, setup = setup, load = load, items = {} }
end
function __realize_vectors(screen)
    -- One list failing leaves the others shown; its error is reported after.
    local failed
    for _, v in ipairs(__vectors) do
        if __is_shown(v.vector, screen) then
            local ok, e = pcall(__realize_vector, v)
            failed = failed or (not ok and e) or nil
        end
    end
    if failed then error(failed, 0) end
end
function __realize_vector(v)
    do
        local n = v.size() or 0
        local rows = v.vector:getNumFullyVisibleItems()
        local offset = v.vector:getItemOffset() or 0
        -- setFocusItem(i): scrolled so that item i is shown.
        local focus = v.vector:getFocusItem()
        if type(focus) == "number" then
            if focus < offset then offset = focus end
            if focus >= offset + rows then offset = focus - rows + 1 end
        end
        offset = math.max(0, math.min(offset, n - rows))
        v.vector:setItemOffset(offset)
        v.vector:setScrollTotal(n)
        v.vector:setScrollRows(rows)
        local bar = v.vector:getBar()
        for k = 1, math.max(rows, #v.items) do
            local item, index = v.items[k], offset + k - 1
            if k <= rows and index < n then
                if not item then
                    -- Items of the vector's kind (labels, buttons, plug-in tiles…).
                    local kind = v.vector:getItemKind() or "Label"
                    item = NI.GUI["insert" .. kind](bar, "")
                    v.setup(item)
                    v.items[k] = item
                end
                item:setActive(true)
                v.load(item, index)
            elseif item then
                item:setActive(false)
            end
        end
    end
end
"#;

/// The Ideas view on the displays (`NI.GUI.insertIdeaSpace`), one column per group
/// (eight from `setGroupOffset`):
/// - the scenes (eight from `setSceneOffset`, then "+" to add one), over the screen
///   buttons that pick them, the focused scene selected;
/// - each group's patterns: those in the focused scene `Assigned`, the focused one
///   `Focused`, then an `Empty` cell;
/// - the groups, the focused one `Focused`, each with its pattern's progress
///   (`Progress`, "bars/played", which the renderer draws per `max-bars`).
const IDEA_SPACE: &str = r#"
__idea_spaces = {}
function __insert_idea_space(parent, app, name)
    local w = NI.GUI.insertBar(parent, name)
    w:setAlign(NI.GUI.ALIGN_WIDGET_DOWN)
    __idea_spaces[#__idea_spaces + 1] = { w = w }
    return w
end
function __realize_idea_spaces(screen)
    local failed
    for _, v in ipairs(__idea_spaces) do
        if __is_shown(v.w, screen) then
            local ok, e = pcall(__realize_idea_space, v)
            failed = failed or (not ok and e) or nil
        end
    end
    if failed then error(failed, 0) end
end
local function vector(parent, kind, name, align)
    local v = NI.GUI["insert" .. kind .. "Vector"](parent, name)
    v:getBar():setAlign(align)
    return v
end
-- Item k of a vector's bar ({ w = widget, ... }), made on first use; items past n
-- hidden.
local function items(v, list, n, make)
    for k = 1, math.max(n, #list) do
        if k <= n and not list[k] then list[k] = make(v:getBar()) end
        if list[k] then list[k].w:setActive(k <= n) end
    end
    v:setScrollTotal(n)
    v:setScrollRows(n)
end
local function build(v)
    local w = v.w
    v.sceneArea = NI.GUI.insertBar(w, "m_sceneArea")
    v.sceneArea:setAlign(NI.GUI.ALIGN_WIDGET_DOWN)
    v.sceneList = vector(v.sceneArea, "Button", "m_sceneList", NI.GUI.ALIGN_WIDGET_RIGHT)
    NI.GUI.insertLabel(v.sceneArea, "m_sceneFocusLine")
    v.patternArea = NI.GUI.insertBar(w, "m_patternArea")
    v.patternColumns = vector(v.patternArea, "Bar", "m_patternColumns", NI.GUI.ALIGN_WIDGET_RIGHT)
    v.patternArea:setFlex(v.patternColumns)
    v.groupArea = NI.GUI.insertBar(w, "m_groupArea")
    v.groupList = vector(v.groupArea, "Bar", "m_groupList", NI.GUI.ALIGN_WIDGET_RIGHT)
    w:setFlex(v.patternArea)
    v.scenes, v.columns, v.groups = {}, {}, {}
end
function __realize_idea_space(v)
    if not v.sceneArea then build(v) end
    local song = NI.DATA.StateHelper.getFocusSong(App)
    if not song then return end
    local scenes, groups = song:getScenes(), song:getGroups()
    local sceneOffset = tonumber(v.w:getSceneOffset()) or 0
    local groupOffset = tonumber(v.w:getGroupOffset()) or 0
    local focusScene = scenes:getFocusObject()
    local focusGroup = NI.DATA.StateHelper.getFocusGroupIndex(App)
    local focusPattern = NI.DATA.StateHelper.getFocusEventPattern(App)

    -- Scenes, then "+" after the last.
    local nScenes = math.max(0, math.min(8, scenes:size() + 1 - sceneOffset))
    items(v.sceneList, v.scenes, nScenes, function(bar) return { w = NI.GUI.insertButton(bar, "") } end)
    for k = 1, nScenes do
        local b, scene = v.scenes[k].w, scenes:at(sceneOffset + k - 1)
        b:setAttribute("IsAddButton", scene == nil and "true" or "false")
        b:setText(scene and scene:getNameParameter():getValue() or "")
        b:setSelected(scene ~= nil and scene == focusScene)
        b:setPaletteColorIndex(scene and scene:getColorParameter():getValue() or -1)
    end

    -- Each group's patterns, and the group below.
    local nGroups = math.max(0, math.min(8, groups:size() - groupOffset))
    items(v.patternColumns, v.columns, nGroups, function(bar)
        local column = NI.GUI.insertBar(bar, "")
        local cells = vector(column, "Button", "m_groupPatterns", NI.GUI.ALIGN_WIDGET_DOWN)
        column:setFlex(cells)
        return { w = column, vector = cells, cells = {} }
    end)
    items(v.groupList, v.groups, nGroups, function(bar)
        local item = NI.GUI.insertBar(bar, "")
        item:setAlign(NI.GUI.ALIGN_WIDGET_DOWN)
        local label = NI.GUI.insertButton(item, "m_label")
        local name = NI.GUI.insertLabel(item, "m_name")
        local progress = NI.GUI.insertBar(item, "m_progress")
        return { w = item, label = label, name = name, progress = progress }
    end)
    for k = 1, nGroups do
        local index = groupOffset + k - 1
        local group = groups:at(index)
        local focused = index == focusGroup
        local muted = group:getMuteParameter():getValue() and "true" or "false"
        local column, item = v.columns[k], v.groups[k]
        local color = group:getColorParameter():getValue() + 1
        for _, x in ipairs({ column.w, item.w }) do
            x:setAttribute("Focused", focused and "true" or "false")
            x:setAttribute("Muted", muted)
            x:setPaletteColorIndex(color)
        end
        item.w:setAttribute("Selected", focused and "true" or "false")
        item.label:setText(NI.DATA.Group.getLabel(index))
        item.name:setText(group:getDisplayName())
        -- "bars/played" through its pattern in the scene as it plays (played in bars,
        -- to a hundredth); empty without one.
        local bars, played = __group_progress(index)
        item.progress:setAttribute("Progress", bars and string.format("%d/%.2f", bars, played) or "")
        -- The group's colour for the parts styled from its palette (the label's text, the
        -- progress bar).
        for _, x in ipairs({ item.label, item.name, item.progress }) do
            x:setPaletteColorIndex(color)
        end

        -- Seven cells: the patterns around the one in the scene, then an empty one.
        local patterns = group:getPatterns()
        local assigned = focusScene and NI.DATA.SceneAccess.getPattern(focusScene, group)
        local n = patterns:size() + 1
        local rows = 7
        local first = 0
        if assigned then
            first = math.max(0, math.min(patterns:calcIndex(assigned) - rows + 2, n - rows))
        end
        items(column.vector, column.cells, math.min(rows, n - first), function(bar) return { w = NI.GUI.insertButton(bar, "") } end)
        for r = 1, math.min(rows, n - first) do
            local cell, pattern = column.cells[r].w, patterns:at(first + r - 1)
            cell:setAttribute("Empty", pattern and "false" or "true")
            cell:setAttribute("Assigned", (pattern ~= nil and pattern == assigned) and "true" or "false")
            cell:setAttribute("Focused", (pattern ~= nil and pattern == focusPattern and focused) and "true" or "false")
            cell:setText(pattern and pattern:getNameParameter():getValue() or "")
            cell:setPaletteColorIndex((pattern and pattern:getColorParameter():getValue() or group:getColorParameter():getValue()) + 1)
        end
    end
end
"#;

/// Widget methods that need the style sheet or the layout: positions, sizes and style
/// properties as NI's widgets report them.
fn layout_method(lua: &Lua, key: &str) -> mlua::Result<Option<mlua::Function>> {
    let id_of = |this: &Table| object_id(&Value::Table(this.clone()));
    Ok(Some(match key {
        "setPos" => lua.create_function(move |lua, (this, x, y): (Table, f64, f64)| {
            if let Some(id) = id_of(&this) {
                let st = state(lua);
                let mut st = st.borrow_mut();
                st.widgets.set(id, "PosX", Prop::Int(x.round() as i64));
                st.widgets.set(id, "PosY", Prop::Int(y.round() as i64));
            }
            Ok(this)
        })?,
        // A style sheet number, e.g. getIntProperty("max-items", 5).
        "getIntProperty" => lua.create_function(move |lua, (this, name, default): (Table, String, Option<i64>)| {
            Ok(id_of(&this).and_then(|id| style_of(lua, id)).and_then(|s| s.num(&name)).map_or(default.unwrap_or(0), i64::from))
        })?,
        "getStringWidth" => lua.create_function(move |lua, (this, text): (Table, Option<String>)| {
            let Some(look) = lua.app_data_ref::<std::rc::Rc<deckard_nwl::draw::Look>>().map(|l| l.clone()) else { return Ok(0) };
            Ok(id_of(&this).and_then(|id| style_of(lua, id)).map_or(0, |s| look.string_width(&s, text.as_deref().unwrap_or(""))))
        })?,
        // getOuterMargin():getTop() …, getMargin():getHeight() (inner, top + bottom) …
        "getOuterMargin" | "getMargin" => {
            let outer = key == "getOuterMargin";
            lua.create_function(move |lua, this: Table| {
                let s = id_of(&this).and_then(|id| style_of(lua, id)).unwrap_or_default();
                let side = |n: &str| if outer { s.outer(n) } else { s.margin(n) };
                let (t, b, l, r) = (side("top"), side("bottom"), side("left"), side("right"));
                let m = lua.create_table()?;
                for (get, v) in
                    [("getTop", t), ("getBottom", b), ("getLeft", l), ("getRight", r), ("getHeight", t + b), ("getWidth", l + r)]
                {
                    m.set(get, lua.create_function(move |_, _: MultiValue| Ok(v))?)?;
                }
                Ok(m)
            })?
        }
        // Set by the scripts, else the style sheet's.
        "getWidth" | "getHeight" => {
            let (prop, css) = if key == "getWidth" { ("Width", "width") } else { ("Height", "height") };
            lua.create_function(move |lua, this: Table| {
                let Some(id) = id_of(&this) else { return Ok(0) };
                let set = state(lua).borrow().widgets.get(id).and_then(|w| w.props.get(prop).cloned());
                Ok(match set {
                    Some(Prop::Int(i)) => i,
                    Some(Prop::Num(n)) => n.round() as i64,
                    _ => style_of(lua, id).and_then(|s| s.num(css)).map_or(0, i64::from),
                })
            })?
        }
        // How many items a vector shows: its style's max-items, else as many as fit its
        // height (once it has an item to measure), else 5.
        "getNumFullyVisibleItems" => lua.create_function(move |lua, this: Table| {
            let Some(id) = id_of(&this) else { return Ok(5) };
            let Some(s) = style_of(lua, id) else {
                // Without the style sheet: a row of the displays' eight, else five.
                let st = state(lua);
                let st = st.borrow();
                let bar = st.widgets.get(id).and_then(|w| match w.props.get("Bar") {
                    Some(Prop::Widget(b)) => st.widgets.get(*b),
                    _ => None,
                });
                return Ok(if bar.is_some_and(|b| matches!(b.props.get("Align"), Some(Prop::Int(0)))) { 8 } else { 5 });
            };
            if let Some(n) = s.num("max-items") {
                return Ok(i64::from(n));
            }
            let item = {
                let st = state(lua);
                let st = st.borrow();
                let bar = st.widgets.get(id).and_then(|w| match w.props.get("Bar") {
                    Some(Prop::Widget(b)) => Some(*b),
                    _ => None,
                });
                bar.and_then(|b| st.widgets.get(b)).and_then(|b| b.children.first().copied())
            };
            // Side by side (`style(true, …)`): as many as fit across (the displays' 960
            // when the list doesn't say).
            let across = {
                let st = state(lua);
                let st = st.borrow();
                let bar = st.widgets.get(id).and_then(|w| match w.props.get("Bar") {
                    Some(Prop::Widget(b)) => Some(*b),
                    _ => None,
                });
                bar.and_then(|b| st.widgets.get(b)).is_some_and(|b| matches!(b.props.get("Align"), Some(Prop::Int(0))))
            };
            if across {
                let room = s.num("width").unwrap_or(960) - s.margin("left") - s.margin("right");
                let each =
                    item.and_then(|i| style_of(lua, i)).map_or(0, |i| i.num("width").unwrap_or(0) + i.outer("left") + i.outer("right"));
                return Ok(if room > 0 && each > 0 { i64::from(room / each) } else { 8 });
            }
            let room = s.num("height").unwrap_or(0) - s.margin("top") - s.margin("bottom");
            let each = item.and_then(|i| style_of(lua, i)).map_or(0, |i| i.num("height").unwrap_or(0) + i.outer("top") + i.outer("bottom"));
            Ok(if room > 0 && each > 0 { i64::from(room / each) } else { 5 })
        })?,
        "getItemOffset" => lua.create_function(move |lua, this: Table| {
            let Some(id) = id_of(&this) else { return Ok(0) };
            Ok(match state(lua).borrow().widgets.get(id).and_then(|w| w.props.get("ItemOffset").cloned()) {
                Some(Prop::Int(i)) => i,
                Some(Prop::Num(n)) => n as i64,
                _ => 0,
            })
        })?,
        "forceAlign" => lua.create_function(|_, this: Table| Ok(this))?,
        // getStringProperty(name, default): a style sheet value as text (a picture's
        // path). DECKARD's own plug-in pictures stand in for "image-<plug-in>": its
        // thumbnails in the chain, its logos elsewhere.
        "getStringProperty" => lua.create_function(move |lua, (this, name, default): (Table, String, Option<String>)| {
            let default = default.unwrap_or_default();
            let Some(id) = id_of(&this) else { return Ok(default) };
            if let Some(key) = name.strip_prefix("image-") {
                let in_chain = state(lua).borrow().widgets.get(id).is_some_and(|w| w.kind.starts_with("Vector<PluginItem>"));
                let own = crate::pattern_view::plugin_picture(key, in_chain);
                let look = lua.app_data_ref::<std::rc::Rc<deckard_nwl::draw::Look>>().map(|l| l.clone());
                if look.is_some_and(|l| l.picture_size(&own).is_some()) {
                    return Ok(own);
                }
            }
            use deckard_nwl::sheet::Value as V;
            Ok(match style_of(lua, id).as_ref().and_then(|s| s.get(&name)) {
                Some(V::Url(u)) => u.clone(),
                Some(V::Ident(i)) => i.clone(),
                Some(V::None) => "no".into(),
                _ => default,
            })
        })?,
        // getColorProperty(name, default) / getColorFromPalette(prop, index, default): a
        // colour as 0xRRGGBBAA, for setBGColor.
        "getColorProperty" => lua.create_function(move |lua, (this, name, default): (Table, String, Option<i64>)| {
            use deckard_nwl::sheet::Value as V;
            let found = id_of(&this).and_then(|id| style_of(lua, id)).and_then(|s| match s.get(&name) {
                Some(V::Color(c)) => Some(u32::from_be_bytes(*c) as i64),
                _ => None,
            });
            Ok(found.unwrap_or(default.unwrap_or(0)))
        })?,
        "getColorFromPalette" => lua.create_function(move |lua, (this, prop, index, default): (Table, String, i64, Option<i64>)| {
            let palette = id_of(&this).and_then(|id| style_of(lua, id)).and_then(|s| s.num(&format!("{prop}-palette")));
            let look = lua.app_data_ref::<std::rc::Rc<deckard_nwl::draw::Look>>().map(|l| l.clone());
            let c = match (look, palette) {
                (Some(l), Some(p)) => l.sheet.palette(p as u32, index.max(0) as u32).map(|c| u32::from_be_bytes(c) as i64),
                _ => None,
            };
            Ok(c.unwrap_or(default.unwrap_or(0)))
        })?,
        // setPicture(picture) / resetPicture(): a picture from the PictureManager.
        "setPicture" => lua.create_function(move |lua, (this, pic): (Table, Value)| {
            if let (Some(id), Value::Table(p)) = (id_of(&this), pic) {
                let path: Option<String> = p.get("__picture").ok();
                state(lua).borrow_mut().widgets.set(id, "PictureUrl", path.map_or(Prop::Nil, Prop::Str));
            }
            Ok(this)
        })?,
        "resetPicture" => lua.create_function(move |lua, this: Table| {
            if let Some(id) = id_of(&this) {
                state(lua).borrow_mut().widgets.set(id, "PictureUrl", Prop::Nil);
            }
            Ok(this)
        })?,
        "resetBGColor" => lua.create_function(move |lua, this: Table| {
            if let Some(id) = id_of(&this) {
                state(lua).borrow_mut().widgets.set(id, "BGColor", Prop::Nil);
            }
            Ok(this)
        })?,
        // insertChild(widget, name): move a widget made elsewhere under this one (the
        // pattern editors, shared by the pages).
        "insertChild" => lua.create_function(move |lua, (this, child, name): (Table, Value, Option<String>)| {
            if let (Some(id), Some(c)) = (id_of(&this), object_id(&child)) {
                state(lua).borrow_mut().widgets.reparent(c, id, name.as_deref().unwrap_or(""));
            }
            Ok(this)
        })?,
        _ => return Ok(None),
    }))
}

/// The style widget `id` gets from the style sheet, if the host has one.
fn style_of(lua: &Lua, id: usize) -> Option<deckard_nwl::draw::Style> {
    let look = lua.app_data_ref::<std::rc::Rc<deckard_nwl::draw::Look>>()?.clone();
    let chain = crate::screen::chain(&state(lua).borrow().widgets, id);
    Some(look.style_of(&chain))
}

/// `style([align | text | flag, ...,] class)`: stores the first argument as `style`
/// (labels' and buttons' text when there's a class after it), the last string as
/// `Class` (the style sheet class) and a number first as `Align` (`ALIGN_WIDGET_*`).
fn style(lua: &Lua) -> mlua::Result<mlua::Function> {
    lua.create_function(move |lua, (this, args): (Table, MultiValue)| {
        if let Some(id) = object_id(&Value::Table(this.clone())) {
            let st = state(lua);
            let mut st = st.borrow_mut();
            st.widgets.set(id, "style", Prop::from_lua(args.front().unwrap_or(&Value::Nil)));
            // style(class), style(text, class), style(align | flag, ..., class).
            let class = match args.front() {
                Some(Value::String(s)) if args.len() == 1 => Some(s.to_string_lossy()),
                Some(Value::String(_)) => match args.get(1) {
                    Some(Value::String(s)) => Some(s.to_string_lossy()),
                    _ => None,
                },
                _ => args.iter().rev().find_map(|v| match v {
                    Value::String(s) => Some(s.to_string_lossy()),
                    _ => None,
                }),
            };
            if let Some(c) = class {
                st.widgets.set(id, "Class", Prop::Str(c));
            }
            if let Some(Value::Integer(_) | Value::Number(_)) = args.front() {
                st.widgets.set(id, "Align", Prop::from_lua(&args[0]));
            }
            // A vector's style(horizontal, class): its items side by side.
            let horizontal = matches!(args.front(), Some(Value::Boolean(true)));
            let bar = st.widgets.get(id).filter(|w| w.kind.starts_with("Vector<")).and_then(|w| match w.props.get("Bar") {
                Some(Prop::Widget(b)) => Some(*b),
                _ => None,
            });
            if let Some(b) = bar {
                st.widgets.set(b, "Align", Prop::Int(if horizontal { 0 } else { 1 }));
            }
        }
        Ok(this)
    })
}

/// Stores the first argument; returns the widget so `insertLabel(...):style(...)` chains.
fn setter(lua: &Lua, prop: String) -> mlua::Result<mlua::Function> {
    lua.create_function(move |lua, (this, args): (Table, MultiValue)| {
        if let Some(id) = object_id(&Value::Table(this.clone())) {
            let v = Prop::from_lua(args.front().unwrap_or(&Value::Nil));
            let st = state(lua);
            let mut st = st.borrow_mut();
            if prop == "Value" {
                hold_peak(&mut st.widgets, id, &v);
            }
            st.widgets.set(id, &prop, v);
            // setLevels(left, right): the record meter's two channels.
            if prop == "Levels" {
                st.widgets.set(id, "Levels2", Prop::from_lua(args.get(1).unwrap_or(&Value::Nil)));
            }
        }
        Ok(this)
    })
}

/// A Mixer strip's meter (its `getLeftMeter()`/`getRightMeter()`) keeps its peak as the
/// script meters it, every controller tick: held [`PEAK_HOLD`] ticks, then falling
/// [`PEAK_FALL`] a tick (as the app's Mixer: ~1.5 s, then 40 dB a second).
fn hold_peak(w: &mut crate::state::Widgets, id: usize, v: &Prop) {
    if !w.get(id).is_some_and(|x| x.kind == "LeftMeter" || x.kind == "RightMeter") {
        return;
    }
    let num = |p: Option<&Prop>| match p {
        Some(Prop::Num(n)) => *n,
        Some(Prop::Int(i)) => *i as f64,
        _ => 0.0,
    };
    let v = num(Some(v));
    let (peak, held) = w.get(id).map_or((0.0, 0.0), |x| (num(x.props.get("Peak")), num(x.props.get("PeakHeld"))));
    let (peak, held) = if v >= peak {
        (v, PEAK_HOLD)
    } else if held > 0.0 {
        (peak, held - 1.0)
    } else {
        ((peak - PEAK_FALL).max(v), 0.0)
    };
    w.set(id, "Peak", Prop::Num(peak));
    w.set(id, "PeakHeld", Prop::Num(held));
}

/// Ticks a meter's peak is held, and how far it then falls a tick (of the meter's 1000
/// over 70 dB: about 2 dB).
const PEAK_HOLD: f64 = 30.0;
const PEAK_FALL: f64 = 29.0;

fn getter(lua: &Lua, prop: String, boolean: bool) -> mlua::Result<mlua::Function> {
    lua.create_function(move |lua, this: Table| {
        let Some(id) = object_id(&Value::Table(this)) else { return Ok(Value::Nil) };
        let st = state(lua);
        let found = st.borrow().widgets.get(id).and_then(|w: &Widget| w.props.get(&prop).cloned());
        match found {
            Some(p) => p.into_lua(lua),
            None if boolean => Ok(Value::Boolean(prop == "Visible" || prop == "Enabled")),
            // Sub-widgets such as getScrollbar()/getHeader() are created on demand.
            None => {
                let child = st.borrow_mut().widgets.insert(Some(id), &prop, "");
                st.borrow_mut().widgets.set(id, &prop, Prop::Widget(child));
                new_object(lua, "Widget", child)
            }
        }
    })
}
