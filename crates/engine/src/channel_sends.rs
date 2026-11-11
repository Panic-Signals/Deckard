//! Channel sends. Routes and fader paths are prepared off the audio thread; mixing
//! uses the existing sound/bus buffers, with no locks or allocations.
use super::*;
#[derive(Clone)]
pub(super) struct Send {
    pub(super) route: Route,
    level: f64,
    group: usize,
    sound: Option<usize>,
    field: &'static str,
    path: Vec<usize>,
    ramps: [GainRamp; 2],
}
fn path(p: &Project, fs: &[Fader], d: deckard_model::AudioDest, parent: Option<usize>, out: &mut Vec<usize>, depth: u8) {
    use deckard_model::AudioDest as D;
    if depth > 8 {
        return;
    }
    match d {
        D::Parent => {
            if let Some(g) = parent {
                path(p, fs, D::Group(g), None, out, depth + 1);
            }
        }
        D::Group(g) => {
            if let Some(grp) = p.groups.get(g) {
                if has_bus(grp) {
                    return;
                }
                if let Some(i) = fs.iter().position(|f| (f.group, f.sound) == (g, None)) {
                    out.push(i);
                }
                if !grp.cue {
                    path(p, fs, grp.channel.dest, None, out, depth + 1);
                }
            }
        }
        D::Sound(g, s) => {
            if let Some(snd) = p.groups.get(g).and_then(|g| g.sounds.get(s)) {
                if has_sound_bus(snd) {
                    return;
                }
                if let Some(i) = fs.iter().position(|f| (f.group, f.sound) == (g, Some(s))) {
                    out.push(i);
                }
                if !snd.cue {
                    path(p, fs, snd.channel.dest, Some(g), out, depth + 1);
                }
            }
        }
        _ => (),
    }
}
pub(super) fn reaches(p: &Project, d: deckard_model::AudioDest, wanted: (usize, Option<usize>), parent: Option<usize>, depth: u8) -> bool {
    use deckard_model::AudioDest as D;
    if depth > 8 {
        return true;
    }
    match d {
        D::Parent => parent.is_some_and(|g| reaches(p, D::Group(g), wanted, None, depth + 1)),
        D::Group(g) => {
            (g, None) == wanted
                || p.groups.get(g).is_some_and(|grp| {
                    reaches(p, grp.channel.dest, wanted, None, depth + 1)
                        || grp.channel.aux_dest.iter().any(|d| reaches(p, *d, wanted, None, depth + 1))
                })
        }
        D::Sound(g, s) => {
            (g, Some(s)) == wanted
                || p.groups.get(g).and_then(|g| g.sounds.get(s)).is_some_and(|snd| {
                    reaches(p, snd.channel.dest, wanted, Some(g), depth + 1)
                        || snd.channel.aux_dest.iter().any(|d| reaches(p, *d, wanted, Some(g), depth + 1))
                })
        }
        _ => false,
    }
}
fn sends(p: &Project, fs: &[Fader], g: usize, s: Option<usize>) -> Vec<Send> {
    let grp = &p.groups[g];
    let (dest, pre, levels, level, pan, muted) = if let Some(s) = s {
        let snd = &grp.sounds[s];
        (
            snd.channel.aux_dest,
            snd.channel.aux_pre,
            snd.aux,
            snd.level,
            snd.pan,
            grp.muted || (snd.muted && snd.channel.audio_mute) || snd.cue || grp.cue,
        )
    } else {
        (grp.channel.aux_dest, grp.channel.aux_pre, grp.aux, grp.level, grp.pan, grp.muted || grp.cue)
    };
    let mut out = Vec::with_capacity(2);
    for i in 0..2 {
        if dest[i] == deckard_model::AudioDest::None || muted || reaches(p, dest[i], (g, s), s.map(|_| g), 0) {
            continue;
        }
        let mut route = dest_route(p, dest[i], s.map(|_| g), 0);
        let mut faders = Vec::new();
        path(p, fs, dest[i], s.map(|_| g), &mut faders, 0);
        if !pre[i] {
            if let Some(index) = fs.iter().position(|f| (f.group, f.sound) == (g, s)) {
                faders.push(index);
            } else {
                route.1 = mul(route.1, channel_gain(level, pan));
            }
        }
        out.push(Send {
            route,
            level: levels[i],
            group: g,
            sound: s,
            field: if i == 0 { "aux1_level" } else { "aux2_level" },
            path: faders,
            ramps: [GainRamp::default(); 2],
        });
    }
    out
}
pub(super) fn build(p: &Project, fs: &[Fader]) -> (Vec<Vec<Vec<Send>>>, Vec<Vec<Send>>) {
    let sound = p
        .groups
        .iter()
        .enumerate()
        .map(|(g, grp)| grp.sounds.iter().enumerate().map(|(s, _)| sends(p, fs, g, Some(s))).collect())
        .collect();
    let groups = (0..p.groups.len()).map(|g| sends(p, fs, g, None)).collect();
    (sound, groups)
}
pub(super) fn order(
    buses: &[Option<Bus>],
    sends: &[Vec<Send>],
    sound_buses: &[Vec<bool>],
    routes: &[Vec<Option<usize>>],
    sound_sends: &[Vec<Vec<Send>>],
    strips: &[(Owner, usize, Channels, StripRoute)],
) -> Vec<usize> {
    fn bus_node(bus: Option<usize>) -> Option<usize> {
        match bus {
            Some(n) if n >= SOUND_BUS => Some(n),
            Some(n) if n >= GROUP_BUS => Some(n - GROUP_BUS),
            _ => None,
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn depth(
        idx: usize,
        nodes: &[usize],
        bs: &[Option<Bus>],
        ss: &[Vec<Send>],
        rts: &[Vec<Option<usize>>],
        sa: &[Vec<Vec<Send>>],
        st: &[(Owner, usize, Channels, StripRoute)],
        memo: &mut [Option<usize>],
        busy: &mut [bool],
    ) -> usize {
        if let Some(d) = memo[idx] {
            return d;
        }
        if busy[idx] {
            return 0;
        }
        busy[idx] = true;
        let node = nodes[idx];
        let step = |target: usize, memo: &mut [Option<usize>], busy: &mut [bool]| {
            nodes.iter().position(|&x| x == target).map_or(0, |next_idx| 1 + depth(next_idx, nodes, bs, ss, rts, sa, st, memo, busy))
        };
        let value = if let Some((g, s)) = decode_sound_bus(node) {
            let main = rts.get(g).and_then(|r| r.get(s)).copied().flatten();
            let aux = sa.get(g).and_then(|r| r.get(s)).into_iter().flatten().map(|x| x.route.0);
            let ext = st
                .iter()
                .filter(|(o, ..)| matches!(*o, Owner::Sound(a, b) | Owner::Chain(a, b, _) if (a, b) == (g, s)))
                .flat_map(|(_, _, _, r)| [r.route.0, r.fallback.0]);
            let targets: Vec<usize> = std::iter::once(main).chain(aux).chain(ext).filter_map(bus_node).collect();
            targets.into_iter().map(|t| step(t, memo, busy)).max().unwrap_or(0)
        } else {
            let g = node;
            let normal = bs.get(g).and_then(|b| b.as_ref()).and_then(|b| b.route.0);
            let aux = ss.get(g).into_iter().flatten().map(|s| s.route.0);
            let ext = bs
                .get(g)
                .and_then(|b| b.as_ref())
                .into_iter()
                .flat_map(|b| &b.chain.units)
                .flat_map(|u| &u.extras)
                .flat_map(|x| [x.route.0, x.fallback.0]);
            let targets: Vec<usize> = std::iter::once(normal).chain(aux).chain(ext).filter_map(bus_node).collect();
            targets.into_iter().map(|t| step(t, memo, busy)).max().unwrap_or(0)
        };
        busy[idx] = false;
        memo[idx] = Some(value);
        value
    }
    let mut nodes = bus_order(buses);
    for (g, row) in sound_buses.iter().enumerate() {
        for (s, &is_bus) in row.iter().enumerate() {
            if is_bus {
                let id = sound_bus_id(g, s);
                if !nodes.contains(&id) {
                    nodes.push(id);
                }
            }
        }
    }
    let mut memo = vec![None; nodes.len()];
    let mut busy = vec![false; nodes.len()];
    let depths: Vec<usize> =
        (0..nodes.len()).map(|i| depth(i, &nodes, buses, sends, routes, sound_sends, strips, &mut memo, &mut busy)).collect();
    let mut indices: Vec<usize> = (0..nodes.len()).collect();
    indices.sort_by_key(|&i| std::cmp::Reverse(depths[i]));
    indices.into_iter().map(|i| nodes[i]).collect()
}
pub(super) fn adopt(old: &mut [Send], new: &mut Vec<Send>) {
    for n in new.iter_mut() {
        if let Some(o) = old.iter().find(|o| o.field == n.field) {
            if n.route.1 == (0.0, 0.0) && o.ramps.iter().any(|r| r.now != (0.0, 0.0) || r.left > 0) {
                n.route.0 = o.route.0;
            }
            n.ramps = o.ramps;
        } else {
            n.ramps = [GainRamp { set: true, ..GainRamp::default() }; 2];
        }
    }
    for o in old.iter() {
        if new.iter().all(|n| n.field != o.field) && o.ramps.iter().any(|r| r.now != (0.0, 0.0) || r.left > 0) && new.len() < new.capacity()
        {
            new.push(Send {
                route: (o.route.0, (0.0, 0.0)),
                level: 0.0,
                group: o.group,
                sound: o.sound,
                field: o.field,
                path: Vec::new(),
                ramps: o.ramps,
            });
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) fn mix(
    sends: &mut [Send],
    kind: usize,
    src: &[f32],
    out: &mut [f32],
    ext: &mut [Vec<f32>],
    buses: &mut [Option<Bus>],
    bufs: &mut [Vec<Option<Box<SoundBufs>>>],
    at: usize,
    inp_at: usize,
    faders: &[Fader],
    lanes: &[modscale::ModLane],
    tick: f64,
    sequencing: bool,
    glide: u32,
) {
    let frames = out.len() / 2;
    for send in sends {
        let dev = if sequencing {
            lanes
                .iter()
                .find(|l| {
                    l.group == send.group && l.sound == send.sound && l.field == send.field && (l.from as f64..l.to as f64).contains(&tick)
                })
                .map_or(0.0, |l| l.value_at(tick))
        } else {
            0.0
        };
        let level = deckard_model::level_gain((send.level + dev).clamp(0.0, 1.0)) as f32;
        let gain = mul(send.route.1, path_gain(faders, &send.path));
        send.ramps[kind].aim((gain.0 * level, gain.1 * level), glide);
        if send.ramps[kind].left == 0 && send.ramps[kind].now == (0.0, 0.0) {
            continue;
        }
        let dst = route_dst(send.route.0, frames, at, inp_at, out, ext, buses, bufs);
        if let Some(dst) = dst {
            for f in 0..frames {
                let gain = send.ramps[kind].next();
                let l = src[f * 2] * gain.0;
                let r = src[f * 2 + 1] * gain.1;
                if l.is_finite() {
                    dst[f * 2] += l;
                }
                if r.is_finite() {
                    dst[f * 2 + 1] += r;
                }
            }
        } else {
            for _ in 0..frames {
                send.ramps[kind].next();
            }
        }
    }
}
