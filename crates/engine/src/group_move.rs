//! Move runtime players at the same block boundary as the edited document.
use crate::{Engine, Live};
impl Engine {
    /// Mark an edit as exchanging two existing Groups. Applied atomically by adopt.
    pub fn prepare_group_swap(&mut self, a: usize, b: usize) {
        self.group_swaps.push((a, b, None));
    }
    pub fn prepare_group_swap_retaining(&mut self, a: usize, b: usize, keep: Option<usize>) {
        self.group_swaps.push((a, b, keep));
    }
    fn release_moving_group(&mut self, g: usize) {
        if let Some(p) = self.midi_inputs.get_mut(g).and_then(Option::as_mut) {
            p.stopped();
        }
        let n = self.sounds.get(g).map_or(0, Vec::len).max(self.instruments.get(g).map_or(0, Vec::len));
        for s in 0..n {
            self.lfo_release(g, s, None);
            self.send_live(g, s, crate::ahead::Ev::Stop);
            if let Some(i) = self.instruments.get(g).and_then(|r| r.get(s)).and_then(Option::as_ref) {
                if let Ok(mut p) = i.plugin.lock() {
                    p.all_notes_off();
                }
            }
            if let Some(smp) = self.sounds.get_mut(g).and_then(|r| r.get_mut(s)).and_then(Option::as_mut) {
                smp.fade_all();
            }
        }
        let mut i = 0;
        while i < self.midi_offs.len() {
            if self.midi_offs[i].held.0 == g {
                let note = self.midi_offs.remove(i);
                self.end_midi_note(note);
            } else {
                i += 1;
            }
        }
        self.note_offs.retain(|e| e.1 != g);
        self.touch.retain(|e| e.group != g);
        self.held_keys.retain(|e| e.group != g);
        self.live.retain(|(_, e)| live_group(e) != Some(g));
        if let Some(r) = &mut self.repeater {
            r.release_group(g);
        }
    }
    pub(super) fn swap_group_runtime(&mut self, a: usize, b: usize, keep: Option<usize>) {
        if keep != Some(a) {
            self.release_moving_group(a);
        }
        if keep != Some(b) {
            self.release_moving_group(b);
        }
        if let Some(r) = &mut self.repeater {
            r.groups_moved(a, b);
        }
        for route in &mut self.mod_routes {
            route.group = moved(route.group, a, b);
        }
        for lfo in &mut self.lfos {
            lfo.lfo.trigger = lfo.lfo.trigger.map(|(g, s)| (moved(g, a, b), s));
        }

        for note in &mut self.note_offs {
            note.1 = moved(note.1, a, b);
        }
        for note in &mut self.midi_offs {
            note.held.0 = moved(note.held.0, a, b);
        }
        for note in &mut self.touch {
            note.group = moved(note.group, a, b);
        }
        for note in &mut self.held_keys {
            note.group = moved(note.group, a, b);
        }
        for (_, event) in &mut self.live {
            remap_live(event, a, b);
        }
        macro_rules! swap { ($($field:ident),*) => { $(if self.$field.len() > a.max(b) { self.$field.swap(a,b); })* }; }
        swap!(sounds, instruments, fx, buses, bufs, sound_aux, group_aux, midi_inputs, live_hint, reuse);
        for g in [a, b] {
            if let Some(p) = self.midi_inputs.get_mut(g).and_then(Option::as_mut) {
                p.group_moved(g);
            }
        }
        if let Some(ahead) = &mut self.ahead {
            if ahead.links.len() > a.max(b) {
                ahead.links.swap(a, b);
            }
            ahead.sent.clear();
        }
        if let crate::Mode::Pattern(g, _) = &mut self.mode {
            *g = moved(*g, a, b);
        }
        self.focus = self.focus.map(|g| moved(g, a, b));
        self.midi_inputs_unsettled &= !(1u64.checked_shl(a as u32).unwrap_or(0) | 1u64.checked_shl(b as u32).unwrap_or(0));
    }
}
fn moved(g: usize, a: usize, b: usize) -> usize {
    if g == a {
        b
    } else if g == b {
        a
    } else {
        g
    }
}
fn live_group(e: &Live) -> Option<usize> {
    match e {
        Live::NoteOn { group, .. }
        | Live::NoteRelease { group, .. }
        | Live::NoteIn { group, .. }
        | Live::MidiInputOut { group, .. }
        | Live::ControlIn { group, .. }
        | Live::Expression { group, .. }
        | Live::RepeatPressure { group, .. }
        | Live::ChokeGroup { group } => Some(*group),
        Live::ChokeAll => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Mode, SampleCache,
        midi_input::{Clock, MidiInput, NoteIn, Outbox},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Probe(Arc<AtomicUsize>);
    impl MidiInput for Probe {
        fn note(&mut self, _: NoteIn, _: Clock, _: &mut Outbox) {}
        fn stopped(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    #[test]
    fn group_move_during_playback_releases_notes_and_moves_input_players() {
        let mut p = crate::tests::project();
        p.groups.push(p.groups[0].clone());
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48000, Mode::Arrangement, &mut cache);
        let a = Arc::new(AtomicUsize::new(0));
        let b = Arc::new(AtomicUsize::new(0));
        e.set_midi_input(0, Some(Box::new(Probe(a.clone()))));
        e.set_midi_input(1, Some(Box::new(Probe(b.clone()))));
        e.held_keys.push(crate::mod_rt::HeldKey { group: 0, sound: 0, pitch: 60, live: [None; 3], order: 0 });
        e.note_offs.push((u64::MAX, 0, 0, 60, 1));
        e.schedule(100, Live::NoteIn { group: 0, sound: 0, pitch: 62, velocity: 100, thru: true });
        e.process(&mut [0.; 128]);
        let position = e.position();
        let mut q = p.clone();
        q.groups.swap(0, 1);
        let (mut fresh, _) = Engine::new_after(&q, None, None, 48000, Mode::Arrangement, &mut cache);
        fresh.prepare_group_swap(0, 1);
        drop(e.adopt(fresh, &q));
        assert_eq!(e.position(), position);
        assert!(e.sequencing);
        assert!(e.note_offs.is_empty());
        assert!(e.held_keys.is_empty());
        assert!(e.live.is_empty());
        assert_eq!(a.load(Ordering::Relaxed), 1);
        assert_eq!(b.load(Ordering::Relaxed), 1);
        e.set_midi_input_param(1, 0, 0.); // descriptor and engine player stay addressable
        e.set_midi_input(1, None);
        assert!(a.load(Ordering::Relaxed) >= 1);
        e.process(&mut [0.; 128]);
        assert!(e.position() > position);
    }
}

fn remap_live(e: &mut Live, a: usize, b: usize) {
    match e {
        Live::NoteOn { group, .. }
        | Live::NoteRelease { group, .. }
        | Live::NoteIn { group, .. }
        | Live::MidiInputOut { group, .. }
        | Live::ControlIn { group, .. }
        | Live::Expression { group, .. }
        | Live::RepeatPressure { group, .. }
        | Live::ChokeGroup { group } => *group = moved(*group, a, b),
        Live::ChokeAll => {}
    }
}

#[cfg(test)]
mod retain_tests {
    use super::*;
    use crate::{Mode, SampleCache};
    #[test]
    fn group_move_retained_notes_keep_their_owner() {
        let mut p = crate::tests::project();
        p.groups.push(p.groups[0].clone());
        let mut cache = SampleCache::default();
        let (mut e, _) = Engine::new(&p, None, 48000, Mode::Arrangement, &mut cache);
        e.note_offs.push((u64::MAX, 0, 0, 60, 1));
        e.held_keys.push(crate::mod_rt::HeldKey { group: 0, sound: 0, pitch: 60, live: [None; 3], order: 0 });
        e.schedule(100, Live::NoteIn { group: 0, sound: 0, pitch: 64, velocity: 0, thru: true });
        let mut q = p.clone();
        q.groups.swap(0, 1);
        let (mut fresh, _) = Engine::new_after(&q, None, None, 48000, Mode::Arrangement, &mut cache);
        fresh.prepare_group_swap_retaining(0, 1, Some(0));
        drop(e.adopt(fresh, &q));
        assert_eq!(e.held_keys[0].group, 1);
        assert!(matches!(e.live[0].1, Live::NoteIn { group: 1, .. }));
        e.note_release(1, 0, 60);
        assert!(e.held_keys.is_empty());
    }

    #[test]
    fn moving_group_keeps_sounding_note_and_routes_note_off_to_new_slot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("held.wav");
        let spec = hound::WavSpec { channels: 1, sample_rate: 48000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut wav = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..48000 {
            wav.write_sample((i as f32 * 0.03).sin() * 0.2).unwrap();
        }
        wav.finalize().unwrap();
        for retain in [false, true] {
            let mut p = crate::tests::project();
            p.groups[0].patterns[0].notes.clear();
            let smp = p.groups[0].sounds[0].sampler.as_mut().unwrap();
            smp.set("envelope_type", 2.);
            smp.set("volume_envelope/sustain", 1.);
            smp.set("volume_envelope/release", 0.);
            smp.zones[0].sample.path = path.to_string_lossy().into_owned();
            smp.zones[0].loop_active = true;
            smp.zones[0].loop_range = (0, 47999);
            p.groups.push(p.groups[0].clone());
            let mut cache = SampleCache::default();
            let (mut e, _) = Engine::new(&p, None, 48000, Mode::Arrangement, &mut cache);
            e.sequencing = false;
            e.note_hold(0, 0, 60, 100);
            e.process(&mut [0.; 1024]);
            assert!(e.active_voices() > 0);
            let mut q = p.clone();
            q.groups.swap(0, 1);
            let (mut fresh, _) = Engine::new_after(&q, None, None, 48000, Mode::Arrangement, &mut cache);
            fresh.prepare_group_swap_retaining(0, 1, retain.then_some(0));
            drop(e.adopt(fresh, &q));
            for _ in 0..20 {
                e.process(&mut [0.; 1024]);
            }
            assert_eq!(e.active_voices() > 0, retain, "retained group keeps sounding");
            if retain {
                assert!(e.sounds[1][0].as_ref().unwrap().voices() > 0);
                e.note_release(1, 0, 60);
                for _ in 0..20 {
                    e.process(&mut [0.; 1024]);
                }
                assert_eq!(e.active_voices(), 0, "release at the moved slot ends the same note");
            }
        }
    }
}
