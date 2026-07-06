import Foundation
import UIKit
import UserNotifications

@MainActor
final class PushNotificationManager: NSObject, ObservableObject {
    static let shared = PushNotificationManager()

    var onTokenReceived: ((String) -> Void)?
    var onRegistrationFailed: ((Error) -> Void)?
    var onRemoteWake: (([AnyHashable: Any], @escaping (UIBackgroundFetchResult) -> Void) -> Void)?

    private override init() {}

    func registerForRemoteNotifications() {
        #if targetEnvironment(simulator)
        return
        #else
        UIApplication.shared.registerForRemoteNotifications()
        #endif
    }

    func handleRegisteredDeviceToken(_ deviceToken: Data) {
        onTokenReceived?(Self.hexToken(from: deviceToken))
    }

    func handleRegistrationFailure(_ error: Error) {
        onRegistrationFailed?(error)
    }

    func handleRemoteNotification(
        _ userInfo: [AnyHashable: Any],
        completion: @escaping (UIBackgroundFetchResult) -> Void
    ) {
        guard let onRemoteWake else {
            completion(.noData)
            return
        }
        onRemoteWake(userInfo, completion)
    }

    static func hexToken(from data: Data) -> String {
        data.map { String(format: "%02x", $0) }.joined()
    }
}
