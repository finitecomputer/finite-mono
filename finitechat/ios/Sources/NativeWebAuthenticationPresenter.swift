import AuthenticationServices
import UIKit

@MainActor
protocol WebAuthenticationPresenting: AnyObject {
    func authenticate(url: URL) async throws -> URL
    func cancel()
}

enum WebAuthenticationPresentationError: Error, Equatable {
    case canceled
    case couldNotStart
    case missingCallback
    case invalidPresentationContext
    case failed
}

@MainActor
final class NativeWebAuthenticationPresenter: NSObject, WebAuthenticationPresenting {
    private var session: ASWebAuthenticationSession?

    nonisolated static func callbackDiagnosticSummary(_ url: URL) -> String {
        let components = URLComponents(url: url, resolvingAgainstBaseURL: false)
        let queryItems = components?.queryItems ?? []
        let codeCount = queryItems.count(where: { $0.name == "code" })
        let stateCount = queryItems.count(where: { $0.name == "state" })
        let errorCount = queryItems.count(where: { $0.name == "error" })
        let unexpectedCount = queryItems.count(where: {
            $0.name != "code"
                && $0.name != "state"
                && $0.name != "error"
                && $0.name != "error_description"
        })
        return [
            "scheme_https=\(components?.scheme == "https")",
            "host_expected=\(components?.host == "finite.computer")",
            "path_expected=\(components?.path == "/auth/ios/callback")",
            "fragment_present=\(components?.fragment != nil)",
            "code_count=\(codeCount)",
            "state_count=\(stateCount)",
            "error_count=\(errorCount)",
            "unexpected_query_count=\(unexpectedCount)",
        ].joined(separator: " ")
    }

    func authenticate(url: URL) async throws -> URL {
        cancel()
        return try await withCheckedThrowingContinuation { continuation in
            let session = ASWebAuthenticationSession(
                url: url,
                callback: .https(
                    host: "finite.computer",
                    path: "/auth/ios/callback"
                )
            ) { [weak self] callbackURL, error in
                Task { @MainActor in
                    self?.session = nil
                    if let error {
                        continuation.resume(
                            throwing: Self.presentationError(for: error)
                        )
                    } else if let callbackURL {
                        continuation.resume(returning: callbackURL)
                    } else {
                        continuation.resume(
                            throwing: WebAuthenticationPresentationError.missingCallback
                        )
                    }
                }
            }
            session.presentationContextProvider = self
            session.prefersEphemeralWebBrowserSession = false
            self.session = session
            if !session.start() {
                self.session = nil
                continuation.resume(
                    throwing: WebAuthenticationPresentationError.couldNotStart
                )
            }
        }
    }

    nonisolated static func presentationError(
        for error: Error
    ) -> WebAuthenticationPresentationError {
        let error = error as NSError
        guard error.domain == ASWebAuthenticationSessionError.errorDomain,
              let code = ASWebAuthenticationSessionError.Code(
                rawValue: error.code
              )
        else {
            return .failed
        }

        switch code {
        case .canceledLogin:
            return .canceled
        case .presentationContextNotProvided, .presentationContextInvalid:
            return .invalidPresentationContext
        @unknown default:
            return .failed
        }
    }

    func cancel() {
        session?.cancel()
        session = nil
    }
}

extension NativeWebAuthenticationPresenter:
    ASWebAuthenticationPresentationContextProviding
{
    func presentationAnchor(
        for session: ASWebAuthenticationSession
    ) -> ASPresentationAnchor {
        let scenes = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
        if let keyWindow = scenes
            .flatMap(\.windows)
            .first(where: \.isKeyWindow)
        {
            return keyWindow
        }
        guard let windowScene = scenes.first else {
            preconditionFailure("Web authentication requires an active window scene.")
        }
        return ASPresentationAnchor(windowScene: windowScene)
    }
}
