#import <Cocoa/Cocoa.h>
#import <UserNotifications/UserNotifications.h>

@interface ModexNotificationDelegate
    : NSObject <NSUserNotificationCenterDelegate, UNUserNotificationCenterDelegate>
@end

void modex_assign_notification_delegate(void);
void modex_log_notification_event(NSString *status);
void modex_log_notification_settings(void);

@implementation ModexNotificationDelegate

- (BOOL)userNotificationCenter:(NSUserNotificationCenter *)center
      shouldPresentNotification:(NSUserNotification *)notification {
  modex_log_notification_event(@"nsuser_will_present");
  return YES;
}

- (void)userNotificationCenter:(UNUserNotificationCenter *)center
       willPresentNotification:(UNNotification *)notification
         withCompletionHandler:
             (void (^)(UNNotificationPresentationOptions options))completionHandler {
  if (@available(macOS 11.0, *)) {
    modex_log_notification_event(@"un_will_present_banner");
    completionHandler(UNNotificationPresentationOptionBanner |
                      UNNotificationPresentationOptionList |
                      UNNotificationPresentationOptionSound);
  } else {
    modex_log_notification_event(@"un_will_present_alert");
    completionHandler(UNNotificationPresentationOptionAlert |
                      UNNotificationPresentationOptionSound);
  }
}

@end

static ModexNotificationDelegate *modexNotificationDelegate = nil;
static NSString *modexLastNotificationError = nil;

void modex_log_notification_event(NSString *status) {
  NSArray<NSString *> *paths = NSSearchPathForDirectoriesInDomains(
      NSApplicationSupportDirectory, NSUserDomainMask, YES);
  NSString *basePath = paths.firstObject;
  if (basePath == nil) {
    return;
  }

  NSString *directory = [basePath stringByAppendingPathComponent:@"Modex"];
  [[NSFileManager defaultManager] createDirectoryAtPath:directory
                            withIntermediateDirectories:YES
                                             attributes:nil
                                                  error:nil];

  NSString *path = [directory stringByAppendingPathComponent:@"notifications.log"];
  unsigned long long millis =
      (unsigned long long)([[NSDate date] timeIntervalSince1970] * 1000);
  NSString *line =
      [NSString stringWithFormat:@"%llu\t%@\tModex\tforeground presentation\n",
                                 millis, status ?: @""];
  NSData *data = [line dataUsingEncoding:NSUTF8StringEncoding];
  if (![[NSFileManager defaultManager] fileExistsAtPath:path]) {
    [data writeToFile:path atomically:YES];
    return;
  }

  NSFileHandle *file = [NSFileHandle fileHandleForWritingAtPath:path];
  if (file == nil) {
    return;
  }
  [file seekToEndOfFile];
  [file writeData:data];
  [file closeFile];
}

void modex_set_last_notification_error(NSString *message) {
  modexLastNotificationError = message ?: @"";
}

const char *modex_last_notification_error(void) {
  return [modexLastNotificationError UTF8String];
}

BOOL modex_has_user_notifications(void) {
  if (NSClassFromString(@"UNUserNotificationCenter") != Nil) {
    return YES;
  }

  NSBundle *bundle = [NSBundle
      bundleWithPath:@"/System/Library/Frameworks/UserNotifications.framework"];
  if (bundle != nil && [bundle load]) {
    return NSClassFromString(@"UNUserNotificationCenter") != Nil;
  }
  modex_set_last_notification_error(@"UNUserNotificationCenter is unavailable");
  return NO;
}

void modex_ensure_notification_delegate(void) {
  if (modexNotificationDelegate == nil) {
    modexNotificationDelegate = [[ModexNotificationDelegate alloc] init];
  }

  if (![NSThread isMainThread]) {
    dispatch_sync(dispatch_get_main_queue(), ^{
      modex_assign_notification_delegate();
    });
    return;
  }

  modex_assign_notification_delegate();
}

void modex_assign_notification_delegate(void) {
  if (modex_has_user_notifications()) {
    [UNUserNotificationCenter currentNotificationCenter].delegate =
        modexNotificationDelegate;
  }
  [NSUserNotificationCenter defaultUserNotificationCenter].delegate =
      modexNotificationDelegate;
}

int modex_send_un_notification(NSString *title, NSString *body) {
  if (!modex_has_user_notifications()) {
    return -10;
  }

  modex_ensure_notification_delegate();
  modex_log_notification_settings();

  UNMutableNotificationContent *content =
      [[UNMutableNotificationContent alloc] init];
  content.title = title;
  content.body = body;
  content.sound = [UNNotificationSound defaultSound];
  if (@available(macOS 12.0, *)) {
    content.interruptionLevel = UNNotificationInterruptionLevelActive;
    content.relevanceScore = 1.0;
  }

  NSString *identifier = [[NSUUID UUID] UUIDString];
  UNNotificationRequest *request =
      [UNNotificationRequest requestWithIdentifier:identifier
                                           content:content
                                           trigger:nil];

  dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
  __block BOOL delivered = NO;
  __block int result = 0;

  UNAuthorizationOptions options =
      UNAuthorizationOptionAlert | UNAuthorizationOptionSound;
  [[UNUserNotificationCenter currentNotificationCenter]
      requestAuthorizationWithOptions:options
                    completionHandler:^(BOOL granted, NSError *error) {
                      if (error != nil) {
                        modex_set_last_notification_error(
                            error.localizedDescription);
                        result = -3;
                        dispatch_semaphore_signal(semaphore);
                        return;
                      }
                      if (!granted) {
                        modex_set_last_notification_error(
                            @"authorization was not granted");
                        result = -2;
                        dispatch_semaphore_signal(semaphore);
                        return;
                      }
                      [[UNUserNotificationCenter currentNotificationCenter]
                          addNotificationRequest:request
                           withCompletionHandler:^(NSError *addError) {
                             delivered = addError == nil;
                             result = delivered ? 1 : -4;
                             if (addError != nil) {
                               modex_set_last_notification_error(
                                   addError.localizedDescription);
                             }
                             dispatch_semaphore_signal(semaphore);
                           }];
                    }];

  dispatch_time_t timeout =
      dispatch_time(DISPATCH_TIME_NOW, (int64_t)(5 * NSEC_PER_SEC));
  if (dispatch_semaphore_wait(semaphore, timeout) != 0) {
    modex_set_last_notification_error(@"request timed out");
    return -5;
  }
  return result;
}

void modex_log_notification_settings(void) {
  if (!modex_has_user_notifications()) {
    return;
  }

  dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
  [[UNUserNotificationCenter currentNotificationCenter]
      getNotificationSettingsWithCompletionHandler:^(
          UNNotificationSettings *settings) {
        NSString *status = [NSString
            stringWithFormat:
                @"settings auth=%ld alert=%ld center=%ld sound=%ld style=%ld"
                 " time=%ld scheduled=%ld",
                (long)settings.authorizationStatus,
                (long)settings.alertSetting,
                (long)settings.notificationCenterSetting,
                (long)settings.soundSetting, (long)settings.alertStyle,
                (long)settings.timeSensitiveSetting,
                (long)settings.scheduledDeliverySetting];
        modex_log_notification_event(status);
        dispatch_semaphore_signal(semaphore);
      }];

  dispatch_time_t timeout =
      dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1 * NSEC_PER_SEC));
  if (dispatch_semaphore_wait(semaphore, timeout) != 0) {
    modex_log_notification_event(@"settings timeout");
  }
}

int modex_send_nsuser_notification(NSString *title, NSString *body) {
  modex_ensure_notification_delegate();
  NSUserNotification *notification = [[NSUserNotification alloc] init];
  notification.title = title;
  notification.informativeText = body;
  notification.soundName = NSUserNotificationDefaultSoundName;

  [[NSUserNotificationCenter defaultUserNotificationCenter]
      deliverNotification:notification];
  return 2;
}

int modex_send_user_notification(const char *title, const char *body) {
  @autoreleasepool {
    NSString *titleString = title ? [NSString stringWithUTF8String:title] : @"";
    NSString *bodyString = body ? [NSString stringWithUTF8String:body] : @"";
    if (titleString == nil || bodyString == nil) {
      return -1;
    }

    int userNotificationsResult =
        modex_send_un_notification(titleString, bodyString);
    return userNotificationsResult;
  }
}
