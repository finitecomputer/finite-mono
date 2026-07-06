import AVFoundation
import AVKit
import QuickLook
import SwiftUI
import UIKit

struct ChatImagePreviewSelection: Identifiable, Equatable {
    let attachments: [ChatMediaAttachment]
    let selected: ChatMediaAttachment

    var id: String {
        selected.attachmentId
    }

    init(attachments: [ChatMediaAttachment], selected: ChatMediaAttachment) {
        let localAttachments = attachments.filter { attachmentLocalURL($0) != nil }
        self.attachments = localAttachments.isEmpty ? [selected] : localAttachments
        self.selected = selected
    }
}

struct ChatAttachmentPreviewItem: Identifiable, Equatable {
    let attachment: ChatMediaAttachment
    let url: URL

    var id: String {
        "\(attachment.attachmentId)|\(url.path)"
    }
}

struct ChatImagePreviewView: View {
    let selection: ChatImagePreviewSelection
    let onDismiss: () -> Void

    @State private var currentID: String
    @State private var zoomScales: [String: CGFloat] = [:]

    init(selection: ChatImagePreviewSelection, onDismiss: @escaping () -> Void) {
        self.selection = selection
        self.onDismiss = onDismiss
        _currentID = State(initialValue: selection.selected.attachmentId)
    }

    private var currentAttachment: ChatMediaAttachment {
        selection.attachments.first { $0.attachmentId == currentID }
            ?? selection.selected
    }

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()

            TabView(selection: $currentID) {
                ForEach(selection.attachments, id: \.attachmentId) { attachment in
                    ChatImagePreviewPage(attachment: attachment) { scale in
                        zoomScales[attachment.attachmentId] = scale
                    }
                    .tag(attachment.attachmentId)
                }
            }
            .tabViewStyle(.page(indexDisplayMode: selection.attachments.count > 1 ? .automatic : .never))
        }
        .overlay(alignment: .top) {
            ChatPreviewTopBar(
                title: currentAttachment.filename,
                shareURL: attachmentLocalURL(currentAttachment),
                onDismiss: onDismiss
            )
        }
        .preferredColorScheme(.dark)
    }
}

private struct ChatImagePreviewPage: View {
    let attachment: ChatMediaAttachment
    let onZoomScaleChange: (CGFloat) -> Void

    @State private var image: UIImage?

    var body: some View {
        Group {
            if let image {
                ZoomableImageView(image: image, onZoomScaleChange: onZoomScaleChange)
                    .ignoresSafeArea()
            } else {
                ProgressView()
                    .tint(.white)
            }
        }
        .task(id: attachment.attachmentId) {
            image = await loadPreviewImage(attachment)
        }
    }

    private func loadPreviewImage(_ attachment: ChatMediaAttachment) async -> UIImage? {
        guard let url = attachmentLocalURL(attachment) else { return nil }
        return await Task.detached(priority: .userInitiated) {
            UIImage(contentsOfFile: url.path)
        }.value
    }
}

struct ChatVideoPreviewView: View {
    let item: ChatAttachmentPreviewItem
    let onDismiss: () -> Void

    @State private var player: AVPlayer?

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()

            if let player {
                VideoPlayer(player: player)
                    .ignoresSafeArea()
            } else {
                ProgressView()
                    .tint(.white)
            }
        }
        .overlay(alignment: .top) {
            ChatPreviewTopBar(
                title: item.attachment.filename,
                shareURL: item.url,
                onDismiss: onDismiss
            )
        }
        .preferredColorScheme(.dark)
        .onAppear {
            let avPlayer = AVPlayer(url: item.url)
            player = avPlayer
            avPlayer.play()
        }
        .onDisappear {
            player?.pause()
            player = nil
        }
    }
}

struct ChatDocumentPreviewView: View {
    let item: ChatAttachmentPreviewItem
    let onDismiss: () -> Void

    var body: some View {
        NavigationStack {
            QuickLookPreviewController(url: item.url)
                .ignoresSafeArea()
                .navigationTitle(previewTitle(item.attachment))
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItemGroup(placement: .topBarTrailing) {
                        ShareLink(item: item.url) {
                            Image(systemName: "square.and.arrow.up")
                        }
                        .accessibilityLabel("Share")

                        GlassCircleCloseButton { onDismiss() }
                    }
                }
        }
    }
}

struct LocalVideoThumbnailView<Placeholder: View>: View {
    let path: String
    let placeholder: () -> Placeholder

    @Environment(\.displayScale) private var displayScale
    @State private var image: CGImage?

    var body: some View {
        GeometryReader { geometry in
            let maxPixelSize = max(64, Int(max(geometry.size.width, geometry.size.height) * displayScale))
            ZStack {
                if let image {
                    Image(decorative: image, scale: displayScale)
                        .resizable()
                        .scaledToFill()
                } else {
                    placeholder()
                }
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
            .clipped()
            .task(id: "\(path)|\(maxPixelSize)") {
                image = await LocalVideoThumbnailLoader.thumbnail(path: path, maxPixelSize: maxPixelSize)
            }
        }
    }
}

private struct ChatPreviewTopBar: View {
    let title: String
    let shareURL: URL?
    let onDismiss: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Button(action: onDismiss) {
                Image(systemName: "xmark")
                    .font(.body.weight(.semibold))
                    .frame(width: 44, height: 44)
                    .contentShape(Rectangle())
            }
            .accessibilityLabel("Close")

            Text(previewTitle(title))
                .font(.headline)
                .lineLimit(1)
                .frame(maxWidth: .infinity)

            if let shareURL {
                ShareLink(item: shareURL) {
                    Image(systemName: "square.and.arrow.up")
                        .font(.body.weight(.semibold))
                        .frame(width: 44, height: 44)
                        .contentShape(Rectangle())
                }
                .accessibilityLabel("Share")
            } else {
                Color.clear.frame(width: 44, height: 44)
            }
        }
        .foregroundStyle(.white)
        .padding(.horizontal, 8)
        .padding(.top, 4)
        .background(
            LinearGradient(
                colors: [.black.opacity(0.58), .black.opacity(0.0)],
                startPoint: .top,
                endPoint: .bottom
            )
            .ignoresSafeArea(edges: .top)
        )
    }
}

private struct QuickLookPreviewController: UIViewControllerRepresentable {
    let url: URL

    func makeCoordinator() -> Coordinator {
        Coordinator(url: url)
    }

    func makeUIViewController(context: Context) -> QLPreviewController {
        let controller = QLPreviewController()
        controller.dataSource = context.coordinator
        return controller
    }

    func updateUIViewController(_ controller: QLPreviewController, context: Context) {
        context.coordinator.url = url
        controller.reloadData()
    }

    final class Coordinator: NSObject, QLPreviewControllerDataSource {
        var url: URL

        init(url: URL) {
            self.url = url
        }

        func numberOfPreviewItems(in controller: QLPreviewController) -> Int {
            1
        }

        func previewController(
            _ controller: QLPreviewController,
            previewItemAt index: Int
        ) -> QLPreviewItem {
            url as NSURL
        }
    }
}

private struct ZoomableImageView: UIViewRepresentable {
    let image: UIImage
    let onZoomScaleChange: (CGFloat) -> Void

    func makeUIView(context: Context) -> ZoomableImageScrollView {
        let view = ZoomableImageScrollView()
        view.onZoomScaleChange = onZoomScaleChange
        view.displayImage = image
        return view
    }

    func updateUIView(_ uiView: ZoomableImageScrollView, context: Context) {
        if uiView.displayImage !== image {
            uiView.displayImage = image
        }
    }
}

private final class ZoomableImageScrollView: UIView, UIScrollViewDelegate {
    private let scrollView = UIScrollView()
    private let imageView = UIImageView()
    var onZoomScaleChange: ((CGFloat) -> Void)?

    var displayImage: UIImage? {
        didSet {
            imageView.image = displayImage
            setNeedsLayout()
        }
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        setup()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is unavailable")
    }

    private func setup() {
        backgroundColor = .clear
        scrollView.delegate = self
        scrollView.minimumZoomScale = 1
        scrollView.maximumZoomScale = 5
        scrollView.showsVerticalScrollIndicator = false
        scrollView.showsHorizontalScrollIndicator = false
        scrollView.bouncesZoom = true
        scrollView.contentInsetAdjustmentBehavior = .never
        scrollView.backgroundColor = .clear
        imageView.contentMode = .scaleAspectFit
        imageView.clipsToBounds = true

        addSubview(scrollView)
        scrollView.addSubview(imageView)

        let doubleTap = UITapGestureRecognizer(target: self, action: #selector(handleDoubleTap(_:)))
        doubleTap.numberOfTapsRequired = 2
        scrollView.addGestureRecognizer(doubleTap)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard !bounds.isEmpty else { return }
        let sizeChanged = scrollView.frame.size != bounds.size
        scrollView.frame = bounds
        if sizeChanged {
            imageView.frame = bounds
            scrollView.setZoomScale(1, animated: false)
            onZoomScaleChange?(1)
        }
    }

    func viewForZooming(in scrollView: UIScrollView) -> UIView? {
        imageView
    }

    func scrollViewDidZoom(_ scrollView: UIScrollView) {
        centerImageView()
        onZoomScaleChange?(scrollView.zoomScale)
    }

    func scrollViewDidEndZooming(
        _ scrollView: UIScrollView,
        with view: UIView?,
        atScale scale: CGFloat
    ) {
        onZoomScaleChange?(scale)
    }

    private func centerImageView() {
        let size = scrollView.bounds.size
        var frame = imageView.frame
        frame.origin.x = frame.width < size.width ? (size.width - frame.width) / 2 : 0
        frame.origin.y = frame.height < size.height ? (size.height - frame.height) / 2 : 0
        imageView.frame = frame
    }

    @objc private func handleDoubleTap(_ gesture: UITapGestureRecognizer) {
        if scrollView.zoomScale > scrollView.minimumZoomScale {
            scrollView.setZoomScale(scrollView.minimumZoomScale, animated: true)
            return
        }
        let point = gesture.location(in: imageView)
        let scale: CGFloat = 3
        let size = CGSize(
            width: scrollView.bounds.width / scale,
            height: scrollView.bounds.height / scale
        )
        scrollView.zoom(
            to: CGRect(
                x: point.x - size.width / 2,
                y: point.y - size.height / 2,
                width: size.width,
                height: size.height
            ),
            animated: true
        )
    }
}

private enum LocalVideoThumbnailLoader {
    nonisolated static func thumbnail(path: String, maxPixelSize: Int) async -> CGImage? {
        await Task.detached(priority: .utility) {
            let asset = AVURLAsset(url: URL(fileURLWithPath: path))
            let generator = AVAssetImageGenerator(asset: asset)
            generator.appliesPreferredTrackTransform = true
            generator.maximumSize = CGSize(width: maxPixelSize, height: maxPixelSize)
            return try? generator.copyCGImage(at: .zero, actualTime: nil)
        }.value
    }
}

func attachmentLocalURL(_ attachment: ChatMediaAttachment) -> URL? {
    guard let path = attachment.localPath?.trimmingCharacters(in: .whitespacesAndNewlines),
          !path.isEmpty
    else {
        return nil
    }
    return URL(fileURLWithPath: path)
}

func attachmentCanDownload(_ attachment: ChatMediaAttachment) -> Bool {
    guard attachmentLocalURL(attachment) == nil,
          attachment.uploadProgressPerMille == nil,
          attachment.downloadProgressPerMille == nil,
          let url = attachment.url?.trimmingCharacters(in: .whitespacesAndNewlines)
    else {
        return false
    }
    return !url.isEmpty
}

func attachmentDeterminateTransferProgress(_ progressPerMille: UInt32?) -> Double? {
    guard let progress = progressPerMille,
          progress > 0
    else {
        return nil
    }
    return Double(min(progress, 1_000)) / 1_000
}

private func previewTitle(_ attachment: ChatMediaAttachment) -> String {
    previewTitle(attachment.filename)
}

private func previewTitle(_ value: String) -> String {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.isEmpty ? "Attachment" : trimmed
}
