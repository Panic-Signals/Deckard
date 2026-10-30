//! The MK3's Settings ▸ MIDI page: MIDI sync (mode, clock offset), the MIDI devices,
//! and Link (enabled, start/stop sync, peers, phase), through NI's scripts.
//!
//! Needs the `ni-res-extract` output in `$DECKARD_SCRIPTS` (default
//! `<workspace>/out/ni-res`); skipped when it's missing.

use deckard_lua_host::state::SyncView;
use deckard_lua_host::{Host, Scripts, State};
use std::path::PathBuf;

fn boot() -> Option<Host> {
    let dir = std::env::var_os("DECKARD_SCRIPTS")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/ni-res"), PathBuf::from);
    let Some(scripts) = Scripts::from_dir(&dir).ok().filter(|s| !s.is_empty()) else {
        eprintln!("no scripts under {}; skipping", dir.display());
        return None;
    };
    let doc = deckard_doc::Document::new_project().expect("a new project").shared();
    let mut host = Host::new(scripts, State::new("M3", Some(doc))).expect("host");
    host.boot().expect("boot");
    host.tick();
    Some(host)
}

/// SETTINGS, then its third tab: MIDI (straight there: the Audio tab between isn't
/// this phase's).
fn midi_page(host: &mut Host) {
    host.press("BUTTON_SETTINGS").unwrap();
    host.tick();
    host.eval_lua("ControllerScriptInterface:getPage(NI.HW.PAGE_SETTINGS):selectTab(3)");
    host.tick();
}

/// Texts of the display's widgets of `kind` (their labels' texts).
fn texts(host: &Host, kind: &str) -> Vec<String> {
    let st = host.state.borrow();
    let w = &st.widgets.all;
    w.iter()
        .enumerate()
        .filter(|(_, x)| x.kind == kind)
        .flat_map(|(id, _)| w.iter().filter(move |c| c.parent == Some(id)))
        .filter_map(|c| match c.props.get("Text") {
            Some(deckard_lua_host::state::Prop::Str(s)) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

const SYNC_CALLS: &[&str] = &[
    "getMidiSyncStatus",
    "setMidiSyncStatus",
    "getCurrentClockOffset",
    "setClockOffset",
    "getMidiClockSyncOffset",
    "setMidiClockSyncOffset",
    "getMidiDevicesCount",
    "getMidiDeviceString",
    "getMidiDeviceState",
    "toggleMidiDevice",
    "getDefaultMidiInput",
    "insertLinkStatus",
    "insertLinkPhaseMeter",
    "getLinkEnabledParameter",
    "getLinkStartStopSyncEnabledParameter",
    "storeSetup",
    "updateMidiDevices",
];

/// Page 1: the sync mode, the clock offset and the devices as the app has them;
/// turning the knobs changes them for the app. Nothing stubbed, no errors.
#[test]
fn mk3_midi_page_sets_the_sync_mode_offset_and_devices() {
    let Some(mut host) = boot() else { return };
    host.set_sync(SyncView {
        midi_sync: 1,
        send_offset: 0,
        receive_offset: 100,
        inputs: vec![("Keys".into(), true), ("Drum Machine".into(), false)],
        outputs: vec![("Synth".into(), true)],
        default_focus: true,
        ..SyncView::default()
    });
    midi_page(&mut host);
    for _ in 0..2 {
        host.tick();
    }
    assert!(host.errors.is_empty(), "{:?}", host.errors);
    let stubs: Vec<String> = host.report().into_iter().map(|(k, ..)| k).filter(|k| SYNC_CALLS.iter().any(|c| k.ends_with(c))).collect();
    assert!(stubs.is_empty(), "{stubs:?}");
    // MODE: Send → Receive.
    host.encoder("ENCODER_SCREEN_1", 0.5).unwrap();
    host.tick();
    assert_eq!(host.state.borrow().sync.midi_sync, 2);
    // CLOCK OFFSET (receiving): 100 (0 ms) up.
    host.encoder("ENCODER_SCREEN_2", 0.5).unwrap();
    host.tick();
    assert!(host.state.borrow().sync.receive_offset > 100);
    // The second input's STATUS: Off → On.
    host.encoder("ENCODER_SCREEN_5", 0.5).unwrap();
    host.tick();
    host.encoder("ENCODER_SCREEN_6", 0.5).unwrap();
    host.tick();
    assert_eq!(host.state.borrow().sync.inputs[1], ("Drum Machine".into(), true));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The Link status and phase widgets show the app's Link; Link's parameters (page 2)
/// switch it for the app; with Link on, Receive isn't offered.
#[test]
fn mk3_midi_page_shows_and_switches_link() {
    let Some(mut host) = boot() else { return };
    host.set_sync(SyncView { link: true, peers: 2, phase: 0.5, ..SyncView::default() });
    midi_page(&mut host);
    host.tick();
    assert_eq!(texts(&host, "HWLinkStatus"), ["2 LINKS"]);
    host.set_sync(SyncView { link: false, ..SyncView::default() });
    host.tick();
    assert_eq!(texts(&host, "HWLinkStatus"), ["LINK OFF"]);
    // With Link on the mode stops at Send.
    host.set_sync(SyncView { link: true, midi_sync: 1, ..SyncView::default() });
    host.encoder("ENCODER_SCREEN_1", 0.5).unwrap();
    host.tick();
    assert_eq!(host.state.borrow().sync.midi_sync, 1);
    // Page 2: Link ENABLED and START SYNC on knobs 5 and 6.
    host.eval_lua(
        "local P = ControllerScriptInterface:getPage(NI.HW.PAGE_SETTINGS); P.ParameterHandler:onPrevNextParameterPage(1); \
         P:updateScreens(true)",
    );
    host.tick();
    let names: Vec<String> = {
        let st = host.state.borrow();
        st.widgets
            .all
            .iter()
            .filter(|w| w.name == "m_pParamNameWidget")
            .filter_map(|w| match w.props.get("Text") {
                Some(deckard_lua_host::state::Prop::Str(s)) if !s.is_empty() => Some(s.clone()),
                _ => None,
            })
            .collect()
    };
    assert!(names.iter().any(|n| n == "ENABLED") && names.iter().any(|n| n == "START SYNC"), "{names:?}");
    host.encoder("ENCODER_SCREEN_5", -0.5).unwrap();
    host.tick();
    assert!(!host.state.borrow().sync.link, "knob 5 left Link");
    // Link's parameters, as the scripts' Workspace has them.
    host.eval_lua(
        "local W = App:getWorkspace(); NI.DATA.ParameterAccess.setBoolParameter(App, W:getLinkEnabledParameter(), false); \
         NI.DATA.ParameterAccess.setBoolParameter(App, W:getLinkStartStopSyncEnabledParameter(), true)",
    );
    let s = host.state.borrow().sync.clone();
    assert_eq!((s.link, s.link_start_stop), (false, true));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
}

/// The MIDI tab's side shows Link: the status under the icon, and the phase meter
/// filling over the bar (drawn: the right half of the right display lights up).
#[test]
fn mk3_midi_page_draws_link() {
    let Some(mut host) = boot() else { return };
    host.set_sync(SyncView { link: true, peers: 1, phase: 0.5, ..SyncView::default() });
    midi_page(&mut host);
    host.tick();
    let (page, _) = host.display().unwrap();
    fn find<'a>(e: &'a deckard_nwl::draw::Element, kind: &str) -> Option<&'a deckard_nwl::draw::Element> {
        if e.kind == kind {
            return Some(e);
        }
        // A stack: only what it shows.
        match (e.kind.as_str(), e.top) {
            ("Stack", Some(t)) => e.children.get(t).and_then(|c| find(c, kind)),
            _ => e.children.iter().find_map(|c| find(c, kind)),
        }
    }
    let status = find(&page, "HWLinkStatus").expect("the MIDI tab's Link status is shown");
    assert_eq!(status.children[0].text.as_deref(), Some("1 LINK"));
    let meter = find(&page, "HWLinkPhaseMeter").expect("and its phase meter");
    assert_eq!(meter.children[0].children[0].width, Some(72), "half the bar");
    let dir = std::env::var_os("DECKARD_SCRIPTS")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/ni-res"), PathBuf::from);
    let Ok(look) = deckard_nwl::draw::Look::mk3(&dir) else { return };
    let canvas = look.render(&page, 960, 272);
    if let Some(out) = std::env::var_os("DECKARD_SYNC_PNG") {
        canvas.write_png(std::path::Path::new(&out)).unwrap();
    }
    let lit = (720..960).flat_map(|x| (60..200).map(move |y| (x, y))).filter(|&(x, y)| {
        let p = canvas.px[y * canvas.width + x];
        p[0] as u32 + p[1] as u32 + p[2] as u32 > 300
    });
    assert!(lit.count() > 50, "the status and the meter are drawn");
}
