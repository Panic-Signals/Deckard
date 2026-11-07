//! The audio interfaces Preferences ▸ Audio lists, and opening the one picked.
//!
//! Core Audio and WASAPI have one device per interface, with all its channels. ALSA
//! has a dozen PCMs per sound card (`hw`, `plughw`, `dmix`, `surround51`…), all named
//! alike, and the raw `hw` PCM speaks only the card's own sample format (packed 24-bit
//! on a MOTU), which cpal can't open; the plug PCMs it can open offer any channel
//! count, so cpal's default config picks stereo. So on Linux the interfaces are the
//! sound cards themselves, each opened through its `plughw` PCM (ALSA converts the
//! format) with the channels its hardware has at the rate DECKARD asks for.
//!
//! On Windows, a build with the `asio` feature can also play through an ASIO driver
//! (see [`AudioDriver`] and the `asio` module below).

use cpal::traits::{DeviceTrait, HostTrait};
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

/// A device and the stream configuration to open it with.
pub type Opened = (cpal::Device, cpal::SupportedStreamConfig);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    In,
    Out,
}

/// The audio system DECKARD plays through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioDriver {
    /// The system's own: Core Audio, WASAPI or ALSA.
    #[default]
    System,
    /// An ASIO driver (Windows, in a build with the `asio` feature).
    Asio,
}

/// Whether this build can play through ASIO.
pub const ASIO_BUILT: bool = cfg!(all(windows, feature = "asio"));

impl AudioDriver {
    pub fn name(self) -> &'static str {
        match self {
            AudioDriver::Asio => "ASIO",
            AudioDriver::System if cfg!(target_os = "macos") => "CoreAudio",
            AudioDriver::System if cfg!(windows) => "WASAPI",
            AudioDriver::System => "ALSA",
        }
    }

    /// This driver, if this build has it; else the system's own.
    pub fn usable(self) -> AudioDriver {
        if ASIO_BUILT { self } else { AudioDriver::System }
    }
}

/// The driver streams are opened through (see [`set_driver`]).
static IN_USE: AtomicU8 = AtomicU8::new(0);

/// The driver streams are opened through now: what plays, when something does.
pub fn driver() -> AudioDriver {
    if IN_USE.load(Ordering::Relaxed) == 1 { AudioDriver::Asio } else { AudioDriver::System }
}

/// Open streams through `driver` from now on (the system's own if this build hasn't
/// got it).
pub fn set_driver(driver: AudioDriver) {
    IN_USE.store((driver.usable() == AudioDriver::Asio) as u8, Ordering::Relaxed);
}

/// The one place the audio system is chosen.
fn host(driver: AudioDriver) -> Result<cpal::Host, String> {
    #[cfg(all(windows, feature = "asio"))]
    if driver == AudioDriver::Asio {
        return cpal::host_from_id(cpal::HostId::Asio).map_err(|e| e.to_string());
    }
    let _ = driver;
    Ok(cpal::default_host())
}

/// Open something through `wanted`. If that's ASIO and it can't be had, the system's own
/// driver takes over, with why ASIO couldn't be had: (what opened, why).
pub fn open_preferring<T>(wanted: AudioDriver, open: impl FnMut(AudioDriver) -> Result<T, String>) -> (Result<T, String>, Option<String>) {
    open_preferring_in(ASIO_BUILT, wanted, open)
}

fn open_preferring_in<T>(
    built: bool,
    wanted: AudioDriver,
    mut open: impl FnMut(AudioDriver) -> Result<T, String>,
) -> (Result<T, String>, Option<String>) {
    match wanted {
        AudioDriver::System => (open(AudioDriver::System), None),
        AudioDriver::Asio if !built => (open(AudioDriver::System), Some("this build of DECKARD has no ASIO".into())),
        AudioDriver::Asio => match open(AudioDriver::Asio) {
            Ok(opened) => (Ok(opened), None),
            Err(why) => (open(AudioDriver::System), Some(why)),
        },
    }
}

/// Look at the ASIO drivers again. It loads each one in turn, so it's only done when
/// asked (a stream open on one stops the others loading; the last list is kept then).
pub fn rescan() {
    asio::rescan();
}

pub fn device_name(d: &cpal::Device) -> String {
    d.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "Unknown device".into())
}

/// The audio interfaces with outputs: (name, output channels).
pub fn output_devices() -> Vec<(String, usize)> {
    output_devices_of(driver())
}

/// The audio interfaces with inputs: (name, input channels).
pub fn input_devices() -> Vec<(String, usize)> {
    input_devices_of(driver())
}

/// What [`output_devices`] lists for `driver`.
pub fn output_devices_of(driver: AudioDriver) -> Vec<(String, usize)> {
    if driver.usable() == AudioDriver::Asio { asio::list(Dir::Out) } else { devices(Dir::Out) }
}

/// What [`input_devices`] lists for `driver`.
pub fn input_devices_of(driver: AudioDriver) -> Vec<(String, usize)> {
    if driver.usable() == AudioDriver::Asio { asio::list(Dir::In) } else { devices(Dir::In) }
}

/// The system's default output (ASIO has no default).
pub fn default_output() -> Option<String> {
    match driver() {
        AudioDriver::Asio => None,
        AudioDriver::System => host(AudioDriver::System).ok()?.default_output_device().map(|d| device_name(&d)),
    }
}

/// The system's default input (ASIO has no default).
pub fn default_input() -> Option<String> {
    match driver() {
        AudioDriver::Asio => None,
        AudioDriver::System => host(AudioDriver::System).ok()?.default_input_device().map(|d| device_name(&d)),
    }
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
    host(AudioDriver::System)
        .and_then(|host| all(&host, dir))
        .map(|ds| ds.iter().map(|d| (device_name(d), default_config(d, dir).map_or(0, |c| c.channels() as usize))).collect())
        .unwrap_or_default()
}

#[cfg(not(target_os = "linux"))]
fn open(name: Option<&str>, dir: Dir) -> Result<Opened, String> {
    if driver() == AudioDriver::Asio {
        return asio::open(name, dir);
    }
    let host = host(AudioDriver::System)?;
    let device = match name {
        Some(name) => all(&host, dir)?.into_iter().find(|d| device_name(d) == name).ok_or_else(|| format!("\"{name}\" isn't connected"))?,
        None => default_device(&host, dir)?,
    };
    match default_config(&device, dir) {
        Ok(config) => Ok((device, config)),
        // No input was picked and the system's default won't say what format it runs (a USB
        // microphone can refuse): any input that answers serves. Only for input: output
        // moving to another device unasked could play somewhere it shouldn't.
        Err(e) if name.is_none() && dir == Dir::In => {
            first_usable(all(&host, dir)?, |d| default_config(d, dir)).ok_or_else(|| format!("{}: {e}", device_name(&device)))
        }
        Err(e) => Err(e),
    }
}

/// The first of `devices` that `config` answers for, with its answer.
#[cfg(not(target_os = "linux"))]
fn first_usable<D, C>(devices: impl IntoIterator<Item = D>, config: impl Fn(&D) -> Result<C, String>) -> Option<(D, C)> {
    devices.into_iter().find_map(|d| config(&d).ok().map(|c| (d, c)))
}

#[cfg(target_os = "linux")]
fn devices(dir: Dir) -> Vec<(String, usize)> {
    linux::cards(dir).into_iter().map(|c| (c.name, linux::hw_caps(&c.hw, dir).map_or(0, |(_, ch)| ch as usize))).collect()
}

#[cfg(target_os = "linux")]
fn open(name: Option<&str>, dir: Dir) -> Result<Opened, String> {
    let host = host(AudioDriver::System)?;
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

/// Whether streams can be opened in `format` through the driver in use: f32 for the
/// system's, the formats ASIO drivers ask for (converted here) for ASIO.
pub fn format_ok(format: cpal::SampleFormat) -> bool {
    use cpal::SampleFormat::{F32, I16, I24, I32};
    format == F32 || (driver() == AudioDriver::Asio && matches!(format, I32 | I24 | I16))
}

/// Frames converted at once between a driver's format and f32. A driver's block larger
/// than this goes through in parts, so the audio thread never allocates for it.
const CONVERT_FRAMES: usize = 8192;

/// The largest f32 that fits every integer format: +1.0 itself would wrap to the most
/// negative 24-bit sample.
const FULL_SCALE: f32 = 8_388_607.0 / 8_388_608.0;

fn f32_to_samples<T: cpal::FromSample<f32>>(from: &[f32], to: &mut [T]) {
    for (t, f) in to.iter_mut().zip(from) {
        *t = T::from_sample_(f.clamp(-1.0, FULL_SCALE));
    }
}

fn samples_to_f32<T: Copy>(from: &[T], to: &mut [f32])
where
    f32: cpal::FromSample<T>,
{
    for (t, f) in to.iter_mut().zip(from) {
        *t = <f32 as cpal::FromSample<T>>::from_sample_(*f);
    }
}

/// Render `out` in the driver's format, in parts that fit `scratch` (whole frames).
fn convert_out<T: cpal::FromSample<f32>>(out: &mut [T], scratch: &mut [f32], render: &mut impl FnMut(&mut [f32])) {
    for part in out.chunks_mut(scratch.len()) {
        let rendered = &mut scratch[..part.len()];
        render(rendered);
        f32_to_samples(rendered, part);
    }
}

/// Take in `data` in the driver's format, in parts that fit `scratch` (whole frames).
fn convert_in<T: Copy>(data: &[T], scratch: &mut [f32], on_data: &mut impl FnMut(&[f32]))
where
    f32: cpal::FromSample<T>,
{
    for part in data.chunks(scratch.len()) {
        let converted = &mut scratch[..part.len()];
        samples_to_f32(part, converted);
        on_data(converted);
    }
}

/// An output callback for a driver's format `T`, around one that renders f32.
fn output_as<T, R>(mut render: R, channels: usize) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo) + Send + 'static
where
    T: cpal::FromSample<f32> + 'static,
    R: FnMut(&mut [f32], &cpal::OutputCallbackInfo) + Send + 'static,
{
    let mut scratch = vec![0f32; CONVERT_FRAMES * channels.max(1)];
    move |out, info| convert_out(out, &mut scratch, &mut |buffer| render(buffer, info))
}

/// An input callback for a driver's format `T`, around one that takes f32.
fn input_as<T, D>(mut on_data: D, channels: usize) -> impl FnMut(&[T], &cpal::InputCallbackInfo) + Send + 'static
where
    T: Copy + 'static,
    f32: cpal::FromSample<T>,
    D: FnMut(&[f32]) + Send + 'static,
{
    let mut scratch = vec![0f32; CONVERT_FRAMES * channels.max(1)];
    move |data, _| convert_in(data, &mut scratch, &mut on_data)
}

/// An output stream that renders f32, whatever `format` the driver takes (see
/// [`format_ok`]).
pub fn build_output<R, E>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    format: cpal::SampleFormat,
    render: R,
    on_error: E,
) -> Result<cpal::Stream, cpal::Error>
where
    R: FnMut(&mut [f32], &cpal::OutputCallbackInfo) + Send + 'static,
    E: FnMut(cpal::Error) + Send + 'static,
{
    use cpal::SampleFormat::{F32, I16, I24, I32};
    let channels = config.channels as usize;
    match format {
        F32 => device.build_output_stream(config, render, on_error, None),
        I32 => device.build_output_stream(config, output_as::<i32, _>(render, channels), on_error, None),
        I24 => device.build_output_stream(config, output_as::<cpal::I24, _>(render, channels), on_error, None),
        I16 => device.build_output_stream(config, output_as::<i16, _>(render, channels), on_error, None),
        other => Err(cpal::Error::with_message(cpal::ErrorKind::UnsupportedConfig, format!("output sample format {other}"))),
    }
}

/// An input stream that delivers f32, whatever format `config` has (see [`format_ok`]).
/// On ASIO it asks for the buffer size the output got: the driver has one for both.
pub fn build_input<D, E>(
    device: &cpal::Device,
    config: cpal::SupportedStreamConfig,
    mut on_data: D,
    mut on_error: E,
) -> Result<cpal::Stream, cpal::Error>
where
    D: FnMut(&[f32]) + Send + 'static,
    E: FnMut(cpal::Error) + Send + 'static,
{
    use cpal::SampleFormat::{F32, I16, I24, I32};
    let (format, channels) = (config.sample_format(), config.channels() as usize);
    let asio = driver() == AudioDriver::Asio;
    let mut stream_config: cpal::StreamConfig = config.into();
    if asio {
        stream_config.buffer_size = asio_buffer();
    }
    let on_error = move |e: cpal::Error| {
        // An ASIO overload arrives on the driver's thread: nothing there that can wait.
        if !(asio && e.kind() == cpal::ErrorKind::Xrun) {
            on_error(e);
        }
    };
    match format {
        F32 => device.build_input_stream(stream_config, move |data: &[f32], _: &cpal::InputCallbackInfo| on_data(data), on_error, None),
        I32 => device.build_input_stream(stream_config, input_as::<i32, _>(on_data, channels), on_error, None),
        I24 => device.build_input_stream(stream_config, input_as::<cpal::I24, _>(on_data, channels), on_error, None),
        I16 => device.build_input_stream(stream_config, input_as::<i16, _>(on_data, channels), on_error, None),
        other => Err(cpal::Error::with_message(cpal::ErrorKind::UnsupportedConfig, format!("input sample format {other}"))),
    }
}

/// The buffer size the ASIO output was opened with (0: the driver's own).
static ASIO_BUFFER: AtomicU32 = AtomicU32::new(0);

pub(crate) fn note_asio_buffer(size: cpal::BufferSize) {
    ASIO_BUFFER.store(if let cpal::BufferSize::Fixed(n) = size { n } else { 0 }, Ordering::Relaxed);
}

fn asio_buffer() -> cpal::BufferSize {
    match ASIO_BUFFER.load(Ordering::Relaxed) {
        0 => cpal::BufferSize::Default,
        n => cpal::BufferSize::Fixed(n),
    }
}

/// A silent input stream on the ASIO `device`, to keep with its output. A driver makes
/// its input and output buffers together: adding inputs later stops it for a moment,
/// which would be heard. Opened before the output, as a failure here leaves that alone.
/// `None`: it has no inputs, or they won't open (then a later input stops it after all).
pub(crate) fn hold_asio_inputs(device: &cpal::Device) -> Option<cpal::Stream> {
    let config = default_config(device, Dir::In).ok()?;
    if !format_ok(config.sample_format()) {
        return None;
    }
    build_input(device, config, |_| {}, |e| eprintln!("audio input error: {e}")).ok()
}

/// The rate to open a driver at that doesn't tell its own until it runs (Realtek's):
/// DECKARD's usual one if it can, else its highest.
#[cfg_attr(not(all(windows, feature = "asio")), allow(dead_code))]
fn rate_among(rates: &[u32]) -> Option<u32> {
    if rates.contains(&48_000) { Some(48_000) } else { rates.iter().copied().max() }
}

/// ASIO, one driver at a time: cpal lists the drivers by loading each in turn, and a
/// driver is loaded for good while a stream is open on it, when the others won't load
/// and the listing ends early. So the list is kept, and asked for again only by
/// [`rescan`]; a stream opens from a device out of it (which holds no driver itself).
#[cfg(all(windows, feature = "asio"))]
mod asio {
    use super::{AudioDriver, Dir, Opened, default_config, device_name};
    use cpal::traits::{DeviceTrait, HostTrait};
    use std::sync::{Mutex, MutexGuard};

    static DEVICES: Mutex<Option<Vec<cpal::Device>>> = Mutex::new(None);

    fn kept() -> MutexGuard<'static, Option<Vec<cpal::Device>>> {
        DEVICES.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn scan() -> Vec<cpal::Device> {
        super::host(AudioDriver::Asio).ok().and_then(|h| h.devices().ok()).map_or_else(Vec::new, |ds| ds.collect())
    }

    pub fn rescan() {
        let fresh = scan();
        let mut list = kept();
        // Shorter: the listing ended at a driver a stream holds off.
        if list.as_ref().is_none_or(|l| fresh.len() >= l.len()) {
            *list = Some(fresh);
        }
    }

    fn devices(dir: Dir) -> Vec<cpal::Device> {
        let all = kept().get_or_insert_with(scan).clone();
        all.into_iter().filter(|d| default_config(d, dir).is_ok()).collect()
    }

    pub fn list(dir: Dir) -> Vec<(String, usize)> {
        devices(dir).iter().map(|d| (device_name(d), default_config(d, dir).map_or(0, |c| c.channels() as usize))).collect()
    }

    pub fn open(name: Option<&str>, dir: Dir) -> Result<Opened, String> {
        let name = name.ok_or("no ASIO driver picked (Preferences ▸ Audio)")?;
        let device = devices(dir).into_iter().find(|d| device_name(d) == name).ok_or_else(|| {
            format!("the ASIO driver \"{name}\" can't be loaded (is the interface connected, and free of other programs?)")
        })?;
        let mut config = default_config(&device, dir)?;
        if config.sample_rate() == 0 {
            let ranges = match dir {
                Dir::Out => device.supported_output_configs().map(|r| r.collect::<Vec<_>>()),
                Dir::In => device.supported_input_configs().map(|r| r.collect::<Vec<_>>()),
            }
            .map_err(|e| e.to_string())?;
            let rates: Vec<u32> = ranges.iter().map(|r| r.max_sample_rate()).collect();
            let rate = super::rate_among(&rates).ok_or_else(|| format!("\"{name}\" offers no sample rate"))?;
            let channels = config.channels();
            if let Some(r) = ranges.into_iter().find(|r| r.channels() == channels && r.max_sample_rate() == rate) {
                config = r.with_sample_rate(rate);
            }
        }
        Ok((device, config))
    }
}

/// Nothing without ASIO: [`set_driver`] and [`AudioDriver::usable`] keep it from being reached.
#[cfg(not(all(windows, feature = "asio")))]
mod asio {
    use super::{Dir, Opened};

    pub fn rescan() {}

    pub fn list(_: Dir) -> Vec<(String, usize)> {
        Vec::new()
    }

    #[cfg_attr(target_os = "linux", allow(dead_code))]
    pub fn open(_: Option<&str>, _: Dir) -> Result<Opened, String> {
        Err("this build of DECKARD has no ASIO".into())
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn a_device_that_wont_answer_is_passed_over() {
        let answer = |d: &&str| if *d == "broken" { Err("no format".to_string()) } else { Ok(d.len()) };
        assert_eq!(first_usable(["broken", "usb", "line"], answer), Some(("usb", 3)));
        assert_eq!(first_usable(["broken"], answer), None);
        assert_eq!(first_usable(Vec::<&str>::new(), answer), None);
    }

    /// Nothing here loads a driver: choosing a host only makes cpal's handle.
    #[test]
    fn the_host_is_the_one_asked_for() {
        assert_eq!(host(AudioDriver::System).unwrap().id(), cpal::default_host().id());
        #[cfg(all(windows, feature = "asio"))]
        assert_eq!(host(AudioDriver::Asio).unwrap().id(), cpal::HostId::Asio);
        #[cfg(not(all(windows, feature = "asio")))]
        assert_eq!(host(AudioDriver::Asio).unwrap().id(), cpal::default_host().id());
    }

    #[test]
    fn asio_is_only_usable_where_it_was_built() {
        assert_eq!(AudioDriver::System.usable(), AudioDriver::System);
        assert_eq!(AudioDriver::Asio.usable(), if ASIO_BUILT { AudioDriver::Asio } else { AudioDriver::System });
        assert_eq!(AudioDriver::Asio.name(), "ASIO");
        assert_ne!(AudioDriver::System.name(), "ASIO");
    }

    /// Each driver tried, in order, by a stand-in for opening one.
    fn opened(
        built: bool,
        wanted: AudioDriver,
        asio: Result<&'static str, &'static str>,
        system: Result<&'static str, &'static str>,
    ) -> (Vec<AudioDriver>, Result<String, String>, Option<String>) {
        let mut tried = Vec::new();
        let (result, why) = open_preferring_in(built, wanted, |d| {
            tried.push(d);
            let r = if d == AudioDriver::Asio { asio } else { system };
            r.map(String::from).map_err(String::from)
        });
        (tried, result, why)
    }

    #[test]
    fn the_system_driver_is_only_tried_itself() {
        let (tried, result, why) = opened(true, AudioDriver::System, Ok("asio"), Ok("system"));
        assert_eq!((tried, result, why), (vec![AudioDriver::System], Ok("system".into()), None));
        let (tried, result, why) = opened(true, AudioDriver::System, Ok("asio"), Err("no device"));
        assert_eq!((tried, result, why), (vec![AudioDriver::System], Err("no device".into()), None), "as before: its error");
    }

    #[test]
    fn asio_plays_when_it_opens() {
        let (tried, result, why) = opened(true, AudioDriver::Asio, Ok("asio"), Ok("system"));
        assert_eq!((tried, result, why), (vec![AudioDriver::Asio], Ok("asio".into()), None));
    }

    #[test]
    fn asio_that_wont_open_falls_back_and_says_why() {
        let (tried, result, why) = opened(true, AudioDriver::Asio, Err("held by another program"), Ok("system"));
        assert_eq!(tried, [AudioDriver::Asio, AudioDriver::System]);
        assert_eq!(result, Ok("system".into()));
        assert_eq!(why.as_deref(), Some("held by another program"));
        // Neither opens: the system driver's error is the result; the reason still shows.
        let (_, result, why) = opened(true, AudioDriver::Asio, Err("held"), Err("no device"));
        assert_eq!((result, why.as_deref()), (Err("no device".into()), Some("held")));
    }

    #[test]
    fn asio_asked_of_a_build_without_it_falls_back_and_says_so() {
        let (tried, result, why) = opened(false, AudioDriver::Asio, Ok("asio"), Ok("system"));
        assert_eq!(tried, [AudioDriver::System], "not tried");
        assert_eq!(result, Ok("system".into()));
        assert!(why.unwrap().contains("no ASIO"));
    }

    #[test]
    fn the_rate_of_a_driver_that_wont_tell() {
        assert_eq!(rate_among(&[44_100, 48_000, 96_000]), Some(48_000));
        assert_eq!(rate_among(&[44_100, 88_200]), Some(88_200));
        assert_eq!(rate_among(&[]), None);
    }

    #[test]
    fn full_scale_stays_in_range_in_every_format() {
        let from = [1.0f32, -1.0, 4.0, -4.0, 0.0, f32::NAN];
        let mut i32s = [1i32; 6];
        f32_to_samples(&from, &mut i32s);
        assert!(i32s[0] > 2_147_000_000 && i32s[2] == i32s[0], "{i32s:?}");
        assert_eq!(&i32s[3..5], &[i32::MIN, 0]);
        assert_eq!(i32s[1], i32::MIN);
        assert_eq!(i32s[5], 0, "NaN is silence");
        let mut i16s = [1i16; 6];
        f32_to_samples(&from, &mut i16s);
        assert_eq!(i16s, [i16::MAX, i16::MIN, i16::MAX, i16::MIN, 0, 0]);
        let mut i24s = [cpal::I24::new(1).unwrap(); 6];
        f32_to_samples(&from, &mut i24s);
        let i24s = i24s.map(|s| s.inner());
        assert_eq!(i24s, [8_388_607, -8_388_608, 8_388_607, -8_388_608, 0, 0], "+1.0 doesn't wrap");
    }

    #[test]
    fn inputs_come_in_full_scale() {
        let mut to = [0f32; 3];
        samples_to_f32(&[i32::MIN, 0, 1 << 30], &mut to);
        assert_eq!(to, [-1.0, 0.0, 0.5]);
        samples_to_f32(&[i16::MIN, 0, 1 << 14], &mut to);
        assert_eq!(to, [-1.0, 0.0, 0.5]);
        samples_to_f32(&[cpal::I24::new(-8_388_608).unwrap(), cpal::I24::new(0).unwrap(), cpal::I24::new(1 << 22).unwrap()], &mut to);
        assert_eq!(to, [-1.0, 0.0, 0.5]);
    }

    /// Whatever size the driver asks for, the f32 callback gets whole frames in order, in
    /// parts no larger than the scratch made up front.
    #[test]
    fn a_driver_block_larger_than_the_scratch_goes_through_in_whole_frames() {
        let (channels, room) = (3, 12);
        let mut scratch = vec![0f32; room];
        let data: Vec<i16> = (0..(room * 2 + 3) as i16).map(|i| i * 1000).collect();
        let mut seen = Vec::new();
        convert_in(&data, &mut scratch, &mut |d| {
            assert!(d.len().is_multiple_of(channels) && d.len() <= room);
            seen.extend(d.iter().map(|v| (v * 32_768.0).round() as i16));
        });
        assert_eq!(seen, data, "in");

        let mut out = vec![0i16; room * 2 + 3];
        let mut next = 0i16;
        convert_out(&mut out, &mut scratch, &mut |d| {
            assert!(d.len().is_multiple_of(channels) && d.len() <= room);
            for s in d {
                *s = next as f32 * 1000.0 / 32_768.0;
                next += 1;
            }
        });
        assert_eq!(out, data, "out");
    }
}
