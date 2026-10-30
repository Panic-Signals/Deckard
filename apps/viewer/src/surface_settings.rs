//! The Settings page of DECKARD's own controller layer (SETTINGS): the app's settings
//! as it shows them, and its knobs' steps into them. The audio and MIDI parts go the
//! way the scripts' Settings page takes them (`audio_settings.rs`,
//! `transport_sync.rs`).

use crate::App;
use crate::settings::{METRONOME_TICKS, MidiSync};
use deckard_lua_host::state::AudioEdit;
use deckard_surface::SettingsView;

/// How often the view is looked up again while the page shows (device lists are slow
/// to ask for).
const REFRESH: std::time::Duration = std::time::Duration::from_secs(1);

/// The count-ins offered (bars; 0 none).
const COUNT_INS: [u32; 4] = [0, 1, 2, 4];
const DENOMINATORS: [i64; 6] = [1, 2, 4, 8, 16, 32];
/// HARDWARE's velocity SCALING (-3 … 3) and Smart Strip SHOW POSITION (0 … 2), as the
/// MK3 manual names them.
const VELOCITY_SCALINGS: [&str; 7] = ["Soft 3", "Soft 2", "Soft 1", "Linear", "Hard 1", "Hard 2", "Hard 3"];
const STRIP_POSITIONS: [&str; 3] = ["Off", "Rec. Only", "On"];

fn on_off(on: bool) -> String {
    if on { "On" } else { "Off" }.to_string()
}

/// `list`'s entry `step` from `at` (kept within it).
fn stepped<T: PartialEq + Copy>(list: &[T], at: T, step: i64) -> T {
    let i = list.iter().position(|v| *v == at).unwrap_or(0) as i64;
    list[(i + step).clamp(0, list.len() as i64 - 1) as usize]
}

impl App {
    /// While the Settings page shows: its view, looked up again now and then (and
    /// after a change).
    pub(crate) fn refresh_surface_settings(&mut self) {
        if self.surface.page() != "settings" {
            self.surface_settings_at = None;
            return;
        }
        if self.surface_settings_at.is_some_and(|t| t.elapsed() < REFRESH) {
            return;
        }
        self.surface_settings_at = Some(std::time::Instant::now());
        self.surface_settings = self.settings_view();
    }

    pub(crate) fn settings_view(&mut self) -> SettingsView {
        let audio = self.audio_view();
        let (ins, outs) = self.midi_ports().clone();
        let s = &self.settings;
        let mut v = SettingsView::default();
        let mut set = |k: &str, value: String| {
            v.values.insert(k.to_string(), value);
        };
        set(
            "count_in",
            match s.count_in_bars {
                0 => "Off".into(),
                1 => "1 Bar".into(),
                n => format!("{n} Bars"),
            },
        );
        set("quantize", on_off(s.input_quantize));
        set("driver", audio.driver.clone());
        set("device", audio.device.clone());
        set("sample_rate", audio.rate.to_string());
        set("buffer_size", audio.buffer.to_string());
        for (key, map, names) in [("in_route", &audio.input_map, &audio.inputs), ("out_route", &audio.output_map, &audio.outputs)] {
            for ch in 0..crate::settings::CHANNELS {
                let to = map.get(ch).copied().flatten().and_then(|c| names.get(c).cloned());
                set(&format!("{key}_{}", ch + 1), to.unwrap_or_else(|| deckard_lua_host::api::nsa::OFF.into()));
            }
        }
        // (Abbreviated while syncing, as CLOCK OFFSET shows beside it.)
        set(
            "midi_sync",
            match s.midi_sync {
                MidiSync::Off => "Off",
                MidiSync::Send => "Send",
                MidiSync::Receive => "Receive",
            }
            .into(),
        );
        set(
            "clock_offset",
            match s.midi_sync {
                MidiSync::Send if s.clock_send_offset_ms > 0 => format!("-{}", s.clock_send_offset_ms),
                MidiSync::Receive => s.clock_receive_offset_ms.to_string(),
                _ => "0".into(),
            },
        );
        set("default_input", if s.sound_midi_focus { "Focus" } else { "None" }.into());
        set("link_start_stop", on_off(s.link_start_stop));
        set("sensitivity", s.pad_sensitivity.to_string());
        set("metronome_volume", format!("{}%", s.metronome_volume));
        set("metronome_auto", on_off(s.metronome_auto));
        set("metronome_ticks", format!("1/{}", s.metronome_ticks));
        set("reload_last", on_off(s.reload_last));
        set("auto_backup", on_off(s.auto_backup));
        set("duplicate", if s.duplicate_patterns { "Scn & Ptn" } else { "Scene" }.into());
        set("link_duplicates", on_off(s.link_duplicates));
        let (bars, beats, sixteenths) = s.pattern_length;
        set("pattern_length", format!("{bars}:{beats}:{sixteenths}"));
        set("pattern_grow", on_off(s.pattern_grow));
        set("velocity_scaling", VELOCITY_SCALINGS[(s.velocity_scaling.clamp(-3, 3) + 3) as usize].to_string());
        set("led_brightness", format!("{}%", s.led_brightness));
        set("strip_position", STRIP_POSITIONS[usize::from(s.strip_position.min(2))].to_string());
        set("touch_auto_write", on_off(s.touch_auto_write));
        set("display_brightness", s.display_brightness.to_string());
        let ports = |prefix: &str, list: &[String]| -> Vec<(String, bool)> {
            list.iter().map(|n| (n.clone(), !s.midi_off.contains(&format!("{prefix}{n}")))).collect()
        };
        v.midi_inputs = ports("in:", &ins);
        v.midi_outputs = ports("out:", &outs);
        // (No device: no rate, so no latency, "inf" as Maschine has it.)
        let ms = |frames: u32| {
            if audio.rate == 0 { f64::INFINITY } else { (f64::from(frames) * 1000.0 / f64::from(audio.rate) * 10.0).round() / 10.0 }
        };
        let shown = |ms: f64| if ms.is_finite() { format!("{ms:.1}ms") } else { "infms".into() };
        let lat = ms(audio.buffer);
        v.audio_status = vec![
            format!("Device Status: {}", if audio.running { "Running" } else { "Missing" }),
            format!("Input: {}", shown(lat)),
            format!("Processing: {}", shown(lat)),
            format!("Output: {}", shown(lat)),
            format!("Total Latency: {}", shown(lat * 3.0)),
        ];
        v.link = s.link;
        v.link_peers = self.link_view().map_or(0, |(peers, _)| peers);
        v
    }

    /// The Settings page's LENGTH in `p`'s bars and beats, as ticks (a sixteenth at the
    /// least).
    pub(crate) fn default_pattern_length(&self, p: &deckard_model::Project) -> i64 {
        let (bars, beats, sixteenths) = self.settings.pattern_length;
        let bar = p.bar_length();
        let beat = bar / p.numerator.max(1);
        let sixteenth = deckard_model::PPQ / 4;
        (i64::from(bars) * bar + i64::from(beats) * beat + i64::from(sixteenths) * sixteenth).max(sixteenth)
    }

    /// A step of one of the settings the Settings page's knobs turn ([`deckard_surface::Action::Setting`]).
    pub(crate) fn step_setting(&mut self, name: &str, index: usize, step: i64) {
        let before = self.settings.clone();
        let up = step > 0;
        match name {
            "count_in" => self.settings.count_in_bars = stepped(&COUNT_INS, self.settings.count_in_bars, step),
            "quantize" => self.settings.input_quantize = up,
            "numerator" | "denominator" => {
                let Some(doc) = &self.doc else { return };
                let (n, d) = {
                    let p = doc.borrow();
                    let p = p.project();
                    (p.numerator, p.denominator)
                };
                let (n, d) = if name == "numerator" { ((n + step).clamp(1, 32), d) } else { (n, stepped(&DENOMINATORS, d, step)) };
                if let Err(e) = doc.borrow_mut().set_time_signature(n, d) {
                    self.status = format!("Time Signature: {e}");
                }
            }
            "device" | "buffer_size" | "in_route" | "out_route" => {
                let audio = self.audio_view();
                let edit = match name {
                    "device" => {
                        let at = audio.devices.iter().position(|d| *d == audio.device).unwrap_or(0) as i64;
                        let Some(d) = audio.devices.get((at + step).clamp(0, audio.devices.len() as i64 - 1).max(0) as usize) else {
                            return;
                        };
                        AudioEdit::Device(d.clone())
                    }
                    "buffer_size" => AudioEdit::Buffer(stepped(&audio.buffers, audio.buffer, step)),
                    _ => {
                        let input = name == "in_route";
                        let (map, names) = if input { (&audio.input_map, &audio.inputs) } else { (&audio.output_map, &audio.outputs) };
                        // (0 not connected, then the interface's channels.)
                        let at = map.get(index).copied().flatten().map_or(0, |c| c as i64 + 1);
                        let to = (at + step).clamp(0, names.len() as i64);
                        AudioEdit::Map(input, index, (to > 0).then(|| to as usize - 1))
                    }
                };
                self.audio_edit(edit);
            }
            "midi_sync" => {
                let at = match self.settings.midi_sync {
                    MidiSync::Off => 0,
                    MidiSync::Send => 1,
                    MidiSync::Receive => 2,
                };
                // (Receive Clock isn't offered while Link is on.)
                self.settings.midi_sync = match (at + step).clamp(0, if self.settings.link { 1 } else { 2 }) {
                    0 => MidiSync::Off,
                    1 => MidiSync::Send,
                    _ => MidiSync::Receive,
                };
            }
            "clock_offset" => match self.settings.midi_sync {
                MidiSync::Send => {
                    self.settings.clock_send_offset_ms = (self.settings.clock_send_offset_ms as i64 + step).clamp(0, 200) as u32
                }
                MidiSync::Receive => {
                    self.settings.clock_receive_offset_ms = (self.settings.clock_receive_offset_ms + step as i32).clamp(-100, 100)
                }
                MidiSync::Off => {}
            },
            // (Focus, then None.)
            "default_input" => self.settings.sound_midi_focus = !up,
            "midi_in" | "midi_out" => {
                let (ins, outs) = self.midi_ports().clone();
                let (prefix, list) = if name == "midi_in" { ("in:", ins) } else { ("out:", outs) };
                let Some(port) = list.get(index) else { return };
                let key = format!("{prefix}{port}");
                if up {
                    self.settings.midi_off.remove(&key);
                } else {
                    self.settings.midi_off.insert(key);
                }
            }
            "link" => {
                if up != self.settings.link {
                    self.toggle_link();
                }
            }
            "link_start_stop" => self.settings.link_start_stop = up,
            "metronome_volume" => self.settings.metronome_volume = (i64::from(self.settings.metronome_volume) + step).clamp(0, 100) as u8,
            "metronome_auto" => self.settings.metronome_auto = up,
            "metronome_ticks" => self.settings.metronome_ticks = stepped(&METRONOME_TICKS, self.settings.metronome_ticks, step),
            "reload_last" => self.settings.reload_last = up,
            "auto_backup" => self.settings.auto_backup = up,
            "duplicate" => self.settings.duplicate_patterns = up,
            "link_duplicates" => self.settings.link_duplicates = up,
            // (A bar a step; with SHIFT, a sixteenth. A sixteenth at the least.)
            "pattern_length" | "pattern_length_fine" => {
                let (bars, beats, sixteenths) = self.settings.pattern_length;
                let at = i64::from((bars * 4 + beats) * 4 + sixteenths);
                let n = (at + if name == "pattern_length" { step * 16 } else { step }).max(1) as u32;
                self.settings.pattern_length = (n / 16, n / 4 % 4, n % 4);
            }
            "pattern_grow" => self.settings.pattern_grow = up,
            "velocity_scaling" => self.settings.velocity_scaling = (i64::from(self.settings.velocity_scaling) + step).clamp(-3, 3) as i8,
            "led_brightness" => self.settings.led_brightness = (i64::from(self.settings.led_brightness) + step).clamp(0, 100) as u8,
            "strip_position" => self.settings.strip_position = (i64::from(self.settings.strip_position) + step).clamp(0, 2) as u8,
            "touch_auto_write" => self.settings.touch_auto_write = up,
            "sensitivity" => self.settings.pad_sensitivity = (i64::from(self.settings.pad_sensitivity) + step).clamp(0, 100) as u8,
            "display_brightness" => {
                self.settings.display_brightness = (i64::from(self.settings.display_brightness) + step).clamp(0, 100) as u8;
                if let Some(m) = &self.mk3 {
                    m.set_display_brightness(self.settings.display_brightness);
                }
            }
            _ => {}
        }
        if self.settings != before {
            self.settings.save();
            if let Some(Ok(out)) = &self.output {
                out.set_metronome_settings(
                    f32::from(self.settings.metronome_volume) / 100.0,
                    self.settings.metronome_auto,
                    self.settings.metronome_ticks,
                );
            }
        }
        // (Shown at once.)
        self.surface_settings_at = None;
        self.refresh_surface_settings();
    }
}
