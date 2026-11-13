#import <Foundation/Foundation.h>
#import <MediaPlayer/MediaPlayer.h>
#import <CoreAudio/CoreAudio.h>
#import <IOBluetooth/IOBluetooth.h>

typedef void (*KRCallback)(void *, int);
static AudioObjectPropertyAddress outputAddress = {
    kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain
};
static UInt32 property(AudioObjectID device, AudioObjectPropertySelector selector) {
    AudioObjectPropertyAddress address = {selector, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    UInt32 value = 0, size = sizeof(value);
    return AudioObjectGetPropertyData(device, &address, 0, NULL, &size, &value) == noErr ? value : 0;
}
static BOOL headsetName(NSString *name) {
    for (NSString *word in @[@"headset", @"headphone", @"airpod", @"beats", @"bluetooth", @"gaming", @"wireless", @"airplay"]) {
        if ([name.lowercaseString containsString:word]) return YES;
    }
    return NO;
}
static BOOL personalOutput(AudioObjectID device) {
    UInt32 transport = property(device, kAudioDevicePropertyTransportType);
    if (transport == kAudioDeviceTransportTypeBluetooth || transport == kAudioDeviceTransportTypeBluetoothLE ||
        transport == kAudioDeviceTransportTypeAirPlay || transport == kAudioDeviceTransportTypeUSB) return YES;
    AudioObjectPropertyAddress address = {kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    CFStringRef name = NULL;
    UInt32 size = sizeof(name);
    if (AudioObjectGetPropertyData(device, &address, 0, NULL, &size, &name) != noErr || !name) return NO;
    return headsetName(CFBridgingRelease(name));
}

// Kept separate so the exact route-loss policy can be tested without hardware.
bool KRMediaShouldStopRoute(bool playing, UInt32 monitored, UInt32 current, bool alive) {
    return playing && monitored != 0 && (current != monitored || !alive);
}

@interface KRMediaBridge : NSObject
@property(nonatomic) KRCallback callback;
@property(nonatomic) void *context;
@property(nonatomic) BOOL playing;
@property(nonatomic) AudioObjectID monitored;
@property(nonatomic) AudioObjectID currentOutput;
@property(nonatomic, copy) AudioObjectPropertyListenerBlock routeListener;
@property(nonatomic, copy) AudioObjectPropertyListenerBlock aliveListener;
@property(nonatomic, strong) id bluetoothObserver;
@property(nonatomic, strong) NSMutableArray *commands;
@property(nonatomic, strong) NSMutableArray *targets;
- (void)send:(int)command;
- (void)monitor:(AudioObjectID)device;
- (void)checkRoute;
- (void)shutdown;
@end
@implementation KRMediaBridge
- (void)send:(int)command {
    if (![NSThread isMainThread]) {
        dispatch_async(dispatch_get_main_queue(), ^{ [self send:command]; });
        return;
    }
    if (_callback) _callback(_context, command);
}
- (void)monitor:(AudioObjectID)device {
    if (_monitored == device) return;
    AudioObjectPropertyAddress alive = {kAudioDevicePropertyDeviceIsAlive, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    if (_monitored) AudioObjectRemovePropertyListenerBlock(_monitored, &alive, dispatch_get_main_queue(), _aliveListener);
    _monitored = device;
    if (device) AudioObjectAddPropertyListenerBlock(device, &alive, dispatch_get_main_queue(), _aliveListener);
}
- (void)checkRoute {
    AudioObjectID current = property(kAudioObjectSystemObject, kAudioHardwarePropertyDefaultOutputDevice);
    AudioObjectID previous = _currentOutput;
    _currentOutput = current;
    if (!_playing) return;
    if (KRMediaShouldStopRoute(_playing, _monitored, current, property(_monitored, kAudioDevicePropertyDeviceIsAlive) != 0)) {
        _playing = NO;
        [self monitor:0];
        [self send:6];
    } else {
        if (!_monitored && current && personalOutput(current)) [self monitor:current];
        if (current && previous && current != previous) [self send:7];
    }
}
- (void)shutdown {
    _callback = NULL;
    [self monitor:0];
    AudioObjectRemovePropertyListenerBlock(kAudioObjectSystemObject, &outputAddress, dispatch_get_main_queue(), _routeListener);
    if (_bluetoothObserver) [[NSNotificationCenter defaultCenter] removeObserver:_bluetoothObserver];
    for (NSUInteger i = 0; i < _commands.count; ++i) [_commands[i] removeTarget:_targets[i]];
    [MPNowPlayingInfoCenter defaultCenter].nowPlayingInfo = nil;
    [MPNowPlayingInfoCenter defaultCenter].playbackState = MPNowPlayingPlaybackStateStopped;
}
@end

void *KRMediaCreate(KRCallback callback, void *context) {
    KRMediaBridge *bridge = [KRMediaBridge new];
    bridge.callback = callback;
    bridge.context = context;
    bridge.currentOutput = property(kAudioObjectSystemObject, kAudioHardwarePropertyDefaultOutputDevice);
    bridge.commands = [NSMutableArray new];
    bridge.targets = [NSMutableArray new];
    __weak KRMediaBridge *weak = bridge;
    bridge.routeListener = ^(UInt32 count, const AudioObjectPropertyAddress *addresses) { [weak checkRoute]; };
    bridge.aliveListener = ^(UInt32 count, const AudioObjectPropertyAddress *addresses) { [weak checkRoute]; };
    AudioObjectAddPropertyListenerBlock(kAudioObjectSystemObject, &outputAddress, dispatch_get_main_queue(), bridge.routeListener);
    bridge.bluetoothObserver = [[NSNotificationCenter defaultCenter] addObserverForName:@"IOBluetoothDeviceDisconnected" object:nil queue:[NSOperationQueue mainQueue] usingBlock:^(NSNotification *note) {
        KRMediaBridge *strong = weak;
        if (!strong.playing) return;
        IOBluetoothDevice *device = [note.object isKindOfClass:IOBluetoothDevice.class] ? note.object : nil;
        UInt32 transport = property(strong.monitored, kAudioDevicePropertyTransportType);
        BOOL wireless = transport == kAudioDeviceTransportTypeBluetooth || transport == kAudioDeviceTransportTypeBluetoothLE || transport == kAudioDeviceTransportTypeAirPlay;
        if (wireless || headsetName(device.name ?: @"")) {
            strong.playing = NO;
            [strong monitor:0];
            [strong send:6];
        }
    }];
    MPRemoteCommandCenter *center = [MPRemoteCommandCenter sharedCommandCenter];
    NSArray *commands = @[center.playCommand, center.pauseCommand, center.stopCommand, center.togglePlayPauseCommand, center.nextTrackCommand, center.previousTrackCommand];
    int codes[] = {1, 2, 2, 3, 4, 5};
    for (NSUInteger i = 0; i < commands.count; ++i) {
        MPRemoteCommand *command = commands[i];
        command.enabled = YES;
        int code = codes[i];
        id target = [command addTargetWithHandler:^MPRemoteCommandHandlerStatus(MPRemoteCommandEvent *event) {
            KRMediaBridge *strong = weak;
            if (!strong || !strong.callback) return MPRemoteCommandHandlerStatusCommandFailed;
            [strong send:code];
            return MPRemoteCommandHandlerStatusSuccess;
        }];
        [bridge.commands addObject:command];
        [bridge.targets addObject:target];
    }
    return (__bridge_retained void *)bridge;
}
void KRMediaUpdate(void *handle, bool playing, const char *title, const char *station, bool live, double elapsed, double duration) {
    KRMediaBridge *bridge = (__bridge KRMediaBridge *)handle;
    if (bridge.playing != playing) {
        bridge.playing = playing;
        AudioObjectID current = property(kAudioObjectSystemObject, kAudioHardwarePropertyDefaultOutputDevice);
        bridge.currentOutput = current;
        [bridge monitor:playing && current && personalOutput(current) ? current : 0];
    }
    MPNowPlayingInfoCenter *center = [MPNowPlayingInfoCenter defaultCenter];
    center.playbackState = playing ? MPNowPlayingPlaybackStatePlaying : MPNowPlayingPlaybackStateStopped;
    if (!playing || !title || !*title) { center.nowPlayingInfo = nil; return; }
    NSMutableDictionary *info = [@{
        MPMediaItemPropertyTitle: [NSString stringWithUTF8String:title] ?: @"",
        MPMediaItemPropertyArtist: [NSString stringWithUTF8String:station] ?: @"",
        MPNowPlayingInfoPropertyIsLiveStream: @(live),
        MPNowPlayingInfoPropertyPlaybackRate: @(playing ? 1.0 : 0.0),
        MPNowPlayingInfoPropertyElapsedPlaybackTime: @(elapsed)
    } mutableCopy];
    if (!live && duration > 0) info[MPMediaItemPropertyPlaybackDuration] = @(duration);
    center.nowPlayingInfo = info;
}
void KRMediaDestroy(void *handle) {
    KRMediaBridge *bridge = CFBridgingRelease(handle);
    [bridge shutdown];
}

// WebDAV secrets live in Keychain, separate from library preferences.
#import <Security/Security.h>
#include <string.h>
static NSDictionary *KRPasswordQuery(const char *account) {
    return @{(__bridge id)kSecClass: (__bridge id)kSecClassGenericPassword,
             (__bridge id)kSecAttrService: @"local.shadow.KaleidoRadioRust.webdav",
             (__bridge id)kSecAttrAccount: [NSString stringWithUTF8String:account]};
}
char *kr_password_load(const char *account) {
    @autoreleasepool {
        NSMutableDictionary *query = [KRPasswordQuery(account) mutableCopy];
        query[(__bridge id)kSecReturnData] = @YES;
        CFTypeRef result = NULL;
        if (SecItemCopyMatching((__bridge CFDictionaryRef)query, &result) != errSecSuccess) return NULL;
        NSData *data = CFBridgingRelease(result);
        NSString *password = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
        return password ? strdup(password.UTF8String) : NULL;
    }
}
void kr_password_free(char *password) { free(password); }
int kr_password_save(const char *account, const char *password) {
    @autoreleasepool {
        NSDictionary *query = KRPasswordQuery(account);
        NSDictionary *attributes = @{(__bridge id)kSecValueData: [[NSString stringWithUTF8String:password] dataUsingEncoding:NSUTF8StringEncoding]};
        OSStatus status = SecItemUpdate((__bridge CFDictionaryRef)query, (__bridge CFDictionaryRef)attributes);
        if (status == errSecItemNotFound) {
            NSMutableDictionary *item = [query mutableCopy];
            [item addEntriesFromDictionary:attributes];
            item[(__bridge id)kSecAttrAccessible] = (__bridge id)kSecAttrAccessibleWhenUnlocked;
            status = SecItemAdd((__bridge CFDictionaryRef)item, NULL);
        }
        return status;
    }
}
int kr_password_delete(const char *account) {
    @autoreleasepool {
        OSStatus status = SecItemDelete((__bridge CFDictionaryRef)KRPasswordQuery(account));
        return status == errSecItemNotFound ? errSecSuccess : status;
    }
}
