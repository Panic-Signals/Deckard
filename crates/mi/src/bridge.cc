// The C interface DECKARD calls: a Plaits voice with its own RAM, rendered a block at a
// time into floats.

#include <new>

#include "plaits/dsp/voice.h"
#include "stmlib/utils/buffer_allocator.h"

namespace {

struct PlaitsVoice {
  plaits::Voice voice;
  stmlib::BufferAllocator allocator;
  char ram[16 * 1024];
};

}  // namespace

extern "C" {

struct DkPlaitsPatch {
  float note;
  float harmonics;
  float timbre;
  float morph;
  float frequency_modulation_amount;
  float timbre_modulation_amount;
  float morph_modulation_amount;
  int engine;
  float decay;
  float lpg_colour;
};

struct DkPlaitsModulations {
  float engine;
  float note;
  float frequency;
  float harmonics;
  float timbre;
  float morph;
  float trigger;
  float level;
  int frequency_patched;
  int timbre_patched;
  int morph_patched;
  int trigger_patched;
  int level_patched;
};

void* dk_plaits_new() {
  PlaitsVoice* v = new (std::nothrow) PlaitsVoice();
  if (!v) return nullptr;
  v->allocator.Init(v->ram, sizeof(v->ram));
  v->voice.Init(&v->allocator);
  return v;
}

void dk_plaits_free(void* p) { delete static_cast<PlaitsVoice*>(p); }

// Back to as made (silent), without allocating.
void dk_plaits_reset(void* p) {
  PlaitsVoice* v = static_cast<PlaitsVoice*>(p);
  v->allocator.Init(v->ram, sizeof(v->ram));
  v->voice.Init(&v->allocator);
}

// Renders `size` (at most 24) frames of the main and aux outputs, -1..1.
void dk_plaits_render(void* p, const DkPlaitsPatch* patch, const DkPlaitsModulations* mods,
                      float* out, float* aux, size_t size) {
  PlaitsVoice* v = static_cast<PlaitsVoice*>(p);
  plaits::Patch pa;
  pa.note = patch->note;
  pa.harmonics = patch->harmonics;
  pa.timbre = patch->timbre;
  pa.morph = patch->morph;
  pa.frequency_modulation_amount = patch->frequency_modulation_amount;
  pa.timbre_modulation_amount = patch->timbre_modulation_amount;
  pa.morph_modulation_amount = patch->morph_modulation_amount;
  pa.engine = patch->engine;
  pa.decay = patch->decay;
  pa.lpg_colour = patch->lpg_colour;
  plaits::Modulations m;
  m.engine = mods->engine;
  m.note = mods->note;
  m.frequency = mods->frequency;
  m.harmonics = mods->harmonics;
  m.timbre = mods->timbre;
  m.morph = mods->morph;
  m.trigger = mods->trigger;
  m.level = mods->level;
  m.frequency_patched = mods->frequency_patched != 0;
  m.timbre_patched = mods->timbre_patched != 0;
  m.morph_patched = mods->morph_patched != 0;
  m.trigger_patched = mods->trigger_patched != 0;
  m.level_patched = mods->level_patched != 0;
  plaits::Voice::Frame frames[plaits::kMaxBlockSize];
  if (size > plaits::kMaxBlockSize) size = plaits::kMaxBlockSize;
  v->voice.Render(pa, m, frames, size);
  for (size_t i = 0; i < size; ++i) {
    out[i] = frames[i].out / 32768.0f;
    aux[i] = frames[i].aux / 32768.0f;
  }
}

}  // extern "C"
