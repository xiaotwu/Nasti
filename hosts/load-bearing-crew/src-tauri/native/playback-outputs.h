#pragma once

#import <CoreAudio/CoreAudio.h>
#import <Foundation/Foundation.h>

extern const AudioObjectPropertyAddress NastiPlaybackDevicesProperty;

NSArray<NSDictionary<NSString *, NSString *> *> * _Nullable NastiPlaybackOutputs(void);
NSString * _Nullable NastiDefaultPlaybackOutputID(void);
