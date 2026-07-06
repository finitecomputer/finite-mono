import Foundation
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct Composer: View {
    @Binding var text: String
    let replyTarget: ChatMessage?
    let canSubmit: Bool
    @Binding var stagedAttachments: [StagedComposerAttachment]
    @Binding var isPhotoPickerPresented: Bool
    @Binding var selectedPhotoItems: [PhotosPickerItem]
    @Binding var isInputFocused: Bool
    let reportError: (String) -> Void
    let onCancelReply: () -> Void
    let onSend: () -> Void
    let onStartVoiceRecording: () -> Void
    let onAttach: () -> Void
    let onCreatePoll: () -> Void

    var body: some View {
        VStack(spacing: 8) {
            if let replyTarget {
                ComposerReplyPreview(
                    message: replyTarget,
                    onCancel: onCancelReply
                )
            }

            if !stagedAttachments.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(stagedAttachments) { item in
                            StagedAttachmentThumbnail(item: item) {
                                withAnimation(.easeOut(duration: 0.16)) {
                                    stagedAttachments.removeAll { $0.id == item.id }
                                }
                            }
                        }
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                }
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }

            VStack(alignment: .leading, spacing: 10) {
                ZStack(alignment: .topLeading) {
                    if text.isEmpty {
                        Text("Message")
                            .font(.body)
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 5)
                            .padding(.vertical, 8)
                            .allowsHitTesting(false)
                            .accessibilityHidden(true)
                    }

                    StickerAwareTextView(
                        text: $text,
                        isFocused: $isInputFocused,
                        maxHeight: 150,
                        onSend: onSend,
                        onImagePaste: stagePastedAttachment
                    )
                    .frame(minHeight: 52)
                    .accessibilityLabel("Message")
                    .accessibilityIdentifier("ComposerMessageField")
                }
                .padding(.horizontal, 12)
                .padding(.top, 8)

                HStack(spacing: 12) {
                    Menu {
                        Button {
                            isPhotoPickerPresented = true
                        } label: {
                            Label("Photos & Videos", systemImage: "photo.on.rectangle")
                        }

                        Button {
                            onAttach()
                        } label: {
                            Label("Files", systemImage: "doc")
                        }

                        Button {
                            onCreatePoll()
                        } label: {
                            Label("Poll", systemImage: "chart.bar.doc.horizontal")
                        }
                    } label: {
                        Image(systemName: "plus")
                            .font(.title3.weight(.regular))
                            .frame(width: 34, height: 34)
                            .contentShape(Circle())
                    }
                    .accessibilityLabel("Attach")
                    .accessibilityIdentifier("AttachButton")
                    .photosPicker(
                        isPresented: $isPhotoPickerPresented,
                        selection: $selectedPhotoItems,
                        maxSelectionCount: remainingPhotoSelectionCount,
                        matching: .any(of: [.images, .videos])
                    )

                    Spacer()

                    if stagedAttachments.isEmpty {
                        Button {
                            onStartVoiceRecording()
                        } label: {
                            Image(systemName: "mic")
                                .font(.title3.weight(.regular))
                                .frame(width: 34, height: 34)
                                .contentShape(Circle())
                        }
                        .accessibilityLabel("Record voice message")
                        .accessibilityIdentifier("VoiceRecordButton")
                    }

                    if showsSendButton {
                        Button {
                            onSend()
                        } label: {
                            Image(systemName: "arrow.up")
                                .font(.body.weight(.bold))
                                .foregroundStyle(.white)
                                .frame(width: 34, height: 34)
                                .background(Circle().fill(Color.accentColor))
                        }
                        .disabled(sendDisabled)
                        .accessibilityLabel("Send")
                        .accessibilityIdentifier("SendButton")
                        .transition(.scale.combined(with: .opacity))
                    }
                }
                .foregroundStyle(.primary)
                .padding(.horizontal, 14)
                .padding(.bottom, 10)
            }
            .frame(maxWidth: .infinity, minHeight: 92, alignment: .topLeading)
            .modifier(ChatComposerSurface())
        }
        .padding(.horizontal, 16)
        .padding(.top, 8)
        .safeAreaPadding(.bottom, 8)
        .background(Color.clear)
        .animation(.easeInOut(duration: 0.16), value: stagedAttachments.isEmpty)
        .animation(.snappy(duration: 0.18), value: showsSendButton)
    }

    private var sendDisabled: Bool {
        stagedAttachments.isEmpty && !canSubmit
    }

    private var showsSendButton: Bool {
        !stagedAttachments.isEmpty || canSubmit
    }

    private var remainingPhotoSelectionCount: Int {
        max(1, maxStagedComposerAttachments - stagedAttachments.count)
    }

    private func stagePastedAttachment(data: Data, mimeType: String) {
        guard stagedAttachments.count < maxStagedComposerAttachments else {
            reportError("Attachment limit is \(maxStagedComposerAttachments) files.")
            return
        }

        do {
            let attachment = try StagedComposerAttachment(
                pastedData: data,
                mimeType: mimeType
            )
            withAnimation(.easeOut(duration: 0.16)) {
                stagedAttachments.append(attachment)
            }
        } catch {
            reportError(String(describing: error))
        }
    }
}

private struct ChatComposerSurface: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26.0, *) {
            GlassEffectContainer(spacing: 0) {
                content
                    .glassEffect(.regular.interactive(), in: .rect(cornerRadius: 28))
            }
        } else {
            content
                .background(
                    .ultraThinMaterial,
                    in: RoundedRectangle(cornerRadius: 28, style: .continuous)
                )
                .overlay {
                    RoundedRectangle(cornerRadius: 28, style: .continuous)
                        .strokeBorder(Color(.separator).opacity(0.18), lineWidth: 0.5)
                }
                .shadow(color: .black.opacity(0.08), radius: 18, x: 0, y: 8)
        }
    }
}

let maxStagedComposerAttachments = 32
let maxComposerAttachmentBytes = 32 * 1024 * 1024

struct StagedComposerAttachment: Identifiable {
    let id: String
    let data: Data
    let filename: String
    let mimeType: String
    let kind: ChatMediaKind
    let thumbnail: UIImage?

    var outboundAttachment: OutboundAttachment {
        OutboundAttachment(
            filename: filename,
            mimeType: mimeType,
            kind: kind,
            bytes: data
        )
    }

    init(fileURL: URL) throws {
        let didStartAccessing = fileURL.startAccessingSecurityScopedResource()
        defer {
            if didStartAccessing {
                fileURL.stopAccessingSecurityScopedResource()
            }
        }

        let data = try Data(contentsOf: fileURL)
        let type = UTType(filenameExtension: fileURL.pathExtension)
        try self.init(
            data: data,
            filename: fileURL.lastPathComponent.isEmpty ? "attachment" : fileURL.lastPathComponent,
            mimeType: type?.preferredMIMEType ?? "application/octet-stream",
            kind: chatMediaKind(for: type)
        )
    }

    init?(photoItem: PhotosPickerItem) async throws {
        guard let data = try await photoItem.loadTransferable(type: Data.self) else {
            return nil
        }
        let type = photoItem.supportedContentTypes.first
        let filename = "attachment-\(UUID().uuidString).\(defaultFilenameExtension(for: type))"
        self = try await Task.detached(priority: .userInitiated) {
            try StagedComposerAttachment(
                data: data,
                filename: filename,
                mimeType: type?.preferredMIMEType ?? "application/octet-stream",
                kind: chatMediaKind(for: type)
            )
        }.value
    }

    init(pastedData data: Data, mimeType: String) throws {
        let type = UTType(mimeType: mimeType)
        let ext = defaultFilenameExtension(for: type)
        let filename = "pasted-\(UUID().uuidString).\(ext)"
        try self.init(
            data: data,
            filename: filename,
            mimeType: type?.preferredMIMEType ?? mimeType,
            kind: chatMediaKind(for: type)
        )
    }

    private init(
        data: Data,
        filename: String,
        mimeType: String,
        kind: ChatMediaKind
    ) throws {
        guard data.count <= maxComposerAttachmentBytes else {
            throw ComposerAttachmentError.tooLarge(filename: filename)
        }
        self.id = UUID().uuidString
        self.data = data
        self.filename = filename
        self.mimeType = mimeType
        self.kind = kind
        self.thumbnail = Self.makeThumbnail(data: data, kind: kind)
    }

    private static func makeThumbnail(data: Data, kind: ChatMediaKind) -> UIImage? {
        guard kind == .image, let image = UIImage(data: data) else { return nil }
        let maxSide: CGFloat = 160
        let scale = min(maxSide / max(image.size.width, image.size.height), 1)
        let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        let renderer = UIGraphicsImageRenderer(size: size)
        return renderer.image { _ in
            image.draw(in: CGRect(origin: .zero, size: size))
        }
    }
}

enum ComposerAttachmentError: LocalizedError {
    case tooLarge(filename: String)

    var errorDescription: String? {
        switch self {
        case .tooLarge(let filename):
            "\(filename) is larger than the 32 MiB attachment limit."
        }
    }
}

private struct StagedAttachmentThumbnail: View {
    let item: StagedComposerAttachment
    let onRemove: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ZStack(alignment: .topTrailing) {
                thumbnail
                    .frame(width: 72, height: 72)
                    .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))

                Button(action: onRemove) {
                    Image(systemName: "xmark.circle.fill")
                        .font(.body)
                        .symbolRenderingMode(.palette)
                        .foregroundStyle(.white, .black.opacity(0.65))
                }
                .buttonStyle(.plain)
                .offset(x: 6, y: -6)
                .accessibilityLabel("Remove \(item.filename)")
            }

            Text(item.filename)
                .font(.caption2)
                .lineLimit(1)
                .frame(width: 72, alignment: .leading)
        }
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private var thumbnail: some View {
        if let image = item.thumbnail {
            Image(uiImage: image)
                .resizable()
                .scaledToFill()
        } else {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(Color(.tertiarySystemFill))
                .overlay {
                    VStack(spacing: 4) {
                        Image(systemName: stagedAttachmentIcon(for: item.kind))
                            .font(.title3)
                        Text(composerMediaLabel(for: item.kind))
                            .font(.caption2.weight(.medium))
                            .lineLimit(1)
                    }
                    .foregroundStyle(.secondary)
                    .padding(6)
                }
        }
    }
}

struct StickerAwareTextView: UIViewRepresentable {
    @Binding var text: String
    @Binding var isFocused: Bool
    var maxHeight: CGFloat = 150
    var onSend: (() -> Void)?
    var onImagePaste: ((Data, String) -> Void)?

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeUIView(context: Context) -> PastableTextView {
        let textView = PastableTextView()
        textView.delegate = context.coordinator
        textView.pasteDelegate = context.coordinator
        textView.maxAllowedHeight = maxHeight
        textView.onImagePaste = { data, mimeType in
            onImagePaste?(data, mimeType)
        }
        textView.onReturnKey = {
            onSend?()
        }
        textView.font = .preferredFont(forTextStyle: .body)
        textView.backgroundColor = .clear
        textView.textContainerInset = UIEdgeInsets(top: 7, left: 0, bottom: 7, right: 0)
        textView.textContainer.lineFragmentPadding = 0
        textView.isScrollEnabled = false
        textView.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        textView.setContentHuggingPriority(.defaultHigh, for: .vertical)
        return textView
    }

    func updateUIView(_ uiView: PastableTextView, context: Context) {
        context.coordinator.parent = self

        if uiView.text != text {
            uiView.text = text
            uiView.recalculateHeight()
        }
        uiView.maxAllowedHeight = maxHeight
        uiView.onImagePaste = { data, mimeType in
            onImagePaste?(data, mimeType)
        }
        uiView.onReturnKey = {
            onSend?()
        }

        if isFocused && !uiView.isFirstResponder {
            DispatchQueue.main.async {
                guard self.isFocused, !uiView.isFirstResponder else { return }
                uiView.becomeFirstResponder()
            }
        } else if !isFocused && uiView.isFirstResponder {
            DispatchQueue.main.async {
                guard !self.isFocused, uiView.isFirstResponder else { return }
                uiView.resignFirstResponder()
            }
        }
    }

    final class Coordinator: NSObject, UITextViewDelegate, UITextPasteDelegate {
        var parent: StickerAwareTextView

        init(parent: StickerAwareTextView) {
            self.parent = parent
        }

        func textPasteConfigurationSupporting(
            _ textPasteConfigurationSupporting: UITextPasteConfigurationSupporting,
            transform item: UITextPasteItem
        ) {
            if let pasteType = preferredImagePasteType(for: item.itemProvider) {
                item.itemProvider.loadDataRepresentation(
                    forTypeIdentifier: pasteType.identifier
                ) { [weak self] data, _ in
                    guard let data else { return }
                    DispatchQueue.main.async {
                        guard let textView = textPasteConfigurationSupporting as? PastableTextView
                        else { return }
                        textView.onImagePaste?(data, pasteType.mimeType)
                        self?.stripAttachments(in: textView)
                    }
                }
                item.setNoResult()
                return
            }
            item.setDefaultResult()
        }

        func textViewDidChange(_ textView: UITextView) {
            (textView as? PastableTextView)?.recalculateHeight()

            guard textView.text.contains("\u{FFFC}") else {
                parent.text = textView.text
                return
            }

            if let data = extractStickerImage(from: textView) {
                stripAttachments(in: textView)
                (textView as? PastableTextView)?.onImagePaste?(data, "image/png")
                return
            }

            parent.text = textView.text
        }

        func textViewDidBeginEditing(_ textView: UITextView) {
            parent.isFocused = true
        }

        func textViewDidEndEditing(_ textView: UITextView) {
            parent.isFocused = false
        }

        private func extractStickerImage(from textView: UITextView) -> Data? {
            let storage = textView.textStorage
            let range = NSRange(location: 0, length: storage.length)

            if #available(iOS 18.0, *) {
                var result: Data?
                storage.enumerateAttributes(in: range) { attributes, _, stop in
                    for (_, value) in attributes {
                        if let glyph = value as? NSAdaptiveImageGlyph,
                           let image = UIImage(data: glyph.imageContent),
                           let data = image.pngData()
                        {
                            result = data
                            stop.pointee = true
                            return
                        }
                    }
                }
                if result != nil {
                    return result
                }
            }

            var result: Data?
            storage.enumerateAttribute(.attachment, in: range) { value, _, stop in
                guard let attachment = value as? NSTextAttachment else { return }
                if let data = attachment.contents {
                    result = data
                } else if let image = attachment.image, let data = image.pngData() {
                    result = data
                } else if let wrapper = attachment.fileWrapper,
                          let data = wrapper.regularFileContents
                {
                    result = data
                } else if let image = attachment.image(
                    forBounds: attachment.bounds,
                    textContainer: nil,
                    characterIndex: 0
                ),
                    let data = image.pngData()
                {
                    result = data
                }
                if result != nil {
                    stop.pointee = true
                }
            }
            return result
        }

        private func stripAttachments(in textView: UITextView) {
            let plain = textView.text.replacingOccurrences(of: "\u{FFFC}", with: "")
            textView.text = plain
            parent.text = plain
        }

        private func preferredImagePasteType(for provider: NSItemProvider) -> ImagePasteType? {
            let preferredTypes: [UTType] = [.gif, .png, .jpeg]
            for type in preferredTypes
            where provider.hasItemConformingToTypeIdentifier(type.identifier) {
                return ImagePasteType(type: type)
            }

            for identifier in provider.registeredTypeIdentifiers {
                guard let type = UTType(identifier), type.conforms(to: .image) else {
                    continue
                }
                return ImagePasteType(identifier: identifier, type: type)
            }

            if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
                return ImagePasteType(type: .image)
            }
            return nil
        }
    }
}

private struct ImagePasteType {
    let identifier: String
    let mimeType: String

    init(type: UTType) {
        self.init(identifier: type.identifier, type: type)
    }

    init(identifier: String, type: UTType) {
        self.identifier = identifier
        self.mimeType = type.preferredMIMEType ?? "image/png"
    }
}

final class PastableTextView: UITextView {
    var onImagePaste: ((Data, String) -> Void)?
    var onReturnKey: (() -> Void)?
    var maxAllowedHeight: CGFloat = 150

    override init(frame: CGRect, textContainer: NSTextContainer?) {
        super.init(frame: frame, textContainer: textContainer)
        configureAccessibility()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        configureAccessibility()
    }

    override var intrinsicContentSize: CGSize {
        let size = sizeThatFits(CGSize(width: bounds.width, height: .greatestFiniteMagnitude))
        return CGSize(width: UIView.noIntrinsicMetric, height: min(size.height, maxAllowedHeight))
    }

    private func configureAccessibility() {
        accessibilityLabel = "Message"
        accessibilityIdentifier = "ComposerMessageField"
    }

    func recalculateHeight() {
        let contentHeight = sizeThatFits(
            CGSize(width: bounds.width, height: .greatestFiniteMagnitude)
        ).height
        let shouldScroll = contentHeight > maxAllowedHeight
        if isScrollEnabled != shouldScroll {
            isScrollEnabled = shouldScroll
        }
        invalidateIntrinsicContentSize()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        recalculateHeight()
    }

    override func paste(_ sender: Any?) {
        let pasteboard = UIPasteboard.general

        let types: [(identifier: String, mimeType: String)] = [
            ("com.compuserve.gif", "image/gif"),
            (UTType.gif.identifier, "image/gif"),
            (UTType.png.identifier, "image/png"),
            (UTType.jpeg.identifier, "image/jpeg")
        ]
        for type in types {
            if let data = pasteboard.data(forPasteboardType: type.identifier) {
                onImagePaste?(data, type.mimeType)
                return
            }
        }

        if pasteboard.hasImages, let image = pasteboard.image, let pngData = image.pngData() {
            onImagePaste?(pngData, "image/png")
            return
        }

        super.paste(sender)
    }

    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        if action == #selector(paste(_:)) && UIPasteboard.general.hasImages {
            return true
        }
        return super.canPerformAction(action, withSender: sender)
    }

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        if let key = presses.first?.key,
           key.keyCode == .keyboardReturnOrEnter,
           !key.modifierFlags.contains(.shift)
        {
            onReturnKey?()
            return
        }
        super.pressesBegan(presses, with: event)
    }
}

private struct ComposerReplyPreview: View {
    let message: ChatMessage
    let onCancel: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Rectangle()
                .fill(Color.accentColor)
                .frame(width: 3, height: 36)
                .clipShape(Capsule())

            VStack(alignment: .leading, spacing: 2) {
                Text("Replying to \(senderLabel)")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Text(snippet)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer(minLength: 8)

            Button {
                onCancel()
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .font(.body)
                    .foregroundStyle(.tertiary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Cancel reply")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.thinMaterial)
    }

    private var senderLabel: String {
        if message.isMine {
            return "You"
        }
        let name = message.senderDisplayName.trimmingCharacters(in: .whitespacesAndNewlines)
        return name.isEmpty ? message.senderDeviceId : name
    }

    private var snippet: String {
        let text = message.displayContent.isEmpty ? message.text : message.displayContent
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            return trimmed.split(separator: "\n").first.map(String.init) ?? trimmed
        }
        if let media = message.media.first {
            return media.filename.isEmpty ? composerMediaLabel(for: media.kind) : media.filename
        }
        return "Message"
    }
}

private func chatMediaKind(for type: UTType?) -> ChatMediaKind {
    guard let type else { return .file }
    if type.conforms(to: .image) {
        return .image
    }
    if type.conforms(to: .movie) {
        return .video
    }
    if type.conforms(to: .audio) {
        return .voiceNote
    }
    return .file
}

private func defaultFilenameExtension(for type: UTType?) -> String {
    if let ext = type?.preferredFilenameExtension {
        return ext
    }
    switch chatMediaKind(for: type) {
    case .image:
        return "jpg"
    case .video:
        return "mov"
    case .voiceNote:
        return "m4a"
    case .file:
        return "bin"
    }
}

private func stagedAttachmentIcon(for kind: ChatMediaKind) -> String {
    switch kind {
    case .image:
        return "photo"
    case .voiceNote:
        return "waveform"
    case .video:
        return "play.rectangle"
    case .file:
        return "doc"
    }
}

private func composerMediaLabel(for kind: ChatMediaKind) -> String {
    switch kind {
    case .image:
        return "Image"
    case .voiceNote:
        return "Voice note"
    case .video:
        return "Video"
    case .file:
        return "File"
    }
}
