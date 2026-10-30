//! The MK3's Settings ▸ Audio tab (NI's `SettingsTabAudio.lua` on `App:getNSA()`): the
//! app tells the controller its audio interface (the output device, its rate, the
//! buffer size, the routing of DECKARD's inputs and outputs, as Preferences ▸ Audio
//! has them) and takes the tab's changes into the settings.

use crate::App;
use deckard_lua_host::state::{AudioEdit, AudioView};

/// How often the view is looked up again (device lists are slow to ask for).
const REFRESH: std::time::Duration = std::time::Duration::from_secs(1);

impl App {
    /// The audio interface as the Settings page shows it.
    pub fn audio_view(&mut self) -> AudioView {
        let outputs = self.outputs.get_or_insert_with(deckard_engine::output::output_devices).clone();
        let inputs = self.inputs.get_or_insert_with(crate::recorder::input_devices).clone();
        let device = self.settings.output_device.clone().or_else(deckard_engine::output::default_output).unwrap_or_default();
        let input = self.settings.input_device.clone().or_else(crate::recorder::default_input).unwrap_or_default();
        let count = |list: &[(String, usize)], d: &str| list.iter().find(|(n, _)| n == d).map_or(0, |(_, c)| *c);
        // The interface's own name for a channel where it has one (BUGS #9), else its number.
        let mut names = |d: &str, input: bool, n: usize| {
            let own = self.channel_names_of(d, input, n);
            (0..n).map(|i| own.get(i).cloned().flatten().unwrap_or_else(|| (i + 1).to_string())).collect::<Vec<_>>()
        };
        let (in_names, out_names) = (names(&input, true, count(&inputs, &input)), names(&device, false, count(&outputs, &device)));
        let (rate, running) = match &self.output {
            Some(Ok(o)) => (o.rate, true),
            _ => (0, false),
        };
        AudioView {
            driver: driver_name().into(),
            devices: outputs.iter().map(|(n, _)| n.clone()).collect(),
            rate,
            buffers: deckard_engine::output::BUFFER_SIZES.to_vec(),
            buffer: self.settings.buffer_frames,
            running,
            inputs: in_names,
            outputs: out_names,
            input_map: self.settings.input_routing.clone(),
            output_map: self.settings.output_routing.clone(),
            device,
        }
    }

    /// The names device `d` gives its first `n` input (or output) channels, looked up
    /// once.
    pub fn channel_names_of(&mut self, d: &str, input: bool, n: usize) -> Vec<Option<String>> {
        let names = self.channel_names.entry((d.to_string(), input)).or_insert_with(|| crate::channel_names::channel_names(d, input, n));
        if names.len() < n && !names.is_empty() {
            *names = crate::channel_names::channel_names(d, input, n);
        }
        names.clone()
    }

    /// A change from the Audio tab, into the settings (saved); the output opens again
    /// with it if it was open.
    pub fn audio_edit(&mut self, e: AudioEdit) {
        let mut restart = false;
        match e {
            AudioEdit::Device(d) => {
                if Some(&d) != self.settings.output_device.as_ref() {
                    self.settings.set_output_device(Some(d));
                    restart = true;
                }
            }
            AudioEdit::Buffer(n) => {
                if n != self.settings.buffer_frames {
                    self.settings.buffer_frames = n;
                    restart = true;
                }
            }
            AudioEdit::Map(input, i, to) => {
                let map = if input { &mut self.settings.input_routing } else { &mut self.settings.output_routing };
                if map.len() <= i {
                    map.resize(i + 1, None);
                }
                map[i] = to;
            }
        }
        self.settings.save();
        if restart && self.output.is_some() {
            self.restart_output();
        }
    }

    /// Each frame: the tab's changes in, and (now and then) the interface out.
    pub fn sync_audio_settings(&mut self) {
        let Some(Ok(c)) = &self.controller else { return };
        let edits = std::mem::take(&mut c.host.state.borrow_mut().audio_edits);
        let stale = !edits.is_empty() || self.audio_view_at.is_none_or(|t| t.elapsed() >= REFRESH);
        for e in edits {
            self.audio_edit(e);
        }
        if !stale {
            return;
        }
        self.audio_view_at = Some(std::time::Instant::now());
        let view = self.audio_view();
        if let Some(Ok(c)) = &self.controller {
            let mut st = c.host.state.borrow_mut();
            if st.audio != view {
                st.audio = view;
            }
        }
    }
}

/// The audio system DECKARD plays through.
fn driver_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "CoreAudio"
    } else if cfg!(windows) {
        "WASAPI"
    } else {
        "ALSA"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tab's routing and buffer changes reach the settings; the view shows them.
    #[test]
    fn the_audio_tab_changes_the_settings() {
        let mut app = App::default();
        app.audio_edit(AudioEdit::Map(false, 3, Some(5)));
        app.audio_edit(AudioEdit::Map(true, 0, None));
        app.audio_edit(AudioEdit::Buffer(1024));
        assert_eq!(app.settings.output_routing[3], Some(5));
        assert_eq!(app.settings.input_routing[0], None);
        assert_eq!(app.settings.buffer_frames, 1024);
        assert!(app.output.is_none(), "the output isn't opened for it");
        let v = app.audio_view();
        assert_eq!((v.buffer, v.output_map[3]), (1024, Some(5)));
        assert_eq!(v.buffers, deckard_engine::output::BUFFER_SIZES);
        assert!(!v.driver.is_empty());
    }

    /// On the MK3: SHIFT + SETTINGS, the AUDIO tab, knob 4 turns the buffer size.
    #[test]
    fn the_mk3_turns_the_buffer_size() {
        let Some(mut h) = crate::tests::harness(true, false) else { return };
        let before = h.state().settings.buffer_frames;
        h.run_steps(2);
        if let Some(Ok(c)) = &mut h.state_mut().controller {
            let _ = c.host.button("BUTTON_SHIFT", true);
            let _ = c.host.press("BUTTON_SETTINGS");
            let _ = c.host.button("BUTTON_SHIFT", false);
            let _ = c.host.press("BUTTON_SCREEN_6");
            c.host.tick();
            let _ = c.host.encoder("ENCODER_SCREEN_4", 0.5);
            c.host.tick();
        }
        h.run_steps(3);
        let sizes = deckard_engine::output::BUFFER_SIZES;
        let next = sizes.iter().position(|&b| b == before).map_or(before, |i| sizes[(i + 1).min(sizes.len() - 1)]);
        assert_eq!(h.state().settings.buffer_frames, next);
    }
}
