//! The Settings page's Audio tab (`NSAHelper.lua`): `App:getNSA()`, the audio
//! interface as the app has it ([`AudioView`]): the driver (one), the devices, the
//! sample rate (the device's own), the buffer size, and the routing of DECKARD's 32
//! inputs and outputs. Changes go to the app as [`AudioEdit`]s. The MIDI parts of NSA
//! (devices, sync) aren't here.

use super::data::set_fn;
use super::{methods, new_object, state};
use crate::state::AudioEdit;
use mlua::{Lua, MultiValue, Table, Value};

/// The name of an interface channel not connected.
pub const OFF: &str = "Off";

fn int(args: &MultiValue, i: usize) -> Option<i64> {
    match args.get(i) {
        Some(Value::Integer(n)) => Some(*n),
        Some(Value::Number(n)) => Some(*n as i64),
        _ => None,
    }
}

fn input(args: &MultiValue, i: usize) -> bool {
    matches!(args.get(i), Some(Value::Boolean(true)))
}

fn text(args: &MultiValue, i: usize) -> Option<String> {
    match args.get(i) {
        Some(Value::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// The channels offered for routing: "Off", then the interface's.
fn channels(lua: &Lua, input: bool) -> Vec<String> {
    let st = state(lua);
    let st = st.borrow();
    let names = if input { &st.audio.inputs } else { &st.audio.outputs };
    std::iter::once(OFF.to_string()).chain(names.iter().cloned()).collect()
}

pub fn install(lua: &Lua, impl_t: &Table) -> mlua::Result<()> {
    let app = methods(lua, impl_t, "App")?;
    set_fn(lua, &app, "getNSA", |lua, _: MultiValue| new_object(lua, "NSA", 0))?;
    let nsa = methods(lua, impl_t, "NSA")?;
    let view = |lua: &Lua| state(lua).borrow().audio.clone();
    set_fn(lua, &nsa, "getDriverCount", move |lua, _: MultiValue| Ok(i64::from(!view(lua).driver.is_empty())))?;
    set_fn(lua, &nsa, "getDriverName", move |lua, _: MultiValue| Ok(view(lua).driver))?;
    set_fn(lua, &nsa, "getCurrentDriver", move |lua, _: MultiValue| Ok(view(lua).driver))?;
    set_fn(lua, &nsa, "setDriver", |_, _: MultiValue| Ok(()))?;
    set_fn(lua, &nsa, "getDeviceCount", move |lua, _: MultiValue| Ok(view(lua).devices.len() as i64))?;
    set_fn(lua, &nsa, "getDeviceName", move |lua, args: MultiValue| {
        let i = int(&args, 1).unwrap_or(-1);
        Ok(usize::try_from(i).ok().and_then(|i| view(lua).devices.get(i).cloned()).unwrap_or_default())
    })?;
    set_fn(lua, &nsa, "getCurrentDevice", move |lua, _: MultiValue| Ok(view(lua).device))?;
    set_fn(lua, &nsa, "setDevice", |lua, args: MultiValue| {
        if let Some(d) = text(&args, 1) {
            let st = state(lua);
            let mut st = st.borrow_mut();
            st.audio.device = d.clone();
            st.audio_edits.push(AudioEdit::Device(d));
        }
        Ok(())
    })?;
    // The device's own rate (DECKARD plays at the rate the device runs at).
    set_fn(lua, &nsa, "getSamplerateCount", |_, _: MultiValue| Ok(1))?;
    set_fn(lua, &nsa, "getSamplerateValue", move |lua, _: MultiValue| Ok(view(lua).rate))?;
    set_fn(lua, &nsa, "getCurrentSamplerate", move |lua, _: MultiValue| Ok(view(lua).rate))?;
    set_fn(lua, &nsa, "setSamplerate", |_, _: MultiValue| Ok(()))?;
    set_fn(lua, &nsa, "getCurrentBufferSizeValue", move |lua, _: MultiValue| Ok(view(lua).buffer))?;
    set_fn(lua, &nsa, "getCurrentBufferSizeIndex", move |lua, _: MultiValue| {
        let v = view(lua);
        Ok(v.buffers.iter().position(|&b| b == v.buffer).unwrap_or(0) as i64)
    })?;
    let set_buffer = |lua: &Lua, frames: u32| {
        let st = state(lua);
        let mut st = st.borrow_mut();
        if st.audio.buffer != frames {
            st.audio.buffer = frames;
            st.audio_edits.push(AudioEdit::Buffer(frames));
        }
    };
    set_fn(lua, &nsa, "setBufferSizeIndex", move |lua, args: MultiValue| {
        let v = view(lua);
        let i = int(&args, 1).unwrap_or(0).clamp(0, v.buffers.len().saturating_sub(1) as i64) as usize;
        if let Some(&b) = v.buffers.get(i) {
            set_buffer(lua, b);
        }
        Ok(())
    })?;
    set_fn(lua, &nsa, "setBufferSize", move |lua, args: MultiValue| {
        if let Some(b) = int(&args, 1).filter(|b| *b > 0) {
            set_buffer(lua, b as u32);
        }
        Ok(())
    })?;
    set_fn(lua, &nsa, "isDeviceRunning", move |lua, _: MultiValue| Ok(view(lua).running))?;
    // Latency in frames: a buffer each way (the device's own isn't known).
    set_fn(lua, &nsa, "getLatency", move |lua, _: MultiValue| Ok(view(lua).buffer))?;
    set_fn(lua, &nsa, "getPhysicalChannelsCount", |lua, args: MultiValue| Ok(channels(lua, input(&args, 1)).len() as i64))?;
    set_fn(lua, &nsa, "getPhysicalChannelString", |lua, args: MultiValue| {
        let i = int(&args, 1).unwrap_or(-1);
        Ok(usize::try_from(i).ok().and_then(|i| channels(lua, input(&args, 2)).get(i).cloned()).unwrap_or_default())
    })?;
    set_fn(lua, &nsa, "getMappedChannel", move |lua, args: MultiValue| {
        let (i, inp) = (int(&args, 1).unwrap_or(-1), input(&args, 2));
        let v = view(lua);
        let map = if inp { &v.input_map } else { &v.output_map };
        let to = usize::try_from(i).ok().and_then(|i| map.get(i).copied().flatten());
        Ok(match to {
            Some(c) => channels(lua, inp).get(c + 1).cloned().unwrap_or_else(|| OFF.into()),
            None => OFF.to_string(),
        })
    })?;
    set_fn(lua, &nsa, "setMappedChannel", |lua, args: MultiValue| {
        let (Some(i), Some(c)) = (int(&args, 1), int(&args, 2)) else { return Ok(()) };
        let inp = input(&args, 3);
        let Ok(i) = usize::try_from(i) else { return Ok(()) };
        // Entry 0 of the list is "Off".
        let to = usize::try_from(c).ok().and_then(|c| c.checked_sub(1));
        let st = state(lua);
        let mut st = st.borrow_mut();
        let map = if inp { &mut st.audio.input_map } else { &mut st.audio.output_map };
        if map.len() <= i {
            map.resize(i + 1, None);
        }
        map[i] = to;
        st.audio_edits.push(AudioEdit::Map(inp, i, to));
        Ok(())
    })?;
    set_fn(lua, &nsa, "storeSetup", |_, _: MultiValue| Ok(()))?;
    Ok(())
}
