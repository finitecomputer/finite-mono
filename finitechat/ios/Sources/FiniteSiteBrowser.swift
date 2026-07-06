import Foundation
import Security
import SwiftUI
import UIKit
import WebKit

struct FiniteSiteBrowserItem: Identifiable, Equatable {
    let id = UUID()
    let url: URL
}

struct FiniteSiteBrowserView: View {
    let url: URL
    let identity: AppNostrIdentity?

    @Environment(\.dismiss) private var dismiss
    @State private var reloadToken = UUID()

    private var title: String {
        URLComponents(url: url, resolvingAgainstBaseURL: false)?.host ?? "Site"
    }

    var body: some View {
        NavigationStack {
            FiniteSiteWebView(url: url, identity: identity, reloadToken: reloadToken)
                .ignoresSafeArea(edges: .bottom)
                .navigationTitle(title)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItemGroup(placement: .topBarTrailing) {
                        Button {
                            reloadToken = UUID()
                        } label: {
                            Image(systemName: "arrow.clockwise")
                        }
                        .accessibilityLabel("Reload")

                        Button {
                            UIApplication.shared.open(url)
                        } label: {
                            Image(systemName: "safari")
                        }
                        .accessibilityLabel("Open in Safari")

                        GlassCircleCloseButton { dismiss() }
                    }
                }
        }
    }
}

struct FiniteSiteWebView: UIViewRepresentable {
    let url: URL
    let identity: AppNostrIdentity?
    let reloadToken: UUID

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .default()
        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.allowsBackForwardNavigationGestures = true
        webView.navigationDelegate = context.coordinator
        webView.backgroundColor = .systemBackground
        webView.scrollView.backgroundColor = .systemBackground
        return webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {
        context.coordinator.parent = self
        context.coordinator.loadIfNeeded(in: webView)
    }

    static func dismantleUIView(_ webView: WKWebView, coordinator: Coordinator) {
        coordinator.cancel()
        webView.navigationDelegate = nil
    }

    final class Coordinator: NSObject, WKNavigationDelegate {
        var parent: FiniteSiteWebView
        private var loadedToken: UUID?
        private var loadTask: Task<Void, Never>?

        init(parent: FiniteSiteWebView) {
            self.parent = parent
        }

        func loadIfNeeded(in webView: WKWebView) {
            guard loadedToken != parent.reloadToken else { return }
            loadedToken = parent.reloadToken
            loadTask?.cancel()

            let url = parent.url
            guard let identity = parent.identity,
                  FiniteSiteNativeSessionPreflight.canPreflight(url: url)
            else {
                webView.load(URLRequest(url: url))
                return
            }

            let cookieStore = webView.configuration.websiteDataStore.httpCookieStore
            loadTask = Task { [weak webView] in
                _ = await FiniteSiteNativeSessionPreflight.prepare(
                    originalURL: url,
                    identity: identity,
                    cookieStore: cookieStore
                )
                guard !Task.isCancelled else { return }
                await MainActor.run {
                    _ = webView?.load(URLRequest(url: url))
                }
            }
        }

        func cancel() {
            loadTask?.cancel()
        }
    }
}

enum FiniteSiteNativeSessionPreflight {
    private static let authPath = "/_finite/auth/native-session"
    private static let clientID = "finite-chat-ios"

    static func canPreflight(url: URL) -> Bool {
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
              let scheme = components.scheme?.lowercased(),
              scheme == "http" || scheme == "https",
              components.host != nil
        else {
            return false
        }
        return true
    }

    static func authURL(for url: URL) -> URL? {
        guard var components = URLComponents(url: url, resolvingAgainstBaseURL: false),
              components.host != nil,
              let scheme = components.scheme?.lowercased(),
              scheme == "http" || scheme == "https"
        else {
            return nil
        }
        components.percentEncodedPath = authPath
        components.percentEncodedQuery = nil
        components.percentEncodedFragment = nil
        return components.url
    }

    static func returnPath(for url: URL) -> String {
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            return "/"
        }
        var value = components.percentEncodedPath.isEmpty ? "/" : components.percentEncodedPath
        if let query = components.percentEncodedQuery, !query.isEmpty {
            value += "?\(query)"
        }
        if let fragment = components.percentEncodedFragment, !fragment.isEmpty {
            value += "#\(fragment)"
        }
        guard value.starts(with: "/"), !value.starts(with: "//") else {
            return "/"
        }
        return value
    }

    static func makeNonce(byteCount: Int = 24) -> String {
        var bytes = [UInt8](repeating: 0, count: byteCount)
        let status = SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
        if status != errSecSuccess {
            return UUID().uuidString.replacingOccurrences(of: "-", with: "")
        } else {
            return Data(bytes)
                .base64EncodedString()
                .replacingOccurrences(of: "+", with: "-")
                .replacingOccurrences(of: "/", with: "_")
                .replacingOccurrences(of: "=", with: "")
        }
    }

    @discardableResult
    static func prepare(
        originalURL: URL,
        identity: AppNostrIdentity,
        cookieStore: WKHTTPCookieStore
    ) async -> Bool {
        guard let authURL = authURL(for: originalURL) else { return false }
        let returnTo = returnPath(for: originalURL)
        let nonce = makeNonce()
        let now = UInt64(Date().timeIntervalSince1970)

        let proof: FiniteSitesNativeSessionProof
        do {
            proof = try finiteSitesNativeViewerSessionProof(
                accountSecretHex: identity.accountSecretHex,
                url: authURL.absoluteString,
                returnTo: returnTo,
                client: clientID,
                nonce: nonce,
                nowUnixSeconds: now
            )
        } catch {
            return false
        }

        guard let body = proof.bodyJson.data(using: .utf8) else { return false }
        var request = URLRequest(url: authURL)
        request.httpMethod = "POST"
        request.httpBody = body
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue(proof.authorizationHeader, forHTTPHeaderField: "Authorization")

        let delegate = NoRedirectURLSessionDelegate()
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpCookieAcceptPolicy = .always
        configuration.httpShouldSetCookies = false
        let session = URLSession(configuration: configuration, delegate: delegate, delegateQueue: nil)
        defer {
            session.finishTasksAndInvalidate()
        }

        do {
            let (_, response) = try await session.data(for: request)
            guard let http = response as? HTTPURLResponse,
                  (300..<400).contains(http.statusCode)
            else {
                return false
            }
            let cookies = HTTPCookie.cookies(
                withResponseHeaderFields: http.headerFields,
                for: authURL
            )
            guard !cookies.isEmpty else { return false }
            for cookie in cookies {
                await cookieStore.setCookieAsync(cookie)
            }
            return true
        } catch {
            return false
        }
    }
}

private final class NoRedirectURLSessionDelegate: NSObject, URLSessionTaskDelegate {
    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest,
        completionHandler: @escaping (URLRequest?) -> Void
    ) {
        completionHandler(nil)
    }
}

private extension HTTPURLResponse {
    var headerFields: [String: String] {
        allHeaderFields.reduce(into: [:]) { result, entry in
            guard let key = entry.key as? String else { return }
            result[key] = String(describing: entry.value)
        }
    }
}

private extension WKHTTPCookieStore {
    func setCookieAsync(_ cookie: HTTPCookie) async {
        await withCheckedContinuation { continuation in
            setCookie(cookie) {
                continuation.resume()
            }
        }
    }
}
