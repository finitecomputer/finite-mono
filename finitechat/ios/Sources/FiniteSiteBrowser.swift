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

/// In-app site viewer. Private Finite Sites authenticate through the Finite
/// Auth Gate: an unauthenticated navigation is redirected (top-level) to the
/// gate, the human signs in there, and the gate returns with a short-lived
/// vouch that the site exchanges for its own viewer cookie. The web view
/// needs no key material or preflight; the old NIP-98
/// `/_finite/auth/native-session` preflight was deleted with that endpoint.
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
        webView.navigationDelegate = nil
    }

    final class Coordinator: NSObject, WKNavigationDelegate {
        var parent: FiniteSiteWebView
        private var loadedToken: UUID?

        init(parent: FiniteSiteWebView) {
            self.parent = parent
        }

        func loadIfNeeded(in webView: WKWebView) {
            guard loadedToken != parent.reloadToken else { return }
            loadedToken = parent.reloadToken
            webView.load(URLRequest(url: parent.url))
        }
    }
}
