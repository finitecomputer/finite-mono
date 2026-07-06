import Foundation
import UIKit

let maxImageUploadBytes = 8 * 1024 * 1024

struct ImageUploadPayload: Sendable {
    let data: Data
    let mimeType: String

    init(sourceData: Data) throws {
        guard !sourceData.isEmpty, let image = UIImage(data: sourceData) else {
            throw ImageUploadError.unreadableImage
        }
        guard image.size.width > 0, image.size.height > 0 else {
            throw ImageUploadError.unreadableImage
        }

        let maxSide: CGFloat = 1024
        let largestSide = max(image.size.width, image.size.height)
        let scale = min(maxSide / largestSide, 1)
        let renderSize = CGSize(
            width: max(1, image.size.width * scale),
            height: max(1, image.size.height * scale)
        )
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        let renderer = UIGraphicsImageRenderer(size: renderSize, format: format)
        let rendered = renderer.image { _ in
            image.draw(in: CGRect(origin: .zero, size: renderSize))
        }
        guard let jpeg = rendered.jpegData(compressionQuality: 0.86), !jpeg.isEmpty else {
            throw ImageUploadError.unreadableImage
        }
        guard jpeg.count <= maxImageUploadBytes else {
            throw ImageUploadError.tooLarge
        }

        data = jpeg
        mimeType = "image/jpeg"
    }
}

enum ImageUploadError: LocalizedError {
    case unreadableImage
    case tooLarge

    var errorDescription: String? {
        switch self {
        case .unreadableImage:
            "That image could not be read."
        case .tooLarge:
            "Images must be 8 MB or smaller."
        }
    }
}
