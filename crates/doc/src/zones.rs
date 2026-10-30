//! A Sampler's zones: pointing a zone at an edited sample, and rebuilding the zone list
//! (adding, deleting, reordering, slices, drum-kit mapping) as one undo step. Zones are
//! an `ObjectVector<Zone>` in the Sampler (`zones.objects.items`, each a
//! `shared_ptr<Zone>`), with its `current` (focused) zone.

use crate::chain::set_param;
use crate::{Document, Op, Target};
use mxprj::decode::{Obj, Path as TreePath, Value};
use std::path::{Path, PathBuf};

const FILE_LOCATION: &str = "NI::MASCOMMON::FileLocation";
const ZONE: &str = "NI::MASCOMMON::DATA::Zone";

/// Where a zone of a new zone list comes from.
#[derive(Debug, Clone)]
pub enum ZoneFrom {
    /// A copy of the Sampler's zone with this index.
    Zone(usize),
    /// A new zone at Maschine's defaults (for a Sampler that has none).
    Template,
}

/// One zone of a new zone list: what to copy, the sample to point it at (`None`: keep
/// the copy's), and parameters to set (`play_range/low`, `key_range/high`,
/// `root_key`, …).
#[derive(Debug, Clone)]
pub struct ZoneSpec {
    pub from: ZoneFrom,
    pub file: Option<PathBuf>,
    pub set: Vec<(&'static str, f64)>,
}

/// A `FileLocation` for a file outside any library: `kind` 0, an absolute path, no
/// alias.
pub(crate) fn file_location(path: &Path) -> Value {
    Value::Obj(Obj {
        type_name: FILE_LOCATION.into(),
        version: 0,
        oid: None,
        export: None,
        fields: vec![("kind".into(), Value::Int(0)), ("path".into(), Value::Str(path.display().to_string()))],
    })
}

fn zone_obj_mut(item: &mut Value) -> Option<&mut Obj> {
    match item {
        Value::Obj(ptr) => match ptr.get_mut("px") {
            Some(Value::Obj(z)) if z.type_name == ZONE => Some(z),
            _ => None,
        },
        _ => None,
    }
}

impl Document {
    /// The Sampler's zone list: (path of its items, path of its `current`).
    fn zone_paths(&self, g: usize, s: usize) -> Result<(TreePath, TreePath), String> {
        let sampler = self.owner(Target::Sampler(g, s)).ok_or("the sound has no Sampler")?;
        Ok((self.field_path(&sampler, &["zones", "objects", "items"])?, self.field_path(&sampler, &["zones", "current"])?))
    }

    /// Point zone `z` at `file` (an edited copy of its sample, `frames` long), with its
    /// play range over the whole file and its loop kept inside it, as one undo step.
    pub fn set_zone_sample(&mut self, g: usize, s: usize, z: usize, file: &Path, frames: usize, label: &str) -> Result<(), String> {
        let zone = self.owner(Target::Zone(g, s, z)).ok_or("no such zone")?;
        let file_path = self.field_path(&zone, &["file"])?;
        let mut ops = vec![Op::Set { path: file_path, value: file_location(file) }];
        let last = frames.saturating_sub(1) as i64;
        let t = Target::Zone(g, s, z);
        ops.extend(self.param_ops_pub(t, "play_range/low", Value::Int(0))?);
        ops.extend(self.param_ops_pub(t, "play_range/high", Value::Int(last))?);
        let p = self.project();
        let zm = p.groups[g].sounds[s].sampler.as_ref().and_then(|x| x.zones.get(z)).cloned().ok_or("no such zone")?;
        let (lo, hi) = zm.loop_range;
        if hi > last || lo > last {
            ops.extend(self.param_ops_pub(t, "loop_range/low", Value::Int(lo.min(last)))?);
            ops.extend(self.param_ops_pub(t, "loop_range/high", Value::Int(last))?);
        }
        self.apply(label, ops)
    }

    /// Replace the Sampler's zones with `specs`, in that order, as one undo step; the
    /// first becomes the focused zone.
    pub fn set_zones(&mut self, g: usize, s: usize, specs: &[ZoneSpec], label: &str) -> Result<(), String> {
        let (items_path, current_path) = self.zone_paths(g, s)?;
        let items = self.items(&items_path)?;
        let mut out = Vec::new();
        for spec in specs {
            let mut item = match spec.from {
                ZoneFrom::Zone(i) => {
                    let src = items.get(i).cloned().ok_or_else(|| format!("no zone {i}"))?;
                    self.fresh_copy(&src)
                }
                ZoneFrom::Template => self.build(|b| b.item(ZONE))?,
            };
            let z = zone_obj_mut(&mut item).ok_or("bad zone")?;
            if let Some(f) = &spec.file {
                *z.get_mut("file").ok_or("zone without file")? = file_location(f);
            }
            for (k, v) in &spec.set {
                set_param(z, k, *v);
            }
            out.push(item);
        }
        let first = out.first().and_then(|it| match it {
            Value::Obj(p) => match p.get("px") {
                Some(Value::Obj(z)) => z.oid,
                _ => None,
            },
            _ => None,
        });
        let ops = vec![
            Op::Set { path: items_path, value: Value::List(out) },
            Op::Set { path: current_path, value: first.map_or(Value::Null, Value::Ref) },
        ];
        self.apply(label, ops)
    }
}
