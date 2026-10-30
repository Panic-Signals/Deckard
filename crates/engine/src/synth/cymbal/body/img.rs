//! Maschine layouts: every body struct declares its fields with their Maschine offsets
//! ([`layout!`]), which gives a [`Walk`] visitor over them (`super::super::attached::Slot`)
//! for comparing whole components with memory dumps of Maschine's own, and for loading
//! state from them.
//!
//! Fields marked `_` live outside the object (heap blocks); they are walked separately.

#[cfg(test)]
use super::super::{attached::Slot, memo::MemoEnvelope};

/// A value with a fixed place and representation in a Maschine object.
#[cfg(test)]
pub(in super::super) trait Walk {
    /// Bytes it occupies.
    const SIZE: usize;
    /// Visit every field at `at + its offset`.
    fn walk(&mut self, at: usize, v: &mut dyn FnMut(usize, Slot));
}

#[cfg(test)]
macro_rules! scalar {
    ($($t:ty => $s:ident),*) => {$(
        impl Walk for $t {
            const SIZE: usize = size_of::<$t>();
            fn walk(&mut self, at: usize, v: &mut dyn FnMut(usize, Slot)) {
                v(at, Slot::$s(self));
            }
        }
    )*};
}
#[cfg(test)]
scalar!(f32 => F, f64 => D, u32 => U, i32 => I, bool => B);

#[cfg(test)]
impl<T: Walk, const N: usize> Walk for [T; N] {
    const SIZE: usize = T::SIZE * N;
    fn walk(&mut self, at: usize, v: &mut dyn FnMut(usize, Slot)) {
        for (i, x) in self.iter_mut().enumerate() {
            x.walk(at + i * T::SIZE, v);
        }
    }
}

/// A heap vector: walked separately ([`walk_vec`]); nothing in the object.
#[cfg(test)]
impl<T> Walk for Vec<T> {
    const SIZE: usize = 0;
    fn walk(&mut self, _at: usize, _v: &mut dyn FnMut(usize, Slot)) {}
}

/// The elements of a heap vector, back to back from `at`.
#[cfg(test)]
pub(in super::super) fn walk_vec<T: Walk>(xs: &mut [T], at: usize, v: &mut dyn FnMut(usize, Slot)) {
    for (i, x) in xs.iter_mut().enumerate() {
        x.walk(at + i * T::SIZE, v);
    }
}

/// A struct with Maschine offsets: `$off name: Type` (offset relative to the struct), or
/// `"heap" name: Type` for a field kept outside the object.
macro_rules! layout {
    (
        $(#[$sm:meta])*
        $name:ident [$size:literal] {
            $( $(#[$m:meta])* $off:literal $f:ident : $t:ty ),* $(,)?
        }
    ) => {
        $(#[$sm])*
        #[derive(Clone, Debug, Default)]
        pub(in crate::synth::cymbal) struct $name {
            $( #[doc = concat!("+", stringify!($off), ":")] $(#[$m])* pub(in crate::synth::cymbal) $f: $t, )*
        }
        #[cfg(test)]
        impl $crate::synth::cymbal::body::img::Walk for $name {
            const SIZE: usize = $size;
            fn walk(
                &mut self,
                at: usize,
                v: &mut dyn FnMut(usize, $crate::synth::cymbal::attached::Slot),
            ) {
                $( layout!(@walk self, at, v, $off, $f); )*
            }
        }
    };
    (@walk $s:ident, $at:ident, $v:ident, $off:literal, $f:ident) => {
        if let Some(o) = $crate::synth::cymbal::body::img::Off::at($off) {
            $crate::synth::cymbal::body::img::Walk::walk(&mut $s.$f, $at + o, $v)
        }
    };
}

/// A field's place: an offset, or `"heap"` (walked separately).
#[cfg(test)]
pub(in super::super) trait Off {
    fn at(self) -> Option<usize>;
}
#[cfg(test)]
impl Off for i32 {
    fn at(self) -> Option<usize> {
        Some(self as usize)
    }
}
#[cfg(test)]
impl Off for &str {
    fn at(self) -> Option<usize> {
        None
    }
}

/// `MemoEnvelope` (0x80 bytes); +0x14 and +0x5c are padding.
#[cfg(test)]
impl Walk for MemoEnvelope {
    const SIZE: usize = 0x80;
    fn walk(&mut self, o: usize, v: &mut dyn FnMut(usize, Slot)) {
        v(o, Slot::U(&mut self.state));
        v(o + 4, Slot::F(&mut self.attack));
        v(o + 8, Slot::F(&mut self.hold));
        v(o + 0xc, Slot::F(&mut self.decay));
        v(o + 0x10, Slot::F(&mut self.release));
        v(o + 0x18, Slot::D(&mut self.attack_step));
        v(o + 0x20, Slot::D(&mut self.hold_len));
        v(o + 0x28, Slot::D(&mut self.decay_step));
        v(o + 0x30, Slot::D(&mut self.sustain));
        v(o + 0x38, Slot::D(&mut self.release_mul));
        v(o + 0x40, Slot::D(&mut self.inc));
        v(o + 0x48, Slot::D(&mut self.dec));
        v(o + 0x50, Slot::D(&mut self.sus));
        v(o + 0x58, Slot::F(&mut self.peak32));
        v(o + 0x60, Slot::D(&mut self.peak));
        v(o + 0x68, Slot::D(&mut self.cur));
        v(o + 0x70, Slot::U(&mut self.count));
        v(o + 0x74, Slot::F(&mut self.sr));
        v(o + 0x78, Slot::D(&mut self.inv_sr));
    }
}
