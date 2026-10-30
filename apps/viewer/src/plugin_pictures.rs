//! DECKARD's own pictures of its plug-ins for the controller's displays (tiles in
//! the chain, logos on the right display), made by the `make_plugin_pictures` test.

/// (style sheet key, thumbnail PNG, logo PNG).
pub const PICTURES: &[(&str, &[u8], &[u8])] = &[
    ("audio", include_bytes!("../assets/plugins/audio_Thumb.png"), include_bytes!("../assets/plugins/audio_Logo.png")),
    ("basssynth", include_bytes!("../assets/plugins/basssynth_Thumb.png"), include_bytes!("../assets/plugins/basssynth_Logo.png")),
    ("beatdelay", include_bytes!("../assets/plugins/beatdelay_Thumb.png"), include_bytes!("../assets/plugins/beatdelay_Logo.png")),
    ("chorus", include_bytes!("../assets/plugins/chorus_Thumb.png"), include_bytes!("../assets/plugins/chorus_Logo.png")),
    ("compressor", include_bytes!("../assets/plugins/compressor_Thumb.png"), include_bytes!("../assets/plugins/compressor_Logo.png")),
    ("cymbal", include_bytes!("../assets/plugins/cymbal_Thumb.png"), include_bytes!("../assets/plugins/cymbal_Logo.png")),
    ("distortion", include_bytes!("../assets/plugins/distortion_Thumb.png"), include_bytes!("../assets/plugins/distortion_Logo.png")),
    ("eq", include_bytes!("../assets/plugins/eq_Thumb.png"), include_bytes!("../assets/plugins/eq_Logo.png")),
    ("filter", include_bytes!("../assets/plugins/filter_Thumb.png"), include_bytes!("../assets/plugins/filter_Logo.png")),
    ("flanger", include_bytes!("../assets/plugins/flanger_Thumb.png"), include_bytes!("../assets/plugins/flanger_Logo.png")),
    ("fm", include_bytes!("../assets/plugins/fm_Thumb.png"), include_bytes!("../assets/plugins/fm_Logo.png")),
    ("freqshifter", include_bytes!("../assets/plugins/freqshifter_Thumb.png"), include_bytes!("../assets/plugins/freqshifter_Logo.png")),
    ("gate", include_bytes!("../assets/plugins/gate_Thumb.png"), include_bytes!("../assets/plugins/gate_Logo.png")),
    ("graindelay", include_bytes!("../assets/plugins/graindelay_Thumb.png"), include_bytes!("../assets/plugins/graindelay_Logo.png")),
    ("grainstretch", include_bytes!("../assets/plugins/grainstretch_Thumb.png"), include_bytes!("../assets/plugins/grainstretch_Logo.png")),
    ("hi-hat", include_bytes!("../assets/plugins/hi-hat_Thumb.png"), include_bytes!("../assets/plugins/hi-hat_Logo.png")),
    ("ice", include_bytes!("../assets/plugins/ice_Thumb.png"), include_bytes!("../assets/plugins/ice_Logo.png")),
    ("kick", include_bytes!("../assets/plugins/kick_Thumb.png"), include_bytes!("../assets/plugins/kick_Logo.png")),
    ("legacyreverb", include_bytes!("../assets/plugins/legacyreverb_Thumb.png"), include_bytes!("../assets/plugins/legacyreverb_Logo.png")),
    ("limiter", include_bytes!("../assets/plugins/limiter_Thumb.png"), include_bytes!("../assets/plugins/limiter_Logo.png")),
    ("lo-fi", include_bytes!("../assets/plugins/lo-fi_Thumb.png"), include_bytes!("../assets/plugins/lo-fi_Logo.png")),
    ("maximizer", include_bytes!("../assets/plugins/maximizer_Thumb.png"), include_bytes!("../assets/plugins/maximizer_Logo.png")),
    ("metaverb", include_bytes!("../assets/plugins/metaverb_Thumb.png"), include_bytes!("../assets/plugins/metaverb_Logo.png")),
    ("percussion", include_bytes!("../assets/plugins/percussion_Thumb.png"), include_bytes!("../assets/plugins/percussion_Logo.png")),
    ("phaser", include_bytes!("../assets/plugins/phaser_Thumb.png"), include_bytes!("../assets/plugins/phaser_Logo.png")),
    ("plaits", include_bytes!("../assets/plugins/plaits_Thumb.png"), include_bytes!("../assets/plugins/plaits_Logo.png")),
    ("polysynth", include_bytes!("../assets/plugins/polysynth_Thumb.png"), include_bytes!("../assets/plugins/polysynth_Logo.png")),
    ("reflex", include_bytes!("../assets/plugins/reflex_Thumb.png"), include_bytes!("../assets/plugins/reflex_Logo.png")),
    ("replicant", include_bytes!("../assets/plugins/replicant_Thumb.png"), include_bytes!("../assets/plugins/replicant_Logo.png")),
    ("resochord", include_bytes!("../assets/plugins/resochord_Thumb.png"), include_bytes!("../assets/plugins/resochord_Logo.png")),
    ("reverb", include_bytes!("../assets/plugins/reverb_Thumb.png"), include_bytes!("../assets/plugins/reverb_Logo.png")),
    ("sampler", include_bytes!("../assets/plugins/sampler_Thumb.png"), include_bytes!("../assets/plugins/sampler_Logo.png")),
    ("saturator", include_bytes!("../assets/plugins/saturator_Thumb.png"), include_bytes!("../assets/plugins/saturator_Logo.png")),
    ("snare", include_bytes!("../assets/plugins/snare_Thumb.png"), include_bytes!("../assets/plugins/snare_Logo.png")),
    ("tom", include_bytes!("../assets/plugins/tom_Thumb.png"), include_bytes!("../assets/plugins/tom_Logo.png")),
    (
        "transientmaster",
        include_bytes!("../assets/plugins/transientmaster_Thumb.png"),
        include_bytes!("../assets/plugins/transientmaster_Logo.png"),
    ),
];

/// Whether DECKARD has a picture of the plug-in with this key.
pub fn has(key: &str) -> bool {
    PICTURES.iter().any(|(k, ..)| *k == key)
}

/// Give DECKARD's controller layer the pictures (thumbnails and logos).
pub fn give(s: &mut deckard_surface::Surface) {
    for (key, thumb, logo) in PICTURES {
        for (bytes, is_thumb) in [(*thumb, true), (*logo, false)] {
            if let Some((w, h, px)) = decode(bytes) {
                s.add_picture(key, is_thumb, w, h, px);
            }
        }
    }
}

/// A PNG as RGBA pixels (w, h, pixels).
fn decode(bytes: &[u8]) -> Option<(usize, usize, Vec<[u8; 4]>)> {
    let mut r = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let buf = &buf[..info.buffer_size()];
    let px = match info.color_type {
        png::ColorType::Rgba => buf.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect(),
        png::ColorType::Rgb => buf.chunks(3).map(|c| [c[0], c[1], c[2], 255]).collect(),
        _ => return None,
    };
    Some((info.width as usize, info.height as usize, px))
}
