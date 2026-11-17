#pragma once

#include <MediaToolbox/MediaToolbox.h>
#include <stdbool.h>

enum { NASTI_SPECTRUM_BANDS = 32 };

typedef struct {
    float rms;
    float subBass;
    float midRange;
    float highPresence;
    float bars[NASTI_SPECTRUM_BANDS];
} NastiSpectrum;

MTAudioProcessingTapRef NastiCreateSpectrumTap(void) CF_RETURNS_RETAINED;
bool NastiReadSpectrum(MTAudioProcessingTapRef tap, NastiSpectrum *spectrum);
