import SwiftUI

extension ChatMediaGalleryItem: Identifiable {
    public var id: String {
        itemId
    }
}

struct ChatMediaGalleryView: View {
    let roomTitle: String
    let items: [ChatMediaGalleryItem]
    let onDownloadAttachment: (ChatMediaGalleryItem) -> Void

    @State private var imagePreviewSelection: ChatImagePreviewSelection?
    @State private var videoPreviewItem: ChatAttachmentPreviewItem?

    private var localImageAttachments: [ChatMediaAttachment] {
        items
            .map(\.attachment)
            .filter { $0.kind == .image && attachmentLocalURL($0) != nil }
    }

    private let columns = [
        GridItem(.flexible(), spacing: 2),
        GridItem(.flexible(), spacing: 2),
        GridItem(.flexible(), spacing: 2),
    ]

    var body: some View {
        Group {
            if items.isEmpty {
                ContentUnavailableView {
                    Label("No Media", systemImage: "photo.on.rectangle.angled")
                } description: {
                    Text("Photos and videos shared in \(roomTitle) will appear here.")
                }
            } else {
                ScrollView {
                    LazyVGrid(columns: columns, spacing: 2) {
                        ForEach(items) { item in
                            ChatMediaGalleryTile(
                                item: item,
                                onTap: {
                                    handleTap(item)
                                }
                            )
                            .accessibilityIdentifier("MediaGalleryTile-\(item.id)")
                        }
                    }
                }
                .background(Color(uiColor: .systemGroupedBackground))
            }
        }
        .navigationTitle("Photos & Videos")
        .navigationBarTitleDisplayMode(.inline)
        .fullScreenCover(item: $imagePreviewSelection) { selection in
            ChatImagePreviewView(selection: selection) {
                imagePreviewSelection = nil
            }
        }
        .fullScreenCover(item: $videoPreviewItem) { item in
            ChatVideoPreviewView(item: item) {
                videoPreviewItem = nil
            }
        }
    }

    private func handleTap(_ item: ChatMediaGalleryItem) {
        let attachment = item.attachment
        if let url = attachmentLocalURL(attachment) {
            switch attachment.kind {
            case .image:
                imagePreviewSelection = ChatImagePreviewSelection(
                    attachments: localImageAttachments,
                    selected: attachment
                )
            case .video:
                videoPreviewItem = ChatAttachmentPreviewItem(attachment: attachment, url: url)
            case .voiceNote, .file:
                break
            }
            return
        }

        if attachmentCanDownload(attachment) {
            onDownloadAttachment(item)
        }
    }
}

private struct ChatMediaGalleryTile: View {
    let item: ChatMediaGalleryItem
    let onTap: () -> Void

    private var attachment: ChatMediaAttachment {
        item.attachment
    }

    var body: some View {
        Button(action: onTap) {
            ZStack {
                thumbnail

                if attachment.kind == .video, attachmentLocalURL(attachment) != nil {
                    Image(systemName: "play.fill")
                        .font(.title3.weight(.semibold))
                        .foregroundStyle(.white)
                        .padding(10)
                        .background(.black.opacity(0.42), in: Circle())
                }

                if let progress = uploadProgress {
                    GalleryProgressOverlay(progress: progress)
                } else if attachment.downloadProgressPerMille != nil {
                    GalleryProgressOverlay(progress: downloadProgress)
                } else if attachmentCanDownload(attachment) {
                    remoteBadge
                }
            }
            .aspectRatio(1, contentMode: .fit)
            .clipped()
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(accessibilityLabel)
    }

    @ViewBuilder
    private var thumbnail: some View {
        if attachment.kind == .image,
           let path = attachmentLocalURL(attachment)?.path
        {
            VerifiedLocalImageView(path: path) {
                placeholder
            }
        } else if attachment.kind == .video,
                  let path = attachmentLocalURL(attachment)?.path
        {
            LocalVideoThumbnailView(path: path) {
                placeholder
            }
        } else {
            placeholder
        }
    }

    private var placeholder: some View {
        Rectangle()
            .fill(Color(uiColor: .secondarySystemGroupedBackground))
            .overlay {
                VStack(spacing: 7) {
                    Image(systemName: iconName(for: attachment.kind))
                        .font(.title3.weight(.semibold))
                    Text(attachment.filename.isEmpty ? mediaLabel(for: attachment.kind) : attachment.filename)
                        .font(.caption)
                        .lineLimit(1)
                }
                .foregroundStyle(.secondary)
                .padding(8)
            }
    }

    private var remoteBadge: some View {
        VStack {
            Spacer()
            HStack {
                Spacer()
                Image(systemName: "arrow.down.circle.fill")
                    .font(.title3)
                    .symbolRenderingMode(.hierarchical)
                    .foregroundStyle(.white)
                    .padding(6)
                    .background(.black.opacity(0.42), in: Circle())
                    .padding(8)
            }
        }
    }

    private var uploadProgress: Double? {
        attachmentDeterminateTransferProgress(attachment.uploadProgressPerMille)
    }

    private var downloadProgress: Double? {
        attachmentDeterminateTransferProgress(attachment.downloadProgressPerMille)
    }

    private var accessibilityLabel: String {
        let label = mediaLabel(for: attachment.kind)
        let filename = attachment.filename.trimmingCharacters(in: .whitespacesAndNewlines)
        return filename.isEmpty ? label : "\(label), \(filename)"
    }
}

private struct GalleryProgressOverlay: View {
    let progress: Double?

    var body: some View {
        ZStack {
            Color.black.opacity(0.36)
            if let progress {
                ProgressView(value: progress)
                    .progressViewStyle(.circular)
                    .tint(.white)
                    .scaleEffect(1.2)
            } else {
                ProgressView()
                    .progressViewStyle(.circular)
                    .tint(.white)
                    .scaleEffect(1.2)
            }
        }
        .accessibilityLabel("Attachment transfer in progress")
    }
}
