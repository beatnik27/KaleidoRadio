#import <Foundation/Foundation.h>
#import <NetFS/NetFS.h>
#import <sys/mount.h>
#include <errno.h>
#include <string.h>

// Reuse only the same remote share and account, never a same-named local disk.
static BOOL KRSMBMatchesURL(NSURL *requested, NSString *user, NSURL *mounted) {
    if (![mounted.scheme.lowercaseString isEqual:@"smb"] || !requested.host.length || !mounted.host.length) return NO;
    if ([requested.host caseInsensitiveCompare:mounted.host] != NSOrderedSame) return NO;
    NSInteger requestedPort = requested.port ? requested.port.integerValue : 445;
    NSInteger mountedPort = mounted.port ? mounted.port.integerValue : 445;
    if (requestedPort != mountedPort) return NO;
    NSCharacterSet *slash = [NSCharacterSet characterSetWithCharactersInString:@"/"];
    NSString *requestedShare = [requested.path stringByTrimmingCharactersInSet:slash].pathComponents.firstObject;
    NSString *mountedShare = [mounted.path stringByTrimmingCharactersInSet:slash].pathComponents.firstObject;
    if (!requestedShare.length || !mountedShare.length || [requestedShare caseInsensitiveCompare:mountedShare] != NSOrderedSame) return NO;
    NSString *mountedUser = [NSURLComponents componentsWithURL:mounted resolvingAgainstBaseURL:NO].user ?: @"";
    if (!user.length) return !mountedUser.length || [mountedUser caseInsensitiveCompare:@"guest"] == NSOrderedSame;
    return [user caseInsensitiveCompare:mountedUser] == NSOrderedSame;
}
static NSString *KRSMBExistingMount(NSURL *requested, NSString *user) {
    struct statfs *mounts = NULL;
    int count = getmntinfo(&mounts, MNT_NOWAIT);
    for (int index = 0; index < count; index++) {
        if (strcmp(mounts[index].f_fstypename, "smbfs") != 0) continue;
        NSURL *local = [NSURL fileURLWithFileSystemRepresentation:mounts[index].f_mntonname isDirectory:YES relativeToURL:nil];
        NSURL *remote = CFBridgingRelease(NetFSCopyURLForRemountingVolume((__bridge CFURLRef)local));
        if (!remote) {
            NSString *from = [NSString stringWithUTF8String:mounts[index].f_mntfromname];
            remote = [NSURL URLWithString:[@"smb:" stringByAppendingString:from]];
        }
        if (KRSMBMatchesURL(requested, user, remote)) return local.path;
    }
    return nil;
}

// Called from a Rust worker. Passwords are passed in memory, never in process
// arguments or stored URLs. NetFS handles SMB negotiation and system sessions.
int KRSMBMount(const char *address, const char *username, const char *password,
               char *path, size_t pathSize, char *error, size_t errorSize) {
    @autoreleasepool {
        NSURL *url = [NSURL URLWithString:[NSString stringWithUTF8String:address]];
        NSString *user = [NSString stringWithUTF8String:username];
        NSString *secret = [NSString stringWithUTF8String:password];
        NSString *existing = KRSMBExistingMount(url, user);
        if (existing && strlcpy(path, existing.fileSystemRepresentation, pathSize) < pathSize) return 0;
        NSMutableDictionary *options = [@{(__bridge id)kNAUIOptionKey: (__bridge id)kNAUIOptionNoUI} mutableCopy];
        if (!user.length) options[(__bridge id)kNetFSUseGuestKey] = @YES;
        NSMutableDictionary *mountOptions = [@{(__bridge id)kNetFSMountFlagsKey: @(MNT_RDONLY)} mutableCopy];
        CFArrayRef mounts = NULL;
        int status = NetFSMountURLSync((__bridge CFURLRef)url, NULL,
            (__bridge CFStringRef)user, (__bridge CFStringRef)secret,
            (__bridge CFMutableDictionaryRef)options, (__bridge CFMutableDictionaryRef)mountOptions, &mounts);
        NSArray *paths = CFBridgingRelease(mounts);
        if ((status == 0 || status == EEXIST) && paths.count && [paths.firstObject isKindOfClass:NSString.class]) {
            if (strlcpy(path, [paths.firstObject fileSystemRepresentation], pathSize) >= pathSize) status = ENAMETOOLONG;
            else return 0;
        }
        if (status == EEXIST) {
            NSString *existing = KRSMBExistingMount(url, user);
            if (existing && strlcpy(path, existing.fileSystemRepresentation, pathSize) < pathSize) return 0;
        }
        NSString *message;
        if (status == EACCES || status == EAUTH || status == EPERM) {
            message = @"SMB sign-in failed. Check the username, password, and share permissions.";
        } else if (status > 0) {
            message = [NSString stringWithFormat:@"Could not connect to SMB share: %s (%d).", strerror(status), status];
        } else {
            message = [NSString stringWithFormat:@"Could not connect to SMB share (error %d). Check the share name, credentials, and Local Network access.", status];
        }
        if (errorSize) strlcpy(error, message.UTF8String, errorSize);
        return status ? status : EIO;
    }
}
