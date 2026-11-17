#include "spectrum.h"

#include <Accelerate/Accelerate.h>
#include <math.h>
#include <stdatomic.h>
#include <stdlib.h>

enum {
    FFT_POWER = 11,
    FFT_SIZE = 1 << FFT_POWER,
    FFT_BINS = FFT_SIZE / 2,
    FEATURE_COUNT = 4 + NASTI_SPECTRUM_BANDS
};

typedef struct {
    unsigned first;
    unsigned pastLast;
} BinRange;

typedef struct {
    FFTSetup fft;
    bool supportedFormat;
    size_t filled;
    float input[FFT_SIZE];
    float window[FFT_SIZE];
    float windowed[FFT_SIZE];
    float real[FFT_BINS];
    float imaginary[FFT_BINS];
    float power[FFT_BINS];
    BinRange ranges[3 + NASTI_SPECTRUM_BANDS];
    _Atomic unsigned sequence;
    _Atomic float output[FEATURE_COUNT];
} SpectrumTap;

static BinRange bins(double sampleRate, double low, double high) {
    double resolution = sampleRate / FFT_SIZE;
    unsigned first = (unsigned)fmax(1, fmin(FFT_BINS - 1, ceil(low / resolution)));
    unsigned pastLast = (unsigned)fmin(FFT_BINS, fmax(first + 1, ceil(high / resolution)));
    return (BinRange){first, pastLast};
}

static void initializeTap(MTAudioProcessingTapRef tap, void *clientInfo, void **storage) {
    (void)tap;
    (void)clientInfo;
    SpectrumTap *state = calloc(1, sizeof(*state));
    if (state) {
        state->fft = vDSP_create_fftsetup(FFT_POWER, kFFTRadix2);
        vDSP_hann_window(state->window, FFT_SIZE, vDSP_HANN_NORM);
        atomic_init(&state->sequence, 0);
        for (size_t i = 0; i < FEATURE_COUNT; ++i) {
            atomic_init(&state->output[i], 0.0f);
        }
    }
    *storage = state;
}

static void finalizeTap(MTAudioProcessingTapRef tap) {
    SpectrumTap *state = MTAudioProcessingTapGetStorage(tap);
    if (state && state->fft) {
        vDSP_destroy_fftsetup(state->fft);
    }
    free(state);
}

static void prepareTap(MTAudioProcessingTapRef tap, CMItemCount maxFrames,
                       const AudioStreamBasicDescription *format) {
    (void)maxFrames;
    SpectrumTap *state = MTAudioProcessingTapGetStorage(tap);
    if (!state) {
        return;
    }
    state->filled = 0;
    atomic_store(&state->sequence, 0);
    state->supportedFormat = state->fft && format->mSampleRate > 0.0 &&
                             format->mFormatID == kAudioFormatLinearPCM &&
                             (format->mFormatFlags & kAudioFormatFlagIsFloat) &&
                             format->mBitsPerChannel == 32;
    if (!state->supportedFormat) {
        return;
    }
    double rate = format->mSampleRate;
    state->ranges[0] = bins(rate, 20, 80);
    state->ranges[1] = bins(rate, 250, 2000);
    state->ranges[2] = bins(rate, 4000, 16000);
    for (size_t i = 0; i < NASTI_SPECTRUM_BANDS; ++i) {
        const double factor = 800.0;
        state->ranges[3 + i] = bins(rate,
            20.0 * pow(factor, (double)i / NASTI_SPECTRUM_BANDS),
            20.0 * pow(factor, (double)(i + 1) / NASTI_SPECTRUM_BANDS));
    }
}

static void unprepareTap(MTAudioProcessingTapRef tap) {
    SpectrumTap *state = MTAudioProcessingTapGetStorage(tap);
    if (state) {
        atomic_store(&state->sequence, 0);
    }
}

static float bandLevel(const float *power, BinRange range) {
    float peak = 0.0f;
    vDSP_maxv(power + range.first, 1, &peak, range.pastLast - range.first);
    // Hann window and real FFT scaling. Map -72 to -18 dBFS into the visual range.
    float amplitude = sqrtf(peak) / (FFT_SIZE / 2.0f);
    return fmaxf(0.0f, fminf(1.0f, (20.0f * log10f(fmaxf(1.0e-6f, amplitude)) + 72.0f) / 54.0f));
}

static void publishSpectrum(SpectrumTap *state) {
    float output[FEATURE_COUNT];
    vDSP_rmsqv(state->input, 1, &output[0], FFT_SIZE);
    output[0] = fminf(1.0f, output[0]);
    vDSP_vmul(state->input, 1, state->window, 1, state->windowed, 1, FFT_SIZE);
    DSPSplitComplex split = {state->real, state->imaginary};
    vDSP_ctoz((const DSPComplex *)state->windowed, 2, &split, 1, FFT_BINS);
    vDSP_fft_zrip(state->fft, &split, 1, FFT_POWER, FFT_FORWARD);
    vDSP_zvmags(&split, 1, state->power, 1, FFT_BINS);
    for (size_t i = 1; i < FEATURE_COUNT; ++i) {
        output[i] = bandLevel(state->power, state->ranges[i - 1]);
    }
    // A reader sees either a complete frame or no frame. The audio callback never waits.
    atomic_fetch_add(&state->sequence, 1);
    for (size_t i = 0; i < FEATURE_COUNT; ++i) {
        atomic_store(&state->output[i], output[i]);
    }
    atomic_fetch_add(&state->sequence, 1);
}

static float monoSample(const AudioBufferList *list, size_t frame) {
    float sum = 0.0f;
    unsigned count = 0;
    for (UInt32 i = 0; i < list->mNumberBuffers; ++i) {
        const AudioBuffer *buffer = &list->mBuffers[i];
        size_t channels = buffer->mNumberChannels;
        if (!buffer->mData || channels == 0 ||
            (frame + 1) * channels > buffer->mDataByteSize / sizeof(float)) {
            continue;
        }
        const float *samples = (const float *)buffer->mData + frame * channels;
        for (size_t channel = 0; channel < channels; ++channel) {
            sum += samples[channel];
        }
        count += channels;
    }
    return count && isfinite(sum) ? sum / count : 0.0f;
}

static void processTap(MTAudioProcessingTapRef tap, CMItemCount frames,
                       MTAudioProcessingTapFlags flags, AudioBufferList *buffers,
                       CMItemCount *framesOut, MTAudioProcessingTapFlags *flagsOut) {
    (void)flags;
    OSStatus result = MTAudioProcessingTapGetSourceAudio(tap, frames, buffers, flagsOut, NULL, framesOut);
    SpectrumTap *state = MTAudioProcessingTapGetStorage(tap);
    if (result != noErr || !state || !state->supportedFormat) {
        return;
    }
    if (*flagsOut & kMTAudioProcessingTapFlag_StartOfStream) {
        state->filled = 0;
    }
    for (size_t frame = 0; frame < (size_t)*framesOut; ++frame) {
        state->input[state->filled++] = monoSample(buffers, frame);
        if (state->filled == FFT_SIZE) {
            publishSpectrum(state);
            state->filled = 0;
        }
    }
}

MTAudioProcessingTapRef NastiCreateSpectrumTap(void) {
    MTAudioProcessingTapCallbacks callbacks;
    callbacks.version = kMTAudioProcessingTapCallbacksVersion_0;
    callbacks.clientInfo = NULL;
    callbacks.init = initializeTap;
    callbacks.finalize = finalizeTap;
    callbacks.prepare = prepareTap;
    callbacks.unprepare = unprepareTap;
    callbacks.process = processTap;
    MTAudioProcessingTapRef tap = NULL;
    MTAudioProcessingTapCreate(kCFAllocatorDefault, &callbacks, kMTAudioProcessingTapCreationFlag_PostEffects, &tap);
    return tap;
}

bool NastiReadSpectrum(MTAudioProcessingTapRef tap, NastiSpectrum *spectrum) {
    SpectrumTap *state = tap ? MTAudioProcessingTapGetStorage(tap) : NULL;
    if (!state) {
        return false;
    }
    unsigned before = atomic_load(&state->sequence);
    if (before == 0 || before % 2 == 1) {
        return false;
    }
    spectrum->rms = atomic_load(&state->output[0]);
    spectrum->subBass = atomic_load(&state->output[1]);
    spectrum->midRange = atomic_load(&state->output[2]);
    spectrum->highPresence = atomic_load(&state->output[3]);
    for (size_t i = 0; i < NASTI_SPECTRUM_BANDS; ++i) {
        spectrum->bars[i] = atomic_load(&state->output[4 + i]);
    }
    return before == atomic_load(&state->sequence);
}
