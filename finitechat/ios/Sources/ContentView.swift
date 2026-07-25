import CoreImage.CIFilterBuiltins
import Photos
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct RoomThreadView: View {
    @Environment(\.finiteTokens) private var tokens
    @ObservedObject var model: AppModel
    let roomID: String
    let topicID: String
    let chatID: String
    let composerLaunch: ComposerLaunch?
    let openDrawer: () -> Void
    @State private var followsBottom = true
    @State private var importingAttachment = false
    @State private var replyDraftMessage: ChatMessage?
    @State private var focusedMessage: ChatMessage?
    @State private var focusedMessageFrame: CGRect = .zero
    @State private var focusedActionsVisible = false
    @State private var reactionPickerContext: ReactionPickerContext?
    @FocusState private var composerFocused: Bool
    @State private var imagePreviewSelection: ChatImagePreviewSelection?
    @State private var videoPreviewItem: ChatAttachmentPreviewItem?
    @State private var documentPreviewItem: ChatAttachmentPreviewItem?
    @State private var selectedPhotoItems: [PhotosPickerItem] = []
    @State private var stagedAttachments: [StagedComposerAttachment] = []
    @State private var showPhotoPicker = false
    @State private var pollComposerDraft: PollComposerDraft?
    @State private var siteBrowserItem: FiniteSiteBrowserItem?
    @StateObject private var voiceRecorder = VoiceRecorder()
    @State private var voiceSendInFlight = false
    @State private var didConsumeComposerLaunch = false

    init(
        model: AppModel,
        roomID: String,
        topicID: String,
        chatID: String,
        composerLaunch: ComposerLaunch? = nil,
        openDrawer: @escaping () -> Void
    ) {
        self.model = model
        self.roomID = roomID
        self.topicID = topicID
        self.chatID = chatID
        self.composerLaunch = composerLaunch
        self.openDrawer = openDrawer
    }

    private var room: AppRoomSummary? {
        model.state?.rooms.first(where: { $0.roomId == roomID })
    }

    private var projection: ChatRoomProjection {
        model.projection(for: roomID)
    }

    private var selectedTopic: AppTopicSummary? {
        model.topics(for: roomID).first(where: { $0.topicId == topicID })
    }

    private var selectedChat: AppChatSummary? {
        selectedTopic?.chats.first(where: { $0.chatId == chatID })
    }

    private var chatNavigationTitle: String {
        if let title = selectedChat?.title.trimmingCharacters(in: .whitespacesAndNewlines),
           !title.isEmpty
        {
            return title
        }
        if let title = selectedTopic?.title.trimmingCharacters(in: .whitespacesAndNewlines),
           !title.isEmpty
        {
            return title
        }
        return room?.displayName ?? "Chat"
    }

    private var latestMessageID: String? {
        projection.messages.last?.messageId
    }

    private var transcriptRows: [ChatTimelineRow] {
        projection.rows
    }

    var body: some View {
        ZStack {
            VStack(spacing: 0) {
                if let room {
                    messageSurface(room: room)
                } else {
                    ContentUnavailableView("Room unavailable", systemImage: "exclamationmark.triangle")
                }
            }

            if let focusedMessage {
                FocusedMessageOverlay(
                    message: focusedMessage,
                    replyTarget: focusedReplyTarget(for: focusedMessage),
                    anchorFrame: focusedMessageFrame,
                    actionsVisible: focusedActionsVisible,
                    onDismiss: {
                        dismissFocusedMessage()
                    },
                    onReact: { emoji in
                        model.react(to: focusedMessage, emoji: emoji)
                        dismissFocusedMessage()
                    },
                    onMoreReaction: {
                        let message = focusedMessage
                        dismissFocusedMessage()
                        DispatchQueue.main.async {
                            reactionPickerContext = ReactionPickerContext(message: message)
                        }
                    },
                    onReply: {
                        replyDraftMessage = focusedMessage
                        composerFocused = true
                        dismissFocusedMessage()
                    },
                    onRetry: {
                        model.retry(focusedMessage)
                        dismissFocusedMessage()
                    },
                    onCopy: {
                        UIPasteboard.general.string = messageClipboardText(focusedMessage)
                        dismissFocusedMessage()
                    },
                    onSaveMedia: saveableImageAttachmentURLs(in: focusedMessage).isEmpty ? nil : {
                        saveImagesFromFocusedMessage(focusedMessage)
                        dismissFocusedMessage()
                    },
                    saveMediaTitle: saveMediaActionTitle(
                        imageCount: saveableImageAttachmentURLs(in: focusedMessage).count
                    ),
                    canReact: messageCanUseSentActions(focusedMessage),
                    canReply: messageCanUseSentActions(focusedMessage),
                    canRetry: messageCanRetry(focusedMessage),
                    canCopy: !messageClipboardText(focusedMessage).isEmpty
                )
                .transition(.opacity.combined(with: .scale(scale: 0.96)))
                .zIndex(10)
            }
        }
        .background(Color(.systemGroupedBackground).ignoresSafeArea())
        .navigationTitle(chatNavigationTitle)
        .navigationBarTitleDisplayMode(.inline)
        .navigationBarBackButtonHidden()
        .chatNavigationBarChrome()
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                ChatDrawerToolbarButton(action: openDrawer)
            }
        }
        .onAppear {
            openDestinationIfNeeded()
        }
        .onChange(of: latestMessageID) { _, _ in
            markRoomReadIfNeeded()
        }
        .fileImporter(
            isPresented: $importingAttachment,
            allowedContentTypes: [.item],
            allowsMultipleSelection: true
        ) { result in
            handleImportedAttachment(result)
        }
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
        .fullScreenCover(item: $documentPreviewItem) { item in
            ChatDocumentPreviewView(item: item) {
                documentPreviewItem = nil
            }
        }
        .sheet(item: $pollComposerDraft) { draft in
            PollComposerView { question, options in
                model.sendPoll(roomID: draft.roomID, question: question, options: options)
            }
        }
        .sheet(item: $reactionPickerContext) { context in
            ReactionEmojiPickerSheet { emoji in
                model.react(to: context.message, emoji: emoji)
            }
            .presentationDetents([.medium, .large])
        }
        .sheet(item: $siteBrowserItem) { item in
            FiniteSiteBrowserView(url: item.url, identity: model.nostrIdentity)
        }
        .onDisappear {
            model.setTyping(roomID: roomID, isTyping: false)
            dismissFocusedMessage(animated: false)
            voiceRecorder.cancelRecording()
        }
        .onChange(of: selectedPhotoItems) { _, items in
            stagePhotoItems(items)
        }
        .task(id: composerLaunch?.id) {
            await consumeComposerLaunch()
        }
    }

    @ViewBuilder
    private func messageSurface(room: AppRoomSummary) -> some View {
        switch room.state {
        case .connected:
            transcriptView(room: room)
        case .waitingForApproval:
            PendingRoomView(room: room, model: model)
        case .joining:
            ProgressView(room.userStatusText)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .unavailableOnDevice:
            UnavailableOnDeviceView(room: room)
        }
    }

    private func transcriptView(room: AppRoomSummary) -> some View {
        ChatTranscriptView(
            roomID: room.roomId,
            rows: transcriptRows,
            messagesById: projection.messagesById,
            onReact: { message, emoji in
                model.react(to: message, emoji: emoji)
            },
            onDownloadAttachment: { message, attachment in
                model.downloadAttachment(roomID: room.roomId, message: message, attachment: attachment)
            },
            onOpenAttachment: { message, attachment in
                handleAttachmentOpen(message: message, attachment: attachment)
            },
            onVotePoll: { message, option in
                model.votePoll(message: message, option: option)
            },
            onRetryMessage: { message in
                model.retry(message)
            },
            onLongPressMessage: { message, frame in
                presentFocusedMessage(message, frame: frame)
            },
            onOpenURL: { url in
                handleOpenURL(url)
            },
            canLoadOlder: room.canLoadOlder,
            onLoadOlderMessages: { beforeMessageID in
                model.loadOlderMessages(roomID: room.roomId, beforeMessageID: beforeMessageID)
            },
            followsBottom: $followsBottom
        )
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(.systemGroupedBackground))
        .ignoresSafeArea(edges: .top)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            composerAccessory
        }
        .accessibilityLabel("Messages")
    }

    private func handleOpenURL(_ url: URL) -> OpenURLAction.Result {
        guard let scheme = url.scheme?.lowercased(),
              scheme == "http" || scheme == "https"
        else {
            return .systemAction
        }
        siteBrowserItem = FiniteSiteBrowserItem(url: url)
        return .handled
    }

    private func messageCanRetry(_ message: ChatMessage) -> Bool {
        guard message.isMine, let outboundDelivery = message.outboundDelivery else { return false }
        if case .failed = outboundDelivery.serverDelivery {
            return true
        }
        return false
    }

    private func messageCanUseSentActions(_ message: ChatMessage) -> Bool {
        guard let outboundDelivery = message.outboundDelivery else { return true }
        if case .delivered = outboundDelivery.serverDelivery {
            return true
        }
        return false
    }

    private func handleImportedAttachment(_ result: Result<[URL], Error>) {
        switch result {
        case .success(let urls):
            stageFileURLs(urls)
        case .failure(let error):
            model.errorText = String(describing: error)
        }
    }

    @ViewBuilder
    private var composerAccessory: some View {
        if let recording = voiceRecorder.state {
            VoiceRecordingComposerView(
                recording: recording,
                isSending: voiceSendInFlight,
                onSend: {
                    sendVoiceRecording()
                },
                onCancel: {
                    cancelVoiceRecording()
                },
                onTogglePause: {
                    toggleVoiceRecordingPause()
                }
            )
            .transition(.move(edge: .bottom).combined(with: .opacity))
        } else {
            RoomComposer(
                initialText: composerLaunch?.draft ?? "",
                canCompose: room?.state == .connected,
                replyTarget: replyDraftMessage,
                stagedAttachments: $stagedAttachments,
                isPhotoPickerPresented: $showPhotoPicker,
                selectedPhotoItems: $selectedPhotoItems,
                isInputFocused: $composerFocused,
                onCancelReply: {
                    replyDraftMessage = nil
                },
                onSend: { text, completion in
                    sendComposerDraft(text: text, completion: completion)
                },
                onTypingIntentChange: { isTyping in
                    updateTypingIntent(isTyping)
                },
                outerHorizontalPadding: tokens.composerHorizontalPadding,
                surfaceRadius: tokens.composerRadius,
                onStartVoiceRecording: {
                    startVoiceRecording()
                }
            ) {
                importingAttachment = true
            } onCreatePoll: {
                pollComposerDraft = PollComposerDraft(roomID: roomID)
            }
        }
    }

    private func handleAttachmentOpen(message: ChatMessage, attachment: ChatMediaAttachment) {
        guard let localURL = attachmentLocalURL(attachment) else {
            if attachmentCanDownload(attachment) {
                model.downloadAttachment(roomID: roomID, message: message, attachment: attachment)
            }
            return
        }

        switch attachment.kind {
        case .image:
            let imageAttachments = message.media.filter { media in
                media.kind == .image && attachmentLocalURL(media) != nil
            }
            imagePreviewSelection = ChatImagePreviewSelection(
                attachments: imageAttachments,
                selected: attachment
            )
        case .video:
            videoPreviewItem = ChatAttachmentPreviewItem(attachment: attachment, url: localURL)
        case .voiceNote, .file:
            documentPreviewItem = ChatAttachmentPreviewItem(attachment: attachment, url: localURL)
        }
    }

    private func saveImagesFromFocusedMessage(_ message: ChatMessage) {
        let urls = saveableImageAttachmentURLs(in: message)
        guard !urls.isEmpty else {
            model.errorText = "No downloaded photos to save."
            return
        }

        Task {
            do {
                _ = try await PhotoLibraryImageSaver.saveImageFiles(urls)
                model.errorText = nil
            } catch {
                model.errorText = String(describing: error)
            }
        }
    }

    private func presentFocusedMessage(_ message: ChatMessage, frame: CGRect) {
        composerFocused = false
        focusedMessageFrame = frame
        withAnimation(.spring(response: 0.28, dampingFraction: 0.78)) {
            focusedMessage = message
            focusedActionsVisible = true
        }
    }

    private func dismissFocusedMessage(animated: Bool = true) {
        let updates = {
            focusedMessage = nil
            focusedActionsVisible = false
        }
        if animated {
            withAnimation(.easeOut(duration: 0.16), updates)
        } else {
            updates()
        }
    }

    private func focusedReplyTarget(for message: ChatMessage) -> ChatMessage? {
        guard let replyToMessageId = message.replyToMessageId else { return nil }
        return projection.messagesById[replyToMessageId]
    }

    private func openDestinationIfNeeded() {
        guard model.state?.selectedRoomId != roomID
                || model.state?.selectedTopicId != topicID
                || model.state?.selectedChatId != chatID
        else {
            return
        }
        model.openChat(roomID: roomID, topicID: topicID, chatID: chatID)
    }

    private func updateTypingIntent(_ isTyping: Bool) {
        guard room?.state == .connected else { return }
        model.setTyping(roomID: roomID, isTyping: isTyping)
    }

    private func markRoomReadIfNeeded() {
        guard let room, room.unreadCount > 0 else { return }
        model.markRoomRead(room)
    }

    private func consumeComposerLaunch() async {
        guard !didConsumeComposerLaunch, let composerLaunch else { return }
        didConsumeComposerLaunch = true
        await Task.yield()
        switch composerLaunch.action {
        case .photos:
            showPhotoPicker = true
        case .files:
            importingAttachment = true
        case .voice:
            startVoiceRecording()
        }
    }

    private func sendComposerDraft(
        text: String,
        completion: @escaping (Bool) -> Void
    ) {
        if stagedAttachments.isEmpty {
            if model.send(roomID: roomID, text: text, replyTo: replyDraftMessage) {
                model.setTyping(roomID: roomID, isTyping: false)
                replyDraftMessage = nil
                completion(true)
            } else {
                completion(false)
            }
            return
        }

        let outbound = stagedAttachments.map(\.outboundAttachment)
        model.sendAttachments(
            roomID: roomID,
            attachments: outbound,
            replyTo: replyDraftMessage,
            captionOverride: text
        ) {
            model.setTyping(roomID: roomID, isTyping: false)
            stagedAttachments = []
            selectedPhotoItems = []
            replyDraftMessage = nil
            completion(true)
        }
    }

    private func startVoiceRecording() {
        guard voiceRecorder.state == nil else { return }
        composerFocused = false
        Task {
            do {
                try await voiceRecorder.startRecording()
            } catch {
                model.errorText = String(describing: error)
            }
        }
    }

    private func sendVoiceRecording() {
        guard voiceRecorder.state != nil, !voiceSendInFlight else { return }
        let caption = voiceRecordingCaption(voiceRecorder.state)
        voiceSendInFlight = true
        Task {
            do {
                let url = try await voiceRecorder.stopRecording()
                defer {
                    try? FileManager.default.removeItem(at: url)
                    voiceSendInFlight = false
                }
                let data = try await Task.detached(priority: .userInitiated) {
                    try Data(contentsOf: url)
                }.value
                let attachment = try VoiceRecordingAttachment.outboundAttachment(data: data)
                model.sendAttachments(
                    roomID: roomID,
                    attachments: [attachment],
                    replyTo: replyDraftMessage,
                    captionOverride: caption
                ) {
                    replyDraftMessage = nil
                }
            } catch {
                voiceRecorder.cancelRecording()
                voiceSendInFlight = false
                model.errorText = String(describing: error)
            }
        }
    }

    private func cancelVoiceRecording() {
        voiceRecorder.cancelRecording()
        voiceSendInFlight = false
    }

    private func toggleVoiceRecordingPause() {
        guard let recording = voiceRecorder.state else { return }
        do {
            switch recording.phase {
            case .recording:
                voiceRecorder.pauseRecording()
            case .paused:
                try voiceRecorder.resumeRecording()
            }
        } catch {
            model.errorText = String(describing: error)
        }
    }

    private func stageFileURLs(_ urls: [URL]) {
        guard !urls.isEmpty else { return }
        Task {
            do {
                let staged = try await Task.detached(priority: .userInitiated) {
                    try urls.map { try StagedComposerAttachment(fileURL: $0) }
                }.value
                appendStagedAttachments(staged)
            } catch {
                model.errorText = String(describing: error)
            }
        }
    }

    private func stagePhotoItems(_ items: [PhotosPickerItem]) {
        guard !items.isEmpty else { return }
        Task {
            do {
                var staged: [StagedComposerAttachment] = []
                staged.reserveCapacity(items.count)
                for item in items {
                    if let attachment = try await StagedComposerAttachment(photoItem: item) {
                        staged.append(attachment)
                    }
                }
                appendStagedAttachments(staged)
            } catch {
                model.errorText = String(describing: error)
            }
            selectedPhotoItems = []
        }
    }

    private func appendStagedAttachments(_ attachments: [StagedComposerAttachment]) {
        guard !attachments.isEmpty else { return }
        let remainingSlots = max(0, maxStagedComposerAttachments - stagedAttachments.count)
        guard remainingSlots > 0 else {
            model.errorText = "Attachment limit is \(maxStagedComposerAttachments) files."
            return
        }
        let accepted = Array(attachments.prefix(remainingSlots))
        stagedAttachments.append(contentsOf: accepted)
        if accepted.count < attachments.count {
            model.errorText = "Attachment limit is \(maxStagedComposerAttachments) files."
        }
    }
}

private struct FocusedMessageOverlay: View {
    let message: ChatMessage
    let replyTarget: ChatMessage?
    let anchorFrame: CGRect
    let actionsVisible: Bool
    let onDismiss: () -> Void
    let onReact: (String) -> Void
    let onMoreReaction: () -> Void
    let onReply: () -> Void
    let onRetry: () -> Void
    let onCopy: () -> Void
    let onSaveMedia: (() -> Void)?
    let saveMediaTitle: String?
    let canReact: Bool
    let canReply: Bool
    let canRetry: Bool
    let canCopy: Bool

    var body: some View {
        GeometryReader { geometry in
            let top = overlayTop(in: geometry)
            let availableHeight = max(1, geometry.size.height - top - 12)

            ZStack {
                Color.black.opacity(0.18)
                    .ignoresSafeArea()
                    .contentShape(Rectangle())
                    .onTapGesture(perform: onDismiss)

                focusedContent(in: geometry)
                .frame(
                    maxWidth: .infinity,
                    maxHeight: availableHeight,
                    alignment: message.isMine ? .topTrailing : .topLeading
                )
                .padding(.top, top)
                .padding(.horizontal, 20)
                .animation(.easeOut(duration: 0.16), value: actionsVisible)
            }
        }
    }

    private func focusedContent(in geometry: GeometryProxy) -> some View {
        ViewThatFits(in: .vertical) {
            focusedStack {
                focusedMessageCard(in: geometry)
            }
            .fixedSize(horizontal: false, vertical: true)

            focusedStack {
                ScrollView(.vertical) {
                    focusedMessageCard(in: geometry)
                        .frame(
                            maxWidth: .infinity,
                            alignment: message.isMine ? .trailing : .leading
                        )
                }
                .scrollBounceBehavior(.basedOnSize)
                .accessibilityIdentifier("FocusedMessageScroller")
            }
        }
    }

    private func focusedStack<MessageContent: View>(
        @ViewBuilder messageContent: () -> MessageContent
    ) -> some View {
        VStack(alignment: message.isMine ? .trailing : .leading, spacing: 10) {
            if canReact {
                FocusedReactionBar(onReact: onReact, onMore: onMoreReaction)
            }

            messageContent()

            if actionsVisible {
                FocusedMessageActionCard(
                    canReply: canReply,
                    canRetry: canRetry,
                    canCopy: canCopy,
                    onReply: onReply,
                    onRetry: onRetry,
                    onCopy: onCopy,
                    onSaveMedia: onSaveMedia,
                    saveMediaTitle: saveMediaTitle
                )
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
    }

    private func focusedMessageCard(in geometry: GeometryProxy) -> some View {
        FocusedChatMessageCard(
            message: message,
            replyTarget: replyTarget
        )
        .frame(maxWidth: min(geometry.size.width * 0.82, 360))
    }

    private func overlayTop(in geometry: GeometryProxy) -> CGFloat {
        let overlayOriginY = geometry.frame(in: .global).minY
        let localAnchorY = anchorFrame.minY - overlayOriginY
        let reactionBarSpace: CGFloat = canReact ? 58 : 0
        let idealTop = localAnchorY - reactionBarSpace
        let maxTop = max(
            12,
            min(
                geometry.size.height * 0.58,
                geometry.size.height - minimumVisibleContentHeight
            )
        )
        return min(max(idealTop, 12), maxTop)
    }

    private var minimumVisibleContentHeight: CGFloat {
        let reactionHeight: CGFloat = canReact ? 58 : 0
        let actionRows = 2 + (canRetry ? 1 : 0) + (onSaveMedia == nil ? 0 : 1)
        let actionsHeight = actionsVisible ? CGFloat(actionRows * 42) + 10 : 0
        return reactionHeight + actionsHeight + 108
    }
}

private struct FocusedReactionBar: View {
    let onReact: (String) -> Void
    let onMore: () -> Void

    var body: some View {
        HStack(spacing: 4) {
            ForEach(focusedReactionEmojis, id: \.self) { emoji in
                Button {
                    UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    onReact(emoji)
                } label: {
                    Text(emoji)
                        .font(.system(size: 24))
                        .frame(width: 42, height: 42)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("React \(emoji)")
                .accessibilityIdentifier("ReactionQuickButton-\(reactionEmojiStableID(emoji))")
            }

            Button {
                UIImpactFeedbackGenerator(style: .light).impactOccurred()
                onMore()
            } label: {
                Image(systemName: "plus")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 32, height: 32)
                    .background(Color(uiColor: .tertiarySystemGroupedBackground), in: Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("More reactions")
            .accessibilityIdentifier("ReactionMoreButton")
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 4)
        .background(.regularMaterial, in: Capsule())
        .shadow(color: .black.opacity(0.14), radius: 14, x: 0, y: 6)
    }
}

private struct FocusedMessageActionCard: View {
    let canReply: Bool
    let canRetry: Bool
    let canCopy: Bool
    let onReply: () -> Void
    let onRetry: () -> Void
    let onCopy: () -> Void
    let onSaveMedia: (() -> Void)?
    let saveMediaTitle: String?

    var body: some View {
        VStack(spacing: 0) {
            if canRetry {
                Button {
                    UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    onRetry()
                } label: {
                    Label("Retry", systemImage: "arrow.clockwise")
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 10)
                }
                .buttonStyle(.plain)

                Divider()
            }

            Button {
                UIImpactFeedbackGenerator(style: .light).impactOccurred()
                onReply()
            } label: {
                Label("Reply", systemImage: "arrowshape.turn.up.left")
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
            }
            .buttonStyle(.plain)
            .disabled(!canReply)

            Divider()

            Button {
                onCopy()
            } label: {
                Label("Copy", systemImage: "doc.on.doc")
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
            }
            .buttonStyle(.plain)
            .disabled(!canCopy)

            if let onSaveMedia, let saveMediaTitle {
                Divider()

                Button {
                    UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    onSaveMedia()
                } label: {
                    Label(saveMediaTitle, systemImage: "square.and.arrow.down")
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 10)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(saveMediaTitle)
            }
        }
        .frame(width: 176)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        .shadow(color: .black.opacity(0.14), radius: 14, x: 0, y: 6)
    }
}

private let focusedReactionEmojis = ["❤️", "👍", "👎", "😂", "😮", "😢"]

private struct ReactionPickerContext: Identifiable {
    let message: ChatMessage

    var id: String {
        message.messageId
    }
}

struct ReactionEmojiSection: Equatable, Identifiable {
    let title: String
    let emojis: [ReactionEmojiChoice]

    var id: String {
        title
    }
}

struct ReactionEmojiChoice: Equatable, Identifiable {
    let emoji: String
    let name: String
    let keywords: [String]

    var id: String {
        emoji
    }

    func matches(_ query: String) -> Bool {
        let normalized = query
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        guard !normalized.isEmpty else { return true }
        if name.lowercased().contains(normalized) {
            return true
        }
        return keywords.contains { keyword in
            keyword.lowercased().contains(normalized)
        }
    }
}

enum ReactionEmojiCatalog {
    static let recent = [
        choice("❤️", "Red heart", "love", "heart"),
        choice("👍", "Thumbs up", "yes", "agree", "like"),
        choice("👎", "Thumbs down", "no", "disagree"),
        choice("😂", "Face with tears of joy", "laugh", "funny"),
        choice("😮", "Surprised face", "wow", "shock"),
        choice("😢", "Crying face", "sad"),
        choice("🔥", "Fire", "hot", "lit"),
        choice("🎉", "Party popper", "celebrate", "party"),
        choice("👀", "Eyes", "looking", "watching"),
        choice("🙏", "Folded hands", "thanks", "please"),
        choice("💯", "Hundred points", "perfect", "agree"),
        choice("🤔", "Thinking face", "think", "hmm"),
    ]

    static let sections = [
        ReactionEmojiSection(title: "Recent", emojis: recent),
        ReactionEmojiSection(title: "Smileys", emojis: [
            choice("😀", "Grinning face", "smile"),
            choice("😃", "Smiling face", "happy"),
            choice("😄", "Smiling eyes", "happy"),
            choice("😁", "Beaming face", "grin"),
            choice("😆", "Squinting face", "laugh"),
            choice("😅", "Grinning sweat", "relief"),
            choice("🤣", "Rolling on the floor laughing", "laugh", "funny"),
            choice("😂", "Face with tears of joy", "laugh", "funny"),
            choice("🙂", "Slightly smiling face", "smile"),
            choice("🙃", "Upside-down face", "silly"),
            choice("😉", "Winking face", "wink"),
            choice("😊", "Smiling face with smiling eyes", "warm"),
            choice("😇", "Smiling face with halo", "angel"),
            choice("😍", "Heart eyes", "love"),
            choice("😘", "Face blowing a kiss", "kiss"),
            choice("😋", "Yum face", "tasty"),
            choice("😜", "Winking tongue", "joke"),
            choice("🤔", "Thinking face", "think", "hmm"),
            choice("🤨", "Raised eyebrow", "skeptical"),
            choice("😐", "Neutral face", "neutral"),
            choice("😑", "Expressionless face", "blank"),
            choice("😶", "Face without mouth", "quiet"),
            choice("😏", "Smirking face", "smirk"),
            choice("😒", "Unamused face", "unimpressed"),
            choice("🙄", "Face with rolling eyes", "eyeroll"),
            choice("😬", "Grimacing face", "grimace"),
            choice("😮", "Surprised face", "wow", "shock"),
            choice("😯", "Hushed face", "surprised"),
            choice("😲", "Astonished face", "amazed"),
            choice("😴", "Sleeping face", "sleep"),
            choice("🤤", "Drooling face", "want"),
            choice("😪", "Sleepy face", "tired"),
            choice("😵", "Dizzy face", "dizzy"),
            choice("🤯", "Exploding head", "mind blown"),
            choice("🥳", "Partying face", "party", "celebrate"),
            choice("🥺", "Pleading face", "please"),
            choice("😭", "Loudly crying face", "cry"),
            choice("😤", "Face with steam", "frustrated"),
            choice("😡", "Pouting face", "angry"),
        ]),
        ReactionEmojiSection(title: "Gestures", emojis: [
            choice("👋", "Waving hand", "hello", "bye"),
            choice("👌", "OK hand", "ok"),
            choice("✌️", "Victory hand", "peace"),
            choice("🤞", "Crossed fingers", "hope"),
            choice("🤟", "Love-you gesture", "love"),
            choice("🤘", "Sign of the horns", "rock"),
            choice("👍", "Thumbs up", "yes", "agree", "like"),
            choice("👎", "Thumbs down", "no", "disagree"),
            choice("👏", "Clapping hands", "applause"),
            choice("🙌", "Raising hands", "celebrate"),
            choice("🙏", "Folded hands", "thanks", "please"),
            choice("🤝", "Handshake", "deal", "agree"),
            choice("💪", "Flexed biceps", "strong"),
            choice("🫡", "Saluting face", "salute"),
        ]),
        ReactionEmojiSection(title: "Hearts", emojis: [
            choice("❤️", "Red heart", "love", "heart"),
            choice("🧡", "Orange heart", "heart"),
            choice("💛", "Yellow heart", "heart"),
            choice("💚", "Green heart", "heart"),
            choice("💙", "Blue heart", "heart"),
            choice("💜", "Purple heart", "heart"),
            choice("🖤", "Black heart", "heart"),
            choice("🤍", "White heart", "heart"),
            choice("💔", "Broken heart", "heartbreak"),
            choice("💕", "Two hearts", "love"),
            choice("💖", "Sparkling heart", "love"),
            choice("💝", "Heart with ribbon", "gift"),
        ]),
        ReactionEmojiSection(title: "Symbols", emojis: [
            choice("⭐️", "Star", "favorite"),
            choice("✨", "Sparkles", "sparkle"),
            choice("🔥", "Fire", "hot", "lit"),
            choice("💯", "Hundred points", "perfect", "agree"),
            choice("🎉", "Party popper", "celebrate", "party"),
            choice("✅", "Check mark", "done", "yes"),
            choice("❌", "Cross mark", "no", "cancel"),
            choice("⚠️", "Warning", "caution"),
            choice("🚀", "Rocket", "ship", "launch"),
            choice("💡", "Light bulb", "idea"),
            choice("👑", "Crown", "king", "queen"),
        ]),
    ]

    static func filteredSections(searchText: String) -> [ReactionEmojiSection] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return sections }

        var seen = Set<String>()
        let matches = sections
            .flatMap(\.emojis)
            .filter { choice in
                guard choice.matches(query), !seen.contains(choice.emoji) else { return false }
                seen.insert(choice.emoji)
                return true
            }
        return matches.isEmpty ? [] : [ReactionEmojiSection(title: "Results", emojis: matches)]
    }

    private static func choice(
        _ emoji: String,
        _ name: String,
        _ keywords: String...
    ) -> ReactionEmojiChoice {
        ReactionEmojiChoice(emoji: emoji, name: name, keywords: keywords)
    }
}

private struct ReactionEmojiPickerSheet: View {
    @Environment(\.dismiss) private var dismiss
    @State private var searchText = ""
    let onSelect: (String) -> Void

    private var sections: [ReactionEmojiSection] {
        ReactionEmojiCatalog.filteredSections(searchText: searchText)
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 18) {
                    if sections.isEmpty {
                        ContentUnavailableView("No matching emoji", systemImage: "magnifyingglass")
                            .frame(maxWidth: .infinity)
                            .padding(.top, 44)
                    } else {
                        ForEach(sections) { section in
                            ReactionEmojiSectionView(section: section) { emoji in
                                UIImpactFeedbackGenerator(style: .light).impactOccurred()
                                onSelect(emoji)
                                dismiss()
                            }
                        }
                    }
                }
                .padding(.horizontal, 18)
                .padding(.vertical, 16)
            }
            .navigationTitle("Reactions")
            .navigationBarTitleDisplayMode(.inline)
            .searchable(text: $searchText, prompt: "Search emoji")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    GlassCircleCloseButton { dismiss() }
                }
            }
        }
    }
}

private struct ReactionEmojiSectionView: View {
    let section: ReactionEmojiSection
    let onSelect: (String) -> Void

    private let columns = Array(
        repeating: GridItem(.flexible(minimum: 40, maximum: 52), spacing: 8),
        count: 6
    )

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(section.title)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.secondary)

            LazyVGrid(columns: columns, spacing: 8) {
                ForEach(section.emojis) { choice in
                    Button {
                        onSelect(choice.emoji)
                    } label: {
                        Text(choice.emoji)
                            .font(.system(size: 30))
                            .frame(width: 44, height: 44)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(choice.name)
                    .accessibilityIdentifier("ReactionEmojiButton-\(reactionEmojiStableID(choice.emoji))")
                }
            }
        }
    }
}

private func reactionEmojiStableID(_ emoji: String) -> String {
    let scalars = emoji.unicodeScalars
        .map { String($0.value, radix: 16, uppercase: true) }
        .joined(separator: "-")
    return scalars.isEmpty ? "empty" : scalars
}

private func messageClipboardText(_ message: ChatMessage) -> String {
    let display = message.displayContent.trimmingCharacters(in: .whitespacesAndNewlines)
    if !display.isEmpty {
        return display
    }
    return message.text.trimmingCharacters(in: .whitespacesAndNewlines)
}

func saveableImageAttachmentURLs(in message: ChatMessage) -> [URL] {
    message.media
        .filter { $0.kind == .image }
        .compactMap(attachmentLocalURL)
}

func saveMediaActionTitle(imageCount: Int) -> String? {
    guard imageCount > 0 else { return nil }
    return imageCount == 1 ? "Save Photo" : "Save Photos"
}

enum PhotoLibraryImageSaveError: Error, CustomStringConvertible {
    case noImages
    case notAuthorized(PHAuthorizationStatus)
    case saveFailed

    var description: String {
        switch self {
        case .noImages:
            "No downloaded photos to save."
        case .notAuthorized:
            "Photo library access was not granted."
        case .saveFailed:
            "Photo library save did not complete."
        }
    }
}

enum PhotoLibraryImageSaver {
    static func saveImageFiles(_ urls: [URL]) async throws -> Int {
        let existingURLs = urls.filter { FileManager.default.fileExists(atPath: $0.path) }
        guard !existingURLs.isEmpty else {
            throw PhotoLibraryImageSaveError.noImages
        }

        let status = await requestAddOnlyAuthorization()
        guard status == .authorized || status == .limited else {
            throw PhotoLibraryImageSaveError.notAuthorized(status)
        }

        try await withCheckedThrowingContinuation {
            (continuation: CheckedContinuation<Void, Error>) in
            PHPhotoLibrary.shared().performChanges {
                for url in existingURLs {
                    PHAssetChangeRequest.creationRequestForAssetFromImage(atFileURL: url)
                }
            } completionHandler: { success, error in
                if let error {
                    continuation.resume(throwing: error)
                } else if success {
                    continuation.resume()
                } else {
                    continuation.resume(throwing: PhotoLibraryImageSaveError.saveFailed)
                }
            }
        }

        return existingURLs.count
    }

    private static func requestAddOnlyAuthorization() async -> PHAuthorizationStatus {
        let current = PHPhotoLibrary.authorizationStatus(for: .addOnly)
        guard current == .notDetermined else { return current }
        return await withCheckedContinuation { continuation in
            PHPhotoLibrary.requestAuthorization(for: .addOnly) { status in
                continuation.resume(returning: status)
            }
        }
    }
}

struct PendingRoomPresentation {
    let room: AppRoomSummary

    var detailText: String? {
        let status = room.status.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !status.isEmpty else { return nil }
        guard status != room.userStatusText else { return nil }
        guard status != room.userStatusText.lowercased() else { return nil }
        guard !Self.isLowLevelAdmissionStatus(status) else { return nil }
        return status
    }

    private static func isLowLevelAdmissionStatus(_ status: String) -> Bool {
        status.localizedCaseInsensitiveContains("accepted Welcome")
            || status.localizedCaseInsensitiveContains("client error:")
    }
}

private struct PendingRoomView: View {
    let room: AppRoomSummary
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "lock.open")
                .font(.largeTitle)
                .foregroundStyle(.secondary)
            Text(room.userStatusText)
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            if let detailText {
                Text(detailText)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            if let notice = model.userNoticeText {
                Label(notice, systemImage: isSubmitting ? "hourglass" : "info.circle")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }

            ProgressView()
                .controlSize(.large)
                .accessibilityLabel(isSubmitting ? "Waiting for Welcome" : room.userStatusText)
        }
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var isSubmitting: Bool {
        room.state == .joining || room.state == .waitingForApproval
    }

    private var detailText: String? {
        PendingRoomPresentation(room: room).detailText
    }
}

private struct UnavailableOnDeviceView: View {
    let room: AppRoomSummary

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "exclamationmark.triangle")
                .font(.largeTitle)
                .foregroundStyle(.secondary)
            Text(room.userStatusText)
                .font(.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct NoticeBarPresentation: Equatable {
    let text: String?

    var visibleText: String? {
        guard let text = text?.trimmingCharacters(in: .whitespacesAndNewlines), !text.isEmpty else {
            return nil
        }
        return text
    }

    var accessibilityIdentifier: String {
        "NoticeBar"
    }
}

struct NoticeBar: View {
    let presentation: NoticeBarPresentation

    init(text: String?) {
        presentation = NoticeBarPresentation(text: text)
    }

    var body: some View {
        if let text = presentation.visibleText {
            Text(text)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal)
                .padding(.vertical, 8)
                .background(.bar)
                .accessibilityIdentifier(presentation.accessibilityIdentifier)
        }
    }
}

private extension AppRoomState {
    var tint: Color {
        switch self {
        case .connected:
            .green
        case .waitingForApproval, .joining:
            .orange
        case .unavailableOnDevice:
            .red
        }
    }
}

#Preview("Focused message — long") {
    ZStack {
        Color(.systemGroupedBackground)
            .ignoresSafeArea()

        FocusedMessageOverlay(
            message: ChatMessage(
                roomId: "preview-room",
                seq: 1,
                messageId: "preview-long-message",
                conversationId: nil,
                chatId: "preview-chat",
                senderAccountId: "agent",
                senderDeviceId: "agent-device",
                senderDisplayName: "Finite",
                senderNpub: nil,
                text: focusedMessagePreviewText,
                displayContent: focusedMessagePreviewText,
                richTextJson: "",
                kind: .message,
                status: .complete,
                finalDelivery: true,
                editOfMessageId: nil,
                payload: Data(focusedMessagePreviewText.utf8),
                replyToMessageId: nil,
                isMine: false,
                outboundDelivery: nil,
                reactions: [],
                media: [],
                readReceipt: nil,
                poll: nil,
                timestampUnixSeconds: 1_700_000_000,
                displayTimestamp: "9:41 AM"
            ),
            replyTarget: nil,
            anchorFrame: CGRect(x: 48, y: 260, width: 280, height: 520),
            actionsVisible: true,
            onDismiss: {},
            onReact: { _ in },
            onMoreReaction: {},
            onReply: {},
            onRetry: {},
            onCopy: {},
            onSaveMedia: nil,
            saveMediaTitle: nil,
            canReact: true,
            canReply: true,
            canRetry: false,
            canCopy: true
        )
    }
}

private let focusedMessagePreviewText = """
An everything bagel is topped with a mix of classic savory seasonings:

• Sesame seeds
• Poppy seeds
• Dried garlic
• Dried onion
• Coarse salt

The preview intentionally continues long enough to exercise the bounded focused-message layout. \
Reactions and actions should remain visible while this message body scrolls independently.
"""
