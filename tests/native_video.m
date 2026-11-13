#import <Foundation/Foundation.h>
#include <assert.h>
#include <math.h>
#include <stdbool.h>
#include <stdio.h>
#include "../native/video_player.m"
static void pump(double seconds) {
    NSDate *until = [NSDate dateWithTimeIntervalSinceNow:seconds];
    while (until.timeIntervalSinceNow > 0) {
        [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
    }
}
int main(int argc, char **argv) {
    @autoreleasepool {
        double initialOffset = argc > 3 && strcmp(argv[2], "--stream-offset") == 0 ? atof(argv[3]) : 0;
        void *video = KRVideoCreate(argv[1], argc > 2 ? argv[2] : NULL, argc > 3 ? argv[3] : NULL, initialOffset, 0);
        assert(video);
        unsigned char *pixels = malloc(1280 * 720 * 4);
        double position = 0, duration = 0; bool seekable = false;
        int frames = 0, state = 0; unsigned width = 0, height = 0;
        bool audioOnly = argc > 2 && strcmp(argv[2], "--audio-only") == 0;
        for (int i = 0; i < 800 && frames < 10; i++) {
            pump(0.01);
            state = KRVideoState(video, &position, &duration, &seekable);
            if (state < 0) { char error[1024]; KRVideoError(video, error, sizeof(error)); fprintf(stderr, "%s\n", error); return 1; }
            if (audioOnly && state == 3) break;
            if (KRVideoFrame(video, pixels, 1280 * 720 * 4, &width, &height)) frames++;
        }
        if (audioOnly) {
            assert(state == 3);
            KRVideoDestroy(video); free(pixels);
            puts("Audio-only MP4 correctly selects audio fallback.");
            return 0;
        }
        KRVideoPlayer *native = (__bridge KRVideoPlayer *)video;
        fprintf(stderr, "state=%d rate=%f control=%ld waiting=%s error=%s\n", state, native.player.rate, (long)native.player.timeControlStatus, native.player.reasonForWaitingToPlay.UTF8String, native.player.currentItem.errorLog.description.UTF8String);
        fprintf(stderr, "frames=%d dimensions=%ux%u position=%.2f duration=%.2f seekable=%d\n", frames, width, height, position, duration, seekable);
        if (argc > 2 && strcmp(argv[2], "--stream-offset") == 0) {
            assert(frames >= 10 && width > 0 && height > 0);
            assert(position >= initialOffset - .2 && position < initialOffset + 5.0);
            KRVideoDestroy(video); free(pixels);
            puts("Streaming playback at requested segment offset passed.");
            return 0;
        }
        if (argc > 2 && strcmp(argv[2], "--stream-preview") == 0) {
            assert(frames >= 10 && width > 0 && height > 0);
            KRVideoPause(video, true); pump(.2);
            KRVideoState(video, &position, &duration, &seekable); double paused = position;
            pump(.2); KRVideoState(video, &position, &duration, &seekable);
            assert(fabs(position - paused) < .1);
            KRVideoSeek(video, 2.0);
            for (int i = 0; i < 300; i++) {
                pump(.01); KRVideoState(video, &position, &duration, &seekable);
                if (!native.seeking && fabs(position - 2.0) < .15) break;
            }
            assert(!native.seeking && fabs(position - 2.0) < .15);
            KRVideoPause(video, false);
            for (int i = 0; i < 300 && position < 2.2; i++) {
                pump(.01); KRVideoState(video, &position, &duration, &seekable);
            }
            assert(position >= 2.2);
            KRVideoDestroy(video); free(pixels);
            puts("Streaming video frames, pause, buffered seek, and resume passed.");
            return 0;
        }
        if (argc > 2 && strcmp(argv[2], "--preview") == 0) {
            assert(frames >= 10 && width > 0 && height > 0);
            KRVideoDestroy(video); free(pixels);
            puts("Video frame playback preview passed.");
            return 0;
        }
        assert(frames >= 10 && width > 0 && height > 0 && duration > 4.9 && seekable);
        KRVideoPause(video, true); pump(.1);
        KRVideoState(video, &position, &duration, &seekable); double paused = position;
        pump(.2); KRVideoState(video, &position, &duration, &seekable);
        assert(fabs(position - paused) < .05);
        KRVideoSeek(video, 1.0); pump(.3);
        KRVideoState(video, &position, &duration, &seekable);
        assert(fabs(position - 1.0) < .05);
        assert(((__bridge KRVideoPlayer *)video).player.rate == 0);
        paused = position;
        KRVideoPause(video, false); pump(.3); KRVideoState(video, &position, &duration, &seekable);
        assert(position > paused + .1);
        KRVideoDestroy(video);
        video = KRVideoCreate(argv[1], argc > 2 ? argv[2] : NULL, argc > 3 ? argv[3] : NULL, 3, 0);
        for (int i = 0; i < 500; i++) {
            pump(.01); state = KRVideoState(video, &position, &duration, &seekable);
            if (state < 0) return 2;
            if (position > 3.2) break;
        }
        fprintf(stderr, "seek position=%.2f\n", position);
        assert(position > 3.2 && position < 4.9);
        for (int i = 0; i < 500 && state != 2; i++) { pump(.01); state = KRVideoState(video, &position, &duration, &seekable); }
        assert(state == 2);
        KRVideoDestroy(video); free(pixels);
        puts("Native video playback, pause, seek, and completion passed.");
    }
}
