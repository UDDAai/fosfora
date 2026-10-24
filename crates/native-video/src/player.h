#pragma once

#include <stddef.h>
#include <stdint.h>

/* C ABI for the macOS playback harness. Rust keeps an equivalent layout and
 * checks the sizes at runtime. The player object is not thread-safe: create,
 * pump, copy, and close it on one thread.
 */

#ifdef __cplusplus
extern "C" {
#endif

typedef struct NvMediaInfo {
    uint32_t width;
    uint32_t height;
    double fps;
    double duration_secs;
    int32_t hardware; /* 1 VideoToolbox can decode, 0 it cannot, -1 unknown */
    int32_t reserved;
    char codec[8];
    char note[256];
} NvMediaInfo;

typedef struct NvFrameInfo {
    double pts_secs;
    double item_secs;
    uint32_t width;
    uint32_t height;
    uint32_t rgba_bytes;
    int32_t held_after;
} NvFrameInfo;

size_t nv_media_info_size(void);
size_t nv_frame_info_size(void);

/* Process physical footprint (Activity Monitor's memory number). */
uint64_t nv_phys_footprint(void);

/* 0 playable, 1 unsupported, -1 could not inspect. */
int nv_probe(const char *path, NvMediaInfo *info, char *err, size_t err_len);

/* +1 player, or NULL. The caller frees it with nv_close. */
void *nv_open(const char *path, NvMediaInfo *info, char *err, size_t err_len);
void nv_close(void *player);

/* rate 0 pauses. Negative rates are ignored: reverse is not supported. */
void nv_set_rate(void *player, double rate);
void nv_seek(void *player, double secs);
void nv_pump(void *player, double seconds);
double nv_position(void *player);

/* 1 copied a tightly packed RGBA frame, 0 none ready, -1 failed. */
int nv_copy_rgba(void *player, uint8_t *dst, size_t dst_cap, NvFrameInfo *info);
void nv_last_error(void *player, char *err, size_t err_len);

/* 0 wrote RGBA of one Metal-sampled pixel, -2 no GPU, -1 failed. */
int nv_metal_sample(void *player, uint8_t rgba_out[4], char *err, size_t err_len);

#ifdef __cplusplus
}
#endif
