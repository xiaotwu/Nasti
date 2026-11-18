#pragma once

#include <stdbool.h>
#include <stdint.h>

typedef struct {
    void *context;
    void (*finish)(void *context);
    void (*restart)(void *context);
    void (*release)(void *context);
} NastiPlaybackSource;

void nasti_native_load(const char *url, const char *audio_url, const char *video_id, const char *title, const char *artist, bool is_video,
                       uint64_t position_ms, bool autoplay, NastiPlaybackSource source);
void nasti_native_prepare(const char *url, const char *audio_url, const char *video_id, const char *title, const char *artist, bool is_video, NastiPlaybackSource source);
bool nasti_native_prepare_parked_audio(const char *video_id);
char *nasti_native_copy_preparation(void);
void nasti_native_seek_preparation(uint64_t position_ms);
bool nasti_native_commit_preparation(bool autoplay, bool align_time, uint64_t position_ms);
void nasti_native_cancel_preparation(void);
void nasti_native_set_playing(bool playing);
void nasti_native_seek(uint64_t position_ms);
void nasti_native_set_volume(float volume);
char *nasti_native_copy_outputs(void);
bool nasti_native_set_output(const char *identifier);
bool nasti_native_set_video_frame(void *window, bool shown, double x, double y, double width, double height);
char *nasti_native_copy_snapshot(void);
void nasti_native_free_snapshot(char *text);
void nasti_native_shutdown(void);
