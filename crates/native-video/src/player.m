#import "player.h"

#import <AVFoundation/AVFoundation.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <QuartzCore/QuartzCore.h>
#import <VideoToolbox/VideoToolbox.h>
#import <mach/mach.h>
#import <math.h>
#import <stdio.h>
#import <string.h>

static void set_err(char *err, size_t err_len, NSString *message) {
    if (err == NULL || err_len == 0) {
        return;
    }
    const char *utf = message.UTF8String;
    if (utf == NULL) {
        utf = "unknown error";
    }
    snprintf(err, err_len, "%s", utf);
}

static NSURL *file_url(const char *path, char *err, size_t err_len) {
    if (path == NULL || path[0] == '\0') {
        set_err(err, err_len, @"empty path");
        return nil;
    }
    return [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
}

static BOOL load_asset_keys(AVURLAsset *asset, NSArray<NSString *> *keys, NSString **error) {
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    [asset loadValuesAsynchronouslyForKeys:keys
                         completionHandler:^{
                           dispatch_semaphore_signal(sem);
                         }];
    if (dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(2.5 * NSEC_PER_SEC))) !=
        0) {
        *error = @"timed out opening the asset";
        return NO;
    }
    for (NSString *key in keys) {
        NSError *key_error = nil;
        AVKeyValueStatus status = [asset statusOfValueForKey:key error:&key_error];
        if (status == AVKeyValueStatusFailed || status == AVKeyValueStatusCancelled) {
            *error = key_error.localizedDescription ?: [NSString stringWithFormat:@"could not read %@", key];
            return NO;
        }
    }
    return YES;
}

static BOOL load_track_keys(AVAssetTrack *track, NSString **error) {
    NSArray<NSString *> *keys = @[
        @"naturalSize",
        @"preferredTransform",
        @"nominalFrameRate",
        @"formatDescriptions",
    ];
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    [track loadValuesAsynchronouslyForKeys:keys
                         completionHandler:^{
                           dispatch_semaphore_signal(sem);
                         }];
    if (dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(2.5 * NSEC_PER_SEC))) !=
        0) {
        *error = @"timed out reading the video track";
        return NO;
    }
    for (NSString *key in keys) {
        NSError *key_error = nil;
        AVKeyValueStatus status = [track statusOfValueForKey:key error:&key_error];
        if (status == AVKeyValueStatusFailed || status == AVKeyValueStatusCancelled) {
            *error = key_error.localizedDescription ?: [NSString stringWithFormat:@"could not read %@", key];
            return NO;
        }
    }
    return YES;
}

static void write_codec(AVAssetTrack *track, NvMediaInfo *info) {
    info->hardware = -1;
    memset(info->codec, 0, sizeof info->codec);
    NSArray *formats = track.formatDescriptions;
    if (formats.count == 0) {
        snprintf(info->note, sizeof info->note, "%s",
                 "no format description; hardware decode was not confirmed");
        return;
    }
    CMFormatDescriptionRef format = (__bridge CMFormatDescriptionRef)formats[0];
    FourCharCode codec = CMFormatDescriptionGetMediaSubType(format);
    char four[5] = {0, 0, 0, 0, 0};
    four[0] = (char)((codec >> 24) & 0xff);
    four[1] = (char)((codec >> 16) & 0xff);
    four[2] = (char)((codec >> 8) & 0xff);
    four[3] = (char)(codec & 0xff);
    for (int i = 0; i < 4; i++) {
        unsigned char byte = (unsigned char)four[i];
        if (byte < 32 || byte > 126) {
            four[i] = '?';
        }
    }
    snprintf(info->codec, sizeof info->codec, "%s", four);
    if (VTIsHardwareDecodeSupported(codec)) {
        info->hardware = 1;
        snprintf(info->note, sizeof info->note, "VideoToolbox hardware decode is supported for %s", four);
    } else {
        info->hardware = 0;
        snprintf(info->note, sizeof info->note,
                 "VideoToolbox hardware decode is not available for %s; AVFoundation may use a software decoder",
                 four);
    }
}

static BOOL fill_info(AVURLAsset *asset, AVAssetTrack *track, NvMediaInfo *info, NSString **error) {
    if (!asset.playable) {
        *error = @"AVFoundation reports this file is not playable";
        return NO;
    }
    Float64 duration = CMTimeGetSeconds(asset.duration);
    if (!isfinite(duration) || duration <= 0.0) {
        *error = @"AVFoundation did not report a positive duration";
        return NO;
    }
    CGSize natural = track.naturalSize;
    uint32_t width = (uint32_t)llround(fabs(natural.width));
    uint32_t height = (uint32_t)llround(fabs(natural.height));
    if (width < 2 || height < 2) {
        *error = @"video track has no usable dimensions";
        return NO;
    }
    float fps = track.nominalFrameRate;
    if (!(fps > 1.0f && fps < 480.0f)) {
        fps = 30.0f;
    }
    memset(info, 0, sizeof *info);
    info->width = width;
    info->height = height;
    info->fps = (double)fps;
    info->duration_secs = duration;
    write_codec(track, info);
    return YES;
}

/* Loads a playable video asset. On failure `*unsupported` is YES when the
 * container or codec is outside what AVFoundation will play, and NO for
 * timeouts and I/O failures. Both are reported to the caller; neither starts
 * a decode.
 */
static AVURLAsset *open_asset(const char *path, AVAssetTrack **track_out, NvMediaInfo *info, int *unsupported,
                              char *err, size_t err_len) {
    *unsupported = 0;
    *track_out = nil;
    NSURL *url = file_url(path, err, err_len);
    if (url == nil) {
        *unsupported = 1;
        return nil;
    }
    AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:nil];
    NSString *error = nil;
    if (!load_asset_keys(asset, @[ @"playable", @"duration", @"tracks" ], &error)) {
        set_err(err, err_len, error);
        return nil;
    }
    if (!asset.playable) {
        *unsupported = 1;
        set_err(err, err_len, @"AVFoundation reports this file is not playable");
        return nil;
    }
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    __block NSArray<AVAssetTrack *> *tracks = nil;
    __block NSError *track_error = nil;
    [asset loadTracksWithMediaType:AVMediaTypeVideo
                 completionHandler:^(NSArray<AVAssetTrack *> *loaded, NSError *failure) {
                   tracks = loaded;
                   track_error = failure;
                   dispatch_semaphore_signal(sem);
                 }];
    if (dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(2.5 * NSEC_PER_SEC))) != 0) {
        set_err(err, err_len, @"timed out loading video tracks");
        return nil;
    }
    if (track_error != nil) {
        *unsupported = 1;
        set_err(err, err_len, track_error.localizedDescription);
        return nil;
    }
    AVAssetTrack *track = tracks.firstObject;
    if (track == nil) {
        *unsupported = 1;
        set_err(err, err_len, @"file has no video track AVFoundation can play");
        return nil;
    }
    if (!load_track_keys(track, &error)) {
        set_err(err, err_len, error);
        return nil;
    }
    if (!fill_info(asset, track, info, &error)) {
        *unsupported = 1;
        set_err(err, err_len, error);
        return nil;
    }
    *track_out = track;
    return asset;
}

@interface NvBox : NSObject
@property(nonatomic, strong) AVPlayer *player;
@property(nonatomic, strong) AVPlayerItem *item;
@property(nonatomic, strong) AVPlayerItemVideoOutput *output;
@property(nonatomic, strong) AVURLAsset *asset;
@property(nonatomic, assign) double rate;
@property(nonatomic, assign) double duration;
@property(nonatomic, assign) int held;
@property(nonatomic, strong) id<MTLDevice> device;
@property(nonatomic, assign) CVMetalTextureCacheRef cache;
@property(nonatomic, copy) NSString *lastError;
@end

@implementation NvBox
- (void)dealloc {
    [_player pause];
    _player = nil;
    _item = nil;
    _output = nil;
    _asset = nil;
    if (_cache != NULL) {
        CVMetalTextureCacheFlush(_cache, 0);
        CFRelease(_cache);
        _cache = NULL;
    }
}
@end

static NvBox *box_from(void *player) { return (__bridge NvBox *)player; }

static NSDictionary *pixel_attributes(void) {
    return @{
        (id)kCVPixelBufferPixelFormatTypeKey : @(kCVPixelFormatType_32BGRA),
        (id)kCVPixelBufferMetalCompatibilityKey : @YES,
        (id)kCVPixelBufferIOSurfacePropertiesKey : @{},
    };
}

static BOOL copy_bgra_to_rgba(CVPixelBufferRef buffer, uint8_t *dst, size_t dst_cap, NvFrameInfo *info,
                              NSString **error) {
    OSType format = CVPixelBufferGetPixelFormatType(buffer);
    if (format != kCVPixelFormatType_32BGRA) {
        *error = [NSString stringWithFormat:@"pixel buffer format %u is not BGRA", (unsigned)format];
        return NO;
    }
    size_t width = CVPixelBufferGetWidth(buffer);
    size_t height = CVPixelBufferGetHeight(buffer);
    size_t stride = CVPixelBufferGetBytesPerRow(buffer);
    if (width == 0 || height == 0 || width > UINT32_MAX || height > UINT32_MAX) {
        *error = @"pixel buffer has no usable dimensions";
        return NO;
    }
    size_t need = width * height * 4;
    if (need > dst_cap) {
        *error = @"RGBA staging buffer is smaller than the frame";
        return NO;
    }
    CVReturn locked = CVPixelBufferLockBaseAddress(buffer, kCVPixelBufferLock_ReadOnly);
    if (locked != kCVReturnSuccess) {
        *error = @"could not lock the pixel buffer";
        return NO;
    }
    uint8_t *src = CVPixelBufferGetBaseAddress(buffer);
    if (src == NULL) {
        CVPixelBufferUnlockBaseAddress(buffer, kCVPixelBufferLock_ReadOnly);
        *error = @"pixel buffer has no CPU address";
        return NO;
    }
    for (size_t y = 0; y < height; y++) {
        uint8_t *row = src + y * stride;
        for (size_t x = 0; x < width; x++) {
            uint8_t *pixel = row + x * 4;
            dst[0] = pixel[2];
            dst[1] = pixel[1];
            dst[2] = pixel[0];
            dst[3] = pixel[3];
            dst += 4;
        }
    }
    CVPixelBufferUnlockBaseAddress(buffer, kCVPixelBufferLock_ReadOnly);
    if (info != NULL) {
        info->width = (uint32_t)width;
        info->height = (uint32_t)height;
        info->rgba_bytes = (uint32_t)need;
    }
    return YES;
}

size_t nv_media_info_size(void) { return sizeof(NvMediaInfo); }

size_t nv_frame_info_size(void) { return sizeof(NvFrameInfo); }

uint64_t nv_phys_footprint(void) {
    task_vm_info_data_t info;
    mach_msg_type_number_t count = TASK_VM_INFO_COUNT;
    kern_return_t result = task_info(mach_task_self(), TASK_VM_INFO, (task_info_t)&info, &count);
    if (result != KERN_SUCCESS) {
        return 0;
    }
    return (uint64_t)info.phys_footprint;
}

int nv_probe(const char *path, NvMediaInfo *info, char *err, size_t err_len) {
    if (info == NULL) {
        set_err(err, err_len, @"missing probe output");
        return -1;
    }
    memset(info, 0, sizeof *info);
    int unsupported = 0;
    AVAssetTrack *track = nil;
    AVURLAsset *asset = open_asset(path, &track, info, &unsupported, err, err_len);
    if (asset == nil || track == nil) {
        return unsupported ? 1 : -1;
    }
    return 0;
}

void *nv_open(const char *path, NvMediaInfo *info, char *err, size_t err_len) {
    NvMediaInfo local;
    NvMediaInfo *out = info != NULL ? info : &local;
    memset(out, 0, sizeof *out);
    int unsupported = 0;
    AVAssetTrack *track = nil;
    AVURLAsset *asset = open_asset(path, &track, out, &unsupported, err, err_len);
    if (asset == nil || track == nil) {
        return NULL;
    }
    AVPlayerItem *item = [AVPlayerItem playerItemWithAsset:asset];
    item.preferredForwardBufferDuration = 0.5;
    AVPlayerItemVideoOutput *output = [[AVPlayerItemVideoOutput alloc] initWithPixelBufferAttributes:pixel_attributes()];
    if (output == nil) {
        set_err(err, err_len, @"could not create AVPlayerItemVideoOutput");
        return NULL;
    }
    output.suppressesPlayerRendering = YES;
    [item addOutput:output];
    AVPlayer *player = [AVPlayer playerWithPlayerItem:item];
    player.actionAtItemEnd = AVPlayerActionAtItemEndPause;
    player.automaticallyWaitsToMinimizeStalling = NO;
    player.muted = YES;
    player.volume = 0;
    NvBox *box = [NvBox new];
    box.asset = asset;
    box.item = item;
    box.output = output;
    box.player = player;
    box.duration = out->duration_secs;
    box.rate = 0;
    box.held = 0;
    [output requestNotificationOfMediaDataChangeWithAdvanceInterval:1.0 / 30.0];
    return CFBridgingRetain(box);
}

void nv_close(void *player) {
    if (player == NULL) {
        return;
    }
    NvBox *box = CFBridgingRelease(player);
    [box.player pause];
    box.player = nil;
}

void nv_set_rate(void *player, double rate) {
    NvBox *box = box_from(player);
    if (box == nil) {
        return;
    }
    if (!isfinite(rate) || rate < 0.0) {
        rate = 0.0;
    }
    box.rate = rate;
    box.player.rate = (float)rate;
    if (rate > 0.0) {
        [box.output requestNotificationOfMediaDataChangeWithAdvanceInterval:1.0 / 30.0];
    }
}

void nv_seek(void *player, double secs) {
    NvBox *box = box_from(player);
    if (box == nil) {
        return;
    }
    if (!isfinite(secs) || secs < 0.0) {
        secs = 0.0;
    }
    if (box.duration > 0.0 && secs > box.duration) {
        secs = fmax(0.0, box.duration - 0.001);
    }
    CMTime time = CMTimeMakeWithSeconds(secs, 600);
    [box.player seekToTime:time toleranceBefore:kCMTimeZero toleranceAfter:kCMTimeZero];
}

void nv_pump(void *player, double seconds) {
    (void)player;
    if (!(seconds > 0.0)) {
        seconds = 0.001;
    }
    CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds, false);
}

double nv_position(void *player) {
    NvBox *box = box_from(player);
    if (box == nil) {
        return 0;
    }
    Float64 secs = CMTimeGetSeconds(box.player.currentTime);
    if (!isfinite(secs) || secs < 0.0) {
        return 0;
    }
    return secs;
}

void nv_last_error(void *player, char *err, size_t err_len) {
    NvBox *box = box_from(player);
    if (box == nil) {
        set_err(err, err_len, @"missing player");
        return;
    }
    set_err(err, err_len, box.lastError ?: @"playback failed");
}

int nv_copy_rgba(void *player, uint8_t *dst, size_t dst_cap, NvFrameInfo *info) {
    NvBox *box = box_from(player);
    if (box == nil || dst == NULL) {
        return -1;
    }
    if (info != NULL) {
        memset(info, 0, sizeof *info);
    }
    if (box.item.status == AVPlayerItemStatusFailed) {
        box.lastError = box.item.error.localizedDescription ?: @"playback failed";
        return -1;
    }
    if (box.held != 0) {
        box.lastError = @"a pixel buffer was still retained";
        return -1;
    }
    CMTime item_time = [box.output itemTimeForHostTime:CACurrentMediaTime()];
    if (![box.output hasNewPixelBufferForItemTime:item_time]) {
        return 0;
    }
    CMTime display = kCMTimeInvalid;
    CVPixelBufferRef buffer = [box.output copyPixelBufferForItemTime:item_time itemTimeForDisplay:&display];
    if (buffer == NULL) {
        return 0;
    }
    box.held += 1;
    NSString *error = nil;
    BOOL copied = copy_bgra_to_rgba(buffer, dst, dst_cap, info, &error);
    CVPixelBufferRelease(buffer);
    box.held -= 1;
    if (!copied) {
        box.lastError = error ?: @"could not copy the pixel buffer";
        return -1;
    }
    if (info != NULL) {
        Float64 pts = CMTimeGetSeconds(display);
        if (!isfinite(pts) || pts < 0.0) {
            pts = CMTimeGetSeconds(item_time);
        }
        if (!isfinite(pts) || pts < 0.0) {
            pts = 0;
        }
        info->pts_secs = pts;
        Float64 item = CMTimeGetSeconds(item_time);
        info->item_secs = isfinite(item) && item > 0.0 ? item : pts;
        info->held_after = box.held;
    }
    return 1;
}

int nv_metal_sample(void *player, uint8_t rgba_out[4], char *err, size_t err_len) {
    NvBox *box = box_from(player);
    if (box == nil || rgba_out == NULL) {
        set_err(err, err_len, @"missing Metal preview target");
        return -1;
    }
    if (box.device == nil) {
        box.device = MTLCreateSystemDefaultDevice();
    }
    if (box.device == nil) {
        set_err(err, err_len, @"no Metal device");
        return -2;
    }
    if (box.cache == NULL) {
        CVMetalTextureCacheRef cache = NULL;
        CVReturn created = CVMetalTextureCacheCreate(kCFAllocatorDefault, NULL, box.device, NULL, &cache);
        if (created != kCVReturnSuccess || cache == NULL) {
            set_err(err, err_len, @"CVMetalTextureCacheCreate failed");
            return -1;
        }
        box.cache = cache;
    }
    CVPixelBufferRef buffer = NULL;
    for (int attempt = 0; attempt < 12 && buffer == NULL; attempt++) {
        CMTime item_time = [box.output itemTimeForHostTime:CACurrentMediaTime()];
        CMTime display = kCMTimeInvalid;
        buffer = [box.output copyPixelBufferForItemTime:item_time itemTimeForDisplay:&display];
        if (buffer == NULL) {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.03, false);
        }
    }
    if (buffer == NULL) {
        set_err(err, err_len, @"no pixel buffer was available for the Metal preview");
        return -1;
    }
    box.held += 1;
    size_t width = CVPixelBufferGetWidth(buffer);
    size_t height = CVPixelBufferGetHeight(buffer);
    OSType format = CVPixelBufferGetPixelFormatType(buffer);
    if (format != kCVPixelFormatType_32BGRA || width == 0 || height == 0) {
        CVPixelBufferRelease(buffer);
        box.held -= 1;
        set_err(err, err_len, @"Metal preview expected a BGRA pixel buffer");
        return -1;
    }
    CVMetalTextureRef cv_texture = NULL;
    CVReturn wrapped = CVMetalTextureCacheCreateTextureFromImage(kCFAllocatorDefault, box.cache, buffer, NULL,
                                                                  MTLPixelFormatBGRA8Unorm, width, height, 0, &cv_texture);
    CVPixelBufferRelease(buffer);
    box.held -= 1;
    if (wrapped != kCVReturnSuccess || cv_texture == NULL) {
        set_err(err, err_len, @"CVMetalTextureCacheCreateTextureFromImage failed");
        return -1;
    }
    id<MTLTexture> texture = CVMetalTextureGetTexture(cv_texture);
    if (texture == nil) {
        CFRelease(cv_texture);
        CVMetalTextureCacheFlush(box.cache, 0);
        set_err(err, err_len, @"CVMetalTextureGetTexture returned nothing");
        return -1;
    }
    const NSUInteger row_bytes = 256;
    id<MTLBuffer> readback = [box.device newBufferWithLength:row_bytes options:MTLResourceStorageModeShared];
    id<MTLCommandQueue> queue = [box.device newCommandQueue];
    id<MTLCommandBuffer> commands = [queue commandBuffer];
    id<MTLBlitCommandEncoder> blit = [commands blitCommandEncoder];
    [blit copyFromTexture:texture
                 sourceSlice:0
                 sourceLevel:0
                sourceOrigin:MTLOriginMake(0, 0, 0)
                  sourceSize:MTLSizeMake(1, 1, 1)
                    toBuffer:readback
           destinationOffset:0
      destinationBytesPerRow:row_bytes
    destinationBytesPerImage:row_bytes];
    [blit endEncoding];
    [commands commit];
    [commands waitUntilCompleted];
    if (commands.status != MTLCommandBufferStatusCompleted) {
        CFRelease(cv_texture);
        CVMetalTextureCacheFlush(box.cache, 0);
        set_err(err, err_len, @"Metal preview blit did not complete");
        return -1;
    }
    uint8_t *bytes = readback.contents;
    rgba_out[0] = bytes[2];
    rgba_out[1] = bytes[1];
    rgba_out[2] = bytes[0];
    rgba_out[3] = bytes[3];
    CFRelease(cv_texture);
    CVMetalTextureCacheFlush(box.cache, 0);
    if (box.held != 0) {
        set_err(err, err_len, @"a pixel buffer was still retained after the Metal preview");
        return -1;
    }
    return 0;
}
