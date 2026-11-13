#import <AVFoundation/AVFoundation.h>
#import <CoreVideo/CoreVideo.h>
#import <CoreImage/CoreImage.h>
#import <QuartzCore/QuartzCore.h>

// One AVPlayer owns both audio and video. Rust owns this object and polls it only
// on the UI thread; the resource-loader callback runs on the main queue as well.
@interface KRVideoPlayer : NSObject <AVAssetResourceLoaderDelegate>
@property(nonatomic, strong) AVPlayer *player;
@property(nonatomic, strong) AVPlayerItemVideoOutput *output;
@property(nonatomic, strong) CIContext *imageContext;
@property(nonatomic, strong) NSURL *url;
@property(nonatomic, copy) NSString *username;
@property(nonatomic, copy) NSString *password;
@property(nonatomic) double offset;
@property(nonatomic) BOOL started;
@property(nonatomic) BOOL paused;
@property(nonatomic) CGAffineTransform transform;
@property(nonatomic) BOOL checkedTracks;
@property(nonatomic) BOOL hasVideo;
@property(nonatomic) BOOL seeking;
@property(nonatomic) float rate;
@end

// Play at the stored rate: plain play resets AVPlayer to 1x, which
// would silently drop a user-chosen playback speed on every resume.
static void KRPlay(KRVideoPlayer *video) {
    if (video.rate != 1.0f) [video.player setRate:video.rate];
    else [video.player play];
}
@implementation KRVideoPlayer
- (BOOL)resourceLoader:(AVAssetResourceLoader *)loader
    shouldWaitForResponseToAuthenticationChallenge:(NSURLAuthenticationChallenge *)challenge {
    NSURLProtectionSpace *space = challenge.protectionSpace;
    NSInteger port = self.url.port ? self.url.port.integerValue : ([self.url.scheme isEqual:@"https"] ? 443 : 80);
    BOOL sameOrigin = [space.host caseInsensitiveCompare:self.url.host] == NSOrderedSame &&
        space.port == port && [space.protocol caseInsensitiveCompare:self.url.scheme] == NSOrderedSame;
    BOOL passwordAuth = [space.authenticationMethod isEqual:NSURLAuthenticationMethodHTTPBasic] ||
        [space.authenticationMethod isEqual:NSURLAuthenticationMethodHTTPDigest];
    if (sameOrigin && passwordAuth && self.username.length && challenge.previousFailureCount == 0) {
        NSURLCredential *credential = [NSURLCredential credentialWithUser:self.username password:self.password
            persistence:NSURLCredentialPersistenceNone];
        [challenge.sender useCredential:credential forAuthenticationChallenge:challenge];
    } else {
        [challenge.sender performDefaultHandlingForAuthenticationChallenge:challenge];
    }
    return YES;
}
@end

void *KRVideoCreate(const char *address, const char *user, const char *password, double offset, float volume) {
    @autoreleasepool {
        NSURL *url = [NSURL URLWithString:[NSString stringWithUTF8String:address]];
        if (!url) return NULL;
        KRVideoPlayer *video = [KRVideoPlayer new];
        video.url = url;
        video.username = user ? [NSString stringWithUTF8String:user] : nil;
        video.password = password ? [NSString stringWithUTF8String:password] : nil;
        video.offset = offset;
        video.transform = CGAffineTransformIdentity;
        AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:nil];
        [asset.resourceLoader setDelegate:video queue:dispatch_get_main_queue()];
        AVPlayerItem *item = [AVPlayerItem playerItemWithAsset:asset];
        video.output = [[AVPlayerItemVideoOutput alloc] initWithPixelBufferAttributes:@{
            (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
            (id)kCVPixelBufferIOSurfacePropertiesKey: @{}
        }];
        video.output.suppressesPlayerRendering = YES;
        [item addOutput:video.output];
        video.player = [AVPlayer playerWithPlayerItem:item];
        video.player.automaticallyWaitsToMinimizeStalling = NO;
        video.rate = 1.0f;
        video.player.volume = volume;
        video.player.actionAtItemEnd = AVPlayerActionAtItemEndPause;
        video.imageContext = [CIContext contextWithOptions:@{kCIContextCacheIntermediates: @NO}];
        return (__bridge_retained void *)video;
    }
}
void KRVideoDestroy(void *handle) {
    KRVideoPlayer *video = (__bridge_transfer KRVideoPlayer *)handle;
    [video.player pause];
    AVURLAsset *asset = (AVURLAsset *)video.player.currentItem.asset;
    [asset.resourceLoader setDelegate:nil queue:NULL];
    [asset cancelLoading];
    [video.player replaceCurrentItemWithPlayerItem:nil];
}
// 0 loading, 1 playing/paused, 2 finished, -1 failed.
int KRVideoState(void *handle, double *position, double *duration, bool *seekable) {
    KRVideoPlayer *video = (__bridge KRVideoPlayer *)handle;
    AVPlayerItem *item = video.player.currentItem;
    *duration = CMTimeGetSeconds(item.duration);
    *position = video.started && !video.seeking ? CMTimeGetSeconds(video.player.currentTime) : video.offset;
    *seekable = NO;
    for (NSValue *value in item.seekableTimeRanges) {
        CMTimeRange range = value.CMTimeRangeValue;
        if (CMTimeGetSeconds(range.duration) > 0) *seekable = YES;
    }
    if (item.status == AVPlayerItemStatusFailed || video.player.status == AVPlayerStatusFailed) return -1;
    if (item.status != AVPlayerItemStatusReadyToPlay) return 0;
    if (!video.checkedTracks) {
        AVAssetTrack *track = nil;
        for (AVPlayerItemTrack *itemTrack in item.tracks) {
            if ([itemTrack.assetTrack.mediaType isEqual:AVMediaTypeVideo]) {
                track = itemTrack.assetTrack;
                break;
            }
        }
        video.hasVideo = track != nil || item.presentationSize.width > 0;
        video.transform = track ? track.preferredTransform : CGAffineTransformIdentity;
        video.checkedTracks = YES;
    }
    if (!video.hasVideo) return 3;
    if (!video.started) {
        video.started = YES;
        if (video.offset > 0) {
            video.seeking = YES;
            __weak KRVideoPlayer *weakVideo = video;
            [video.player seekToTime:CMTimeMakeWithSeconds(video.offset, 600) toleranceBefore:kCMTimeZero
                toleranceAfter:kCMTimeZero completionHandler:^(BOOL finished) {
                    KRVideoPlayer *strongVideo = weakVideo;
                    strongVideo.seeking = NO;
                    if (finished && strongVideo && !strongVideo.paused) KRPlay(strongVideo);
                }];
        } else if (!video.paused) {
            KRPlay(video);
        }
    }
    if (isfinite(*duration) && *duration > 0 && *position >= *duration - 0.02 &&
        video.player.rate == 0 && !video.paused && !video.seeking) return 2;
    return 1;
}
void KRVideoPause(void *handle, bool paused) {
    KRVideoPlayer *video = (__bridge KRVideoPlayer *)handle;
    video.paused = paused;
    if (paused) [video.player pause];
    else if (video.started) KRPlay(video);
}
void KRVideoSeek(void *handle, double position) {
    KRVideoPlayer *video = (__bridge KRVideoPlayer *)handle;
    video.offset = position;
    if (!video.started) return;
    video.seeking = YES;
    __weak KRVideoPlayer *weakVideo = video;
    // Scrub seeks do not need frame exactness: a small tolerance lets the
    // player land on the nearest keyframe instead of decoding forward to
    // the exact frame on every drag release.
    [video.player seekToTime:CMTimeMakeWithSeconds(position, 600)
        toleranceBefore:CMTimeMakeWithSeconds(0.5, 600)
        toleranceAfter:CMTimeMakeWithSeconds(0.5, 600) completionHandler:^(BOOL finished) {
            KRVideoPlayer *strongVideo = weakVideo;
            strongVideo.seeking = NO;
            if (finished && strongVideo && !strongVideo.paused) KRPlay(strongVideo);
        }];
}
void KRVideoVolume(void *handle, float volume) {
    ((__bridge KRVideoPlayer *)handle).player.volume = volume;
}
void KRVideoRate(void *handle, float rate) {
    KRVideoPlayer *video = (__bridge KRVideoPlayer *)handle;
    video.rate = fminf(2.0f, fmaxf(0.25f, rate));
    if (video.started && !video.paused) KRPlay(video);
}
// Furthest downloaded point on the player's own timeline, for the
// seek bar's buffered fill. 0 when nothing is loaded yet.
double KRVideoBuffered(void *handle) {
    KRVideoPlayer *video = (__bridge KRVideoPlayer *)handle;
    AVPlayerItem *item = video.player.currentItem;
    if (!item) return 0;
    double buffered = 0;
    for (NSValue *value in item.loadedTimeRanges) {
        CMTimeRange range = value.CMTimeRangeValue;
        double end = CMTimeGetSeconds(range.start) + CMTimeGetSeconds(range.duration);
        if (isfinite(end) && end > buffered) buffered = end;
    }
    return buffered;
}
// True while the player is ready and expected to advance but its rate is
// zero: the download has fallen behind, or a seek is still waiting for
// data. Paused, seeking, and natural-end states are not stalls.
bool KRVideoStalled(void *handle) {
    KRVideoPlayer *video = (__bridge KRVideoPlayer *)handle;
    AVPlayerItem *item = video.player.currentItem;
    if (item.status != AVPlayerItemStatusReadyToPlay) return false;
    if (!video.started || video.paused || video.seeking) return false;
    if (video.player.rate != 0) return false;
    double duration = CMTimeGetSeconds(item.duration);
    double position = CMTimeGetSeconds(video.player.currentTime);
    if (isfinite(duration) && duration > 0 && position >= duration - 0.5) return false;
    return true;
}
void KRVideoError(void *handle, char *buffer, size_t capacity) {
    KRVideoPlayer *video = (__bridge KRVideoPlayer *)handle;
    NSString *message = video.player.currentItem.error.localizedDescription ?: video.player.error.localizedDescription;
    const char *text = (message ?: @"Video could not be played. Check the format, URL, or credentials.").UTF8String;
    if (capacity) { strlcpy(buffer, text, capacity); }
}
// Copy a fitted RGBA frame into a fixed bounded buffer, preserving aspect ratio.
// CI handles pixel conversion; only new frames are transferred to the GPU texture.
bool KRVideoFrame(void *handle, unsigned char *rgba, size_t capacity, unsigned *width, unsigned *height) {
    @autoreleasepool {
        KRVideoPlayer *video = (__bridge KRVideoPlayer *)handle;
        if (!video.checkedTracks || !video.hasVideo) return false;
        CMTime time = [video.output itemTimeForHostTime:CACurrentMediaTime()];
        if (![video.output hasNewPixelBufferForItemTime:time]) return false;
        CVPixelBufferRef pixel = [video.output copyPixelBufferForItemTime:time itemTimeForDisplay:NULL];
        if (!pixel) return false;
        CIImage *image = [CIImage imageWithCVPixelBuffer:pixel];
        image = [image imageByApplyingTransform:video.transform];
        CGRect bounds = image.extent;
        if (!isfinite(bounds.size.width) || !isfinite(bounds.size.height) || bounds.size.width <= 0 || bounds.size.height <= 0) {
            CVPixelBufferRelease(pixel); return false;
        }
        CGFloat scale = MIN(1.0, MIN(1280.0 / bounds.size.width, 720.0 / bounds.size.height));
        image = [image imageByApplyingTransform:CGAffineTransformMakeTranslation(-bounds.origin.x, -bounds.origin.y)];
        image = [image imageByApplyingTransform:CGAffineTransformMakeScale(scale, scale)];
        *width = MAX(1, (unsigned)floor(bounds.size.width * scale));
        *height = MAX(1, (unsigned)floor(bounds.size.height * scale));
        if ((size_t)*width * *height * 4 > capacity) { CVPixelBufferRelease(pixel); return false; }
        CGColorSpaceRef color = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        [video.imageContext render:image toBitmap:rgba rowBytes:*width * 4
            bounds:CGRectMake(0, 0, *width, *height) format:kCIFormatRGBA8 colorSpace:color];
        CGColorSpaceRelease(color);
        CVPixelBufferRelease(pixel);
        return true;
    }
}
