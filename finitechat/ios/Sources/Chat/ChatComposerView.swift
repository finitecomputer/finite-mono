import Foundation
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct Composer: View {
    @Binding var text: String
    let replyTarget: ChatMessage?
    let canSubmit: Bool
    var isSending = false
    @Binding var stagedAttachments: [StagedComposerAttachment]
    @Binding var isPhotoPickerPresented: Bool
    @Binding var selectedPhotoItems: [PhotosPickerItem]
    var isInputFocused: FocusState<Bool>.Binding
    let onCancelReply: () -> Void
    let onSend: () -> Void
    var placeholder = "Message"
    var allowsPhotoAttachments = true
    var messageFieldAccessibilityIdentifier = ComposerAccessibility.messageField
    var sendAccessibilityLabel = "Send"
    var sendAccessibilityIdentifier = "SendButton"
    var outerHorizontalPadding: CGFloat = 16
    var surfaceRadius: CGFloat = 28
    var onStartVoiceRecording: (() -> Void)?
    var onSelectPhotos: (() -> Void)?
    var onAttach: (() -> Void)?
    var onCreatePoll: (() -> Void)?

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
                TextField(
                    placeholder,
                    text: $text,
                    prompt: Text(placeholder),
                    axis: .vertical
                )
                .textFieldStyle(.plain)
                .lineLimit(1 ... 6)
                .focused(isInputFocused)
                .onChange(of: text) { _, _ in
                    FinitePerformance.recordComposerEdit()
                }
                .accessibilityIdentifier(messageFieldAccessibilityIdentifier)
                .frame(minHeight: 52)
                .padding(.horizontal, 16)
                .padding(.vertical, 8)

                HStack(spacing: 12) {
                    if showsAttachmentMenu {
                        attachmentMenu
                            .disabled(isSending)
                    }

                    Spacer()

                    if stagedAttachments.isEmpty, let onStartVoiceRecording {
                        Button {
                            onStartVoiceRecording()
                        } label: {
                            Image(systemName: "mic")
                                .font(.title3.weight(.regular))
                                .frame(width: 34, height: 34)
                                .contentShape(Circle())
                        }
                        .disabled(isSending)
                        .accessibilityLabel("Record voice message")
                        .accessibilityIdentifier("VoiceRecordButton")
                    }

                    if showsSendButton {
                        Button {
                            onSend()
                        } label: {
                            if isSending {
                                ProgressView()
                                    .tint(.white)
                                    .frame(width: 34, height: 34)
                                    .background(Circle().fill(Color.accentColor))
                            } else {
                                Image(systemName: "arrow.up")
                                    .font(.body.weight(.bold))
                                    .foregroundStyle(.white)
                                    .frame(width: 34, height: 34)
                                    .background(Circle().fill(Color.accentColor))
                            }
                        }
                        .disabled(sendDisabled)
                        .accessibilityLabel(sendAccessibilityLabel)
                        .accessibilityIdentifier(sendAccessibilityIdentifier)
                        .transition(.scale.combined(with: .opacity))
                    }
                }
                .foregroundStyle(.primary)
                .padding(.horizontal, 14)
                .padding(.bottom, 10)
            }
            .frame(maxWidth: .infinity, minHeight: 92, alignment: .topLeading)
            .modifier(ChatComposerSurface(radius: surfaceRadius))
        }
        .padding(.horizontal, outerHorizontalPadding)
        .padding(.top, 8)
        .safeAreaPadding(.bottom, 8)
        .background(Color.clear)
        .animation(.easeInOut(duration: 0.16), value: stagedAttachments.isEmpty)
        .animation(.snappy(duration: 0.18), value: showsSendButton)
    }

    @ViewBuilder
    private var attachmentMenu: some View {
        Menu {
            if allowsPhotoAttachments {
                Button {
                    if let onSelectPhotos {
                        onSelectPhotos()
                    } else {
                        isPhotoPickerPresented = true
                    }
                } label: {
                    Label("Photos & Videos", systemImage: "photo.on.rectangle")
                }
            }

            if let onAttach {
                Button(action: onAttach) {
                    Label("Files", systemImage: "doc")
                }
            }

            if let onCreatePoll {
                Button(action: onCreatePoll) {
                    Label("Poll", systemImage: "chart.bar.doc.horizontal")
                }
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
    }

    private var showsAttachmentMenu: Bool {
        allowsPhotoAttachments || onAttach != nil || onCreatePoll != nil
    }

    private var sendDisabled: Bool {
        isSending || (stagedAttachments.isEmpty && !canSubmit)
    }

    private var showsSendButton: Bool {
        !stagedAttachments.isEmpty || canSubmit
    }

    private var remainingPhotoSelectionCount: Int {
        max(1, maxStagedComposerAttachments - stagedAttachments.count)
    }
}

struct RoomComposer: View {
    let canCompose: Bool
    let replyTarget: ChatMessage?
    @Binding var stagedAttachments: [StagedComposerAttachment]
    @Binding var isPhotoPickerPresented: Bool
    @Binding var selectedPhotoItems: [PhotosPickerItem]
    var isInputFocused: FocusState<Bool>.Binding
    let onCancelReply: () -> Void
    let onSend: (String, @escaping (Bool) -> Void) -> Void
    let onTypingIntentChange: (Bool) -> Void
    var outerHorizontalPadding: CGFloat = 16
    var surfaceRadius: CGFloat = 28
    var onStartVoiceRecording: (() -> Void)?
    var onAttach: (() -> Void)?
    var onCreatePoll: (() -> Void)?
    @State private var text: String

    init(
        initialText: String,
        canCompose: Bool,
        replyTarget: ChatMessage?,
        stagedAttachments: Binding<[StagedComposerAttachment]>,
        isPhotoPickerPresented: Binding<Bool>,
        selectedPhotoItems: Binding<[PhotosPickerItem]>,
        isInputFocused: FocusState<Bool>.Binding,
        onCancelReply: @escaping () -> Void,
        onSend: @escaping (String, @escaping (Bool) -> Void) -> Void,
        onTypingIntentChange: @escaping (Bool) -> Void,
        outerHorizontalPadding: CGFloat = 16,
        surfaceRadius: CGFloat = 28,
        onStartVoiceRecording: (() -> Void)? = nil,
        onAttach: (() -> Void)? = nil,
        onCreatePoll: (() -> Void)? = nil
    ) {
        self.canCompose = canCompose
        self.replyTarget = replyTarget
        _stagedAttachments = stagedAttachments
        _isPhotoPickerPresented = isPhotoPickerPresented
        _selectedPhotoItems = selectedPhotoItems
        self.isInputFocused = isInputFocused
        self.onCancelReply = onCancelReply
        self.onSend = onSend
        self.onTypingIntentChange = onTypingIntentChange
        self.outerHorizontalPadding = outerHorizontalPadding
        self.surfaceRadius = surfaceRadius
        self.onStartVoiceRecording = onStartVoiceRecording
        self.onAttach = onAttach
        self.onCreatePoll = onCreatePoll
        _text = State(initialValue: initialText)
    }

    var body: some View {
        Composer(
            text: $text,
            replyTarget: replyTarget,
            canSubmit: canSubmit,
            stagedAttachments: $stagedAttachments,
            isPhotoPickerPresented: $isPhotoPickerPresented,
            selectedPhotoItems: $selectedPhotoItems,
            isInputFocused: isInputFocused,
            onCancelReply: onCancelReply,
            onSend: send,
            outerHorizontalPadding: outerHorizontalPadding,
            surfaceRadius: surfaceRadius,
            onStartVoiceRecording: onStartVoiceRecording,
            onAttach: onAttach,
            onCreatePoll: onCreatePoll
        )
        .task(id: hasMeaningfulText) {
            if hasMeaningfulText {
                try? await Task.sleep(for: .milliseconds(250))
                guard !Task.isCancelled else { return }
            }
            onTypingIntentChange(hasMeaningfulText)
        }
    }

    private var hasMeaningfulText: Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var canSubmit: Bool {
        canCompose && hasMeaningfulText
    }

    private func send() {
        onSend(text) { success in
            guard success else { return }
            text = ""
        }
    }
}

enum ComposerLaunchAction: Hashable {
    case photos
    case files
    case voice
}

struct NewChatComposer: View {
    var isInputFocused: FocusState<Bool>.Binding
    let placeholder: String
    let onStartChat: (String, ComposerLaunchAction?, @escaping (Bool) -> Void) -> Void
    var onDraftPresenceChange: (Bool) -> Void = { _ in }
    var outerHorizontalPadding: CGFloat = 16
    var surfaceRadius: CGFloat = 28
    @State private var stagedAttachments: [StagedComposerAttachment] = []
    @State private var isPhotoPickerPresented = false
    @State private var selectedPhotoItems: [PhotosPickerItem] = []
    @State private var isStartingChat = false
    @State private var text = ""

    var body: some View {
        Composer(
            text: $text,
            replyTarget: nil,
            canSubmit: canSubmit,
            isSending: isStartingChat,
            stagedAttachments: $stagedAttachments,
            isPhotoPickerPresented: $isPhotoPickerPresented,
            selectedPhotoItems: $selectedPhotoItems,
            isInputFocused: isInputFocused,
            onCancelReply: {},
            onSend: {
                beginChat(launchAction: nil)
            },
            placeholder: placeholder,
            messageFieldAccessibilityIdentifier: "HomeComposerField",
            sendAccessibilityLabel: "Start new chat",
            sendAccessibilityIdentifier: "HomeComposerSend",
            outerHorizontalPadding: outerHorizontalPadding,
            surfaceRadius: surfaceRadius,
            onStartVoiceRecording: {
                beginChat(launchAction: .voice)
            },
            onSelectPhotos: {
                beginChat(launchAction: .photos)
            },
            onAttach: {
                beginChat(launchAction: .files)
            }
        )
        .onChange(of: hasDraft) { _, hasDraft in
            onDraftPresenceChange(hasDraft)
        }
    }

    private var hasDraft: Bool {
        !text.isEmpty
    }

    private var canSubmit: Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !isStartingChat
    }

    private func beginChat(launchAction: ComposerLaunchAction?) {
        guard !isStartingChat else { return }
        let draft = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !draft.isEmpty || launchAction != nil else { return }

        isStartingChat = true
        onStartChat(draft, launchAction) { success in
            isStartingChat = false
            guard success else { return }
            text = ""
        }
    }
}

enum ComposerAccessibility {
    static let messageField = "ComposerMessageField"
}

private struct ChatComposerSurface: ViewModifier {
    let radius: CGFloat

    func body(content: Content) -> some View {
        if #available(iOS 26.0, *) {
            GlassEffectContainer(spacing: 0) {
                content
                    .glassEffect(
                        .regular.interactive(),
                        in: .rect(cornerRadius: radius)
                    )
            }
        } else {
            content
                .background(
                    .ultraThinMaterial,
                    in: RoundedRectangle(
                        cornerRadius: radius,
                        style: .continuous
                    )
                )
                .overlay {
                    RoundedRectangle(
                        cornerRadius: radius,
                        style: .continuous
                    )
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
