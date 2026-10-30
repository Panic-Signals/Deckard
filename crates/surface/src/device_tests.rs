use std::path::PathBuf;
use std::time::Instant;

use crate::Action;
use crate::device::{Device, Out, Print, PrintedNote, devices};
use crate::tests_support::view;

/// A fresh user folder with device `test`: its manifest and `main.lua`. The guard
/// removes it when dropped.
fn test_device(main: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    std::fs::create_dir_all(dir.join("devices/test")).unwrap();
    std::fs::write(
        dir.join("devices/test/device.lua"),
        r#"return { name = "Test", ports = { daw = { "Test DAW" } }, exclusive = { "daw" } }"#,
    )
    .unwrap();
    std::fs::write(dir.join("devices/test/main.lua"), main).unwrap();
    (tmp, dir)
}

fn midi(role: &str, bytes: &[u8]) -> Out {
    Out::Midi { role: role.into(), bytes: bytes.to_vec() }
}

#[test]
fn a_device_manifest_is_found() {
    let (_tmp, dir) = test_device("return {}");
    let found = devices(Some(&dir));
    let t = found.iter().find(|d| d.id == "test").expect("the test device");
    assert_eq!(t.name, "Test");
    assert_eq!(t.ports, [("daw".to_string(), vec!["Test DAW".to_string()])]);
    assert_eq!(t.exclusive, ["daw"]);
    assert!(found.iter().all(|d| d.id != "mk3"));
}

#[test]
fn midi_in_reaches_the_script_and_sends_back() {
    let (_tmp, dir) = test_device("return { midi = function(role, b) deckard.midi_send(role, { 0x90, b[2], 5 }) end }");
    let mut d = Device::new("test", Some(dir));
    assert_eq!(d.error(), None);
    assert_eq!(d.midi("daw", &[0x90, 11, 127], &view(), Instant::now()), [midi("daw", &[0x90, 11, 5])]);
}

#[test]
fn open_and_close_run() {
    let (_tmp, dir) = test_device(
        "return { open = function() deckard.midi_send('daw', { 0xF0, 0, 0x20, 0x29, 2, 0x0E, 0x10, 1, 0xF7 }) end,
                  close = function() deckard.midi_send('daw', { 0xF0, 0, 0x20, 0x29, 2, 0x0E, 0x10, 0, 0xF7 }) end }",
    );
    let mut d = Device::new("test", Some(dir));
    assert_eq!(d.open(&view(), Instant::now()), [midi("daw", &[0xF0, 0, 0x20, 0x29, 2, 0x0E, 0x10, 1, 0xF7])]);
    assert_eq!(d.close(), [midi("daw", &[0xF0, 0, 0x20, 0x29, 2, 0x0E, 0x10, 0, 0xF7])]);
}

#[test]
fn a_device_never_loads_mk3_pages() {
    let (_tmp, dir) = test_device(
        "return { open = function()
             deckard.message(tostring(pcall(require, 'pages.control')) .. ' ' .. tostring(pcall(require, 'lib.util')))
         end }",
    );
    let mut d = Device::new("test", Some(dir));
    assert_eq!(d.open(&view(), Instant::now()), [Out::Message("false true".into())]);
}

#[test]
fn launch_and_print_actions() {
    let (_tmp, dir) = test_device(
        "return { tick = function()
             deckard.launch_scene(2)
             deckard.print_notes { length = 4, notes = { { 60, 0.5, 0.25, 100 } } }
         end }",
    );
    let mut d = Device::new("test", Some(dir));
    let out = d.tick(&view(), Instant::now());
    assert!(out.contains(&Out::App(Action::LaunchScene(1))), "{out:?}");
    assert!(
        out.contains(&Out::Print(Print {
            length: 4.0,
            notes: vec![PrintedNote { pitch: 60, start: 0.5, length: 0.25, velocity: 100 }],
            track: None,
            device: "test".into(),
        })),
        "{out:?}"
    );
}

#[test]
fn the_device_sees_the_view() {
    let (_tmp, dir) =
        test_device("return { tick = function() deckard.message(deckard.state().groups[2].name .. ' ' .. deckard.device()) end }");
    let mut d = Device::new("test", Some(dir));
    assert_eq!(d.tick(&view(), Instant::now()), [Out::Message("Bass test".into())]);
}

#[test]
fn a_failing_device_script_reports_and_keeps_the_last() {
    let (_tmp, dir) = test_device("return { tick = function() deckard.message('old') end }");
    let mut d = Device::new("test", Some(dir.clone()));
    std::fs::write(dir.join("devices/test/main.lua"), "return { tick = function( end }").unwrap();
    assert!(!d.reload());
    assert!(d.error().is_some());
    assert_eq!(d.tick(&view(), Instant::now()), [Out::Message("old".into())]);
}

#[test]
fn a_runtime_error_is_reported() {
    let (_tmp, dir) = test_device("return { midi = function() error('boom') end }");
    let mut d = Device::new("test", Some(dir));
    assert_eq!(d.midi("daw", &[0x90, 1, 1], &view(), Instant::now()), []);
    assert!(d.error().is_some_and(|e| e.contains("boom")), "{:?}", d.error());
}

#[test]
fn only_the_devices_own_files_reload_it() {
    let (_tmp, dir) = test_device("return { open = function() deckard.message('opened') end }");
    let mut d = Device::new("test", Some(dir.clone()));
    d.open(&view(), Instant::now());
    std::fs::create_dir_all(dir.join("pages")).unwrap();
    std::fs::write(dir.join("pages/control.lua"), "return {}").unwrap();
    assert!(!d.reload(), "an MK3 page isn't the device's");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("lib/mine.lua"), "return {}").unwrap();
    assert!(d.reload(), "lib/ is shared");
}
