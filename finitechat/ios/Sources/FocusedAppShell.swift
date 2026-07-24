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
    @State private var homeComposerFocusRequest = 0
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
                focusRequest: homeComposerFocusRequest,
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
                openHome: openHomeFromDrawer,
                startNewChat: startNewChatFromDrawer,
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
                topic.chats.map { chat in
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
                    id: topic.topicId,
                    title: topic.title,
                    chats: topic.chats.map { chat in
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
        let onCreated: @MainActor () -> Void = {
            guard let destination = selectedDestination else {
                completion(false)
                return
            }

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

    private func openHomeFromDrawer() {
        path.removeAll()
        dismissDrawer()
    }

    private func startNewChatFromDrawer() {
        path.removeAll()
        dismissDrawer()
        Task { @MainActor in
            homeComposerFocusRequest += 1
        }
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
    var focusRequest = 0
    let startChat: (String, ComposerLaunchAction?, @escaping (Bool) -> Void) -> Void
    let openChat: (ChatDestination) -> Void
    let chooseAgent: () -> Void
    @State private var draft = ""
    @FocusState private var isComposerFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
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
                    }

                    Spacer(minLength: 72)
                }
                .padding(.horizontal, tokens.pagePadding)
                .padding(.bottom, tokens.sectionSpacing)
            }
            .scrollDismissesKeyboard(.interactively)

            if agentName != nil {
                NewChatComposer(
                    text: $draft,
                    isInputFocused: $isComposerFocused,
                    placeholder: "What do you want to work on?",
                    onStartChat: startChat,
                    outerHorizontalPadding: tokens.composerHorizontalPadding,
                    surfaceRadius: tokens.composerRadius
                )
            }
        }
        .background(Color(.systemBackground))
        .navigationTitle("Home")
        .navigationBarTitleDisplayMode(.inline)
        .task(id: focusRequest) {
            guard focusRequest > 0 else { return }
            isComposerFocused = true
        }
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

struct ChatDrawerOverlay: View {
    @Environment(\.finiteTokens) private var tokens
    let isPresented: Bool
    let agentName: String
    let groups: [ChatTopicGroup]
    let selectedChatID: String?
    let dismiss: () -> Void
    let openHome: () -> Void
    let startNewChat: () -> Void
    let openSettings: () -> Void
    let openChat: (ChatDestination) -> Void

    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                if isPresented {
                    Color.black.opacity(0.34)
                        .ignoresSafeArea()
                        .contentShape(Rectangle())
                        .onTapGesture(perform: dismiss)
                        .transition(.opacity)

                    drawerPanel
                        .frame(width: min(tokens.drawerWidth, proxy.size.width * 0.88))
                        .frame(maxHeight: .infinity)
                        .transition(.move(edge: .leading))
                }
            }
        }
        .accessibilityHidden(!isPresented)
    }

    private var drawerPanel: some View {
        VStack(alignment: .leading, spacing: 0) {
            drawerHeader

            Divider()

            List {
                Section {
                    DrawerNavigationRow(
                        title: "Home",
                        systemImage: "house",
                        isSelected: selectedChatID == nil,
                        action: openHome
                    )
                    .accessibilityIdentifier("DrawerHomeButton")
                }

                Section("Topics") {
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
                                openChat: openChat
                            )
                        }
                    }
                }
            }
            .listStyle(.plain)
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

            Divider()

            Button(action: openSettings) {
                HStack(spacing: 12) {
                    Image(systemName: "gearshape")
                        .frame(width: 24)
                    Text("Settings")
                    Spacer()
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.horizontal, 16)
            .frame(minHeight: 52)
            .accessibilityIdentifier("DrawerSettingsButton")
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
}

private struct DrawerNavigationRow: View {
    let title: String
    let systemImage: String
    let isSelected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Label(title, systemImage: systemImage)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .listRowBackground(selectionBackground)
        .listRowSeparator(.hidden)
    }

    private var selectionBackground: some View {
        RoundedRectangle(cornerRadius: 10)
            .fill(isSelected ? Color.accentColor.opacity(0.13) : .clear)
    }
}

private struct DrawerTopicRows: View {
    let group: ChatTopicGroup
    let selectedChatID: String?
    let openChat: (ChatDestination) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Label {
                Text(group.title)
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(1)
            } icon: {
                Image(systemName: "number")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tint)
                    .frame(width: 24, height: 24)
                    .background(.tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 7))
            }
            .padding(.horizontal, 8)
            .frame(minHeight: 36)

            ForEach(group.chats) { chat in
                Button {
                    openChat(chat)
                } label: {
                    HStack(spacing: 8) {
                        Image(systemName: "bubble.left")
                            .font(.caption)
                            .foregroundStyle(.tertiary)
                            .frame(width: 14)

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
                .padding(.leading, 24)
                .accessibilityIdentifier("DrawerChat-\(chat.chatID)")
            }
        }
        .padding(.vertical, 2)
        .listRowBackground(Color.clear)
        .listRowSeparator(.hidden)
        .listRowInsets(EdgeInsets(top: 0, leading: 8, bottom: 0, trailing: 8))
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
            Image(systemName: "bubble.left.and.sparkles")
                .font(.system(size: 56))
                .foregroundStyle(.tint)

            VStack(spacing: 8) {
                Text("Your agent, in your pocket")
                    .font(.largeTitle.bold())
                    .multilineTextAlignment(.center)
                Text("Sign in securely, then this iPhone will receive the encrypted key for your existing Finite account.")
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }

            if let errorMessage {
                Text(errorMessage)
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .multilineTextAlignment(.center)
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
            Spacer()
        }
        .padding(tokens.pagePadding)
        .background(Color(.systemGroupedBackground))
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
                id: "home",
                title: "Home",
                chats: FocusedPreviewFixtures.recent.filter { $0.topicID == "home" }
            ),
            ChatTopicGroup(
                id: "writing",
                title: "Writing",
                chats: FocusedPreviewFixtures.recent.filter { $0.topicID == "writing" }
            ),
        ],
        selectedChatID: "ship-ios",
        dismiss: {},
        openHome: {},
        startNewChat: {},
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
