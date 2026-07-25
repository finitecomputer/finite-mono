import SwiftUI

struct ChatDrawerToolbarButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: "line.3.horizontal")
        }
        .accessibilityLabel("Chats")
        .accessibilityIdentifier("ChatPickerButton")
    }
}

struct ChatDestination: Hashable, Identifiable {
    let roomID: String
    let topicID: String
    let chatID: String
    let title: String
    let preview: String
    let updatedSequence: UInt64
    var composerLaunch: ComposerLaunch? = nil

    var id: String {
        "\(roomID)|\(topicID)|\(chatID)"
    }
}

func findReusableEmptyHomeChatDestination(
    roomID: String,
    topics: [AppTopicSummary]
) -> ChatDestination? {
    findReusableEmptyChatDestination(
        roomID: roomID,
        topicID: "home",
        topics: topics
    )
}

func findReusableEmptyChatDestination(
    roomID: String,
    topicID: String,
    topics: [AppTopicSummary]
) -> ChatDestination? {
    guard let topic = topics.first(where: {
        $0.roomId == roomID && $0.topicId == topicID && !$0.archived
    }) else {
        return nil
    }
    guard let chat = topic.chats
        .filter({ !$0.archived && $0.messageCount == 0 })
        .max(by: { left, right in
            if left.updatedSeq != right.updatedSeq {
                return left.updatedSeq < right.updatedSeq
            }
            if left.startedSeq != right.startedSeq {
                return left.startedSeq < right.startedSeq
            }
            return left.chatId < right.chatId
        })
    else {
        return nil
    }
    return ChatDestination(
        roomID: roomID,
        topicID: topic.topicId,
        chatID: chat.chatId,
        title: chat.title,
        preview: chat.lastMessagePreview,
        updatedSequence: chat.updatedSeq
    )
}

struct ComposerLaunch: Hashable {
    let id: UUID
    let action: ComposerLaunchAction
    let draft: String
}

private struct PendingHomeSubmission {
    let text: String
    let launchAction: ComposerLaunchAction?
    let intentKey: String
}

struct ChatTopicGroup: Identifiable {
    let roomID: String
    let id: String
    let title: String
    let chats: [ChatDestination]
}

struct FiniteDesignTokens {
    var pagePadding: CGFloat = 20
    var sectionSpacing: CGFloat = 24
    var controlSpacing: CGFloat = 12
    var panelRadius: CGFloat = 30
    var drawerWidth: CGFloat = 320
    var homeHeroMarkSize: CGFloat = 104
    var homeHeroTopSpacing: CGFloat = 92
    var recentBadgeSpacing: CGFloat = 8
    var composerHorizontalPadding: CGFloat = 20
    var composerRadius: CGFloat = 28
}

private struct FiniteDesignTokensKey: EnvironmentKey {
    static let defaultValue = FiniteDesignTokens()
}

extension EnvironmentValues {
    var finiteTokens: FiniteDesignTokens {
        get { self[FiniteDesignTokensKey.self] }
        set { self[FiniteDesignTokensKey.self] = newValue }
    }
}

struct ContentView: View {
    @Environment(\.scenePhase) private var scenePhase
    @ObservedObject var model: AppModel
    @State private var path: [ChatDestination] = []
    @State private var showsDrawer = false
    @State private var showsSettings = false
    @State private var showsAgentPicker = false
    @State private var pendingHomeSubmission: PendingHomeSubmission?

    var body: some View {
        Group {
            if model.requiresNostrLogin {
                AccountLinkView(
                    phase: model.accountLinkPhase,
                    errorMessage: model.developerErrorText,
                    beginLink: {
                        model.beginAccountLink()
                    }
                )
            } else {
                appShell
            }
        }
        .task(id: model.requiresNostrLogin) {
            guard !model.requiresNostrLogin else { return }
            model.startFromForeground()
        }
        .onChange(of: scenePhase) { _, phase in
            guard phase == .active, !model.requiresNostrLogin else { return }
            model.startFromForeground()
        }
    }

    private var appShell: some View {
        NavigationStack(path: $path) {
            FocusedHomeView(
                agentName: model.pairedAgent?.displayName,
                recentChats: recentChats,
                startChat: startHomeChat,
                openChat: open,
                chooseAgent: {
                    showsAgentPicker = true
                }
            )
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    ChatDrawerToolbarButton(action: presentDrawer)
                }
            }
            .navigationDestination(for: ChatDestination.self) { destination in
                RoomThreadView(
                    model: model,
                    roomID: destination.roomID,
                    topicID: destination.topicID,
                    chatID: destination.chatID,
                    composerLaunch: destination.composerLaunch,
                    openDrawer: presentDrawer
                )
            }
        }
        .overlay {
            ChatDrawerOverlay(
                isPresented: showsDrawer,
                agentName: model.pairedAgent?.displayName ?? "Agent",
                groups: chatGroups,
                selectedChatID: path.last?.chatID,
                dismiss: dismissDrawer,
                startNewChat: startNewChatFromDrawer,
                createTopic: createTopicFromDrawer,
                startTopicChat: startTopicChatFromDrawer,
                renameChat: renameChatFromDrawer,
                archiveChat: archiveChatFromDrawer,
                openSettings: openSettingsFromDrawer,
                openChat: openFromDrawer
            )
            .allowsHitTesting(showsDrawer)
            .zIndex(20)
        }
        .sheet(isPresented: $showsSettings) {
            FocusedSettingsView(
                agentName: model.pairedAgent?.displayName,
                accountLabel: model.myNpub ?? "Linked account",
                chooseAgent: {
                    showsSettings = false
                    Task { @MainActor in
                        showsAgentPicker = true
                    }
                },
                signOut: model.signOutAndDeleteEverything
            )
        }
        .sheet(isPresented: $showsAgentPicker) {
            AgentPickerView(
                agents: model.availableAgents.map {
                    AgentChoice(
                        id: $0.roomId,
                        name: $0.displayName,
                        detail: $0.userStatusText,
                        isSelected: $0.roomId == model.pairedAgent?.roomId
                    )
                },
                choose: { choice in
                    guard let room = model.availableAgents.first(where: { $0.roomId == choice.id })
                    else {
                        return
                    }
                    _ = model.pairAgent(room) {
                        showsAgentPicker = false
                        path.removeAll()
                    }
                }
            )
            .presentationDetents([.medium, .large])
        }
        .onChange(of: model.availableAgents.map(\.roomId)) { _, agents in
            if model.pairedAgent == nil, !agents.isEmpty {
                showsAgentPicker = true
            }
        }
    }

    private var recentChats: [ChatDestination] {
        Array(allChats.prefix(3))
    }

    private var allChats: [ChatDestination] {
        guard let roomID = model.pairedAgent?.roomId else { return [] }
        return model.topics(for: roomID)
            .flatMap { topic in
                topic.chats.filter { !$0.archived }.map { chat in
                    ChatDestination(
                        roomID: roomID,
                        topicID: topic.topicId,
                        chatID: chat.chatId,
                        title: chat.title,
                        preview: chat.lastMessagePreview,
                        updatedSequence: chat.updatedSeq
                    )
                }
            }
            .sorted {
                if $0.updatedSequence == $1.updatedSequence {
                    return $0.id > $1.id
                }
                return $0.updatedSequence > $1.updatedSequence
            }
    }

    private var chatGroups: [ChatTopicGroup] {
        guard let roomID = model.pairedAgent?.roomId else { return [] }
        return model.topics(for: roomID)
            .map { topic in
                ChatTopicGroup(
                    roomID: roomID,
                    id: topic.topicId,
                    title: topic.title,
                    chats: topic.chats.filter { !$0.archived }.map { chat in
                        ChatDestination(
                            roomID: roomID,
                            topicID: topic.topicId,
                            chatID: chat.chatId,
                            title: chat.title,
                            preview: chat.lastMessagePreview,
                            updatedSequence: chat.updatedSeq
                        )
                    }
                    .sorted { $0.updatedSequence > $1.updatedSequence }
                )
            }
            .filter { !$0.chats.isEmpty }
    }

    private func startHomeChat(
        _ text: String,
        launchAction: ComposerLaunchAction?,
        completion: @escaping (Bool) -> Void
    ) {
        let pending = pendingHomeSubmission.flatMap {
            $0.text == text && $0.launchAction == launchAction ? $0 : nil
        } ?? PendingHomeSubmission(
            text: text,
            launchAction: launchAction,
            intentKey: "ios-home-\(UUID().uuidString.lowercased())"
        )
        pendingHomeSubmission = pending
        if let reusableDestination = reusableEmptyHomeChatDestination {
            reuseEmptyHomeChat(
                reusableDestination,
                text: text,
                launchAction: launchAction,
                completion: completion
            )
            return
        }
        let onCreated: @MainActor () -> Void = {
            guard let destination = selectedDestination else {
                completion(false)
                return
            }
            finishHomeNavigation(
                to: destination,
                text: text,
                launchAction: launchAction,
                completion: completion
            )
        }
        let onFailure: @MainActor () -> Void = {
            completion(false)
        }
        let started: Bool
        if launchAction == nil {
            started = model.startHomeChat(
                text: text,
                intentKey: pending.intentKey,
                onStarted: onCreated,
                onFailure: onFailure
            )
        } else {
            started = model.createHomeChat(
                intentKey: pending.intentKey,
                onCreated: onCreated,
                onFailure: onFailure
            )
        }
        if !started {
            completion(false)
        }
    }

    private var reusableEmptyHomeChatDestination: ChatDestination? {
        guard let roomID = model.pairedAgent?.roomId else { return nil }
        return findReusableEmptyHomeChatDestination(
            roomID: roomID,
            topics: model.topics(for: roomID)
        )
    }

    private func reuseEmptyHomeChat(
        _ destination: ChatDestination,
        text: String,
        launchAction: ComposerLaunchAction?,
        completion: @escaping (Bool) -> Void
    ) {
        let opened = model.openChat(
            roomID: destination.roomID,
            topicID: destination.topicID,
            chatID: destination.chatID
        ) {
            if launchAction == nil,
               !model.send(roomID: destination.roomID, text: text)
            {
                completion(false)
                return
            }
            finishHomeNavigation(
                to: destination,
                text: text,
                launchAction: launchAction,
                completion: completion
            )
        }
        if !opened {
            completion(false)
        }
    }

    private func finishHomeNavigation(
        to destination: ChatDestination,
        text: String,
        launchAction: ComposerLaunchAction?,
        completion: @escaping (Bool) -> Void
    ) {
        let routedDestination: ChatDestination
        if let launchAction {
            routedDestination = ChatDestination(
                roomID: destination.roomID,
                topicID: destination.topicID,
                chatID: destination.chatID,
                title: destination.title,
                preview: destination.preview,
                updatedSequence: destination.updatedSequence,
                composerLaunch: ComposerLaunch(
                    id: UUID(),
                    action: launchAction,
                    draft: text
                )
            )
        } else {
            routedDestination = destination
        }

        pendingHomeSubmission = nil
        path.append(routedDestination)
        completion(true)
    }

    private func open(_ destination: ChatDestination) {
        _ = model.openChat(
            roomID: destination.roomID,
            topicID: destination.topicID,
            chatID: destination.chatID
        ) {
            path.append(destination)
        }
    }

    private func openFromDrawer(_ destination: ChatDestination) {
        _ = model.openChat(
            roomID: destination.roomID,
            topicID: destination.topicID,
            chatID: destination.chatID
        ) {
            path = [destination]
            withAnimation(.snappy(duration: 0.24)) {
                showsDrawer = false
            }
        }
    }

    private func dismissDrawer() {
        withAnimation(.snappy(duration: 0.24)) {
            showsDrawer = false
        }
    }

    private func presentDrawer() {
        withAnimation(.snappy(duration: 0.28)) {
            showsDrawer = true
        }
    }

    private func startNewChatFromDrawer() {
        path.removeAll()
        dismissDrawer()
    }

    private func createTopicFromDrawer(
        _ title: String,
        completion: @escaping (Bool) -> Void
    ) {
        guard let roomID = model.pairedAgent?.roomId else {
            completion(false)
            return
        }
        let started = model.createTopic(
            roomID: roomID,
            title: title,
            onCreated: {
                guard let destination = selectedDestination else {
                    completion(false)
                    return
                }
                path = [destination]
                completion(true)
            },
            onFailure: {
                completion(false)
            }
        )
        if !started {
            completion(false)
        }
    }

    private func startTopicChatFromDrawer(
        _ group: ChatTopicGroup,
        completion: @escaping (Bool) -> Void
    ) {
        if let existing = findReusableEmptyChatDestination(
            roomID: group.roomID,
            topicID: group.id,
            topics: model.topics(for: group.roomID)
        ) {
            let opened = model.openChat(
                roomID: existing.roomID,
                topicID: existing.topicID,
                chatID: existing.chatID
            ) {
                path = [existing]
                dismissDrawer()
                completion(true)
            }
            if !opened {
                completion(false)
            }
            return
        }

        let started = model.startTopicChat(
            roomID: group.roomID,
            topicID: group.id,
            onStarted: {
                guard let destination = selectedDestination else {
                    completion(false)
                    return
                }
                path = [destination]
                dismissDrawer()
                completion(true)
            },
            onFailure: {
                completion(false)
            }
        )
        if !started {
            completion(false)
        }
    }

    private func renameChatFromDrawer(
        _ chat: ChatDestination,
        title: String,
        completion: @escaping (Bool) -> Void
    ) {
        let started = model.renameChat(
            roomID: chat.roomID,
            topicID: chat.topicID,
            chatID: chat.chatID,
            title: title,
            onRenamed: {
                completion(true)
            },
            onFailure: {
                completion(false)
            }
        )
        if !started {
            completion(false)
        }
    }

    private func archiveChatFromDrawer(_ chat: ChatDestination) {
        _ = model.archiveChat(
            roomID: chat.roomID,
            topicID: chat.topicID,
            chatID: chat.chatID,
            onArchived: {
                if path.last?.chatID == chat.chatID {
                    path.removeAll()
                }
            }
        )
    }

    private func openSettingsFromDrawer() {
        dismissDrawer()
        Task { @MainActor in
            showsSettings = true
        }
    }

    private var selectedDestination: ChatDestination? {
        guard let state = model.state,
              let roomID = state.selectedRoomId,
              let topicID = state.selectedTopicId,
              let chatID = state.selectedChatId,
              let topic = state.topics.first(where: {
                  $0.roomId == roomID && $0.topicId == topicID
              }),
              let chat = topic.chats.first(where: { $0.chatId == chatID })
        else {
            return nil
        }
        return ChatDestination(
            roomID: roomID,
            topicID: topicID,
            chatID: chatID,
            title: chat.title,
            preview: chat.lastMessagePreview,
            updatedSequence: chat.updatedSeq
        )
    }
}

struct FocusedHomeView: View {
    @Environment(\.finiteTokens) private var tokens
    let agentName: String?
    let recentChats: [ChatDestination]
    let startChat: (String, ComposerLaunchAction?, @escaping (Bool) -> Void) -> Void
    let openChat: (ChatDestination) -> Void
    let chooseAgent: () -> Void
    @FocusState private var isComposerFocused: Bool
    @State private var hasComposerDraft = false

    var body: some View {
        ScrollView {
            VStack(spacing: tokens.sectionSpacing) {
                Spacer(minLength: tokens.homeHeroTopSpacing)

                VStack(spacing: 16) {
                    FiniteLogoMark()
                        .fill(.tint)
                        .frame(
                            width: tokens.homeHeroMarkSize,
                            height: tokens.homeHeroMarkSize
                        )
                        .accessibilityLabel("Finite logo")

                    Text("It’s time to build")
                        .font(.title2.weight(.semibold))

                    if let agentName {
                        Text("with \(agentName)")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity)

                if agentName == nil {
                    Button(action: chooseAgent) {
                        Label("Choose an agent", systemImage: "sparkles")
                    }
                    .buttonStyle(.glassProminent)
                    .controlSize(.large)
                } else if !recentChats.isEmpty {
                    RecentChatBadges(
                        chats: recentChats,
                        spacing: tokens.recentBadgeSpacing,
                        openChat: openChat
                    )
                    .opacity(hasComposerDraft ? 0 : 1)
                    .allowsHitTesting(!hasComposerDraft)
                    .accessibilityHidden(hasComposerDraft)
                    .animation(.easeOut(duration: 0.18), value: hasComposerDraft)
                }

                Spacer(minLength: 72)
            }
            .padding(.horizontal, tokens.pagePadding)
            .padding(.bottom, tokens.sectionSpacing)
        }
        .scrollDismissesKeyboard(.interactively)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if agentName != nil {
                NewChatComposer(
                    isInputFocused: $isComposerFocused,
                    placeholder: "What do you want to work on?",
                    onStartChat: startChat,
                    onDraftPresenceChange: { hasComposerDraft = $0 },
                    outerHorizontalPadding: tokens.composerHorizontalPadding,
                    surfaceRadius: tokens.composerRadius
                )
                .background {
                    Color(.systemBackground)
                        .ignoresSafeArea(edges: .bottom)
                }
            }
        }
        .background(Color(.systemBackground))
        .navigationTitle("Home")
        .navigationBarTitleDisplayMode(.inline)
    }
}

private struct RecentChatBadges: View {
    let chats: [ChatDestination]
    let spacing: CGFloat
    let openChat: (ChatDestination) -> Void

    var body: some View {
        GlassEffectContainer(spacing: spacing) {
            VStack(spacing: spacing) {
                ForEach(chats) { chat in
                    Button {
                        openChat(chat)
                    } label: {
                        Label(chat.title, systemImage: "bubble.left")
                            .font(.subheadline.weight(.medium))
                            .lineLimit(1)
                    }
                    .buttonStyle(.glass)
                    .controlSize(.small)
                    .accessibilityLabel("Recent chat: \(chat.title)")
                    .accessibilityIdentifier("RecentChat-\(chat.chatID)")
                }
            }
            .padding(12)
        }
        .frame(maxWidth: .infinity)
    }
}

private enum DrawerNameEditor: Identifiable {
    case newTopic
    case renameChat(ChatDestination)

    var id: String {
        switch self {
        case .newTopic:
            "new-topic"
        case .renameChat(let chat):
            "rename-chat-\(chat.id)"
        }
    }

    var title: String {
        switch self {
        case .newTopic:
            "New topic"
        case .renameChat:
            "Rename chat"
        }
    }

    var explanation: String {
        switch self {
        case .newTopic:
            "Topics keep related chats together."
        case .renameChat:
            "Choose a name that makes this chat easy to find later."
        }
    }

    var initialName: String {
        switch self {
        case .newTopic:
            ""
        case .renameChat(let chat):
            chat.title == "New chat" ? "" : chat.title
        }
    }

    var submitTitle: String {
        switch self {
        case .newTopic:
            "Create"
        case .renameChat:
            "Save"
        }
    }
}

struct ChatDrawerOverlay: View {
    @Environment(\.finiteTokens) private var tokens
    let isPresented: Bool
    let agentName: String
    let groups: [ChatTopicGroup]
    let selectedChatID: String?
    let dismiss: () -> Void
    let startNewChat: () -> Void
    let createTopic: (String, @escaping (Bool) -> Void) -> Void
    let startTopicChat: (ChatTopicGroup, @escaping (Bool) -> Void) -> Void
    let renameChat: (ChatDestination, String, @escaping (Bool) -> Void) -> Void
    let archiveChat: (ChatDestination) -> Void
    let openSettings: () -> Void
    let openChat: (ChatDestination) -> Void
    @State private var dismissOffset: CGFloat = 0
    @State private var isDismissDragging = false
    @State private var nameEditor: DrawerNameEditor?

    var body: some View {
        GeometryReader { proxy in
            let drawerWidth = min(tokens.drawerWidth, proxy.size.width * 0.88)
            let revealProgress = max(0, 1 + dismissOffset / max(drawerWidth, 1))

            ZStack(alignment: .leading) {
                if isPresented {
                    Color.black.opacity(0.34 * revealProgress)
                        .ignoresSafeArea()
                        .contentShape(Rectangle())
                        .onTapGesture(perform: dismiss)
                        .transition(.opacity)

                    drawerPanel
                        .frame(width: drawerWidth)
                        .frame(maxHeight: .infinity)
                        .offset(x: dismissOffset)
                        .disabled(isDismissDragging)
                        .simultaneousGesture(dismissGesture(drawerWidth: drawerWidth))
                        .transition(.move(edge: .leading))
                }
            }
        }
        .accessibilityHidden(!isPresented)
        .onChange(of: isPresented) { _, presented in
            if !presented {
                dismissOffset = 0
                isDismissDragging = false
                nameEditor = nil
            }
        }
        .sheet(item: $nameEditor) { editor in
            DrawerNameEditorSheet(editor: editor) { name, completion in
                switch editor {
                case .newTopic:
                    createTopic(name, completion)
                case .renameChat(let chat):
                    renameChat(chat, name, completion)
                }
            }
            .presentationDetents([.medium])
            .presentationDragIndicator(.visible)
        }
    }

    private var drawerPanel: some View {
        VStack(alignment: .leading, spacing: 0) {
            drawerHeader

            Divider()

            List {
                Section {
                    if groups.isEmpty {
                        Label("No chats yet", systemImage: "bubble.left")
                            .foregroundStyle(.secondary)
                            .listRowBackground(Color.clear)
                            .listRowSeparator(.hidden)
                    } else {
                        ForEach(groups) { group in
                            DrawerTopicRows(
                                group: group,
                                selectedChatID: selectedChatID,
                                startNewChat: startTopicChat,
                                renameChat: {
                                    nameEditor = .renameChat($0)
                                },
                                archiveChat: archiveChat,
                                openChat: openChat
                            )
                        }
                    }
                } header: {
                    HStack {
                        Text("Topics")
                        Spacer()
                        Button {
                            nameEditor = .newTopic
                        } label: {
                            Image(systemName: "plus")
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("New topic")
                        .accessibilityIdentifier("DrawerNewTopicButton")
                    }
                }
            }
            .listStyle(.plain)
            .environment(\.defaultMinListRowHeight, 34)
            .scrollContentBackground(.hidden)
            .contentMargins(.vertical, 8)

            Button(action: startNewChat) {
                Label("New chat", systemImage: "plus")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.glassProminent)
            .controlSize(.large)
            .padding(.horizontal, 12)
            .padding(.bottom, 8)
            .accessibilityIdentifier("DrawerNewChatButton")
        }
        .background {
            Rectangle()
                .fill(.regularMaterial)
                .ignoresSafeArea()
        }
        .shadow(color: .black.opacity(0.2), radius: 28, x: 12)
    }

    private var drawerHeader: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                FiniteLogoMark()
                    .fill(.tint)
                    .frame(width: 24, height: 24)
                    .accessibilityHidden(true)

                Text("Finite")
                    .font(.headline)

                Spacer()

                Button(action: dismiss) {
                    Image(systemName: "sidebar.left")
                }
                .buttonStyle(.glass)
                .accessibilityLabel("Close navigation")
                .accessibilityIdentifier("DrawerCloseButton")
            }

            Button(action: openSettings) {
                HStack(spacing: 12) {
                    Image(systemName: "sparkles")
                        .foregroundStyle(.tint)
                        .frame(width: 30, height: 30)
                        .background(.tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 9))

                    VStack(alignment: .leading, spacing: 2) {
                        Text(agentName)
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text("Paired agent")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }

                    Spacer()

                    Image(systemName: "chevron.right")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.tertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Paired agent: \(agentName)")
        }
        .padding(.horizontal, 16)
        .padding(.top, 8)
        .padding(.bottom, 14)
    }

    private func dismissGesture(drawerWidth: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 10)
            .onChanged { value in
                guard value.translation.width < 0,
                      abs(value.translation.width) > abs(value.translation.height)
                else {
                    return
                }
                isDismissDragging = true
                dismissOffset = max(-drawerWidth, value.translation.width)
            }
            .onEnded { value in
                let horizontal = abs(value.translation.width) > abs(value.translation.height)
                let passedDistance = value.translation.width < -(drawerWidth * 0.28)
                let passedPrediction = value.predictedEndTranslation.width < -(drawerWidth * 0.45)
                if horizontal && (passedDistance || passedPrediction) {
                    dismiss()
                } else {
                    withAnimation(.snappy(duration: 0.22)) {
                        dismissOffset = 0
                    }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.22) {
                        isDismissDragging = false
                    }
                }
            }
    }
}

private struct DrawerTopicRows: View {
    let group: ChatTopicGroup
    let selectedChatID: String?
    let startNewChat: (ChatTopicGroup, @escaping (Bool) -> Void) -> Void
    let renameChat: (ChatDestination) -> Void
    let archiveChat: (ChatDestination) -> Void
    let openChat: (ChatDestination) -> Void
    @State private var isExpanded = true
    @State private var isStartingChat = false

    var body: some View {
        Group {
            topicRow
            if isExpanded {
                ForEach(group.chats) { chat in
                    chatRow(chat)
                }
            }
        }
    }

    private var topicRow: some View {
        HStack(spacing: 6) {
            Button {
                withAnimation(.snappy(duration: 0.2)) {
                    isExpanded.toggle()
                }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "number")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.tint)
                        .frame(width: 24, height: 24)
                        .background(.tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 7))

                    Text(group.title)
                        .font(.subheadline.weight(.semibold))
                        .lineLimit(1)

                    Spacer(minLength: 0)

                    Image(systemName: "chevron.right")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(isExpanded ? 90 : 0))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("\(isExpanded ? "Collapse" : "Expand") \(group.title)")
            .accessibilityValue(isExpanded ? "Expanded" : "Collapsed")

            Button(action: beginNewChat) {
                if isStartingChat {
                    ProgressView()
                        .controlSize(.small)
                } else {
                    Image(systemName: "plus.message")
                }
            }
            .frame(width: 28, height: 28)
            .buttonStyle(.plain)
            .disabled(isStartingChat)
            .accessibilityLabel("New chat in \(group.title)")
            .accessibilityIdentifier("DrawerNewChat-\(group.id)")
        }
        .padding(.horizontal, 8)
        .frame(minHeight: 36)
        .contentShape(Rectangle())
        .contextMenu {
            Button(action: beginNewChat) {
                Label("New Chat", systemImage: "plus.message")
            }
        }
        .padding(.vertical, 2)
        .listRowBackground(Color.clear)
        .listRowSeparator(.hidden)
        .listRowInsets(EdgeInsets(top: 0, leading: 8, bottom: 0, trailing: 8))
    }

    private func chatRow(_ chat: ChatDestination) -> some View {
        Button {
            openChat(chat)
        } label: {
            HStack {
                Text(chat.title.isEmpty ? "New chat" : chat.title)
                    .font(.subheadline)
                    .lineLimit(1)

                Spacer(minLength: 0)
            }
            .padding(.horizontal, 8)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .frame(minHeight: 34)
        .background(
            selectedChatID == chat.chatID
                ? Color.accentColor.opacity(0.13)
                : .clear,
            in: RoundedRectangle(cornerRadius: 9)
        )
        .contextMenu {
            Button {
                renameChat(chat)
            } label: {
                Label("Rename", systemImage: "pencil")
            }
            Button {
                archiveChat(chat)
            } label: {
                Label("Archive", systemImage: "archivebox")
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("DrawerChat-\(chat.chatID)")
        .listRowBackground(Color.clear)
        .listRowSeparator(.hidden)
        .listRowInsets(EdgeInsets(top: 0, leading: 8, bottom: 0, trailing: 8))
    }

    private func beginNewChat() {
        guard !isStartingChat else { return }
        isStartingChat = true
        startNewChat(group) { _ in
            isStartingChat = false
        }
    }
}

private struct DrawerNameEditorSheet: View {
    @Environment(\.dismiss) private var dismiss
    let editor: DrawerNameEditor
    let save: (String, @escaping (Bool) -> Void) -> Void
    @State private var name: String
    @State private var isSaving = false
    @State private var saveFailed = false
    @FocusState private var nameFocused: Bool

    init(
        editor: DrawerNameEditor,
        save: @escaping (String, @escaping (Bool) -> Void) -> Void
    ) {
        self.editor = editor
        self.save = save
        _name = State(initialValue: editor.initialName)
    }

    private var normalizedName: String {
        name.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var nameIsValid: Bool {
        !normalizedName.isEmpty && normalizedName.utf8.count <= 120
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $name)
                        .focused($nameFocused)
                        .textInputAutocapitalization(.sentences)
                        .submitLabel(.done)
                        .onSubmit(saveName)
                        .accessibilityIdentifier("DrawerNameField")
                } footer: {
                    if normalizedName.utf8.count > 120 {
                        Text("Name is too long.")
                            .foregroundStyle(.red)
                    } else if saveFailed {
                        Text("Couldn’t save that change. Try again.")
                            .foregroundStyle(.red)
                    } else {
                        Text(editor.explanation)
                    }
                }
            }
            .navigationTitle(editor.title)
            .navigationBarTitleDisplayMode(.inline)
            .interactiveDismissDisabled(isSaving)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", role: .cancel) {
                        dismiss()
                    }
                    .disabled(isSaving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(action: saveName) {
                        if isSaving {
                            ProgressView()
                        } else {
                            Text(editor.submitTitle)
                        }
                    }
                    .disabled(!nameIsValid || isSaving)
                    .accessibilityIdentifier("DrawerNameSaveButton")
                }
            }
            .task {
                nameFocused = true
            }
        }
    }

    private func saveName() {
        guard nameIsValid, !isSaving else { return }
        isSaving = true
        saveFailed = false
        save(normalizedName) { succeeded in
            isSaving = false
            if succeeded {
                dismiss()
            } else {
                saveFailed = true
            }
        }
    }
}

struct AgentChoice: Identifiable {
    let id: String
    let name: String
    let detail: String
    let isSelected: Bool
}

struct AgentPickerView: View {
    let agents: [AgentChoice]
    let choose: (AgentChoice) -> Void

    var body: some View {
        NavigationStack {
            Group {
                if agents.isEmpty {
                    ContentUnavailableView(
                        "No agents available",
                        systemImage: "sparkles",
                        description: Text("Your agents will appear after account linking finishes.")
                    )
                } else {
                    List(agents) { agent in
                        Button {
                            choose(agent)
                        } label: {
                            HStack(spacing: 14) {
                                Image(systemName: "sparkles")
                                    .frame(width: 30, height: 30)
                                    .foregroundStyle(.tint)
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(agent.name)
                                        .foregroundStyle(.primary)
                                    Text(agent.detail)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                                Spacer()
                                if agent.isSelected {
                                    Image(systemName: "checkmark.circle.fill")
                                        .foregroundStyle(.tint)
                                }
                            }
                        }
                    }
                }
            }
            .navigationTitle("Choose agent")
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}

struct FocusedSettingsView: View {
    let agentName: String?
    let accountLabel: String
    let chooseAgent: () -> Void
    let signOut: () -> Void

    var body: some View {
        NavigationStack {
            Form {
                Section("Agent") {
                    Button(action: chooseAgent) {
                        LabeledContent(
                            "Paired agent",
                            value: agentName ?? "Choose"
                        )
                    }
                }

                Section("Account") {
                    LabeledContent("Signed in", value: accountLabel)
                    Button("Sign out and remove local data", role: .destructive, action: signOut)
                }
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}

enum AccountLinkPhase: Equatable {
    case ready
    case authenticating
    case waiting
}

struct AccountLinkView: View {
    @Environment(\.finiteTokens) private var tokens
    let phase: AccountLinkPhase
    let errorMessage: String?
    let beginLink: () -> Void

    var body: some View {
        VStack(spacing: tokens.sectionSpacing) {
            Spacer()

            VStack(spacing: 20) {
                FiniteLogoMark()
                    .fill(.tint)
                    .frame(width: 112, height: 112)
                    .accessibilityLabel("Finite")

                Text("Your agent, in your pocket")
                    .font(.title.bold())
                    .multilineTextAlignment(.center)
            }

            Spacer()

            VStack(spacing: tokens.controlSpacing) {
                if let errorMessage {
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .accessibilityHidden(true)

                        Text(errorMessage)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .padding(14)
                    .background(
                        Color.red.opacity(0.1),
                        in: RoundedRectangle(cornerRadius: 16)
                    )
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("AccountLinkError")
                    .transition(.move(edge: .bottom).combined(with: .opacity))
                }

                Button(action: beginLink) {
                    HStack {
                        if phase != .ready {
                            ProgressView()
                        }
                        Text(buttonTitle)
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(.glassProminent)
                .controlSize(.large)
                .disabled(phase != .ready)
                .accessibilityIdentifier("BeginAccountLink")
            }
        }
        .padding(tokens.pagePadding)
        .background(Color(.systemGroupedBackground))
        .animation(.snappy, value: errorMessage)
    }

    private var buttonTitle: String {
        switch phase {
        case .ready:
            "Continue with Finite"
        case .authenticating:
            "Signing in…"
        case .waiting:
            "Finishing secure link…"
        }
    }
}

private enum FocusedPreviewFixtures {
    static let recent = [
        ChatDestination(
            roomID: "agent",
            topicID: "home",
            chatID: "ship-ios",
            title: "Ship the iOS app",
            preview: "Let’s keep the first release intentionally narrow.",
            updatedSequence: 30
        ),
        ChatDestination(
            roomID: "agent",
            topicID: "writing",
            chatID: "launch-note",
            title: "Launch note",
            preview: "The crown jewel is back.",
            updatedSequence: 20
        ),
        ChatDestination(
            roomID: "agent",
            topicID: "home",
            chatID: "week-plan",
            title: "Plan the week",
            preview: "Three priorities, then everything else waits.",
            updatedSequence: 10
        ),
    ]
}

#Preview("Home — paired") {
    NavigationStack {
        FocusedHomeView(
            agentName: "Ada",
            recentChats: FocusedPreviewFixtures.recent,
            startChat: { _, _, completion in completion(true) },
            openChat: { _ in },
            chooseAgent: {}
        )
    }
}

#Preview("Home — first run") {
    NavigationStack {
        FocusedHomeView(
            agentName: nil,
            recentChats: [],
            startChat: { _, _, completion in completion(true) },
            openChat: { _ in },
            chooseAgent: {}
        )
    }
}

#Preview("Home — paired, no recents") {
    NavigationStack {
        FocusedHomeView(
            agentName: "Ada",
            recentChats: [],
            startChat: { _, _, completion in completion(true) },
            openChat: { _ in },
            chooseAgent: {}
        )
    }
}

#Preview("Chat drawer") {
    ChatDrawerOverlay(
        isPresented: true,
        agentName: "Ada",
        groups: [
            ChatTopicGroup(
                roomID: "agent",
                id: "home",
                title: "Home",
                chats: FocusedPreviewFixtures.recent.filter { $0.topicID == "home" }
            ),
            ChatTopicGroup(
                roomID: "agent",
                id: "writing",
                title: "Writing",
                chats: FocusedPreviewFixtures.recent.filter { $0.topicID == "writing" }
            ),
        ],
        selectedChatID: "ship-ios",
        dismiss: {},
        startNewChat: {},
        createTopic: { _, completion in completion(true) },
        startTopicChat: { _, completion in completion(true) },
        renameChat: { _, _, completion in completion(true) },
        archiveChat: { _ in },
        openSettings: {},
        openChat: { _ in }
    )
}

#Preview("Agent picker") {
    AgentPickerView(
        agents: [
            AgentChoice(id: "ada", name: "Ada", detail: "Connected", isSelected: true),
            AgentChoice(id: "linus", name: "Linus", detail: "Connected", isSelected: false),
        ],
        choose: { _ in }
    )
}

#Preview("Settings") {
    FocusedSettingsView(
        agentName: "Ada",
        accountLabel: "npub1finite…crownjewel",
        chooseAgent: {},
        signOut: {}
    )
}

#Preview("Account link") {
    AccountLinkView(phase: .ready, errorMessage: nil, beginLink: {})
}

#Preview("Account link — finishing") {
    AccountLinkView(phase: .waiting, errorMessage: nil, beginLink: {})
}

#Preview("Account link — error") {
    AccountLinkView(
        phase: .ready,
        errorMessage: "This iPhone could not finish linking. Please try again.",
        beginLink: {}
    )
}
