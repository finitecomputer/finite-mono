import SwiftUI
import UIKit

enum EdgeBlurFadeDirection {
    case top
    case bottom
}

enum BottomSafeAreaInsets {
    static var current: CGFloat {
        guard let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene,
              let window = scene.windows.first(where: \.isKeyWindow)
        else {
            return 0
        }
        return window.safeAreaInsets.bottom
    }
}

struct BottomEdgeBlurFade: UIViewRepresentable {
    var height: CGFloat

    func makeUIView(context: Context) -> EdgeBlurFadeView {
        EdgeBlurFadeView(direction: .bottom)
    }

    func updateUIView(_ uiView: EdgeBlurFadeView, context: Context) {
        uiView.preferredHeight = height
    }
}

final class EdgeBlurFadeView: UIView {
    var preferredHeight: CGFloat = 0 {
        didSet {
            guard abs(preferredHeight - oldValue) > 0.5 else { return }
            invalidateIntrinsicContentSize()
        }
    }

    private let blurView = UIVisualEffectView(effect: UIBlurEffect(style: .systemThickMaterial))
    private let tintView = UIView()
    private let gradientMask = CAGradientLayer()

    init(direction: EdgeBlurFadeDirection) {
        super.init(frame: .zero)
        isUserInteractionEnabled = false
        backgroundColor = .clear
        isOpaque = false

        blurView.translatesAutoresizingMaskIntoConstraints = false
        addSubview(blurView)

        tintView.translatesAutoresizingMaskIntoConstraints = false
        tintView.isUserInteractionEnabled = false
        tintView.backgroundColor = UIColor.systemGroupedBackground.withAlphaComponent(0.68)
        addSubview(tintView)

        NSLayoutConstraint.activate([
            blurView.topAnchor.constraint(equalTo: topAnchor),
            blurView.leadingAnchor.constraint(equalTo: leadingAnchor),
            blurView.trailingAnchor.constraint(equalTo: trailingAnchor),
            blurView.bottomAnchor.constraint(equalTo: bottomAnchor),
            tintView.topAnchor.constraint(equalTo: topAnchor),
            tintView.leadingAnchor.constraint(equalTo: leadingAnchor),
            tintView.trailingAnchor.constraint(equalTo: trailingAnchor),
            tintView.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])

        switch direction {
        case .bottom:
            gradientMask.colors = [
                UIColor.clear.cgColor,
                UIColor.black.withAlphaComponent(0.62).cgColor,
                UIColor.black.cgColor,
            ]
            gradientMask.locations = [0, 0.42, 1]
        case .top:
            gradientMask.colors = [
                UIColor.black.cgColor,
                UIColor.black.withAlphaComponent(0.62).cgColor,
                UIColor.clear.cgColor,
            ]
            gradientMask.locations = [0, 0.48, 1]
        }
        gradientMask.startPoint = CGPoint(x: 0.5, y: 0)
        gradientMask.endPoint = CGPoint(x: 0.5, y: 1)
        layer.mask = gradientMask
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override var intrinsicContentSize: CGSize {
        CGSize(width: UIView.noIntrinsicMetric, height: preferredHeight)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        gradientMask.frame = bounds
    }
}

typealias BottomEdgeBlurFadeView = EdgeBlurFadeView
