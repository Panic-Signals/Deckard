//! Hosting Audio Units, with Apple's own: they're on every Mac. Run on the main thread,
//! where plug-ins load (no test harness).

#[cfg(target_os = "macos")]
fn main() {
    imp::an_effect_processes_and_keeps_its_settings();
    imp::modulation_isnt_saved();
    imp::an_instrument_plays();
    imp::a_missing_one_says_so();
    println!("au_host: ok");
    deckard_plugins::exit(0)
}

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
mod imp {
    use deckard_plugins::state::au_id;
    use deckard_plugins::{Host, MAX_BLOCK, Shared, Slot, Transport};

    const RATE: f64 = 48_000.0;
    const SLOT: Slot = Slot::Master { slot: 0 };

    fn code(s: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*s)
    }

    fn delay(host: &mut Host, slot: Slot, chunk: &[u8]) -> Shared {
        let id = au_id("Apple", code(b"aufx"), code(b"dely")).unwrap();
        host.instance(slot, &id, "AUDelay", chunk, RATE).expect("AUDelay loads")
    }

    fn param(p: &deckard_plugins::Plugin, title: &str) -> u32 {
        let list = p.parameters();
        list.iter().find(|i| i.title.to_lowercase().contains(title)).unwrap_or_else(|| panic!("{title} in {list:?}")).id
    }

    pub fn an_effect_processes_and_keeps_its_settings() {
        let mut host = Host::default();
        let shared = delay(&mut host, SLOT, &[]);
        let chunk = {
            let mut p = shared.lock().unwrap();
            let wet = param(&p, "dry/wet");
            let time = param(&p, "delay time");
            p.set_param(wet, 1.0);
            p.set_param(time, 0.01);
            assert!(p.take_dirty(), "a change is an edit");
            assert!(!p.take_dirty());
            assert!((p.param_value(wet) - 1.0).abs() < 1e-3);
            assert!(!p.param_text(wet, 1.0).is_empty());

            // All wet, a click comes out later than it went in.
            let mut click = vec![0f32; MAX_BLOCK];
            click[0] = 1.0;
            let silence = vec![0f32; MAX_BLOCK];
            let t = Transport { tempo: 120.0, numerator: 4, denominator: 4, ..Transport::default() };
            let (l, _) = p.process_input(MAX_BLOCK, &t, Some((&click, &click)));
            assert_eq!(l.len(), MAX_BLOCK);
            let mut heard = l.iter().skip(1).any(|s| s.abs() > 1e-4);
            assert!(l[0].abs() < 0.5, "wet only: the click isn't out at once ({})", l[0]);
            for _ in 0..200 {
                let (l, _) = p.process_input(MAX_BLOCK, &t, Some((&silence, &silence)));
                heard |= l.iter().any(|s| s.abs() > 1e-4);
            }
            assert!(heard, "the click comes out delayed");

            let chunk = p.save_chunk();
            assert_eq!(u32::from_le_bytes(chunk[4..8].try_into().unwrap()), 3, "an AU's chunk is its ClassInfo");
            assert!(chunk[12..].starts_with(b"<?xml"), "as XML");
            p.mark_saved(chunk.clone());
            chunk
        };

        // A new instance from the chunk has those settings.
        let other = delay(&mut host, Slot::Master { slot: 1 }, &chunk);
        let p = other.lock().unwrap();
        let wet = param(&p, "dry/wet");
        assert!((p.param_value(wet) - 1.0).abs() < 1e-3, "restored: {}", p.param_value(wet));

        // The handle reads and sets without the instance held.
        let h = p.params().expect("a handle");
        drop(p);
        h.set(wet, 0.25);
        assert!((h.value(wet) - 0.25).abs() < 1e-3);
    }

    pub fn modulation_isnt_saved() {
        let mut host = Host::default();
        let shared = delay(&mut host, SLOT, &[]);
        let mut p = shared.lock().unwrap();
        let wet = param(&p, "dry/wet");
        p.set_param(wet, 0.5);
        p.modulate(wet, 0.9);
        assert_eq!(p.modulated(wet), Some(0.9));
        assert!((p.param_value(wet) - 0.5).abs() < 1e-3, "the knob stays");
        let chunk = p.save_chunk();
        // Turning the knob while modulated moves the knob only.
        p.set_param(wet, 0.25);
        assert_eq!(p.modulated(wet), Some(0.9));
        p.unmodulate(wet);
        assert_eq!(p.modulated(wet), None);
        assert!((p.param_value(wet) - 0.25).abs() < 1e-3, "plays the knob again");
        p.load_chunk(&chunk).unwrap();
        assert!((p.param_value(wet) - 0.5).abs() < 1e-3, "saved the knob, not the modulation: {}", p.param_value(wet));
        // A state loaded while modulated is the knob from then on.
        p.modulate(wet, 0.9);
        p.load_chunk(&chunk).unwrap();
        assert_eq!(p.modulated(wet), Some(0.9), "still modulated");
        p.unmodulate(wet);
        assert!((p.param_value(wet) - 0.5).abs() < 1e-3, "the loaded knob, not the old one: {}", p.param_value(wet));
    }

    pub fn an_instrument_plays() {
        let mut host = Host::default();
        let id = au_id("Apple", code(b"aumu"), code(b"samp")).unwrap();
        let shared = host.instance(SLOT, &id, "AUSampler", &[], RATE).expect("AUSampler loads");
        let mut p = shared.lock().unwrap();
        assert!(p.takes_controller(1));
        let t = Transport { tempo: 120.0, playing: true, numerator: 4, denominator: 4, ..Transport::default() };
        p.note_on(60, 127);
        let mut heard = false;
        for _ in 0..50 {
            let (l, _) = p.process(MAX_BLOCK, &t);
            heard |= l.iter().any(|s| s.abs() > 1e-4);
        }
        assert!(heard, "a note sounds");
        p.all_notes_off();
        // It lets the host set how many outputs it has, up to 16 channels in all: it's
        // given 8 stereo pairs (as Kontakt's is given 16), each one there to read.
        assert_eq!(p.output_count(), 8);
        assert!((0..8).all(|b| p.output_bus(b, MAX_BLOCK).is_some_and(|(l, r)| l.len() == MAX_BLOCK && r.len() == MAX_BLOCK)));
    }

    pub fn a_missing_one_says_so() {
        let mut host = Host::default();
        let id = au_id("Nobody", code(b"aufx"), code(b"none")).unwrap();
        let e = host.instance(SLOT, &id, "Nothing", &[], RATE).err().expect("no such plug-in");
        assert!(e.contains("not installed"), "{e}");
    }
}
