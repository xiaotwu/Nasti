#import "playback.h"
#import "playback-outputs.h"
#import "spectrum.h"

#import <AppKit/AppKit.h>
#import <AVFoundation/AVFoundation.h>
#import <QuartzCore/QuartzCore.h>
#import <WebKit/WebKit.h>
#include <math.h>

@interface NastiVideoSurface : NSView
@property(nonatomic, readonly) AVPlayerLayer *playerLayer;
@end

@implementation NastiVideoSurface

- (instancetype)initWithFrame:(NSRect)frame {
    self = [super initWithFrame:frame];
    if (self) {
        self.wantsLayer = YES;
        self.layer.cornerRadius = 16.0;
        self.layer.masksToBounds = YES;
        self.layer.backgroundColor = NSColor.blackColor.CGColor;
        _playerLayer = [AVPlayerLayer playerLayerWithPlayer:nil];
        _playerLayer.videoGravity = AVLayerVideoGravityResizeAspect;
        _playerLayer.frame = self.bounds;
        [self.layer addSublayer:_playerLayer];
    }
    return self;
}

- (void)layout {
    [super layout];
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    self.playerLayer.frame = self.bounds;
    [CATransaction commit];
}

- (NSView *)hitTest:(NSPoint)point {
    (void)point;
    return nil;
}

@end

static NSView *findWebView(NSView *view) {
    if ([view isKindOfClass:WKWebView.class]) {
        return view;
    }
    for (NSView *child in view.subviews) {
        NSView *found = findWebView(child);
        if (found) {
            return found;
        }
    }
    return nil;
}

static uint64_t milliseconds(CMTime time) {
    double value = CMTimeGetSeconds(time) * 1000.0;
    return isfinite(value) && value > 0.0 && value < (double)INT64_MAX ? (uint64_t)llround(value) : 0;
}

static NSDictionary *spectrumDictionary(MTAudioProcessingTapRef tap) {
    NastiSpectrum spectrum;
    if (!NastiReadSpectrum(tap, &spectrum)) {
        return nil;
    }
    NSMutableArray<NSNumber *> *bars = [NSMutableArray arrayWithCapacity:NASTI_SPECTRUM_BANDS];
    for (size_t i = 0; i < NASTI_SPECTRUM_BANDS; ++i) {
        [bars addObject:@(spectrum.bars[i])];
    }
    return @{
        @"rms": @(spectrum.rms),
        @"subBass": @(spectrum.subBass),
        @"midRange": @(spectrum.midRange),
        @"highPresence": @(spectrum.highPresence),
        @"bars32": bars
    };
}

@interface NastiPlayerController : NSObject
- (void)loadURL:(NSURL *)url identifier:(NSString *)identifier video:(BOOL)video
     positionMs:(uint64_t)position playbackRequested:(BOOL)requested source:(NastiPlaybackSource)source;
- (void)setPlaybackRequested:(BOOL)requested;
- (void)startAlignedAtMilliseconds:(uint64_t)position;
- (void)seekMilliseconds:(uint64_t)position;
- (void)setVolume:(float)volume;
- (void)setAudioOutputUID:(NSString *)identifier;
- (BOOL)setVideoFrame:(NSRect)rect window:(NSWindow *)window;
- (void)clearVideoFrame;
- (NSDictionary *)snapshot;
- (void)shutdown;
@end

static void *itemStatusContext = &itemStatusContext;

@implementation NastiPlayerController {
    AVPlayer *_player;
    AVURLAsset *_asset;
    AVPlayerItem *_item;
    MTAudioProcessingTapRef _tap;
    id _endNotification;
    id _errorNotification;
    NSString *_identifier;
    NSString *_playbackError;
    NastiVideoSurface *_videoSurface;
    NastiPlaybackSource _source;
    uint64_t _requestedPosition;
    uint64_t _seekRevision;
    BOOL _videoMode;
    BOOL _wantsPlaying;
    BOOL _transportUsed;
    BOOL _positionPending;
    BOOL _itemReady;
    BOOL _ended;
}

- (instancetype)init {
    self = [super init];
    if (self) {
        _player = [[AVPlayer alloc] init];
        _player.automaticallyWaitsToMinimizeStalling = NO;
        _player.actionAtItemEnd = AVPlayerActionAtItemEndPause;
    }
    return self;
}

- (void)replaceCurrentAsset {
    _seekRevision += 1;
    [_asset cancelLoading];
    [_player pause];
    _videoSurface.hidden = YES;
    _videoSurface.playerLayer.player = nil;
    if (_item) {
        [_item cancelPendingSeeks];
        [_item removeObserver:self forKeyPath:@"status" context:itemStatusContext];
    }
    if (_endNotification) {
        [NSNotificationCenter.defaultCenter removeObserver:_endNotification];
        _endNotification = nil;
    }
    if (_errorNotification) {
        [NSNotificationCenter.defaultCenter removeObserver:_errorNotification];
        _errorNotification = nil;
    }
    [_player replaceCurrentItemWithPlayerItem:nil];
    _item = nil;
    _asset = nil;
    if (_source.release) {
        _source.release(_source.context);
        _source = (NastiPlaybackSource){0};
    }
    if (_tap) {
        CFRelease(_tap);
        _tap = NULL;
    }
    _itemReady = NO;
    _ended = NO;
}

- (void)reportError:(NSError *)error {
    _wantsPlaying = NO;
    _positionPending = NO;
    _playbackError = error
        ? [NSString stringWithFormat:@"媒体播放失败（%@ %ld）", error.domain, (long)error.code]
        : @"媒体播放失败";
    [_player pause];
    _videoSurface.hidden = YES;
    if (_source.finish) {
        _source.finish(_source.context);
    }
}

- (void)loadURL:(NSURL *)url identifier:(NSString *)identifier video:(BOOL)video
     positionMs:(uint64_t)position playbackRequested:(BOOL)requested source:(NastiPlaybackSource)source {
    [self replaceCurrentAsset];
    _source = source;
    _identifier = [identifier copy];
    _videoMode = video;
    _playbackError = nil;
    _wantsPlaying = requested;
    _transportUsed = requested;
    _requestedPosition = position;
    _positionPending = position != 0;
    if (!url || (!url.isFileURL && !([url.scheme isEqualToString:@"http"] &&
                                    [url.host isEqualToString:@"127.0.0.1"]))) {
        _playbackError = @"授权曲目缺少有效媒体地址";
        if (_source.finish) {
            _source.finish(_source.context);
        }
        return;
    }

    AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:nil];
    _asset = asset;
    __weak NastiPlayerController *weakSelf = self;
    [asset loadTracksWithMediaType:AVMediaTypeAudio completionHandler:^(NSArray<AVAssetTrack *> *tracks, NSError *error) {
        dispatch_async(dispatch_get_main_queue(), ^{
            NastiPlayerController *player = weakSelf;
            if (!player || player->_asset != asset) {
                return;
            }
            if (error) {
                [player reportError:error];
                return;
            }
            [player attachItem:[AVPlayerItem playerItemWithAsset:asset] audioTrack:tracks.firstObject];
        });
    }];
}

- (void)attachItem:(AVPlayerItem *)item audioTrack:(AVAssetTrack *)track {
    _item = item;
    if (track) {
        _tap = NastiCreateSpectrumTap();
        if (_tap) {
            AVMutableAudioMixInputParameters *params = [AVMutableAudioMixInputParameters audioMixInputParametersWithTrack:track];
            params.audioTapProcessor = _tap;
            AVMutableAudioMix *mix = [AVMutableAudioMix audioMix];
            mix.inputParameters = @[params];
            item.audioMix = mix;
        }
    }
    __weak NastiPlayerController *weakSelf = self;
    _endNotification = [NSNotificationCenter.defaultCenter
        addObserverForName:AVPlayerItemDidPlayToEndTimeNotification object:item
        queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *notification) {
            NastiPlayerController *player = weakSelf;
            if (player && player->_item == notification.object) {
                player->_ended = YES;
                player->_wantsPlaying = NO;
                player->_positionPending = NO;
                if (player->_source.finish) {
                    player->_source.finish(player->_source.context);
                }
            }
        }];
    _errorNotification = [NSNotificationCenter.defaultCenter
        addObserverForName:AVPlayerItemFailedToPlayToEndTimeNotification object:item
        queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *notification) {
            NastiPlayerController *player = weakSelf;
            if (player && player->_item == notification.object) {
                [player reportError:notification.userInfo[AVPlayerItemFailedToPlayToEndTimeErrorKey]];
            }
        }];
    [item addObserver:self forKeyPath:@"status" options:NSKeyValueObservingOptionInitial | NSKeyValueObservingOptionNew
              context:itemStatusContext];
    [_player replaceCurrentItemWithPlayerItem:item];
}

- (void)observeValueForKeyPath:(NSString *)keyPath ofObject:(id)object
                       change:(NSDictionary<NSKeyValueChangeKey, id> *)change context:(void *)context {
    if (context != itemStatusContext) {
        [super observeValueForKeyPath:keyPath ofObject:object change:change context:context];
        return;
    }
    __weak NastiPlayerController *weakSelf = self;
    dispatch_async(dispatch_get_main_queue(), ^{
        [weakSelf updateItemStatus:object];
    });
}

- (void)updateItemStatus:(AVPlayerItem *)item {
    if (item != _item) {
        return;
    }
    if (item.status == AVPlayerItemStatusFailed) {
        [self reportError:item.error];
        return;
    }
    if (item.status != AVPlayerItemStatusReadyToPlay || _itemReady) {
        return;
    }
    _itemReady = YES;
    if (_videoMode) {
        _videoSurface.playerLayer.player = _player;
        _videoSurface.hidden = NO;
    }
    if (_positionPending) {
        [self seekMilliseconds:_requestedPosition];
    } else {
        [self applyPlaybackIntent];
    }
}

- (void)applyPlaybackIntent {
    if (_itemReady && !_positionPending && !_playbackError && _wantsPlaying) {
        [_player play];
    } else {
        [_player pause];
    }
}

- (void)setPlaybackRequested:(BOOL)requested {
    if (!_identifier || _playbackError) {
        return;
    }
    _wantsPlaying = requested;
    _transportUsed = YES;
    if (_ended && requested) {
        [self seekMilliseconds:0];
    } else {
        [self applyPlaybackIntent];
    }
}

- (void)startAlignedAtMilliseconds:(uint64_t)position {
    uint64_t duration = milliseconds(_item.duration);
    uint64_t target = duration ? MIN(duration, position) : position;
    _wantsPlaying = YES;
    _transportUsed = YES;
    [_player setRate:1.0f time:CMTimeMake((int64_t)target, 1000) atHostTime:kCMTimeInvalid];
}

- (void)seekMilliseconds:(uint64_t)position {
    if (!_identifier || _playbackError) {
        return;
    }
    if (_ended && _source.restart) {
        _source.restart(_source.context);
    }
    uint64_t duration = _item ? milliseconds(_item.duration) : 0;
    _requestedPosition = duration > 0 ? MIN(position, duration) : MIN(position, (uint64_t)INT64_MAX);
    _positionPending = YES;
    _ended = NO;
    if (!_itemReady) {
        return;
    }
    [_player pause];
    uint64_t revision = ++_seekRevision;
    [_item cancelPendingSeeks];
    CMTime target = CMTimeMake((int64_t)_requestedPosition, 1000);
    __weak NastiPlayerController *weakSelf = self;
    [_player seekToTime:target toleranceBefore:kCMTimeZero toleranceAfter:kCMTimeZero
       completionHandler:^(BOOL finished) {
        (void)finished;
        dispatch_async(dispatch_get_main_queue(), ^{
            NastiPlayerController *player = weakSelf;
            if (player && player->_seekRevision == revision) {
                player->_positionPending = NO;
                [player applyPlaybackIntent];
            }
        });
    }];
}

- (void)setVolume:(float)volume {
    _player.volume = fmaxf(0.0f, fminf(1.0f, volume));
}

- (void)setAudioOutputUID:(NSString *)identifier {
    if (_player.audioOutputDeviceUniqueID != identifier &&
        ![_player.audioOutputDeviceUniqueID isEqualToString:identifier]) {
        _player.audioOutputDeviceUniqueID = identifier;
    }
}

- (BOOL)setVideoFrame:(NSRect)rect window:(NSWindow *)window {
    NSView *webView = findWebView(window.contentView);
    NSView *container = webView.superview;
    if (!container) {
        [self clearVideoFrame];
        return NO;
    }
    if (!webView.isFlipped) {
        rect.origin.y = NSHeight(webView.bounds) - NSMaxY(rect);
    }
    rect = NSIntersectionRect(rect, webView.bounds);
    if (NSIsEmptyRect(rect)) {
        [self clearVideoFrame];
        return YES;
    }
    NSRect frame = [webView convertRect:rect toView:container];
    if (!_videoSurface) {
        _videoSurface = [[NastiVideoSurface alloc] initWithFrame:frame];
    }
    if (_videoSurface.superview != container) {
        [_videoSurface removeFromSuperview];
        [container addSubview:_videoSurface positioned:NSWindowAbove relativeTo:webView];
    }
    if (!NSEqualRects(_videoSurface.frame, frame)) {
        _videoSurface.frame = frame;
    }
    BOOL show = _videoMode && _itemReady && !_playbackError;
    _videoSurface.playerLayer.player = show ? _player : nil;
    _videoSurface.hidden = !show;
    return YES;
}

- (void)clearVideoFrame {
    _videoSurface.playerLayer.player = nil;
    [_videoSurface removeFromSuperview];
    _videoSurface = nil;
}

- (NSString *)status {
    if (!_identifier) {
        return @"idle";
    }
    if (_playbackError) {
        return @"error";
    }
    if (_ended) {
        return @"ended";
    }
    if (!_itemReady || _positionPending) {
        return @"loading";
    }
    switch (_player.timeControlStatus) {
        case AVPlayerTimeControlStatusPlaying:
            return @"playing";
        case AVPlayerTimeControlStatusWaitingToPlayAtSpecifiedRate:
            return @"loading";
        case AVPlayerTimeControlStatusPaused:
            return _transportUsed ? @"paused" : @"ready";
    }
}

- (NSDictionary *)snapshot {
    NSString *status = [self status];
    uint64_t duration = _item ? milliseconds(_item.duration) : 0;
    uint64_t position = _positionPending ? _requestedPosition : milliseconds(_player.currentTime);
    if (_ended && duration > 0) {
        position = duration;
    }
    return @{
        @"videoId": _identifier ?: (id)NSNull.null,
        @"mode": _videoMode ? @"video" : @"audio",
        @"status": status,
        @"positionMs": @(position),
        @"durationMs": @(duration),
        @"volume": @(_player.volume),
        @"error": _playbackError ?: (id)NSNull.null,
        @"spectrum": [status isEqualToString:@"playing"] ? spectrumDictionary(_tap) ?: (id)NSNull.null : (id)NSNull.null
    };
}

- (void)shutdown {
    [self replaceCurrentAsset];
    [self clearVideoFrame];
}

@end

static NastiPlayerController *controller;
static NastiPlayerController *candidate;
static NSString *selectedOutput;
static AudioObjectPropertyListenerBlock outputListener;

static NastiPlayerController *currentController(void) {
    if (!controller) {
        controller = [[NastiPlayerController alloc] init];
        [controller setAudioOutputUID:selectedOutput];
    }
    return controller;
}

static void applyOutput(NSString *identifier) {
    selectedOutput = [identifier copy];
    [controller setAudioOutputUID:selectedOutput];
    [candidate setAudioOutputUID:selectedOutput];
}

static BOOL containsOutput(NSArray<NSDictionary<NSString *, NSString *> *> *outputs, NSString *identifier) {
    for (NSDictionary<NSString *, NSString *> *device in outputs) {
        if ([device[@"id"] isEqualToString:identifier]) return YES;
    }
    return NO;
}

static void validateOutput(NSArray<NSDictionary<NSString *, NSString *> *> *outputs) {
    if (outputs && selectedOutput && !containsOutput(outputs, selectedOutput)) {
        applyOutput(nil);
    }
}

static void observeOutputDevices(void) {
    if (outputListener) return;
    AudioObjectPropertyListenerBlock listener = ^(UInt32 count, const AudioObjectPropertyAddress *addresses) {
        (void)count;
        (void)addresses;
        if (selectedOutput) validateOutput(NastiPlaybackOutputs());
    };
    if (AudioObjectAddPropertyListenerBlock(kAudioObjectSystemObject, &NastiPlaybackDevicesProperty,
                                            dispatch_get_main_queue(), listener) == noErr) {
        outputListener = listener;
    }
}

void nasti_native_load(const char *url, const char *video_id, bool is_video,
                       uint64_t position_ms, bool autoplay, NastiPlaybackSource source) {
    @autoreleasepool {
        NSString *urlString = [NSString stringWithUTF8String:url];
        [currentController() loadURL:[NSURL URLWithString:urlString]
                          identifier:[NSString stringWithUTF8String:video_id]
                               video:is_video positionMs:position_ms playbackRequested:autoplay source:source];
    }
}

void nasti_native_prepare(const char *url, const char *video_id, bool is_video, NastiPlaybackSource source) {
    @autoreleasepool {
        nasti_native_cancel_preparation();
        candidate = [[NastiPlayerController alloc] init];
        [candidate setAudioOutputUID:selectedOutput];
        [candidate loadURL:[NSURL URLWithString:[NSString stringWithUTF8String:url]]
                identifier:[NSString stringWithUTF8String:video_id]
                     video:is_video positionMs:0 playbackRequested:NO source:source];
    }
}

void nasti_native_seek_preparation(uint64_t position_ms) {
    [candidate seekMilliseconds:position_ms];
}

bool nasti_native_commit_preparation(bool autoplay, bool align_time, uint64_t position_ms) {
    NSDictionary *prepared = [candidate snapshot];
    if (![prepared[@"status"] isEqualToString:@"ready"]) {
        return false;
    }
    float volume = [[currentController() snapshot][@"volume"] floatValue];
    [controller shutdown];
    controller = candidate;
    candidate = nil;
    [controller setVolume:volume];
    if (autoplay && align_time) {
        [controller startAlignedAtMilliseconds:position_ms];
    } else {
        [controller setPlaybackRequested:autoplay];
        if (align_time) {
            uint64_t prepared_position = [prepared[@"positionMs"] unsignedLongLongValue];
            uint64_t drift = MAX(prepared_position, position_ms) - MIN(prepared_position, position_ms);
            if (drift > 80) [controller seekMilliseconds:position_ms];
        }
    }
    return true;
}

void nasti_native_cancel_preparation(void) {
    [candidate shutdown];
    candidate = nil;
}

void nasti_native_set_playing(bool playing) {
    [currentController() setPlaybackRequested:playing];
}

void nasti_native_seek(uint64_t position_ms) {
    [currentController() seekMilliseconds:position_ms];
}

void nasti_native_set_volume(float volume) {
    [currentController() setVolume:volume];
}

bool nasti_native_set_output(const char *identifier) {
    @autoreleasepool {
        NSString *output = identifier ? [NSString stringWithUTF8String:identifier] : nil;
        if (identifier && (!output || !containsOutput(NastiPlaybackOutputs(), output))) {
            return false;
        }
        applyOutput(output);
        if (output) observeOutputDevices();
        return true;
    }
}

bool nasti_native_set_video_frame(void *window, bool shown, double x, double y, double width, double height) {
    @autoreleasepool {
        if (!shown) {
            [controller clearVideoFrame];
            return true;
        }
        return [currentController() setVideoFrame:NSMakeRect(x, y, width, height) window:(__bridge NSWindow *)window];
    }
}

static char *copyJSON(NSDictionary *value) {
    if (!value) return NULL;
    NSData *json = [NSJSONSerialization dataWithJSONObject:value options:0 error:NULL];
    if (!json) {
        return NULL;
    }
    char *buffer = malloc(json.length + 1);
    if (buffer) {
        memcpy(buffer, json.bytes, json.length);
        buffer[json.length] = '\0';
    }
    return buffer;
}

char *nasti_native_copy_snapshot(void) {
    @autoreleasepool {
        return copyJSON([currentController() snapshot]);
    }
}

char *nasti_native_copy_preparation(void) {
    @autoreleasepool {
        return copyJSON([candidate snapshot]);
    }
}

char *nasti_native_copy_outputs(void) {
    @autoreleasepool {
        NSArray *outputs = NastiPlaybackOutputs();
        if (!outputs) return NULL;
        validateOutput(outputs);
        return copyJSON(@{
            @"supported": @YES,
            @"devices": outputs,
            @"defaultDeviceId": NastiDefaultPlaybackOutputID() ?: (id)NSNull.null,
            @"selectedDeviceId": selectedOutput ?: (id)NSNull.null,
        });
    }
}

void nasti_native_free_snapshot(char *text) {
    free(text);
}

void nasti_native_shutdown(void) {
    if (!NSThread.isMainThread) {
        dispatch_async(dispatch_get_main_queue(), ^{ nasti_native_shutdown(); });
        return;
    }
    nasti_native_cancel_preparation();
    [controller shutdown];
    controller = nil;
    selectedOutput = nil;
    if (outputListener) {
        AudioObjectRemovePropertyListenerBlock(kAudioObjectSystemObject, &NastiPlaybackDevicesProperty,
                                               dispatch_get_main_queue(), outputListener);
        outputListener = nil;
    }
}
