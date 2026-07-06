import Foundation
import CryptoKit
import ImageIO
import SwiftUI
import UIKit

final class RemoteImageMemoryCache: @unchecked Sendable {
    static let shared = RemoteImageMemoryCache()

    private let cache = NSCache<NSURL, UIImage>()

    init() {
        cache.countLimit = 200
        cache.totalCostLimit = 100 * 1024 * 1024
    }

    func image(for url: URL) -> UIImage? {
        cache.object(forKey: url as NSURL)
    }

    func setImage(_ image: UIImage, for url: URL) {
        let cost = image.images?.reduce(0) { $0 + imageCost($1) } ?? imageCost(image)
        cache.setObject(image, forKey: url as NSURL, cost: cost)
    }

    private func imageCost(_ image: UIImage) -> Int {
        guard let cgImage = image.cgImage else { return 0 }
        return cgImage.bytesPerRow * cgImage.height
    }
}

final class RemoteImageDiskCache: @unchecked Sendable {
    static let shared = RemoteImageDiskCache()

    private let directory: URL

    init(directory: URL? = nil) {
        if let directory {
            self.directory = directory
        } else {
            let root = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
                .first ?? FileManager.default.temporaryDirectory
            self.directory = root
                .appendingPathComponent("FiniteChat", isDirectory: true)
                .appendingPathComponent("RemoteImageCache", isDirectory: true)
        }
    }

    func data(for url: URL) -> Data? {
        guard !url.isFileURL else { return nil }
        return try? Data(contentsOf: cacheURL(for: url))
    }

    func setData(_ data: Data, for url: URL) {
        guard !url.isFileURL else { return }
        do {
            try FileManager.default.createDirectory(
                at: directory,
                withIntermediateDirectories: true
            )
            try data.write(to: cacheURL(for: url), options: [.atomic])
        } catch {
            // Avatar disk-cache failures should never block chat rendering.
        }
    }

    private func cacheURL(for url: URL) -> URL {
        directory.appendingPathComponent(cacheKey(for: url))
    }

    private func cacheKey(for url: URL) -> String {
        let digest = SHA256.hash(data: Data(url.absoluteString.utf8))
        return digest.map { String(format: "%02x", $0) }.joined()
    }
}

@MainActor
final class CachedRemoteImageLoader: ObservableObject {
    @Published private(set) var image: UIImage?

    private var currentURL: URL?
    private var task: Task<Void, Never>?

    deinit {
        task?.cancel()
    }

    func load(url: URL) {
        guard currentURL != url else { return }
        currentURL = url
        task?.cancel()

        if let cached = RemoteImageMemoryCache.shared.image(for: url) {
            image = cached
            return
        }

        image = nil
        task = Task {
            do {
                let data: Data
                if let cached = await Task.detached(operation: {
                    RemoteImageDiskCache.shared.data(for: url)
                }).value {
                    data = cached
                } else {
                    data = try await Self.loadData(url: url)
                }
                let decoded = await Task.detached {
                    Self.decodeImage(data: data)
                }.value
                guard !Task.isCancelled, let decoded else { return }
                await Task.detached {
                    RemoteImageDiskCache.shared.setData(data, for: url)
                }.value
                RemoteImageMemoryCache.shared.setImage(decoded, for: url)
                image = decoded
            } catch {
                // Keep the initials placeholder visible on failed avatar loads.
            }
        }
    }

    nonisolated private static func loadData(url: URL) async throws -> Data {
        if url.isFileURL {
            let fileURL = url.withoutQueryAndFragment()
            return try Data(contentsOf: fileURL)
        }
        let (data, _) = try await URLSession.shared.data(from: url)
        return data
    }

    nonisolated private static func decodeImage(data: Data) -> UIImage? {
        if data.isGIFData,
           let animated = UIImage.animatedGIF(data: data)
        {
            return animated
        }
        return UIImage(data: data)
    }
}

struct CachedRemoteImage<Content: View, Placeholder: View>: View {
    let url: URL
    var animatedContentMode: UIView.ContentMode = .scaleAspectFill
    @ViewBuilder let content: (Image) -> Content
    @ViewBuilder let placeholder: () -> Placeholder

    @StateObject private var loader = CachedRemoteImageLoader()

    var body: some View {
        Group {
            if let image = loader.image {
                if image.images != nil {
                    AnimatedRemoteImageView(image: image, contentMode: animatedContentMode)
                } else {
                    content(Image(uiImage: image))
                }
            } else {
                placeholder()
            }
        }
        .task(id: url) {
            loader.load(url: url)
        }
    }
}

private struct AnimatedRemoteImageView: UIViewRepresentable {
    let image: UIImage
    let contentMode: UIView.ContentMode

    func makeUIView(context: Context) -> UIImageView {
        let imageView = UIImageView()
        imageView.contentMode = contentMode
        imageView.clipsToBounds = true
        imageView.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        imageView.setContentCompressionResistancePriority(.defaultLow, for: .vertical)
        imageView.image = image
        return imageView
    }

    func updateUIView(_ imageView: UIImageView, context: Context) {
        imageView.image = image
        imageView.contentMode = contentMode
    }
}

private extension Data {
    var isGIFData: Bool {
        count >= 4 && prefix(4).elementsEqual([0x47, 0x49, 0x46, 0x38])
    }
}

private extension UIImage {
    static func animatedGIF(data: Data) -> UIImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let frameCount = CGImageSourceGetCount(source)
        guard frameCount > 1 else { return nil }

        var frames: [UIImage] = []
        var duration = 0.0
        for frameIndex in 0..<frameCount {
            guard let cgImage = CGImageSourceCreateImageAtIndex(source, frameIndex, nil) else {
                continue
            }
            frames.append(UIImage(cgImage: cgImage))
            duration += Self.gifFrameDelay(source: source, frameIndex: frameIndex)
        }
        guard !frames.isEmpty else { return nil }
        return UIImage.animatedImage(with: frames, duration: duration)
    }

    private static func gifFrameDelay(source: CGImageSource, frameIndex: Int) -> Double {
        let properties = CGImageSourceCopyPropertiesAtIndex(source, frameIndex, nil) as? [String: Any]
        let gif = properties?[kCGImagePropertyGIFDictionary as String] as? [String: Any]
        let unclamped = gif?[kCGImagePropertyGIFUnclampedDelayTime as String] as? Double
        let clamped = gif?[kCGImagePropertyGIFDelayTime as String] as? Double
        return max(unclamped ?? clamped ?? 0.1, 0.02)
    }
}

private extension URL {
    func withoutQueryAndFragment() -> URL {
        guard var components = URLComponents(url: self, resolvingAgainstBaseURL: false) else {
            return self
        }
        components.query = nil
        components.fragment = nil
        return components.url ?? self
    }
}
