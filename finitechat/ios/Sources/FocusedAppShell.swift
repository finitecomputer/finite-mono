import SwiftUI

struct ChatDestination: Hashable, Identifiable {
    let roomID: String
    let topicID: String
    let chatID: String
    let title: String
    let preview: String
    let updatedSequence: UInt64

    var id: String {
        "\(roomID)|\(topicID)|\(chatID)"
    }
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
    var composerRadius: CGFloat = 28
    var panelRadius: CGFloat = 30
    var drawerWidth: CGFloat = 340
    var homeHeroMarkSize: CGFloat = 104
    var homeHeroTopSpacing: CGFloat = 92
    var recentBadgeSpacing: CGFloat = 8
    var homeDockBottomSpacing: CGFloat = 10
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
    @State private var pendingHomeSubmission: (text: String, intentKey: String)?

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
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        showsSettings = true
                    } label: {
                        Label("Settings", systemImage: "gearshape")
                    }
                }
            }
            .navigationDestination(for: ChatDestination.self) { destination in
                RoomThreadView(
                    model: model,
                    roomID: destination.roomID,
                    openDrawer: {
                        withAnimation(.snappy(duration: 0.28)) {
                            showsDrawer = true
                        }
                    }
                )
            }
        }
        .overlay {
            if showsDrawer {
                ChatDrawerOverlay(
                    agentName: model.pairedAgent?.displayName ?? "Agent",
                    groups: chatGroups,
                    selectedChatID: path.last?.chatID,
                    dismiss: {
                        withAnimation(.snappy(duration: 0.24)) {
                            showsDrawer = false
                        }
                    },
                    openChat: openFromDrawer
                )
                .transition(.opacity)
                .zIndex(20)
            }
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

    private func startHomeChat(_ text: String, completion: @escaping (Bool) -> Void) {
        let pending = pendingHomeSubmission.flatMap { $0.text == text ? $0 : nil }
            ?? (text: text, intentKey: "ios-home-\(UUID().uuidString.lowercased())")
        pendingHomeSubmission = pending
        let started = model.startHomeChat(
            text: text,
            intentKey: pending.intentKey,
            onStarted: {
                pendingHomeSubmission = nil
                guard let destination = selectedDestination else {
                    completion(false)
                    return
                }
                path.append(destination)
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
    let startChat: (String, @escaping (Bool) -> Void) -> Void
    let openChat: (ChatDestination) -> Void
    let chooseAgent: () -> Void
    @State private var draft = ""
    @State private var isStartingChat = false

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
                HomeComposer(
                    text: $draft,
                    isSending: isStartingChat,
                    send: submitDraft
                )
                .padding(.horizontal, tokens.pagePadding)
                .padding(.bottom, tokens.homeDockBottomSpacing)
            }
        }
        .background(Color(.systemBackground))
        .navigationTitle("Home")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func submitDraft() {
        let message = draft
        isStartingChat = true
        startChat(message) { success in
            isStartingChat = false
            if success {
                draft = ""
            }
        }
    }
}

private struct HomeComposer: View {
    @Environment(\.finiteTokens) private var tokens
    @Binding var text: String
    let isSending: Bool
    let send: () -> Void

    private var canSend: Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !isSending
    }

    var body: some View {
        GlassEffectContainer(spacing: 10) {
            HStack(alignment: .bottom, spacing: 10) {
                TextField("What do you want to work on?", text: $text, axis: .vertical)
                    .lineLimit(1 ... 6)
                    .textFieldStyle(.plain)
                    .padding(.leading, 6)
                    .accessibilityIdentifier("HomeComposerField")

                Button(action: send) {
                    Image(systemName: isSending ? "ellipsis" : "arrow.up")
                        .font(.headline)
                        .frame(width: 38, height: 38)
                }
                .buttonStyle(.glassProminent)
                .disabled(!canSend)
                .accessibilityLabel("Start new chat")
                .accessibilityIdentifier("HomeComposerSend")
            }
            .padding(12)
            .glassEffect(
                .regular.interactive(),
                in: .rect(cornerRadius: tokens.composerRadius)
            )
        }
    }
}

private struct RecentChatBadges: View {
    let chats: [ChatDestination]
    let spacing: CGFloat
    let openChat: (ChatDestination) -> Void

    var body: some View {
        VStack(spacing: 10) {
            Text("Recent")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                .textCase(.uppercase)
                .tracking(0.7)

            ScrollView(.horizontal) {
                GlassEffectContainer(spacing: spacing) {
                    HStack(spacing: spacing) {
                        ForEach(chats) { chat in
                            Button {
                                openChat(chat)
                            } label: {
                                Label(chat.title, systemImage: "bubble.left")
                                    .font(.subheadline.weight(.medium))
                                    .lineLimit(1)
                                    .frame(width: 82, alignment: .leading)
                            }
                            .buttonStyle(.glass)
                            .controlSize(.small)
                            .accessibilityLabel("Recent chat: \(chat.title)")
                            .accessibilityIdentifier("RecentChat-\(chat.chatID)")
                        }
                    }
                    .padding(.horizontal, 2)
                }
            }
            .scrollIndicators(.hidden)
            .contentMargins(.horizontal, 1, for: .scrollContent)
        }
        .frame(maxWidth: .infinity)
    }
}

struct ChatDrawerOverlay: View {
    @Environment(\.finiteTokens) private var tokens
    let agentName: String
    let groups: [ChatTopicGroup]
    let selectedChatID: String?
    let dismiss: () -> Void
    let openChat: (ChatDestination) -> Void

    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Color.black.opacity(0.24)
                    .ignoresSafeArea()
                    .contentShape(Rectangle())
                    .onTapGesture(perform: dismiss)

                VStack(alignment: .leading, spacing: 0) {
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(agentName)
                                .font(.headline)
                            Text("Chats")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button(action: dismiss) {
                            Image(systemName: "xmark")
                        }
                        .buttonStyle(.glass)
                        .accessibilityLabel("Close chats")
                    }
                    .padding()

                    Divider()

                    if groups.isEmpty {
                        ContentUnavailableView("No chats yet", systemImage: "bubble.left")
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else {
                        List {
                            ForEach(groups) { group in
                                Section(group.title) {
                                    ForEach(group.chats) { chat in
                                        Button {
                                            openChat(chat)
                                        } label: {
                                            HStack {
                                                VStack(alignment: .leading, spacing: 3) {
                                                    Text(chat.title)
                                                        .lineLimit(1)
                                                    if !chat.preview.isEmpty {
                                                        Text(chat.preview)
                                                            .font(.caption)
                                                            .foregroundStyle(.secondary)
                                                            .lineLimit(1)
                                                    }
                                                }
                                                Spacer()
                                                if selectedChatID == chat.chatID {
                                                    Image(systemName: "checkmark")
                                                        .foregroundStyle(.tint)
                                                }
                                            }
                                        }
                                        .buttonStyle(.plain)
                                    }
                                }
                            }
                        }
                        .listStyle(.sidebar)
                        .scrollContentBackground(.hidden)
                    }
                }
                .frame(width: min(tokens.drawerWidth, proxy.size.width - 32))
                .frame(maxHeight: .infinity)
                .glassEffect(.regular, in: .rect(cornerRadius: tokens.panelRadius))
                .padding(.vertical, 8)
                .padding(.leading, 8)
                .shadow(color: .black.opacity(0.16), radius: 24, x: 8)
            }
        }
        .accessibilityIdentifier("ChatDrawer")
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
            startChat: { _, completion in completion(true) },
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
            startChat: { _, completion in completion(true) },
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
            startChat: { _, completion in completion(true) },
            openChat: { _ in },
            chooseAgent: {}
        )
    }
}

#Preview("Chat drawer") {
    ChatDrawerOverlay(
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
