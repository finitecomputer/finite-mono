import SwiftUI

struct ProfileAvatar: View {
    let displayName: String?
    let pictureURL: String?
    var size: CGFloat
    var fallbackSystemImage: String?
    var fallbackTint: Color = .secondary
    var fallbackBackground: Color = Color(.tertiarySystemFill)

    init(
        displayName: String?,
        pictureURL: String?,
        size: CGFloat = 40,
        fallbackSystemImage: String? = nil,
        fallbackTint: Color = .secondary,
        fallbackBackground: Color = Color(.tertiarySystemFill)
    ) {
        self.displayName = displayName
        self.pictureURL = pictureURL
        self.size = size
        self.fallbackSystemImage = fallbackSystemImage
        self.fallbackTint = fallbackTint
        self.fallbackBackground = fallbackBackground
    }

    init(profile: AppProfileSummary, size: CGFloat = 40) {
        self.init(
            displayName: profile.displayName,
            pictureURL: profile.picture,
            size: size
        )
    }

    var body: some View {
        ZStack {
            Circle()
                .fill(fallbackBackground)

            if let url = pictureURL.flatMap(validURL) {
                CachedRemoteImage(url: url) { image in
                    image
                        .resizable()
                        .scaledToFill()
                } placeholder: {
                    fallbackContent
                }
            } else {
                fallbackContent
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
    }

    @ViewBuilder
    private var fallbackContent: some View {
        if let fallbackSystemImage, initialText == "#" {
            Image(systemName: fallbackSystemImage)
                .font(.system(size: max(13, size * 0.42), weight: .semibold))
                .foregroundStyle(fallbackTint)
        } else {
            Text(initialText)
                .font(.system(size: max(13, size * 0.36), weight: .semibold))
                .foregroundStyle(fallbackTint)
        }
    }

    private var initialText: String {
        let trimmed = displayName?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard let first = trimmed.first else { return "#" }
        return String(first).uppercased()
    }

    private func validURL(_ value: String) -> URL? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        return URL(string: trimmed)
    }
}
