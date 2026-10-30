//! MIDI controllers driven by scripts (`deckard_surface::device`; the Launchpad Pro
//! [MK3] is the first): a device opens when every port its `device.lua` names is there
//! (in and out), its input goes to its script on the UI thread, what the script sends
//! goes out, and it closes when a port goes. Ports a device takes exclusively aren't
//! note inputs.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use deckard_surface::View;
use deckard_surface::device::{Device, DeviceManifest, Out, devices};

/// An open device: its script and its ports by role.
struct Open {
    device: Device,
    manifest: DeviceManifest,
    /// (role, input port's name, the connection).
    ins: Vec<(String, String, midir::MidiInputConnection<()>)>,
    /// (role, output port's name, the connection).
    outs: Vec<(String, String, midir::MidiOutputConnection)>,
    rx: Receiver<(String, Vec<u8>)>,
}

#[derive(Default)]
pub struct MidiDevices {
    /// The user's controller folder (their own devices, and their copies of DECKARD's).
    user: Option<PathBuf>,
    manifests: Vec<DeviceManifest>,
    open: Vec<Open>,
    /// The port lists last synced with.
    ports: Option<(Vec<String>, Vec<String>)>,
    /// What to wake when bytes come.
    repaint: Option<eframe::egui::Context>,
    /// Made by [`MidiDevices::new`] (the app's): a default one (tests') opens nothing.
    enabled: bool,
    /// Only these devices (a test's own, so it never opens real hardware).
    only: Option<Vec<String>>,
    /// A device's ports were there but didn't connect: try again then.
    retry_at: Option<Instant>,
}

/// How long to wait before trying a device's ports again.
const RETRY: std::time::Duration = std::time::Duration::from_secs(2);

/// The port in `names` that is role `fragments` (its name contains one of them).
fn find<'a>(names: &'a [String], fragments: &[String]) -> Option<&'a String> {
    names.iter().find(|n| fragments.iter().any(|f| n.contains(f.as_str())))
}

/// Port names DECKARD plays notes from: those not switched off, and not a device's own.
pub fn note_inputs(all: &[String], off: &std::collections::BTreeSet<String>, devices: &MidiDevices) -> Vec<String> {
    all.iter().filter(|n| !off.contains(&format!("in:{n}")) && !devices.exclusive(n)).cloned().collect()
}

impl MidiDevices {
    pub fn new(user: Option<PathBuf>) -> MidiDevices {
        let manifests = devices(user.as_deref());
        MidiDevices { user, manifests, enabled: true, ..Default::default() }
    }

    /// Only devices `ids` (from `user`'s folder or DECKARD's).
    #[cfg(test)]
    pub fn only(user: Option<PathBuf>, ids: &[&str]) -> MidiDevices {
        let only: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
        let manifests = devices(user.as_deref()).into_iter().filter(|m| only.contains(&m.id)).collect();
        MidiDevices { user, manifests, enabled: true, only: Some(only), ..Default::default() }
    }

    pub fn set_context(&mut self, ctx: &eframe::egui::Context) {
        self.repaint = Some(ctx.clone());
    }

    /// Whether a device is open.
    pub fn active(&self) -> bool {
        !self.open.is_empty()
    }

    /// The names of the devices open.
    /// The devices there are (none in tests unless asked for).
    pub fn manifests(&self) -> &[DeviceManifest] {
        &self.manifests
    }

    pub fn names(&self) -> Vec<String> {
        self.open.iter().map(|o| o.manifest.name.clone()).collect()
    }

    /// Whether input port `name` belongs to a device exclusively (open or not, so its
    /// notes never play while it opens).
    pub fn exclusive(&self, name: &str) -> bool {
        self.manifests.iter().any(|m| {
            m.ports.iter().any(|(role, fragments)| m.exclusive.contains(role) && fragments.iter().any(|f| name.contains(f.as_str())))
        })
    }

    /// Whether the port lists differ from the last sync's.
    pub fn ports_changed(&self, ins: &[String], outs: &[String]) -> bool {
        self.ports.as_ref().is_none_or(|(i, o)| i != ins || o != outs) || self.retry_at.is_some_and(|t| Instant::now() >= t)
    }

    /// Open the devices whose ports came, close those whose ports went: what their
    /// scripts asked for (the MIDI already sent).
    pub fn sync(&mut self, ins: &[String], outs: &[String], v: &View, now: Instant) -> Vec<Out> {
        if !self.enabled || !self.ports_changed(ins, outs) {
            return Vec::new();
        }
        self.ports = Some((ins.to_vec(), outs.to_vec()));
        self.retry_at = None;
        self.manifests = devices(self.user.as_deref());
        if let Some(only) = &self.only {
            self.manifests.retain(|m| only.contains(&m.id));
        }
        let mut out = Vec::new();
        // Gone: a port of theirs went.
        let mut i = 0;
        while i < self.open.len() {
            let o = &self.open[i];
            let there = o.ins.iter().all(|(_, n, _)| ins.contains(n)) && o.outs.iter().all(|(_, n, _)| outs.contains(n));
            if there {
                i += 1;
            } else {
                let mut o = self.open.remove(i);
                // (Its ports are gone: what `close` sends goes nowhere.)
                out.extend(o.device.close().into_iter().filter(|x| !matches!(x, Out::Midi { .. })));
            }
        }
        for m in self.manifests.clone() {
            if self.open.iter().any(|o| o.manifest.id == m.id) || m.ports.is_empty() {
                continue;
            }
            let found: Option<Vec<(String, String, String)>> = m
                .ports
                .iter()
                .map(|(role, fragments)| Some((role.clone(), find(ins, fragments)?.clone(), find(outs, fragments)?.clone())))
                .collect();
            let Some(found) = found else { continue };
            match self.connect(m, &found) {
                Some(o) => {
                    self.open.push(o);
                    let last = self.open.len() - 1;
                    let asked = self.open[last].device.open(v, now);
                    out.extend(self.route(last, asked));
                }
                None => self.retry_at = Some(now + RETRY),
            }
        }
        out
    }

    /// Connect device `m`'s ports: (role, input name, output name) each.
    fn connect(&self, m: DeviceManifest, ports: &[(String, String, String)]) -> Option<Open> {
        let (tx, rx) = channel();
        let mut ins = Vec::new();
        let mut outs = Vec::new();
        for (role, in_name, out_name) in ports {
            let mut input = midir::MidiInput::new("DECKARD device").ok()?;
            // (SysEx too: the Launchpad's layout changes and prints.)
            input.ignore(midir::Ignore::Time);
            let port = input.ports().into_iter().find(|p| input.port_name(p).is_ok_and(|n| &n == in_name))?;
            let (tx, role_in, repaint): (Sender<_>, String, _) = (tx.clone(), role.clone(), self.repaint.clone());
            let conn = input
                .connect(
                    &port,
                    "DECKARD device in",
                    move |_, bytes, _| {
                        let _ = tx.send((role_in.clone(), bytes.to_vec()));
                        if let Some(ctx) = &repaint {
                            ctx.request_repaint();
                        }
                    },
                    (),
                )
                .ok()?;
            ins.push((role.clone(), in_name.clone(), conn));
            let output = midir::MidiOutput::new("DECKARD device").ok()?;
            let port = output.ports().into_iter().find(|p| output.port_name(p).is_ok_and(|n| &n == out_name))?;
            outs.push((role.clone(), out_name.clone(), output.connect(&port, "DECKARD device out").ok()?));
        }
        let device = Device::new(&m.id, self.user.clone());
        Some(Open { device, manifest: m, ins, outs, rx })
    }

    /// Send device `i`'s MIDI; the rest for the app.
    fn route(&mut self, i: usize, asked: Vec<Out>) -> Vec<Out> {
        let o = &mut self.open[i];
        let mut rest = Vec::new();
        for a in asked {
            match a {
                Out::Midi { role, bytes } => {
                    if let Some((_, _, conn)) = o.outs.iter_mut().find(|(r, _, _)| *r == role) {
                        let _ = conn.send(&bytes);
                    }
                }
                other => rest.push(other),
            }
        }
        rest
    }

    /// Each open device: its edited scripts reloaded, the bytes that came, its timer.
    pub fn poll(&mut self, v: &View, now: Instant) -> Vec<Out> {
        let mut out = Vec::new();
        for i in 0..self.open.len() {
            let mut asked = self.open[i].device.poll(now);
            while let Ok((role, bytes)) = self.open[i].rx.try_recv() {
                asked.extend(self.open[i].device.midi(&role, &bytes, v, now));
            }
            asked.extend(self.open[i].device.tick(v, now));
            out.extend(self.route(i, asked));
        }
        out
    }

    /// The open devices' script errors.
    pub fn errors(&self) -> Vec<String> {
        self.open.iter().filter_map(|o| o.device.error()).collect()
    }

    /// Close every device (the app quits): each back to its own mode.
    pub fn close_all(&mut self) {
        for i in 0..self.open.len() {
            let asked = self.open[i].device.close();
            self.route(i, asked);
        }
        self.open.clear();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use midir::os::unix::{VirtualInput, VirtualOutput};
    use std::sync::{Arc, Mutex};

    /// A user folder with device `test` on ports named `port`; its script answers a
    /// note with a `FocusScene` and a byte back, and says hello on opening. The guard
    /// removes it when dropped.
    fn folder(port: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join("devices/test")).unwrap();
        std::fs::write(
            dir.join("devices/test/device.lua"),
            format!(r#"return {{ name = "Test", ports = {{ daw = {{ "{port}" }} }}, exclusive = {{ "daw" }} }}"#),
        )
        .unwrap();
        std::fs::write(
            dir.join("devices/test/main.lua"),
            "return { open = function() deckard.midi_send('daw', { 0xB0, 1, 2 }) end,
                      midi = function(role, b) if b[1] == 0x90 then deckard.focus_scene(b[2]) end end }",
        )
        .unwrap();
        (tmp, dir)
    }

    fn ports() -> (Vec<String>, Vec<String>) {
        let ins = midir::MidiInput::new("t").unwrap();
        let outs = midir::MidiOutput::new("t").unwrap();
        (
            ins.ports().iter().filter_map(|p| ins.port_name(p).ok()).collect(),
            outs.ports().iter().filter_map(|p| outs.port_name(p).ok()).collect(),
        )
    }

    /// Wait (up to a second) for `f`.
    fn wait(mut f: impl FnMut() -> bool) -> bool {
        let t = Instant::now();
        while t.elapsed().as_millis() < 1000 {
            if f() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_device_opens_runs_and_closes_with_its_ports() {
        let port = format!("DECKARD Test DAW {}", std::process::id());
        let (_tmp, dir) = folder(&port);
        // The device: what DECKARD sends it, and a way to send DECKARD bytes.
        let got = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let g2 = got.clone();
        let Ok(dev_in) = midir::MidiInput::new("dev") else { return };
        let Ok(dev_in) = dev_in.create_virtual(&port, move |_, b, _| g2.lock().unwrap().push(b.to_vec()), ()) else { return };
        let Ok(Ok(mut dev_out)) = midir::MidiOutput::new("dev").map(|o| o.create_virtual(&port)) else { return };
        if !wait(|| ports().0.contains(&port) && ports().1.contains(&port)) {
            return; // (No virtual ports here.)
        }
        let mut d = MidiDevices::only(Some(dir), &["test"]);
        assert!(d.exclusive(&port));
        assert_eq!(note_inputs(&[port.clone(), "Keys".into()], &Default::default(), &d), ["Keys"]);
        let (ins, outs) = ports();
        let v = View::default();
        d.sync(&ins, &outs, &v, Instant::now());
        assert_eq!(d.names(), ["Test"]);
        assert!(wait(|| got.lock().unwrap().contains(&vec![0xB0, 1, 2])), "open's bytes reached the device");
        dev_out.send(&[0x90, 3, 100]).unwrap();
        let mut acts = Vec::new();
        assert!(wait(|| {
            acts.extend(d.poll(&v, Instant::now()));
            !acts.is_empty()
        }));
        assert_eq!(acts, [Out::App(deckard_surface::Action::FocusScene(2))]);
        // Unplugged: closed.
        drop(dev_out);
        drop(dev_in);
        assert!(wait(|| !ports().1.contains(&port)));
        let (ins, outs) = ports();
        d.sync(&ins, &outs, &v, Instant::now());
        assert!(!d.active());
    }
}
