#import <Foundation/Foundation.h>
#include <assert.h>
#include "../native/smb_mount.m"
int main(int argc, char **argv) {
    @autoreleasepool {
        if (argc == 3) {
            NSString *existing = KRSMBExistingMount([NSURL URLWithString:[NSString stringWithUTF8String:argv[1]]], [NSString stringWithUTF8String:argv[2]]);
            if (!existing) { puts("No matching SMB mount found."); return 1; }
            puts(existing.fileSystemRepresentation);
            return 0;
        }
        NSURL *requested = [NSURL URLWithString:@"smb://server/Sandisk/"];
        assert(KRSMBMatchesURL(requested, @"Mark", [NSURL URLWithString:@"smb://Mark@SERVER/SanDisk/"]));
        assert(KRSMBMatchesURL(requested, @"Mark", [NSURL URLWithString:@"smb://Mark@server:445/Sandisk/"]));
        assert(!KRSMBMatchesURL(requested, @"Mark", [NSURL URLWithString:@"file:///Volumes/Sandisk/"]));
        assert(!KRSMBMatchesURL(requested, @"Mark", [NSURL URLWithString:@"smb://Other@server/Sandisk/"]));
        assert(!KRSMBMatchesURL(requested, @"Mark", [NSURL URLWithString:@"smb://Mark@other/Sandisk/"]));
        assert(!KRSMBMatchesURL(requested, @"Mark", [NSURL URLWithString:@"smb://Mark@server/Other/"]));
        assert(!KRSMBMatchesURL(requested, @"Mark", [NSURL URLWithString:@"smb://Mark@server:1445/Sandisk/"]));
        assert(KRSMBMatchesURL(requested, @"", [NSURL URLWithString:@"smb://guest@server/Sandisk/"]));
        assert(!KRSMBMatchesURL(requested, @"", [NSURL URLWithString:@"smb://Mark@server/Sandisk/"]));
        puts("SMB mount identity checks passed.");
    }
}
