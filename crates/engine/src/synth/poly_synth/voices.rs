//! Notes in, voices assigned (docs/synth-poly-synth.md §3.3): `P5::processMidiEvent`,
//! `allocateVoice`, `triggerNoteOn`/`triggerNoteOff`, `sustainedNotesOff` and
//! `allSoundOff`.
//!
//! Voices sit in a circular doubly-linked list (`next`/`prev`). `head` is the voice the
//! next note takes; released voices move behind `tail`, so the longest-released voice
//! is reused first. When every voice holds a key (`tail == −1`) the next note steals
//! `head`. A note whose voice still rings (or was released) gets that voice again.

use super::engine::{NO_NOTE, P5, SLOTS};
use super::params::{id, note_pitch};

impl P5 {
    /// MIDI note on (`processMidiEvent`, status 0x9n with velocity > 0).
    pub fn note_on(&mut self, note: u8, velocity: u8) {
        let note = note.min(127);
        if velocity == 0 {
            self.note_off(note);
            return;
        }
        let Some(v) = self.allocate_voice(self.head, note as i32) else { return };
        self.notes[v] = note as i32;
        self.trigger_note_on(v, velocity as i32);
        if self.unison != 0 {
            self.last_note = note as i32;
        }
        self.sustained[v] = false;
        self.sostenuto[v] = false;
    }

    /// MIDI note off: every voice playing `note` releases (or is marked sustained while
    /// the pedal / Hold is down) and goes to the end of the allocation list.
    pub fn note_off(&mut self, note: u8) {
        let note = note.min(127) as i32;
        let mut i = 0;
        while i < self.voices as usize {
            if self.notes[i] == note {
                self.sustained[i] = self.sustain;
                if !self.sustain && !self.sostenuto[i] {
                    self.trigger_note_off(i);
                    self.move_to_tail(i);
                    self.notes[i] = -300 - note;
                }
            }
            i += 1;
        }
    }

    /// Sustain pedal (CC 64).
    pub fn sustain_pedal(&mut self, down: bool) {
        if down {
            self.sustain = true;
        } else {
            self.sustained_notes_off();
        }
    }

    /// Mod wheel (CC 1, 0..127).
    pub fn mod_wheel(&mut self, value: u8) {
        self.set_parameter_value(id::WHEEL, value as f32 * f32::from_bits(0x3c01_0204));
    }

    /// Pitch bend (14 bit, 8192 = centre).
    pub fn pitch_bend(&mut self, value: u16) {
        let v = (value.min(16383) as f64 * 6.103_515_625e-5) as f32;
        let v = P5::quantize(id::BEND, v);
        self.program[id::BEND as usize] = super::params::clamp01(v);
        self.bend_value = v;
        self.retarget_bend();
    }

    /// Move voice `i` behind `tail` (or make it the head when no voice was free).
    fn move_to_tail(&mut self, i: usize) {
        let i = i as i32;
        let t = self.tail;
        if t == -1 {
            if i != self.head {
                let h2 = self.prev[self.head as usize];
                if i != h2 {
                    self.move_after(i, h2);
                }
            }
            self.head = i;
        } else if i != t {
            self.move_after(i, t);
        }
        self.tail = i;
    }

    /// Unlink voice `i` and insert it after `at`.
    fn move_after(&mut self, i: i32, at: i32) {
        let (iu, au) = (i as usize, at as usize);
        let (p, n) = (self.prev[iu], self.next[iu]);
        self.next[p as usize] = n;
        self.prev[n as usize] = p;
        self.next[iu] = self.next[au];
        self.prev[iu] = at;
        self.prev[self.next[au] as usize] = i;
        self.next[au] = i;
    }

    /// `P5::allocateVoice(first, note)`: the voice for a new note, moved to sit just
    /// before `first` in the list.
    fn allocate_voice(&mut self, first: i32, note: i32) -> Option<usize> {
        for i in 0..self.voices as usize {
            if self.notes[i] == note || self.notes[i] == -300 - note {
                let ii = i as i32;
                if self.head == ii {
                    self.head = self.next[i];
                    if self.tail == ii {
                        self.tail = -1;
                    }
                } else if self.tail == ii {
                    self.tail = self.prev[i];
                }
                if first == ii {
                    return Some(i);
                }
                let before = self.prev[first as usize];
                if before == ii {
                    return Some(i);
                }
                self.move_after(ii, before);
                return Some(i);
            }
        }
        if first < 0 {
            return None;
        }
        if self.tail == -1 {
            // Every voice busy: walk back from before `first` to the head (the oldest).
            let stop = self.prev[self.head as usize];
            let mut x = self.prev[first as usize];
            let mut guard = 0;
            while self.prev[x as usize] != stop && guard < SLOTS {
                x = self.prev[x as usize];
                guard += 1;
            }
            if x == self.head {
                self.head = self.next[self.head as usize];
            }
            if x == first {
                return Some(first as usize);
            }
            let before = self.prev[first as usize];
            if before != x {
                self.move_after(x, before);
            }
            Some(x as usize)
        } else {
            let x = self.head;
            self.head = self.next[x as usize];
            if x == self.tail {
                self.tail = -1;
            }
            if x == first {
                return Some(first as usize);
            }
            let before = self.prev[first as usize];
            if before != x {
                self.move_after(x, before);
            }
            Some(x as usize)
        }
    }

    /// Envelope starts for a key press at `velocity`: attack from where the envelope is
    /// toward the velocity level, or straight to decay if it is already above it.
    fn start_envs(&mut self, i: usize, amp: f32, flt: f32) {
        let (att, dec, sus) = (self.amp_attack, self.amp_decay, self.amp_sustain);
        let (fatt, fdec, fsus) = (self.flt_attack, self.flt_decay, self.flt_sustain);
        let v = &mut self.v[i];
        let a = v.amp.value;
        if a > amp {
            v.amp.floor = (a + 0.02).mul_add(sus, -0.02);
            v.amp.rem = a - v.amp.floor;
            v.amp.coef = dec;
            v.amp.target = a;
            v.amp.stage = 1;
        } else {
            v.amp.inc = amp * att;
            v.amp.target = amp;
            v.amp.stage = 0;
        }
        let f = v.flt.value;
        if f > flt {
            v.flt.stage = 1;
            v.flt.coef = fdec;
            v.flt.floor = f * fsus;
            v.flt.target = f;
            v.flt.rem = f - v.flt.floor;
        } else {
            v.flt.target = flt;
            v.flt.inc = flt * fatt;
            v.flt.stage = 0;
            v.flt.rem = 1.0;
        }
    }

    /// Envelope levels for `velocity`: `0.5 + sens·(vel/128 − 0.5)`.
    fn vel_levels(&self, velocity: i32) -> (f32, f32) {
        let x = (velocity as f32).mul_add(0.0078125, -0.5);
        (self.vel_amp.mul_add(x, 0.5), self.vel_flt.mul_add(x, 0.5))
    }

    /// Voices that hold a key (velocity > 0) among those playing.
    fn held(&self) -> usize {
        self.v[..self.voices as usize].iter().filter(|v| v.velocity > 0.0).count()
    }

    /// `P5::triggerNoteOn(0, voice, velocity)`.
    fn trigger_note_on(&mut self, i: usize, velocity: i32) {
        let note = self.notes[i] as f32;
        self.v[i].note = note;
        let p = note + 0.0;
        if self.unison != 0 {
            self.uni_note = note;
            self.glide_target = p;
            if p == self.glide {
                self.glide_step = 0.0;
                for k in 0..self.voices as usize {
                    self.v[k].pitch = p + self.v[k].spread;
                    self.refresh_freqs(k);
                }
            } else {
                self.glide_step = self.glide_step_for(p);
            }
            self.uni_held = 1;
            let held = self.held();
            if self.voice_trig != 0 {
                if self.lfo_synced == 0 && held == 0 {
                    self.lfo_phase = 1.0;
                }
            } else if held == 0 {
                // First key: every voice starts.
                let (amp, flt) = self.vel_levels(velocity);
                for k in 0..self.voices as usize {
                    self.v[k].active = 1;
                    if self.v[k].amp.value < 0.0 {
                        self.v[k].amp.value = 0.0;
                    }
                    self.start_envs(k, amp, flt);
                }
            } else {
                // Legato: Unison retriggers attacks, Unison with Legato only moves on.
                let (amp, flt) = self.vel_levels(velocity);
                let retrig = self.uni_retrig != 0;
                let (att, dec, sus) = (self.amp_attack, self.amp_decay, self.amp_sustain);
                let (fatt, fdec, fsus) = (self.flt_attack, self.flt_decay, self.flt_sustain);
                for k in 0..self.voices as usize {
                    let v = &mut self.v[k];
                    v.amp.target = amp;
                    if (v.amp.stage == 0 || retrig) && v.amp.value < amp {
                        v.amp.stage = 0;
                        v.amp.inc = amp * att;
                    } else {
                        v.amp.coef = dec;
                        v.amp.floor = (amp + 0.02).mul_add(sus, -0.02);
                        v.amp.rem = v.amp.value - v.amp.floor;
                        v.amp.stage = 1;
                    }
                    v.flt.target = flt;
                    if (v.flt.stage == 0 || retrig) && v.flt.value < flt {
                        v.flt.stage = 0;
                        v.flt.inc = flt * fatt;
                        v.flt.rem = 1.0;
                    } else {
                        v.flt.coef = fdec;
                        v.flt.floor = flt * fsus;
                        v.flt.rem = v.flt.value - v.flt.floor;
                        v.flt.stage = 1;
                    }
                }
            }
            self.v[i].velocity = velocity as f32;
            return;
        }
        if self.voice_trig != 0 {
            // The envelopes start at the next LFO cycle; the LFO restarts on a first key.
            if self.lfo_synced == 0 && self.held() == 0 {
                self.lfo_phase = 1.0;
            }
        } else {
            let (amp, flt) = self.vel_levels(velocity);
            self.v[i].active = 1;
            self.start_envs(i, amp, flt);
        }
        self.v[i].pitch = p;
        self.v[i].freq_a = self.hz(p + self.v[i].off_a);
        self.v[i].freq_b = self.hz(self.kbd_b.mul_add(p, self.v[i].off_b));
        self.uni_note = note;
        self.v[i].velocity = velocity as f32;
    }

    /// `P5::triggerNoteOff(voice)`.
    pub(crate) fn trigger_note_off(&mut self, i: usize) {
        let note = self.notes[i] as f32;
        self.v[i].note = note;
        if self.unison == 0 {
            self.release_voice(i);
        } else if self.uni_note == note && self.voices != 0 {
            let mut count = 0;
            let mut best = 1000.0f32;
            for k in 0..self.voices as usize {
                if self.v[k].velocity > 0.0 {
                    let d = (note - self.v[k].note) as i32;
                    let db = (note - best) as i32;
                    count += 1;
                    if (d.wrapping_mul(d) as u32) < (db.wrapping_mul(db) as u32) && k != i {
                        best = self.v[k].note;
                    }
                }
            }
            if self.v[i].velocity > 0.0 {
                if count == 1 {
                    for k in 0..self.voices as usize {
                        self.release_voice(k);
                    }
                    self.uni_held = 0;
                    self.v[i].velocity = 0.0;
                    return;
                }
                if count > 1 {
                    self.last_note = best as i32;
                    self.uni_note = best;
                    let target = note_pitch(self.last_note);
                    self.glide_target = target;
                    if target != self.glide {
                        self.glide_step =
                            if self.glide < target { self.glide_rate * self.inv_sr } else { -(self.glide_rate * self.inv_sr) };
                    } else {
                        self.glide_step = 0.0;
                        for k in 0..self.voices as usize {
                            self.v[k].pitch = target + self.v[k].spread;
                            self.refresh_freqs(k);
                        }
                    }
                }
            }
        }
        self.v[i].velocity = 0.0;
    }

    /// Pedal / Hold up: release every voice released while it was down.
    pub(crate) fn sustained_notes_off(&mut self) {
        self.sustain = false;
        let mut i = 0;
        while i < self.voices as usize {
            if self.sustained[i] && !self.sostenuto[i] {
                self.sustained[i] = false;
                self.trigger_note_off(i);
                self.move_to_tail(i);
                self.notes[i] = -300 - self.notes[i];
            }
            i += 1;
        }
    }

    /// `P5::allSoundOff`: every voice reset (oscillator phases re-seeded from a fixed
    /// generator, so this is deterministic), the list rebuilt, bend centred; with Hold
    /// on, note 60 is latched again.
    pub fn all_sound_off(&mut self) {
        let mut r: i32 = 0;
        let cut = self.cutoff;
        for v in self.v.iter_mut() {
            v.velocity = 0.0;
            v.note = 255.0;
            r = r.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
            let a = r as f32 * 4.656613e-10;
            r = r.wrapping_mul(0x41c6_4e6d).wrapping_add(0x3039);
            let b = r as f32 * 4.656613e-10;
            v.phase_a = a;
            v.phase_b = b;
            v.prev_a = a;
            v.state_a = (0.0 <= a) as i32;
            v.prev_b = b;
            v.state_b = (0.0 <= b) as i32;
            v.corner_b = if b <= 0.0 { 0.0 } else { 2.0 };
            v.freq_a_d = [0.0; 2];
            v.inc_a = 0.0;
            v.ring_b = [0.0; 4];
            v.ring_bm = [0.0; 4];
            v.ring_a = [0.0; 4];
            v.cut_d = [0.0; 2];
            v.cutm_d = [0.0; 2];
            v.sync_d = [-1e20; 2];
            v.amp.rem = 0.0;
            v.amp.value = 0.0;
            v.amp.floor = 0.0;
            v.flt.value = 0.0;
            v.active = 0;
            v.out_b = [0.0; 2];
            v.env_d = [0.0; 2];
            v.z1 = [0.0; 2];
            v.z2 = [0.0; 2];
            v.amp_d = [0.0; 4];
            v.cut = cut + v.det_cut;
        }
        self.uni_note = 36.0;
        self.reset_bend();
        self.wm_a = [0.0; 2];
        self.pwm_a = [0.0; 2];
        for i in 0..SLOTS {
            self.sustained[i] = false;
            self.sostenuto[i] = false;
            self.trigger_note_off(i);
            self.notes[i] = NO_NOTE;
        }
        self.rebuild_list();
        self.sustain = false;
        if self.hold != 0 {
            self.relatch();
        }
    }

    /// `initVoiceQueue` (all notes off, CC 123–127): voices keep sounding their release.
    #[allow(dead_code)]
    pub fn init_voice_queue(&mut self) {
        for i in 0..SLOTS {
            self.sustained[i] = false;
            self.sostenuto[i] = false;
            self.trigger_note_off(i);
            self.notes[i] = -300;
        }
        self.rebuild_list();
        self.sustain = false;
        if self.hold != 0 {
            self.relatch();
        }
    }

    /// The allocation list in order 0, 1, … `voices − 1`, all free.
    fn rebuild_list(&mut self) {
        for k in 0..SLOTS {
            self.next[k] = k as i32 + 1;
            self.prev[k] = k as i32 - 1;
        }
        let last = self.voices as i32 - 1;
        self.prev[0] = last;
        if last >= 0 {
            self.next[last as usize] = 0;
        }
        self.tail = last;
        self.head = 0;
    }

    /// Hold on after a reset: note 60 plays again, latched.
    fn relatch(&mut self) {
        if let Some(v) = self.allocate_voice(0, 60) {
            self.notes[v] = 60;
            self.trigger_note_on(v, 64);
            if self.unison != 0 {
                self.last_note = 60;
            }
            self.sustained[v] = false;
            self.sostenuto[v] = false;
        }
        self.sustain = true;
        self.note_off(60);
    }
}
