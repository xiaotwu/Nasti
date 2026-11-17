#import "playback-outputs.h"

const AudioObjectPropertyAddress NastiPlaybackDevicesProperty = {
    kAudioHardwarePropertyDevices, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain
};

static NSData *readObjectIDs(AudioObjectID object, const AudioObjectPropertyAddress *property) {
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(object, property, 0, NULL, &size) != noErr) {
        return nil;
    }
    NSMutableData *data = [NSMutableData dataWithLength:size];
    if (size != 0 &&
        AudioObjectGetPropertyData(object, property, 0, NULL, &size, data.mutableBytes) != noErr) {
        return nil;
    }
    data.length = size;
    return data;
}

static UInt32 readUInt32(AudioObjectID object, AudioObjectPropertySelector selector) {
    AudioObjectPropertyAddress property = {
        selector, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain
    };
    UInt32 value = 0;
    UInt32 size = sizeof(value);
    return AudioObjectGetPropertyData(object, &property, 0, NULL, &size, &value) == noErr ? value : 0;
}

static NSString *readString(AudioObjectID object, AudioObjectPropertySelector selector) {
    AudioObjectPropertyAddress property = {
        selector, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain
    };
    CFStringRef value = NULL;
    UInt32 size = sizeof(value);
    if (AudioObjectGetPropertyData(object, &property, 0, NULL, &size, &value) != noErr) {
        return nil;
    }
    return CFBridgingRelease(value);
}

static NSString *outputKind(AudioDeviceID device, NSData *outputs) {
    UInt32 transport = readUInt32(device, kAudioDevicePropertyTransportType);
    if (transport == kAudioDeviceTransportTypeBuiltIn) return @"builtin";
    if (transport == kAudioDeviceTransportTypeHDMI || transport == kAudioDeviceTransportTypeDisplayPort) {
        return @"display";
    }
    const AudioStreamID *streams = outputs.bytes;
    for (NSUInteger index = 0; index < outputs.length / sizeof(*streams); ++index) {
        switch (readUInt32(streams[index], kAudioStreamPropertyTerminalType)) {
            case kAudioStreamTerminalTypeHDMI:
            case kAudioStreamTerminalTypeDisplayPort:
                return @"display";
            case kAudioStreamTerminalTypeHeadphones:
                return @"headphones";
            case kAudioStreamTerminalTypeSpeaker:
            case kAudioStreamTerminalTypeLFESpeaker:
            case kAudioStreamTerminalTypeReceiverSpeaker:
                return @"speaker";
        }
    }
    return transport == kAudioDeviceTransportTypeAirPlay ? @"speaker" : @"other";
}

NSArray<NSDictionary<NSString *, NSString *> *> *NastiPlaybackOutputs(void) {
    NSData *deviceData = readObjectIDs(kAudioObjectSystemObject, &NastiPlaybackDevicesProperty);
    if (!deviceData) return nil;
    NSMutableArray<NSDictionary<NSString *, NSString *> *> *outputs = [NSMutableArray array];
    const AudioDeviceID *devices = deviceData.bytes;
    const AudioObjectPropertyAddress streams = {
        kAudioDevicePropertyStreams, kAudioDevicePropertyScopeOutput, kAudioObjectPropertyElementMain
    };
    for (NSUInteger index = 0; index < deviceData.length / sizeof(*devices); ++index) {
        AudioDeviceID device = devices[index];
        if (readUInt32(device, kAudioDevicePropertyDeviceIsAlive) == 0) continue;
        NSData *outputStreams = readObjectIDs(device, &streams);
        if (outputStreams.length == 0) continue;
        NSString *identifier = readString(device, kAudioDevicePropertyDeviceUID);
        NSString *name = readString(device, kAudioObjectPropertyName);
        if (identifier.length == 0 || name.length == 0) continue;
        [outputs addObject:@{@"id": identifier, @"name": name, @"kind": outputKind(device, outputStreams)}];
    }
    [outputs sortUsingComparator:^NSComparisonResult(NSDictionary *left, NSDictionary *right) {
        BOOL leftBuiltIn = [left[@"kind"] isEqualToString:@"builtin"];
        BOOL rightBuiltIn = [right[@"kind"] isEqualToString:@"builtin"];
        if (leftBuiltIn != rightBuiltIn) return leftBuiltIn ? NSOrderedAscending : NSOrderedDescending;
        return [left[@"name"] localizedStandardCompare:right[@"name"]];
    }];
    return outputs;
}

NSString *NastiDefaultPlaybackOutputID(void) {
    AudioDeviceID device = readUInt32(kAudioObjectSystemObject, kAudioHardwarePropertyDefaultOutputDevice);
    return device == kAudioObjectUnknown ? nil : readString(device, kAudioDevicePropertyDeviceUID);
}
