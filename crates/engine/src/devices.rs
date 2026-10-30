//! The audio interfaces Preferences ▸ Audio lists, and opening the one picked.
//!
//! Core Audio and WASAPI have one device per interface, with all its channels. ALSA
//! has a dozen PCMs per sound card (`hw`, `plughw`, `dmix`, `surround51`…), all named
//! alike, and the raw `hw` PCM speaks only the card's own sample format (packed 24-bit
//! on a MOTU), which cpal can't open; the plug PCMs it can open offer any channel
//! count, so cpal's default config picks stereo. So on Linux the interfaces are the
//! sound cards themselves, each opened through its `plughw` PCM (ALSA converts the
//! format) with the channels its hardware has at the rate DECKARD asks for.

use cpal::traits::{DeviceTrait, HostTrait};

/// A device and the stream configuration to open it with.
pub type Opened = (cpal::Device, cpal::SupportedStreamConfig);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    In,
    Out,
}

pub fn device_name(d: &cpal::Device) -> String {
    d.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "Unknown device".into())
}

/// The audio interfaces with outputs: (name, output channels).
pub fn output_devices() -> Vec<(String, usize)> {
    devices(Dir::Out)
}

/// The audio interfaces with inputs: (name, input channels).
pub fn input_devices() -> Vec<(String, usize)> {
    devices(Dir::In)
}

/// The system's default output.
pub fn default_output() -> Option<String> {
    cpal::default_host().default_output_device().map(|d| device_name(&d))
}

/// The system's default input.
pub fn default_input() -> Option<String> {
    cpal::default_host().default_input_device().map(|d| device_name(&d))
}

/// The output device called `name` (see [`output_devices`]; `None`: the system's
/// default), with the configuration to open it with.
pub fn open_output(name: Option<&str>) -> Result<Opened, String> {
    open(name, Dir::Out)
}

/// The input device called `name` (see [`input_devices`]; `None`: the system's
/// default), with the configuration to open it with.
pub fn open_input(name: Option<&str>) -> Result<Opened, String> {
    open(name, Dir::In)
}

fn default_config(d: &cpal::Device, dir: Dir) -> Result<cpal::SupportedStreamConfig, String> {
    match dir {
        Dir::Out => d.default_output_config(),
        Dir::In => d.default_input_config(),
    }
    .map_err(|e| e.to_string())
}

fn default_device(host: &cpal::Host, dir: Dir) -> Result<cpal::Device, String> {
    match dir {
        Dir::Out => host.default_output_device().ok_or("no audio output device"),
        Dir::In => host.default_input_device().ok_or("no audio input"),
    }
    .map_err(String::from)
}

#[cfg(not(target_os = "linux"))]
fn all(host: &cpal::Host, dir: Dir) -> Result<Vec<cpal::Device>, String> {
    match dir {
        Dir::Out => host.output_devices().map(|ds| ds.collect()),
        Dir::In => host.input_devices().map(|ds| ds.collect()),
    }
    .map_err(|e| e.to_string())
}

#[cfg(not(target_os = "linux"))]
fn devices(dir: Dir) -> Vec<(String, usize)> {
    all(&cpal::default_host(), dir)
        .map(|ds| ds.iter().map(|d| (device_name(d), default_config(d, dir).map_or(0, |c| c.channels() as usize))).collect())
        .unwrap_or_default()
}

#[cfg(not(target_os = "linux"))]
fn open(name: Option<&str>, dir: Dir) -> Result<Opened, String> {
    let host = cpal::default_host();
    let device = match name {
        Some(name) => all(&host, dir)?.into_iter().find(|d| device_name(d) == name).ok_or_else(|| format!("\"{name}\" isn't connected"))?,
        None => default_device(&host, dir)?,
    };
    let config = default_config(&device, dir)?;
    Ok((device, config))
}

#[cfg(target_os = "linux")]
fn devices(dir: Dir) -> Vec<(String, usize)> {
    linux::cards(dir).into_iter().map(|c| (c.name, linux::hw_caps(&c.hw, dir).map_or(0, |(_, ch)| ch as usize))).collect()
}

#[cfg(target_os = "linux")]
fn open(name: Option<&str>, dir: Dir) -> Result<Opened, String> {
    let host = cpal::default_host();
    let Some(name) = name else {
        let device = default_device(&host, dir)?;
        let config = default_config(&device, dir)?;
        return Ok((device, config));
    };
    let card = linux::cards(dir).into_iter().find(|c| c.name == name).ok_or_else(|| format!("\"{name}\" isn't connected"))?;
    let (rate, channels) = linux::hw_caps(&card.hw, dir).map_err(|e| format!("\"{name}\": {e}"))?;
    let device = host
        .device_by_id(&cpal::DeviceId::new(cpal::HostId::Alsa, &card.pcm))
        .ok_or_else(|| format!("\"{name}\" isn't connected ({} is missing)", card.pcm))?;
    let ranges = match dir {
        Dir::Out => device.supported_output_configs().map(|r| r.collect::<Vec<_>>()),
        Dir::In => device.supported_input_configs().map(|r| r.collect::<Vec<_>>()),
    }
    .map_err(|e| format!("\"{name}\": {e}"))?;
    // All the hardware's channels, as f32 (the plug PCM converts), at its rate.
    let fits = |r: &cpal::SupportedStreamConfigRange| {
        r.sample_format() == cpal::SampleFormat::F32
            && r.channels() as u32 == channels
            && (r.min_sample_rate()..=r.max_sample_rate()).contains(&rate)
    };
    let config = match ranges.into_iter().find(fits) {
        Some(r) => r.with_sample_rate(rate),
        None => default_config(&device, dir)?,
    };
    Ok((device, config))
}

#[cfg(target_os = "linux")]
mod linux {
    use super::Dir;

    /// The rate asked of an interface (its nearest if it hasn't got it).
    const RATE: u32 = 48_000;

    /// A sound card's PCM: as ALSA names it ("UltraLite-mk5, USB Audio"), the plug PCM
    /// that opens it in any format, and the raw one that tells what it can do.
    pub struct Card {
        pub name: String,
        pub pcm: String,
        pub hw: String,
        /// ALSA's id for the card ("UltraLitemk5").
        pub id: String,
    }

    fn direction(dir: Dir) -> alsa::Direction {
        match dir {
            Dir::Out => alsa::Direction::Playback,
            Dir::In => alsa::Direction::Capture,
        }
    }

    /// The sound cards' PCMs that play (or capture), in card order.
    pub fn cards(dir: Dir) -> Vec<Card> {
        let mut cards = Vec::new();
        for card in alsa::card::Iter::new().filter_map(Result::ok) {
            let Ok(ctl) = alsa::Ctl::new(&format!("hw:{}", card.get_index()), false) else { continue };
            let Ok(info) = ctl.card_info() else { continue };
            let (Ok(id), Ok(card_name)) = (info.get_id(), info.get_name()) else { continue };
            for dev in alsa::ctl::DeviceIter::new(&ctl) {
                let Ok(pcm) = ctl.pcm_info(dev as u32, 0, direction(dir)) else { continue };
                let name = match pcm.get_name() {
                    Ok(n) if !n.is_empty() => format!("{card_name}, {n}"),
                    _ => card_name.to_string(),
                };
                cards.push(Card {
                    name,
                    pcm: format!("plughw:CARD={id},DEV={dev}"),
                    hw: format!("hw:CARD={id},DEV={dev}"),
                    id: id.to_string(),
                });
            }
        }
        // Two cards of a kind (two graphics cards' HDMI outputs): told apart by card.
        let names: Vec<String> = cards.iter().map(|c| c.name.clone()).collect();
        for c in &mut cards {
            if names.iter().filter(|n| **n == c.name).count() > 1 {
                c.name = format!("{} ({})", c.name, c.id);
            }
        }
        cards
    }

    /// What the raw PCM `hw` does: its rate nearest [`RATE`], and its channels at it.
    pub fn hw_caps(hw: &str, dir: Dir) -> Result<(u32, u32), String> {
        let busy = |e: &alsa::Error| std::io::Error::from_raw_os_error(e.errno()).kind() == std::io::ErrorKind::ResourceBusy;
        let pcm = alsa::pcm::PCM::new(hw, direction(dir), true)
            .map_err(|e| if busy(&e) { "in use by another program (the desktop's sound server?)".to_string() } else { e.to_string() })?;
        let params = alsa::pcm::HwParams::any(&pcm).map_err(|e| e.to_string())?;
        let rate = params.set_rate_near(RATE, alsa::ValueOr::Nearest).map_err(|e| e.to_string())?;
        let channels = params.get_channels_max().map_err(|e| e.to_string())?;
        Ok((rate, channels))
    }
}
