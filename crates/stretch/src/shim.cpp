// C functions over Signalsmith Stretch (vendor/, MIT), for src/lib.rs.
#include "signalsmith-stretch.h"

using Stretch = signalsmith::stretch::SignalsmithStretch<float>;

extern "C" {

void *deckard_stretch_new(int channels, float sample_rate, int cheaper) {
    auto *s = new Stretch(1234L);
    if (cheaper) {
        s->presetCheaper(channels, sample_rate);
    } else {
        s->presetDefault(channels, sample_rate);
    }
    return s;
}

void deckard_stretch_free(void *s) { delete static_cast<Stretch *>(s); }

void deckard_stretch_reset(void *s) { static_cast<Stretch *>(s)->reset(); }

int deckard_stretch_input_latency(void *s) { return static_cast<Stretch *>(s)->inputLatency(); }

int deckard_stretch_output_latency(void *s) { return static_cast<Stretch *>(s)->outputLatency(); }

void deckard_stretch_transpose(void *s, float factor) { static_cast<Stretch *>(s)->setTransposeFactor(factor); }

void deckard_stretch_formant(void *s, float factor, int compensate) {
    static_cast<Stretch *>(s)->setFormantFactor(factor, compensate != 0);
}

void deckard_stretch_seek(void *s, const float *const *inputs, int frames, double rate) {
    static_cast<Stretch *>(s)->seek(inputs, frames, rate);
}

void deckard_stretch_process(void *s, const float *const *inputs, int in_frames, float *const *outputs, int out_frames) {
    static_cast<Stretch *>(s)->process(inputs, in_frames, outputs, out_frames);
}

}
