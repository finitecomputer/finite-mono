import SwiftUI
import UIKit

enum NavigationChrome {
    static func appearanceWithoutSeparator(
        configure: (UINavigationBarAppearance) -> Void = { _ in }
    ) -> UINavigationBarAppearance {
        let appearance = UINavigationBarAppearance()
        appearance.configureWithDefaultBackground()
        appearance.shadowColor = .clear
        appearance.shadowImage = UIImage()
        configure(appearance)
        return appearance
    }

    static func chatBlurredAppearance() -> UINavigationBarAppearance {
        let appearance = UINavigationBarAppearance()
        appearance.configureWithTransparentBackground()
        appearance.backgroundEffect = UIBlurEffect(style: .systemThickMaterial)
        appearance.backgroundColor = .clear
        appearance.shadowColor = .clear
        appearance.shadowImage = UIImage()
        return appearance
    }

    static func configure() {
        apply(to: UINavigationBar.appearance(), appearance: appearanceWithoutSeparator())
    }

    static func applyListNavigationChrome(to navigationBar: UINavigationBar) {
        let appearance = appearanceWithoutSeparator()
        apply(to: navigationBar, appearance: appearance)
        navigationBar.isTranslucent = true
    }

    static func applyChatNavigationBlur(to navigationItem: UINavigationItem) {
        let appearance = chatBlurredAppearance()
        navigationItem.standardAppearance = appearance
        navigationItem.scrollEdgeAppearance = appearance
        navigationItem.compactAppearance = appearance
        navigationItem.compactScrollEdgeAppearance = appearance
    }

    static func clearChatNavigationBlur(from navigationItem: UINavigationItem) {
        navigationItem.standardAppearance = nil
        navigationItem.scrollEdgeAppearance = nil
        navigationItem.compactAppearance = nil
        navigationItem.compactScrollEdgeAppearance = nil
    }

    private static func apply(to navigationBar: UINavigationBar, appearance: UINavigationBarAppearance) {
        navigationBar.standardAppearance = appearance
        navigationBar.compactAppearance = appearance
        navigationBar.scrollEdgeAppearance = appearance
        navigationBar.compactScrollEdgeAppearance = appearance
    }

    private static func apply(to navigationBar: UINavigationBarAppearanceProxy, appearance: UINavigationBarAppearance) {
        navigationBar.standardAppearance = appearance
        navigationBar.compactAppearance = appearance
        navigationBar.scrollEdgeAppearance = appearance
        navigationBar.compactScrollEdgeAppearance = appearance
    }
}

private protocol UINavigationBarAppearanceProxy: AnyObject {
    var standardAppearance: UINavigationBarAppearance { get set }
    var compactAppearance: UINavigationBarAppearance? { get set }
    var scrollEdgeAppearance: UINavigationBarAppearance? { get set }
    var compactScrollEdgeAppearance: UINavigationBarAppearance? { get set }
}

extension UINavigationBar: UINavigationBarAppearanceProxy {}

struct ChatNavigationBarChromeModifier: ViewModifier {
    func body(content: Content) -> some View {
        content
            .toolbarBackground(.hidden, for: .navigationBar)
            .background(ChatNavigationBarChromeAnchor())
    }
}

struct ListNavigationBarChromeModifier: ViewModifier {
    func body(content: Content) -> some View {
        content
            .background(ListNavigationBarChromeAnchor())
    }
}

private struct ChatNavigationBarChromeAnchor: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> ChatNavigationBarChromeViewController {
        ChatNavigationBarChromeViewController()
    }

    func updateUIViewController(_ uiViewController: ChatNavigationBarChromeViewController, context: Context) {}
}

private struct ListNavigationBarChromeAnchor: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> ListNavigationBarChromeViewController {
        ListNavigationBarChromeViewController()
    }

    func updateUIViewController(_ uiViewController: ListNavigationBarChromeViewController, context: Context) {}
}

private final class ChatNavigationBarChromeViewController: UIViewController {
    override func loadView() {
        view = UIView(frame: .zero)
        view.isHidden = true
        view.isUserInteractionEnabled = false
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        NavigationChrome.applyChatNavigationBlur(to: navigationItem)
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        guard isMovingFromParent || isBeingDismissed else { return }
        NavigationChrome.clearChatNavigationBlur(from: navigationItem)
        if let navigationBar = navigationController?.navigationBar {
            NavigationChrome.applyListNavigationChrome(to: navigationBar)
        }
    }
}

private final class ListNavigationBarChromeViewController: UIViewController {
    override func loadView() {
        view = UIView(frame: .zero)
        view.isHidden = true
        view.isUserInteractionEnabled = false
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        guard let navigationBar = navigationController?.navigationBar else { return }
        NavigationChrome.applyListNavigationChrome(to: navigationBar)
    }
}

extension View {
    func chatNavigationBarChrome() -> some View {
        modifier(ChatNavigationBarChromeModifier())
    }

    func listNavigationBarChrome() -> some View {
        modifier(ListNavigationBarChromeModifier())
    }
}
