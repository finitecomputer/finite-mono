import SwiftUI
import UIKit
import UserNotifications

final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        NavigationChrome.configure()
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    func application(
        _ application: UIApplication,
        didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        PushNotificationManager.shared.handleRegisteredDeviceToken(deviceToken)
    }

    func application(
        _ application: UIApplication,
        didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
        PushNotificationManager.shared.handleRegistrationFailure(error)
    }

    func application(
        _ application: UIApplication,
        didReceiveRemoteNotification userInfo: [AnyHashable: Any],
        fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void
    ) {
        PushNotificationManager.shared.handleRemoteNotification(
            userInfo,
            completion: completionHandler
        )
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions)
            -> Void
    ) {
        completionHandler([])
    }
}

@main
struct FiniteChatApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var model = AppModel(requiresNostrLogin: true)

    var body: some Scene {
        WindowGroup {
            ContentView(model: model)
                .onAppear {
                    configurePushNotifications()
                }
        }
    }

    private func configurePushNotifications() {
        let manager = PushNotificationManager.shared
        let appModel = model
        manager.onTokenReceived = { [weak appModel] token in
            appModel?.registerPushToken(token)
        }
        manager.onRegistrationFailed = { [weak appModel] error in
            appModel?.notePushRegistrationFailed(error)
        }
        manager.onRemoteWake = { [weak appModel] userInfo, completion in
            guard let appModel else {
                completion(.noData)
                return
            }
            appModel.handleRemotePushWake(userInfo: userInfo, completion: completion)
        }
        manager.registerForRemoteNotifications()
    }
}
