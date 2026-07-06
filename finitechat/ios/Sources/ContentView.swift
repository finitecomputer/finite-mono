import CoreImage.CIFilterBuiltins
import Photos
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

private enum AppSheet: Identifiable {
    case newChat
    case myProfile
    case scan
    case invite
    case settings

    var id: String {
        switch self {
        case .newChat:
            "newChat"
        case .myProfile:
            "myProfile"
        case .scan:
            "scan"
        case .invite:
            "invite"
        case .settings:
            "settings"
        }
    }
}

struct ContentView: View {
    @Environment(\.scenePhase) private var scenePhase
    @ObservedObject var model: AppModel
    @StateObject private var people = NostrPeopleModel()
    @State private var selectedTab: AppTab = .home
    @State private var sheet: AppSheet?
    @State private var chatPath: [String] = []
    @State private var lastAppliedSelectedRoomID: String?
    @State private var scheduledRoomRouteID: String?

    var body: some View {
        Group {
            if model.requiresNostrLogin {
                NostrLoginView(model: model)
            } else {
                authenticatedShell
            }
        }
        .sheet(item: $sheet) { destination in
            switch destination {
            case .newChat:
                ChatPeoplePickerSheet(
                    model: model,
                    people: people,
                    existingRoom: nil,
                    onOpenAgents: {
                        sheet = nil
                        selectedTab = .agents
                    }
                ) { room in
                    routeSelectedRoom(room.roomId)
                }
            case .myProfile:
                MyNostrProfileSheet(
                    identity: model.nostrIdentity,
                    myNpub: model.myNpub,
                    accountID: model.activeAccountID,
                    profile: model.myProfile,
                    serverURL: model.serverURL,
                    showsSecretKey: false,
                    onUploadImage: { data, mimeType in
                        await model.uploadImage(data: data, mimeType: mimeType)
                    }
                ) { displayName, about, picture in
                    await model.saveMyProfile(
                        displayName: displayName,
                        about: about,
                        picture: picture
                    )
                }
            case .scan:
                ScanSheet(model: model) { profile in
                    startChatFromScannedProfile(profile)
                }
            case .invite:
                InviteSheet(invite: model.state?.activeInvite)
            case .settings:
                SettingsSheet(model: model) { profile in
                    startChatFromScannedProfile(profile)
                }
            }
        }
        .task(id: model.requiresNostrLogin) {
            startRuntimeIfAuthenticated()
        }
        .onChange(of: scenePhase) { _, phase in
            guard phase == .active else { return }
            startRuntimeFromForegroundIfAuthenticated()
        }
        .onChange(of: model.requiresNostrLogin) { _, requiresLogin in
            if requiresLogin {
                lastAppliedSelectedRoomID = nil
                scheduledRoomRouteID = nil
                schedulePathUpdate([])
            } else {
                lastAppliedSelectedRoomID = model.state?.selectedRoomId
            }
        }
        .onChange(of: model.state?.selectedRoomId) { _, selectedRoomID in
            guard !model.requiresNostrLogin else { return }
            routeSelectedRoomIfNeeded(selectedRoomID)
        }
    }

    private func startRuntimeIfAuthenticated() {
        guard !model.requiresNostrLogin else { return }
        model.startFromForeground()
        lastAppliedSelectedRoomID = model.state?.selectedRoomId
    }

    private func startRuntimeFromForegroundIfAuthenticated() {
        guard !model.requiresNostrLogin else { return }
        model.startFromForeground()
        lastAppliedSelectedRoomID = model.state?.selectedRoomId
    }

    @ViewBuilder
    private var authenticatedShell: some View {
        switch selectedTab {
        case .home:
            homeStack
        case .chats, .people, .agents:
            tabbedShell
        }
    }

    private var tabbedShell: some View {
        TabView(selection: $selectedTab) {
            chatsStack
                .tabItem {
                    Label(AppTab.chats.title, systemImage: AppTab.chats.systemImage)
                        .accessibilityIdentifier(AppTab.chats.accessibilityIdentifier)
                }
                .tag(AppTab.chats)

            peopleStack
                .tabItem {
                    Label(AppTab.people.title, systemImage: AppTab.people.systemImage)
                        .accessibilityIdentifier(AppTab.people.accessibilityIdentifier)
                }
                .tag(AppTab.people)

            agentsStack
                .tabItem {
                    Label(AppTab.agents.title, systemImage: AppTab.agents.systemImage)
                        .accessibilityIdentifier(AppTab.agents.accessibilityIdentifier)
                }
                .tag(AppTab.agents)

            Color.clear
                .accessibilityHidden(true)
                .tabItem {
                    Label(AppTab.home.title, systemImage: AppTab.home.systemImage)
                        .accessibilityIdentifier(AppTab.home.accessibilityIdentifier)
                }
                .tag(AppTab.home)
        }
    }

    private var chatsStack: some View {
        NavigationStack(path: $chatPath) {
            RoomListView(
                model: model,
                people: people,
                present: { destination in
                    sheet = destination
                },
                open: { room in
                    model.openRoom(room)
                    routeSelectedRoom(room.roomId)
                },
                openAgents: {
                    selectedTab = .agents
                }
            )
            .navigationDestination(for: String.self) { roomID in
                RoomThreadView(model: model, people: people, roomID: roomID) {
                    sheet = .invite
                }
                .toolbar(.hidden, for: .tabBar)
            }
        }
    }

    private var peopleStack: some View {
        NavigationStack {
            PeopleView(
                model: model,
                people: people,
                startProfileChat: { profile in
                    startChatFromScannedProfile(profile)
                },
                showMyProfile: {
                    sheet = .myProfile
                },
                showNewChat: {
                    sheet = .newChat
                },
                showScan: {
                    sheet = .scan
                },
                showSettings: {
                    sheet = .settings
                }
            )
        }
    }

    private var agentsStack: some View {
        NavigationStack {
            AgentsView(
                model: model,
                openRoom: { room in
                    selectedTab = .chats
                    model.openRoom(room)
                    routeSelectedRoom(room.roomId)
                },
                showSettings: {
                    sheet = .settings
                }
            )
        }
    }

    private var homeStack: some View {
        NavigationStack {
            HomeView(
                model: model,
                openChats: {
                    selectedTab = .chats
                },
                openPeople: {
                    sheet = .newChat
                },
                openAgents: {
                    selectedTab = .agents
                },
                openRoom: { room in
                    model.openRoom(room)
                    routeSelectedRoom(room.roomId)
                },
                showScan: {
                    sheet = .scan
                },
                showSettings: {
                    sheet = .settings
                }
            )
        }
    }

    private func routeSelectedRoomIfNeeded(_ selectedRoomID: String?) {
        guard let selectedRoomID else {
            lastAppliedSelectedRoomID = nil
            scheduledRoomRouteID = nil
            schedulePathUpdate([])
            return
        }
        guard selectedRoomID != lastAppliedSelectedRoomID else { return }
        lastAppliedSelectedRoomID = selectedRoomID
        scheduledRoomRouteID = selectedRoomID
        selectedTab = .chats
        schedulePathUpdate([selectedRoomID])
    }

    private func routeSelectedRoom(_ selectedRoomID: String) {
        lastAppliedSelectedRoomID = selectedRoomID
        scheduledRoomRouteID = selectedRoomID
        selectedTab = .chats
        schedulePathUpdate([selectedRoomID])
    }

    @discardableResult
    private func startChatFromScannedProfile(_ profile: AppProfileSummary) -> Bool {
        let queued = model.startProfileChat(for: profile) { room in
            routeSelectedRoom(room.roomId)
            sheet = nil
        }
        if queued {
            sheet = nil
        }
        return queued
    }

    private func schedulePathUpdate(_ nextPath: [String]) {
        Task { @MainActor in
            if let expectedRouteID = nextPath.last,
               scheduledRoomRouteID != expectedRouteID
            {
                return
            }
            if nextPath.isEmpty, scheduledRoomRouteID != nil {
                return
            }
            guard chatPath != nextPath else { return }
            chatPath = nextPath
        }
    }
}

private struct RoomListView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var people: NostrPeopleModel
    let present: (AppSheet) -> Void
    let open: (AppRoomSummary) -> Void
    let openAgents: () -> Void
    @State private var searchText = ""
    @State private var showingNewRoom = false

    private var filteredRooms: [AppRoomSummary] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else { return model.rooms }
        return model.rooms.filter { room in
            room.displayName.lowercased().contains(query)
                || room.lastMessagePreview.lowercased().contains(query)
                || room.userStatusText.lowercased().contains(query)
        }
    }

    var body: some View {
        List {
            if model.rooms.isEmpty {
                VStack(spacing: 14) {
                    ContentUnavailableView(
                        "No chats yet",
                        systemImage: "bubble.left.and.text.bubble",
                        description: Text("Start a chat from a profile code or your Nostr follows.")
                    )

                    HStack(spacing: 10) {
                        Button {
                            showingNewRoom = true
                        } label: {
                            Label("New Chat", systemImage: "person.badge.plus")
                        }
                        .buttonStyle(.borderedProminent)
                        .accessibilityIdentifier("EmptyChatsNewChatButton")

                        Button {
                            present(.scan)
                        } label: {
                            Label("Scan", systemImage: "qrcode.viewfinder")
                        }
                        .buttonStyle(.bordered)
                        .accessibilityIdentifier("EmptyChatsScanButton")
                    }
                }
                .padding(.vertical, 28)
                .frame(maxWidth: .infinity)
                .listRowSeparator(.hidden)
            } else if filteredRooms.isEmpty {
                ContentUnavailableView("No matching chats", systemImage: "magnifyingglass")
                    .padding(.vertical, 28)
                    .frame(maxWidth: .infinity)
                    .listRowSeparator(.hidden)
            } else {
                ForEach(Array(filteredRooms.enumerated()), id: \.element.roomId) { index, room in
                    Button {
                        open(room)
                    } label: {
                        RoomRow(room: room)
                    }
                    .buttonStyle(.plain)
                    .listRowSeparator(index == 0 ? .hidden : .visible, edges: .top)
                    .accessibilityIdentifier("RoomRow-\(room.roomId)")
                }
            }
        }
        .listStyle(.plain)
        .navigationTitle("Chats")
        .listNavigationBarChrome()
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button {
                    present(.myProfile)
                } label: {
                    Label("My profile code", systemImage: "person.crop.circle")
                        .labelStyle(.iconOnly)
                }
                .accessibilityIdentifier("ChatsMyProfileButton")
            }

            ToolbarItemGroup(placement: .topBarTrailing) {
                Button {
                    present(.scan)
                } label: {
                    Label("Scan code", systemImage: "qrcode.viewfinder")
                        .labelStyle(.iconOnly)
                }
                .accessibilityIdentifier("ChatsScanButton")

                Button {
                    showingNewRoom = true
                } label: {
                    Label("New chat", systemImage: "plus")
                        .labelStyle(.iconOnly)
                }
                .accessibilityIdentifier("ChatsNewChatButton")

                Button {
                    present(.settings)
                } label: {
                    Label("Settings", systemImage: "gearshape")
                        .labelStyle(.iconOnly)
                }
                .accessibilityIdentifier("TopSettingsButton")
            }
        }
        .searchable(
            text: $searchText,
            placement: .navigationBarDrawer(displayMode: .automatic),
            prompt: "Search chats"
        )
        .sheet(isPresented: $showingNewRoom) {
            ChatPeoplePickerSheet(
                model: model,
                people: people,
                existingRoom: nil,
                onOpenAgents: openAgents
            ) { room in
                open(room)
            }
        }
        .safeAreaInset(edge: .bottom) {
            NoticeBar(text: model.userNoticeText)
        }
    }
}

private struct ChatPeoplePickerSheet: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var model: AppModel
    @ObservedObject var people: NostrPeopleModel
    let existingRoom: AppRoomSummary?
    var onOpenAgents: (() -> Void)?
    let onCreated: (AppRoomSummary) -> Void

    private enum NewConversationMode: String, CaseIterable, Identifiable {
        case chat
        case group

        var id: String { rawValue }

        var title: String {
            switch self {
            case .chat:
                return "Chat"
            case .group:
                return "Group"
            }
        }
    }

    @State private var roomName = ""
    @State private var selectedProfiles: [AppProfileSummary] = []
    @State private var searchText = ""
    @State private var conversationMode: NewConversationMode = .chat
    @State private var showingScan = false
    @State private var parseError: String?
    @FocusState private var focused: Bool

    private var selfAccountID: String? {
        model.nostrIdentity?.accountID ?? model.state?.identity.accountId
    }

    private var selectedIDs: Set<String> {
        Set(selectedProfiles.map(\.accountId))
    }

    private var existingMemberAccountIDs: Set<String> {
        guard let existingRoom,
              let details = model.state?.roomDetails,
              details.roomId == existingRoom.roomId
        else {
            return []
        }
        return Set(details.members.map(\.accountId))
    }

    private var filteredFollowProfiles: [NostrFollowProfile] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let existingMembers = existingMemberAccountIDs
        return people.profiles
            .filter { profile in
                profile.pubkey != selfAccountID
                    && (isCreatingGroup || !selectedIDs.contains(profile.pubkey))
                    && !existingMembers.contains(profile.pubkey)
            }
            .filter { profile in
                guard !query.isEmpty else { return true }
                return profile.displayName.lowercased().contains(query)
                    || profile.npub.lowercased().contains(query)
                    || profile.pubkey.lowercased().contains(query)
                    || (profile.about?.lowercased().contains(query) ?? false)
            }
    }

    private var filteredKnownProfiles: [AppProfileSummary] {
        let followIDs = Set(people.profiles.map(\.pubkey))
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let existingMembers = existingMemberAccountIDs
        return (model.state?.profiles ?? [])
            .filter { profile in
                profile.accountId != selfAccountID
                    && (isCreatingGroup || !selectedIDs.contains(profile.accountId))
                    && !existingMembers.contains(profile.accountId)
                    && !followIDs.contains(profile.accountId)
            }
            .filter { profile in
                guard !query.isEmpty else { return true }
                return profile.displayName.lowercased().contains(query)
                    || profile.npub.lowercased().contains(query)
                    || profile.accountId.lowercased().contains(query)
                    || (profile.about?.lowercased().contains(query) ?? false)
            }
            .sorted {
                $0.displayName.localizedCaseInsensitiveCompare($1.displayName) == .orderedAscending
            }
    }

    private var allPickerProfiles: [AppProfileSummary] {
        let followProfiles = filteredFollowProfiles.map(\.appProfileSummary)
        var seen = Set<String>()
        var combined: [AppProfileSummary] = []
        for profile in followProfiles + filteredKnownProfiles {
            if seen.insert(profile.accountId).inserted {
                combined.append(profile)
            }
        }
        return combined.sorted {
            $0.displayName.localizedCaseInsensitiveCompare($1.displayName) == .orderedAscending
        }
    }

    private var groupedPickerProfiles: [(letter: String, profiles: [AppProfileSummary])] {
        let grouped = Dictionary(grouping: allPickerProfiles) { profile in
            guard let first = profile.displayName.first else { return "#" }
            let letter = String(first).uppercased()
            return first.isLetter ? letter : "#"
        }
        return grouped.keys.sorted { lhs, rhs in
            if lhs == "#" { return false }
            if rhs == "#" { return true }
            return lhs.localizedStandardCompare(rhs) == .orderedAscending
        }
        .map { (letter: $0, profiles: grouped[$0] ?? []) }
    }

    private var isCreatingGroup: Bool {
        existingRoom == nil && conversationMode == .group
    }

    private var sheetTitle: String {
        if existingRoom != nil {
            return "Add People"
        }
        if conversationMode == .group {
            return "New Group"
        }
        return "New Chat"
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    newChatSearchField

                    if let notice = model.actionNoticeText {
                        newChatNoticeBanner(notice)
                    }

                    if let parseError {
                        newChatErrorBanner(parseError)
                    }

                    if existingRoom == nil {
                        if conversationMode == .group {
                            groupMemberActionsCard
                        } else {
                            newChatActionsCard
                        }
                    } else {
                        addPeopleActionsCard
                    }

                    if showsGroupNameField {
                        newChatGroupNameCard
                    }

                    if !selectedProfiles.isEmpty, !isCreatingGroup {
                        selectedPeopleCard
                    }

                    peopleListContent
                }
                .padding(.horizontal, 16)
                .padding(.top, 8)
                .padding(.bottom, isCreatingGroup ? 8 : 24)
                .animation(nil, value: selectedIDs)
            }
            .background(Color(.systemGroupedBackground))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .principal) {
                    Text(sheetTitle)
                        .font(.headline)
                }

                if isCreatingGroup {
                    ToolbarItem(placement: .topBarLeading) {
                        Button {
                            exitGroupMode()
                        } label: {
                            Image(systemName: "chevron.left")
                                .font(.body.weight(.semibold))
                        }
                        .accessibilityLabel("Back")
                    }
                }

                ToolbarItem(placement: .topBarTrailing) {
                    if existingRoom != nil {
                        Button(primaryActionTitle, action: create)
                            .disabled(primaryActionDisabled)
                            .accessibilityIdentifier("NewRoomCreateButton")
                    }
                }

                ToolbarItem(placement: .topBarTrailing) {
                    GlassCircleCloseButton { dismiss() }
                }
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if isCreatingGroup {
                    groupCreateFloatingButton
                }
            }
            .task(id: "\(model.activeAccountID ?? "")|\(model.serverURL)") {
                await people.loadIfNeeded(accountID: model.activeAccountID, serverURL: model.serverURL)
            }
            .refreshable {
                await people.refresh(accountID: model.activeAccountID, serverURL: model.serverURL)
            }
            .sheet(isPresented: $showingScan) {
                ScanSheet(
                    model: model,
                    onStartProfileChat: handleScannedProfile,
                    onRoomJoined: { dismiss() }
                )
            }
            .task {
                focused = existingRoom == nil && conversationMode == .group
            }
            .onChange(of: conversationMode) { _, mode in
                parseError = nil
                roomName = ""
                if mode == .chat && selectedProfiles.count > 1 {
                    selectedProfiles = Array(selectedProfiles.prefix(1))
                }
            }
        }
    }

    private var newChatSearchField: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)

            TextField("Name, username, or number", text: $searchText)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .accessibilityIdentifier("NewRoomSearchField")
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 11)
        .background(Color(.systemBackground), in: Capsule())
        .shadow(color: .black.opacity(0.06), radius: 8, y: 2)
    }

    private var showsGroupNameLabel: Bool {
        focused || !roomName.isEmpty
    }

    private var newChatGroupNameCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            if showsGroupNameLabel {
                Text("Group name")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 4)
            }

            NewChatCard {
                TextField("Group name", text: $roomName)
                    .focused($focused)
                    .submitLabel(.done)
                    .onSubmit(create)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
                    .accessibilityIdentifier("NewRoomNameField")
            }

            if !selectedProfiles.isEmpty {
                groupSelectedMemberChips
            }
        }
    }

    private var groupSelectedMemberChips: some View {
        LazyVGrid(
            columns: [GridItem(.adaptive(minimum: 112), spacing: 8, alignment: .leading)],
            alignment: .leading,
            spacing: 8
        ) {
            ForEach(selectedProfiles, id: \.accountId) { profile in
                GroupMemberChip(profile: profile) {
                    removeProfile(profile)
                }
            }
        }
        .accessibilityIdentifier("NewRoomSelectedMembersStrip")
    }

    private var groupCreateFloatingButton: some View {
        GroupCreateFloatingButton(
            title: "Create",
            isDisabled: primaryActionDisabled,
            action: create
        )
        .accessibilityIdentifier("NewRoomCreateButton")
    }

    private var groupMemberActionsCard: some View {
        NewChatCard {
            NewChatActionRow(
                title: "Paste",
                systemImage: "doc.on.clipboard"
            ) {
                addCode(
                    UIPasteboard.general.string ?? "",
                    startDirectChatIfPossible: false
                )
            }
            .accessibilityIdentifier("NewRoomPasteProfileButton")

            NewChatCardDivider()

            NewChatActionRow(
                title: "Scan",
                systemImage: "qrcode.viewfinder"
            ) {
                showingScan = true
            }
            .accessibilityIdentifier("NewRoomScanProfileButton")
        }
    }

    private func newChatNoticeBanner(_ notice: String) -> some View {
        Label(notice, systemImage: "exclamationmark.circle")
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 4)
    }

    private func newChatErrorBanner(_ message: String) -> some View {
        Label(message, systemImage: "exclamationmark.triangle")
            .font(.subheadline)
            .foregroundStyle(.red)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 4)
    }

    private var newChatActionsCard: some View {
        NewChatCard {
            NewChatActionRow(
                title: "New group",
                systemImage: "person.2",
                isActive: conversationMode == .group
            ) {
                conversationMode = .group
            }
            .accessibilityIdentifier("NewRoomConversationModePicker")

            NewChatCardDivider()

            NewChatActionRow(
                title: "New agent chat",
                systemImage: "sparkles"
            ) {
                dismiss()
                onOpenAgents?()
            }

            NewChatCardDivider()

            NewChatActionRow(
                title: "Paste",
                systemImage: "doc.on.clipboard"
            ) {
                addCode(
                    UIPasteboard.general.string ?? "",
                    startDirectChatIfPossible: startsDirectChatFromEnteredCode
                )
            }
            .accessibilityIdentifier("NewRoomPasteProfileButton")

            NewChatCardDivider()

            NewChatActionRow(
                title: "Scan",
                systemImage: "qrcode.viewfinder"
            ) {
                showingScan = true
            }
            .accessibilityIdentifier("NewRoomScanProfileButton")
        }
    }

    private var addPeopleActionsCard: some View {
        NewChatCard {
            NewChatActionRow(
                title: "Paste",
                systemImage: "doc.on.clipboard"
            ) {
                addCode(
                    UIPasteboard.general.string ?? "",
                    startDirectChatIfPossible: startsDirectChatFromEnteredCode
                )
            }
            .accessibilityIdentifier("NewRoomPasteProfileButton")

            NewChatCardDivider()

            NewChatActionRow(
                title: "Scan",
                systemImage: "qrcode.viewfinder"
            ) {
                showingScan = true
            }
            .accessibilityIdentifier("NewRoomScanProfileButton")
        }
    }

    private var selectedPeopleCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Selected")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 4)

            NewChatCard {
                ForEach(Array(selectedProfiles.enumerated()), id: \.element.accountId) { index, profile in
                    if index > 0 {
                        NewChatCardDivider()
                    }

                    HStack(spacing: 12) {
                        NewChatPersonRow(profile: profile)

                        Button(role: .destructive) {
                            removeProfile(profile)
                        } label: {
                            Image(systemName: "minus.circle.fill")
                                .foregroundStyle(.red)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Remove \(profile.displayName)")
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                }

                if existingRoom != nil || conversationMode == .group {
                    NewChatCardDivider()

                    Button(action: create) {
                        Label(primaryActionLabel, systemImage: primaryActionSystemImage)
                            .frame(maxWidth: .infinity, alignment: .center)
                            .padding(.vertical, 12)
                    }
                    .buttonStyle(.plain)
                    .disabled(primaryActionDisabled)
                    .accessibilityIdentifier("NewRoomPrimaryActionButton")
                }
            }
        }
    }

    @ViewBuilder
    private var peopleListContent: some View {
        if people.isLoading && people.profiles.isEmpty && filteredKnownProfiles.isEmpty {
            HStack {
                Spacer()
                ProgressView("Loading people...")
                Spacer()
            }
            .padding(.vertical, 32)
        } else if groupedPickerProfiles.isEmpty {
            ContentUnavailableView(
                searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    ? emptyPeopleTitle
                    : "No matches",
                systemImage: "person.crop.circle.badge.questionmark",
                description: Text(people.statusText ?? "Paste or scan a profile code.")
            )
            .frame(maxWidth: .infinity)
            .padding(.vertical, 24)
        } else if isCreatingGroup {
            groupContactsList
        } else {
            ForEach(groupedPickerProfiles, id: \.letter) { section in
                VStack(alignment: .leading, spacing: 8) {
                    Text(section.letter)
                        .font(.title3.weight(.bold))
                        .padding(.horizontal, 4)

                    NewChatCard {
                        ForEach(Array(section.profiles.enumerated()), id: \.element.accountId) { index, profile in
                            if index > 0 {
                                NewChatCardDivider()
                            }

                            Button {
                                chooseProfile(profile)
                            } label: {
                                HStack(spacing: 12) {
                                    NewChatPersonRow(profile: profile)

                                    if isCreatingGroup {
                                        GroupContactSelectionIndicator(
                                            isSelected: selectedIDs.contains(profile.accountId)
                                        )
                                    }
                                }
                                .padding(.horizontal, 16)
                                .padding(.vertical, 10)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
        }
    }

    private var groupContactsList: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let minimumGroupMembersHint {
                newChatNoticeBanner(minimumGroupMembersHint)
            }

            Text("Contacts")
                .font(.title3.weight(.bold))
                .padding(.horizontal, 4)

            NewChatCard {
                ForEach(Array(allPickerProfiles.enumerated()), id: \.element.accountId) { index, profile in
                    if index > 0 {
                        NewChatCardDivider()
                    }

                    Button {
                        chooseProfile(profile)
                    } label: {
                        HStack(spacing: 12) {
                            NewChatPersonRow(profile: profile)

                            GroupContactSelectionIndicator(
                                isSelected: selectedIDs.contains(profile.accountId)
                            )
                        }
                        .padding(.horizontal, 16)
                        .padding(.vertical, 10)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }

    private var trimmedName: String {
        roomName.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var primaryActionDisabled: Bool {
        if selectedProfiles.isEmpty {
            return true
        }
        if existingRoom != nil {
            return false
        }
        if conversationMode == .group {
            return trimmedName.isEmpty || selectedProfiles.count < 2
        }
        return selectedProfiles.count != 1
    }

    private var minimumGroupMembersHint: String? {
        guard isCreatingGroup, selectedProfiles.count == 1 else { return nil }
        return "Choose at least one more person for the group."
    }

    private var showsGroupNameField: Bool {
        existingRoom == nil && conversationMode == .group
    }

    private var primaryActionTitle: String {
        if existingRoom != nil {
            return "Add"
        }
        return conversationMode == .group ? "Create" : "Start"
    }

    private var primaryActionLabel: String {
        if existingRoom != nil {
            return "Add People"
        }
        return conversationMode == .group ? "Create Group Chat" : "Start Chat"
    }

    private var primaryActionSystemImage: String {
        if existingRoom != nil || conversationMode == .group {
            return "person.2.badge.plus"
        }
        return "bubble.left.and.bubble.right"
    }

    private var startsDirectChatFromEnteredCode: Bool {
        existingRoom == nil && conversationMode == .chat && selectedProfiles.isEmpty
    }

    @discardableResult
    private func handleScannedProfile(_ profile: AppProfileSummary) -> Bool {
        guard addProfile(profile) else { return false }
        if startsDirectChatFromEnteredCode {
            create()
        }
        return true
    }

    private func addCode(
        _ rawValue: String,
        startDirectChatIfPossible: Bool = false
    ) {
        let trimmed = rawValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        do {
            let target = try profileTarget(from: trimmed)
            let npub = target.npub
            let accountID = target.accountID
            if accountID == selfAccountID {
                parseError = "That is your own profile code."
                return
            }
            if existingMemberAccountIDs.contains(accountID) {
                parseError = "That person is already in this chat."
                return
            }
            let profile = profileSummary(accountID: accountID, npub: npub)
            guard addProfile(profile) else { return }
            parseError = nil
            if startDirectChatIfPossible {
                create()
            }
        } catch {
            parseError = "That code is not a valid profile code."
        }
    }

    private func chooseProfile(_ profile: AppProfileSummary) {
        if isCreatingGroup, selectedIDs.contains(profile.accountId) {
            removeProfile(profile)
            return
        }
        let shouldStartDirectChat = existingRoom == nil
            && conversationMode == .chat
            && selectedProfiles.isEmpty
        guard addProfile(profile) else { return }
        if shouldStartDirectChat {
            create()
        }
    }

    @discardableResult
    private func addProfile(_ profile: AppProfileSummary) -> Bool {
        guard profile.accountId != selfAccountID else {
            parseError = "That is your own profile code."
            return false
        }
        guard !existingMemberAccountIDs.contains(profile.accountId) else {
            parseError = "That person is already in this chat."
            return false
        }
        guard !selectedIDs.contains(profile.accountId) else {
            parseError = nil
            return false
        }
        withoutSelectionAnimation {
            selectedProfiles.append(profile)
        }
        parseError = nil
        return true
    }

    private func removeProfile(_ profile: AppProfileSummary) {
        withoutSelectionAnimation {
            selectedProfiles.removeAll { $0.accountId == profile.accountId }
        }
    }

    private func withoutSelectionAnimation(_ action: () -> Void) {
        var transaction = Transaction()
        transaction.disablesAnimations = true
        withTransaction(transaction, action)
    }

    private func exitGroupMode() {
        conversationMode = .chat
        roomName = ""
        selectedProfiles = []
        parseError = nil
        searchText = ""
    }

    private func profileSummary(accountID: String, npub: String) -> AppProfileSummary {
        if let profile = model.state?.profiles.first(where: { $0.accountId == accountID }) {
            return profile
        }
        if let follow = people.profiles.first(where: { $0.pubkey == accountID }) {
            return follow.appProfileSummary
        }
        return AppProfileSummary(
            accountId: accountID,
            npub: npub,
            displayName: shortenedDisplayNpub(npub),
            about: nil,
            picture: nil,
            stale: true,
            isAgent: false
        )
    }

    private var emptyPeopleTitle: String {
        existingRoom == nil ? "No people yet" : "No other people found"
    }

    private func create() {
        guard !selectedProfiles.isEmpty else { return }
        if isCreatingGroup {
            guard !trimmedName.isEmpty, selectedProfiles.count >= 2 else { return }
        }
        if let existingRoom {
            guard model.addMembers(to: existingRoom, profiles: selectedProfiles, onSuccess: {
                dismiss()
            }) else { return }
        } else {
            guard model.startNewChat(named: trimmedName, with: selectedProfiles, onCreated: { room in
                onCreated(room)
                dismiss()
            }) else { return }
        }
    }

}

private enum NewGroupChatCodeError: Error {
    case missingNpub
}

@MainActor
func profileSummaryFromScannedProfileCode(
    _ value: String,
    model: AppModel
) throws -> AppProfileSummary {
    let target = try profileTarget(from: value)
    let accountID = target.accountID
    let npub = target.npub
    if let profile = model.state?.profiles.first(where: { $0.accountId == accountID }) {
        return profile
    }
    return AppProfileSummary(
        accountId: accountID,
        npub: npub,
        displayName: shortenedDisplayNpub(npub),
        about: nil,
        picture: nil,
        stale: true,
        isAgent: false
    )
}

func isFiniteChatInviteCode(_ value: String) -> Bool {
    value
        .trimmingCharacters(in: .whitespacesAndNewlines)
        .lowercased()
        .hasPrefix("finite://join")
}

private struct ProfileCodeTarget {
    let accountID: String
    let npub: String
}

private func profileTarget(from value: String) throws -> ProfileCodeTarget {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    if isHexAccountID(trimmed) {
        let accountID = trimmed.lowercased()
        return ProfileCodeTarget(
            accountID: accountID,
            npub: try npubFromAccountId(accountId: accountID)
        )
    }

    let npub = try profileNpub(from: trimmed)
    return ProfileCodeTarget(
        accountID: try accountIdFromNpub(npub: npub),
        npub: npub
    )
}

func profileNpub(from value: String) throws -> String {
    let identifier = try profileNip19Identifier(from: value)
    if identifier.lowercased().hasPrefix("npub1") {
        return identifier
    }
    let accountID = try accountIdFromNpub(npub: identifier)
    return try npubFromAccountId(accountId: accountID)
}

private func profileNip19Identifier(from value: String) throws -> String {
    var candidate = value.trimmingCharacters(in: .whitespacesAndNewlines)
    if candidate.lowercased().hasPrefix("nostr:") {
        candidate = String(candidate.dropFirst("nostr:".count))
    }
    if isNostrProfileIdentifier(candidate) {
        return candidate
    }
    if let components = URLComponents(string: candidate) {
        for item in components.queryItems ?? [] {
            let itemName = item.name.lowercased()
            if (itemName == "npub" || itemName == "nprofile"),
               let value = item.value,
               isNostrProfileIdentifier(value)
            {
                return value
            }
        }
    }
    guard let range = firstProfileIdentifierRange(in: candidate) else {
        throw NewGroupChatCodeError.missingNpub
    }
    let suffix = candidate[range.lowerBound...]
    let separators = CharacterSet.whitespacesAndNewlines
        .union(CharacterSet(charactersIn: "\"'<>&#?/\\"))
    let identifier = suffix.prefix { character in
        !character.unicodeScalars.contains { scalar in
            separators.contains(scalar)
        }
    }
    return String(identifier)
}

private func isNostrProfileIdentifier(_ value: String) -> Bool {
    let normalized = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    return normalized.hasPrefix("npub1") || normalized.hasPrefix("nprofile1")
}

private func firstProfileIdentifierRange(in value: String) -> Range<String.Index>? {
    let npubRange = value.range(of: "npub1", options: [.caseInsensitive])
    let nprofileRange = value.range(of: "nprofile1", options: [.caseInsensitive])
    switch (npubRange, nprofileRange) {
    case (.some(let left), .some(let right)):
        return left.lowerBound < right.lowerBound ? left : right
    case (.some(let range), .none), (.none, .some(let range)):
        return range
    case (.none, .none):
        return nil
    }
}

private func isHexAccountID(_ value: String) -> Bool {
    guard value.count == 64 else { return false }
    return value.allSatisfy(\.isHexDigit)
}

private let newChatCardCornerRadius: CGFloat = 28

private struct GroupContactSelectionIndicator: View {
    let isSelected: Bool

    var body: some View {
        ZStack {
            Image(systemName: "circle")
                .font(.title2)
                .foregroundStyle(Color(.tertiaryLabel))
                .opacity(isSelected ? 0 : 1)

            Image(systemName: "checkmark.circle.fill")
                .font(.title2)
                .foregroundStyle(Color.accentColor)
                .opacity(isSelected ? 1 : 0)
        }
        .frame(width: 28, height: 28)
        .accessibilityHidden(true)
    }
}

private struct GroupMemberChip: View {
    let profile: AppProfileSummary
    let onRemove: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            ProfileAvatar(profile: profile, size: 28)

            Text(profile.displayName)
                .font(.subheadline)
                .foregroundStyle(.primary)
                .lineLimit(1)

            Button(action: onRemove) {
                Image(systemName: "xmark")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .padding(4)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Remove \(profile.displayName)")
        }
        .padding(.leading, 4)
        .padding(.trailing, 8)
        .padding(.vertical, 4)
        .background(Color(.secondarySystemGroupedBackground), in: Capsule())
    }
}

private struct GroupCreateFloatingButton: View {
    let title: String
    let isDisabled: Bool
    let action: () -> Void

    private var fadeHeight: CGFloat {
        MessageCollectionLayout.groupCreateFadeHeight(
            safeAreaBottom: BottomSafeAreaInsets.current
        )
    }

    var body: some View {
        VStack(spacing: 0) {
            Button(action: action) {
                Text(title)
                    .font(.body.weight(.semibold))
                    .frame(maxWidth: .infinity)
            }
            .modifier(GroupCreateFloatingButtonStyle())
            .disabled(isDisabled)
        }
        .padding(.horizontal, 16)
        .padding(.top, 12)
        .safeAreaPadding(.bottom, 8)
        .background(alignment: .bottom) {
            BottomEdgeBlurFade(height: fadeHeight)
                .frame(height: fadeHeight)
                .allowsHitTesting(false)
        }
        .background(Color.clear)
        .ignoresSafeArea(edges: .bottom)
    }
}

private struct GroupCreateFloatingButtonStyle: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26.0, *) {
            content
                .buttonStyle(.glassProminent)
                .buttonBorderShape(.capsule)
                .controlSize(.large)
                .tint(.accentColor)
        } else {
            content
                .buttonStyle(.borderedProminent)
                .buttonBorderShape(.capsule)
                .controlSize(.large)
        }
    }
}

private struct NewChatCard<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        VStack(spacing: 0) {
            content
        }
        .background(
            Color(.systemBackground),
            in: RoundedRectangle(cornerRadius: newChatCardCornerRadius, style: .continuous)
        )
    }
}

private struct NewChatCardDivider: View {
    var body: some View {
        Divider()
            .padding(.leading, 58)
    }
}

private struct NewChatActionRow: View {
    let title: String
    let systemImage: String
    var isActive = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 14) {
                Image(systemName: systemImage)
                    .font(.body)
                    .foregroundStyle(isActive ? Color.accentColor : .primary)
                    .frame(width: 28)

                Text(title)
                    .foregroundStyle(.primary)

                Spacer()

                Image(systemName: "chevron.right")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

private struct NewChatPersonRow: View {
    let profile: AppProfileSummary

    var body: some View {
        HStack(spacing: 12) {
            ProfileAvatar(profile: profile)

            Text(profile.displayName)
                .foregroundStyle(.primary)
                .lineLimit(1)

            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .combine)
    }
}

private struct NewGroupFollowRow: View {
    let profile: NostrFollowProfile

    var body: some View {
        HStack(spacing: 12) {
            ProfileAvatar(displayName: profile.displayName, pictureURL: profile.pictureURL, size: 40)
            .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 4) {
                Text(profile.displayName)
                    .foregroundStyle(.primary)
                    .lineLimit(1)

                Text(profile.about ?? profile.shortenedNpub)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)

                Label(profile.inviteAvailability.userStatusText, systemImage: statusSystemImage)
                    .font(.caption2)
                    .foregroundStyle(statusTint)
                    .lineLimit(1)
            }

            Spacer(minLength: 8)

            Image(systemName: "plus.circle")
                .foregroundStyle(.secondary)
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
    private var statusSystemImage: String {
        switch profile.inviteAvailability {
        case .available:
            return "checkmark.circle.fill"
        case .unavailable:
            return "exclamationmark.circle"
        case .unknown:
            return "clock"
        }
    }

    private var statusTint: Color {
        switch profile.inviteAvailability {
        case .available:
            return .green
        case .unavailable:
            return .orange
        case .unknown:
            return .secondary
        }
    }
}

private struct RoomRow: View {
    let room: AppRoomSummary

    var body: some View {
        HStack(spacing: 12) {
            RoomAvatar(room: room)

            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text(room.displayName)
                        .font(.body.weight(.semibold))
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    if room.unreadCount > 0 {
                        Text("\(room.unreadCount)")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 7)
                            .padding(.vertical, 3)
                            .background(Capsule().fill(Color.accentColor))
                    }
                }

                Text(rowSubtitle)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .padding(.vertical, 8)
        .accessibilityElement(children: .combine)
    }

    private var rowSubtitle: String {
        if !room.lastMessagePreview.isEmpty {
            return room.lastMessagePreview
        }
        switch room.state {
        case .connected:
            return "No messages yet"
        case .waitingForApproval, .joining, .unavailableOnDevice:
            return room.userStatusText
        }
    }
}

private struct RoomAvatar: View {
    let room: AppRoomSummary

    var body: some View {
        ZStack(alignment: .bottomTrailing) {
            ProfileAvatar(displayName: room.displayName, pictureURL: room.picture, size: 40)

            Circle()
                .fill(room.state.tint)
                .frame(width: 10, height: 10)
                .overlay(Circle().stroke(Color(.systemBackground), lineWidth: 2))
        }
        .frame(width: 40, height: 40)
        .accessibilityHidden(true)
    }

}

private struct RoomOptionsSheet: View {
    @Environment(\.dismiss) private var dismiss
    let showAddPeople: () -> Void
    let showInvite: () -> Void
    let showRoomDetails: () -> Void
    let showMediaGallery: () -> Void

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Button {
                        dismiss()
                        showAddPeople()
                    } label: {
                        SettingsRowLabel(
                            title: "Add people",
                            subtitle: nil,
                            systemImage: "person.badge.plus"
                        )
                    }

                    Button {
                        dismiss()
                        showInvite()
                    } label: {
                        SettingsRowLabel(
                            title: "Invite",
                            subtitle: nil,
                            systemImage: "qrcode"
                        )
                    }

                    Button {
                        dismiss()
                        showRoomDetails()
                    } label: {
                        SettingsRowLabel(
                            title: "Room info",
                            subtitle: nil,
                            systemImage: "info.circle"
                        )
                    }

                    Button {
                        dismiss()
                        showMediaGallery()
                    } label: {
                        SettingsRowLabel(
                            title: "Media gallery",
                            subtitle: nil,
                            systemImage: "photo.on.rectangle.angled"
                        )
                    }
                }
            }
            .navigationTitle("Room")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    GlassCircleCloseButton { dismiss() }
                }
            }
        }
    }
}

private struct RoomThreadView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var people: NostrPeopleModel
    let roomID: String
    let showInvite: () -> Void
    @State private var followsBottom = true
    @State private var importingAttachment = false
    @State private var replyDraftMessage: ChatMessage?
    @State private var focusedMessage: ChatMessage?
    @State private var focusedMessageFrame: CGRect = .zero
    @State private var focusedActionsVisible = false
    @State private var reactionPickerContext: ReactionPickerContext?
    @State private var composerFocused = false
    @State private var imagePreviewSelection: ChatImagePreviewSelection?
    @State private var videoPreviewItem: ChatAttachmentPreviewItem?
    @State private var documentPreviewItem: ChatAttachmentPreviewItem?
    @State private var showMediaGallery = false
    @State private var showRoomDetails = false
    @State private var showRoomOptions = false
    @State private var showAddPeople = false
    @State private var composerText = ""
    @State private var selectedPhotoItems: [PhotosPickerItem] = []
    @State private var stagedAttachments: [StagedComposerAttachment] = []
    @State private var showPhotoPicker = false
    @State private var pollComposerDraft: PollComposerDraft?
    @State private var siteBrowserItem: FiniteSiteBrowserItem?
    @StateObject private var voiceRecorder = VoiceRecorder()
    @State private var voiceSendInFlight = false

    private var room: AppRoomSummary? {
        model.state?.rooms.first(where: { $0.roomId == roomID })
    }

    private var projection: ChatRoomProjection {
        model.projection(for: roomID)
    }

    private var mediaGalleryItems: [ChatMediaGalleryItem] {
        guard let gallery = model.state?.mediaGallery,
              gallery.roomId == roomID
        else {
            return []
        }
        return gallery.items
    }

    private var roomDetails: AppRoomDetailsState? {
        guard let details = model.state?.roomDetails,
              details.roomId == roomID
        else {
            return nil
        }
        return details
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
        .navigationTitle(room?.displayName ?? "Chat")
        .navigationBarTitleDisplayMode(.inline)
        .chatNavigationBarChrome()
        .toolbar {
            if let room, room.state == .connected {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        showRoomOptions = true
                    } label: {
                        Image(systemName: "ellipsis.circle")
                    }
                    .accessibilityLabel("Room options")
                    .accessibilityIdentifier("RoomOptionsButton")
                }
            }
        }
        .sheet(isPresented: $showRoomOptions) {
            RoomOptionsSheet(
                showAddPeople: {
                    showRoomOptions = false
                    Task { @MainActor in
                        showAddPeople = true
                    }
                },
                showInvite: {
                    showRoomOptions = false
                    Task { @MainActor in
                        if let room {
                            _ = model.createInvite(for: room) {
                                showInvite()
                            }
                        }
                    }
                },
                showRoomDetails: {
                    showRoomOptions = false
                    Task { @MainActor in
                        showRoomDetails = true
                    }
                },
                showMediaGallery: {
                    showRoomOptions = false
                    Task { @MainActor in
                        showMediaGallery = true
                    }
                }
            )
            .presentationDetents([.medium])
        }
        .sheet(isPresented: $showAddPeople) {
            if let room {
                ChatPeoplePickerSheet(model: model, people: people, existingRoom: room) { _ in }
            } else {
                ContentUnavailableView("Room unavailable", systemImage: "exclamationmark.triangle")
            }
        }
        .navigationDestination(isPresented: $showRoomDetails) {
            RoomDetailsView(
                details: roomDetails,
                mediaItems: mediaGalleryItems,
                onDownloadAttachment: { item in
                    model.downloadAttachment(
                        roomID: roomID,
                        messageID: item.messageId,
                        attachment: item.attachment
                    )
                },
                onCreateInvite: {
                    if let room {
                        _ = model.createInvite(for: room) {
                            showInvite()
                        }
                    }
                },
                onAddPeople: {
                    showAddPeople = true
                },
                onRefreshDevices: {
                    model.refreshDevices()
                },
                onRevokeDevice: { device in
                    model.revokeDevice(device)
                },
                onUploadImage: { data, mimeType in
                    await model.uploadImage(data: data, mimeType: mimeType)
                },
                onSaveMetadata: { roomID, displayName, picture in
                    await model.saveRoomMetadata(
                        roomID: roomID,
                        displayName: displayName,
                        picture: picture
                    )
                }
            )
        }
        .navigationDestination(isPresented: $showMediaGallery) {
            ChatMediaGalleryView(
                roomTitle: room?.displayName ?? "this chat",
                items: mediaGalleryItems,
                onDownloadAttachment: { item in
                    model.downloadAttachment(
                        roomID: roomID,
                        messageID: item.messageId,
                        attachment: item.attachment
                    )
                }
            )
        }
        .onAppear {
            if let room {
                model.openRoom(room)
            }
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
        .onChange(of: composerText) { _, text in
            updateTypingIntent(text)
        }
    }

    @ViewBuilder
    private func messageSurface(room: AppRoomSummary) -> some View {
        switch room.state {
        case .connected:
            transcriptView(room: room) {
                composerAccessory
            }
        case .waitingForApproval:
            PendingRoomView(room: room, model: model)
        case .joining:
            ProgressView(room.userStatusText)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .unavailableOnDevice:
            UnavailableOnDeviceView(room: room)
        }
    }

    private func transcriptView<AccessoryContent: View>(
        room: AppRoomSummary,
        @ViewBuilder accessoryContent: () -> AccessoryContent
    ) -> some View {
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
            accessoryContent: accessoryContent(),
            canLoadOlder: room.canLoadOlder,
            onLoadOlderMessages: { beforeMessageID in
                model.loadOlderMessages(roomID: room.roomId, beforeMessageID: beforeMessageID)
            },
            followsBottom: $followsBottom
        )
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(.systemGroupedBackground))
        .ignoresSafeArea(edges: [.top, .bottom])
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
            Composer(
                text: $composerText,
                replyTarget: replyDraftMessage,
                canSubmit: model.canSend(roomID: roomID, text: composerText),
                stagedAttachments: $stagedAttachments,
                isPhotoPickerPresented: $showPhotoPicker,
                selectedPhotoItems: $selectedPhotoItems,
                isInputFocused: $composerFocused,
                reportError: { message in
                    model.errorText = message
                },
                onCancelReply: {
                    replyDraftMessage = nil
                },
                onSend: {
                    sendComposerDraft()
                },
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

    private func updateTypingIntent(_ text: String) {
        guard room?.state == .connected else { return }
        let isTyping = !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        model.setTyping(roomID: roomID, isTyping: isTyping)
    }

    private func markRoomReadIfNeeded() {
        guard let room, room.unreadCount > 0 else { return }
        model.markRoomRead(room)
    }

    private func sendComposerDraft() {
        if stagedAttachments.isEmpty {
            if model.send(roomID: roomID, text: composerText, replyTo: replyDraftMessage) {
                composerText = ""
                model.setTyping(roomID: roomID, isTyping: false)
                replyDraftMessage = nil
            }
            return
        }

        let outbound = stagedAttachments.map(\.outboundAttachment)
        model.sendAttachments(
            roomID: roomID,
            attachments: outbound,
            replyTo: replyDraftMessage,
            captionOverride: composerText
        ) {
            composerText = ""
            model.setTyping(roomID: roomID, isTyping: false)
            stagedAttachments = []
            selectedPhotoItems = []
            replyDraftMessage = nil
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
            ZStack {
                Color.black.opacity(0.18)
                    .ignoresSafeArea()
                    .contentShape(Rectangle())
                    .onTapGesture(perform: onDismiss)

                VStack(alignment: message.isMine ? .trailing : .leading, spacing: 10) {
                    if canReact {
                        FocusedReactionBar(onReact: onReact, onMore: onMoreReaction)
                    }

                    FocusedChatMessageCard(
                        message: message,
                        replyTarget: replyTarget
                    )
                    .frame(maxWidth: min(geometry.size.width * 0.82, 360))

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
                .frame(
                    maxWidth: .infinity,
                    maxHeight: .infinity,
                    alignment: message.isMine ? .topTrailing : .topLeading
                )
                .padding(.top, overlayTop(in: geometry))
                .padding(.horizontal, 20)
                .animation(.easeOut(duration: 0.16), value: actionsVisible)
            }
        }
    }

    private func overlayTop(in geometry: GeometryProxy) -> CGFloat {
        let overlayOriginY = geometry.frame(in: .global).minY
        let localAnchorY = anchorFrame.minY - overlayOriginY
        let reactionBarSpace: CGFloat = canReact ? 58 : 0
        let idealTop = localAnchorY - reactionBarSpace
        let maxTop = max(12, geometry.size.height * 0.58)
        return min(max(idealTop, 12), maxTop)
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
                .accessibilityLabel(isSubmitting ? "Requesting access" : room.userStatusText)
        }
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var isSubmitting: Bool {
        model.inviteJoinSubmissionRoomID == room.roomId
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

private struct ScanSheet: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var model: AppModel
    let onStartProfileChat: (AppProfileSummary) -> Bool
    var onRoomJoined: (() -> Void)? = nil
    @State private var scanError: String?

    var body: some View {
        NavigationStack {
            VStack(spacing: 12) {
                if let notice = model.actionNoticeText {
                    Label(notice, systemImage: "exclamationmark.circle")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 16)
                }

                QRCodeScannerPanel(expandsVertically: true) { value in
                    processScannedTarget(value)
                }
                .padding(.horizontal, 16)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .accessibilityIdentifier("ScanCameraScanner")
            }
            .padding(.top, 8)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(.systemGroupedBackground))
            .navigationTitle("Scan")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    GlassCircleCloseButton { dismiss() }
                }
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                VStack(spacing: 12) {
                    if let scanError {
                        Label(scanError, systemImage: "exclamationmark.triangle")
                            .font(.subheadline)
                            .foregroundStyle(.red)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }

                    if let profile = model.activeProfile {
                        VStack(alignment: .leading, spacing: 8) {
                            ProfileRow(profile: profile)
                            Button {
                                if onStartProfileChat(profile) {
                                    dismiss()
                                }
                            } label: {
                                Label("Start Chat", systemImage: "bubble.left.and.bubble.right")
                                    .frame(maxWidth: .infinity)
                            }
                            .buttonStyle(.bordered)
                        }
                    }

                    Button {
                        pasteAndContinue()
                    } label: {
                        Text(model.scanInFlight ? "Opening" : "Paste")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.large)
                    .buttonBorderShape(.capsule)
                    .disabled(model.scanInFlight)
                    .accessibilityIdentifier("ScanPasteButton")
                }
                .padding(.horizontal, 16)
                .padding(.top, 12)
                .padding(.bottom, 8)
                .background(Color(.systemGroupedBackground))
            }
        }
    }

    private func processScannedTarget(_ value: String) {
        scanError = nil
        model.scanDraft = value
        continueWithTarget()
    }

    private func pasteAndContinue() {
        let value = UIPasteboard.general.string ?? ""
        guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            scanError = "There is no invite or profile code on the clipboard."
            return
        }
        processScannedTarget(value)
    }

    private func continueWithTarget() {
        let value = model.scanDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else {
            scanError = nil
            dismiss()
            return
        }

        if !isFiniteChatInviteCode(value) {
            do {
                let profile = try profileSummaryFromScannedProfileCode(value, model: model)
                scanError = nil
                if onStartProfileChat(profile) {
                    model.scanDraft = ""
                    dismiss()
                }
            } catch {
                scanError = "That code is not a valid invite or profile code."
            }
            return
        }

        guard !model.scanInFlight else { return }
        model.scanTarget { result in
            handleScanResult(result)
        }
    }

    private func handleScanResult(_ result: AppScanTargetResult) {
        switch result {
        case .empty:
            scanError = nil
            dismiss()
        case .profile(let profile):
            if onStartProfileChat(profile) {
                scanError = nil
                dismiss()
            }
        case .room:
            scanError = nil
            dismiss()
            onRoomJoined?()
        case .unavailable:
            break
        }
    }
}

private struct InviteSheet: View {
    @Environment(\.dismiss) private var dismiss
    let invite: AppInviteState?

    var body: some View {
        NavigationStack {
            VStack(spacing: 18) {
                if let invite {
                    QRCodeView(value: invite.inviteUrl)
                        .frame(width: 220, height: 220)
                        .accessibilityLabel("Invite QR")

                    Text(invite.inviteUrl)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .textSelection(.enabled)
                        .lineLimit(4)

                    ShareLink(item: invite.inviteUrl) {
                        Label("Share", systemImage: "square.and.arrow.up")
                    }
                    .buttonStyle(.borderedProminent)
                } else {
                    ContentUnavailableView("Invite unavailable", systemImage: "qrcode")
                }
            }
            .padding()
            .navigationTitle("Invite")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    GlassCircleCloseButton { dismiss() }
                }
            }
        }
    }
}

private struct SettingsSheet: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject var model: AppModel
    let onStartProfileChat: (AppProfileSummary) -> Bool
    @State private var showingMyProfile = false
    @State private var showingScan = false
    @State private var confirmingSignOut = false

    var body: some View {
        NavigationStack {
            Form {
                Section("Profile") {
                    Button {
                        showingMyProfile = true
                    } label: {
                        HStack(spacing: 12) {
                            if let profile = model.myProfile {
                                ProfileAvatar(profile: profile)
                            } else {
                                Image(systemName: "person.crop.circle")
                                    .font(.title2)
                                    .foregroundStyle(.secondary)
                                    .frame(width: 40, height: 40)
                            }

                            VStack(alignment: .leading, spacing: 3) {
                                Text(model.myProfile?.displayName ?? "My Profile")
                                    .foregroundStyle(.primary)
                                Text(profileSubtitle)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                            }
                            Spacer(minLength: 0)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("SettingsMyProfileButton")
                }

                Section("Codes") {
                    Button {
                        showingScan = true
                    } label: {
                        SettingsRowLabel(
                            title: "Scan code",
                            subtitle: "Invite, profile, or agent code",
                            systemImage: "qrcode.viewfinder"
                        )
                    }
                    .buttonStyle(.plain)
                }

                Section {
                    DisclosureGroup {
                        LabeledContent("Server", value: model.serverURL)
                        if model.serverURL != RuntimeConfig.defaultServerURL {
                            Button {
                                model.useDefaultServer()
                            } label: {
                                Label("Use Deployed Server", systemImage: "network")
                            }
                            .accessibilityIdentifier("UseDefaultServerButton")
                        }
                        LabeledContent("Configured Device", value: model.deviceID)

                        if let state = model.state {
                            LabeledContent("Account", value: state.identity.accountId)
                            LabeledContent("Runtime Device", value: state.identity.deviceId)
                            LabeledContent("Revision", value: "\(state.rev)")
                            LabeledContent("Persistence", value: model.developerPersistenceSummary)
                        }
                        if let status = model.developerRuntimeStatus {
                            LabeledContent("Runtime Status", value: status)
                        }
                        if let notice = model.userNoticeText {
                            LabeledContent("Last Notice", value: notice)
                        }
                        if let storePath = model.runtimeStorePath {
                            VStack(alignment: .leading, spacing: 6) {
                                Text("Client Store")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                Text(storePath)
                                    .font(.caption)
                                    .textSelection(.enabled)
                            }
                        }
                        if let errorText = model.developerErrorText {
                            VStack(alignment: .leading, spacing: 6) {
                                Text("Last Error")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                Text(errorText)
                                    .font(.caption)
                                    .textSelection(.enabled)
                            }
                        }
                        if let state = model.state {
                            if !state.profiles.isEmpty {
                                Text("Profiles")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(.secondary)
                                ForEach(state.profiles, id: \.accountId) { profile in
                                    ProfileRow(profile: profile)
                                }
                            }

                            Text("Devices")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                            if state.devices.isEmpty {
                                Text("No devices found")
                                    .foregroundStyle(.secondary)
                            } else {
                                ForEach(state.devices, id: \.listID) { device in
                                    DeviceRow(device: device) {
                                        model.revokeDevice(device)
                                    }
                                }
                            }

                            Button {
                                model.refreshDevices()
                            } label: {
                                Label("Refresh Devices", systemImage: "arrow.clockwise")
                            }
                            .accessibilityIdentifier("RefreshDevicesButton")
                        }
                        if !model.developerDiagnostics.isEmpty {
                            LabeledContent(
                                "Debug Events",
                                value: "\(model.developerDiagnostics.count)"
                            )
                            HStack {
                                Button {
                                    UIPasteboard.general.string = model.developerDiagnosticsExport
                                } label: {
                                    Label("Copy Logs", systemImage: "doc.on.doc")
                                }
                                ShareLink(item: model.developerDiagnosticsExport) {
                                    Label("Share Logs", systemImage: "square.and.arrow.up")
                                }
                            }
                            ForEach(model.developerDiagnosticsPreview) { entry in
                                VStack(alignment: .leading, spacing: 4) {
                                    Text("\(entry.category) / \(entry.event)")
                                        .font(.caption.weight(.medium))
                                    if !entry.details.isEmpty {
                                        Text(developerDiagnosticDetails(entry.details))
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                            .textSelection(.enabled)
                                    }
                                }
                            }
                        }
                    } label: {
                        SettingsRowLabel(
                            title: "Developer diagnostics",
                            subtitle: "Redacted local copy and share only",
                            systemImage: "doc.text.magnifyingglass"
                        )
                    }

                    Button(role: .destructive) {
                        confirmingSignOut = true
                    } label: {
                        Label("Sign Out and Delete Local Data", systemImage: "rectangle.portrait.and.arrow.right")
                    }
                }
            }
            .navigationTitle("Settings")
            .task {
                model.refreshDevices()
            }
            .sheet(isPresented: $showingMyProfile) {
                MyNostrProfileSheet(
                    identity: model.nostrIdentity,
                    myNpub: model.myNpub,
                    accountID: model.activeAccountID,
                    profile: model.myProfile,
                    serverURL: model.serverURL,
                    onUploadImage: { data, mimeType in
                        await model.uploadImage(data: data, mimeType: mimeType)
                    }
                ) { displayName, about, picture in
                    await model.saveMyProfile(
                        displayName: displayName,
                        about: about,
                        picture: picture
                    )
                }
            }
            .sheet(isPresented: $showingScan) {
                ScanSheet(model: model, onStartProfileChat: onStartProfileChat)
            }
            .confirmationDialog(
                "Delete this device's Finite Chat data?",
                isPresented: $confirmingSignOut,
                titleVisibility: .visible
            ) {
                Button("Delete Everything", role: .destructive) {
                    model.signOutAndDeleteEverything()
                    dismiss()
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("This removes local chats, config, and the saved nsec from this device.")
            }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    GlassCircleCloseButton { dismiss() }
                }
            }
        }
    }

    private var profileSubtitle: String {
        if let npub = model.myNpub {
            return shortenedDisplayNpub(npub)
        }
        return "Signed in on this phone"
    }
}

private struct SettingsRowLabel: View {
    let title: String
    let subtitle: String?
    let systemImage: String

    var body: some View {
        Label {
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .foregroundStyle(.primary)
                if let subtitle, !subtitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text(subtitle)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
        } icon: {
            Image(systemName: systemImage)
                .foregroundStyle(.secondary)
                .frame(width: 28)
        }
    }
}

func shortenedDisplayNpub(_ npub: String) -> String {
    guard npub.count > 18 else { return npub }
    return "\(npub.prefix(10))...\(npub.suffix(4))"
}

private func developerDiagnosticDetails(_ details: [String: String]) -> String {
    details
        .sorted { $0.key < $1.key }
        .map { "\($0.key)=\($0.value)" }
        .joined(separator: " ")
}

private struct ProfileRow: View {
    let profile: AppProfileSummary

    var body: some View {
        HStack(spacing: 12) {
            ProfileAvatar(profile: profile)

            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Text(profile.displayName)
                        .font(.body)
                        .lineLimit(1)
                    if profile.stale {
                        Text("Stale")
                            .font(.caption.weight(.medium))
                            .foregroundStyle(.secondary)
                    }
                }

                Text(profile.about ?? profile.npub)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }

            Spacer(minLength: 8)
        }
        .accessibilityElement(children: .combine)
    }
}

private struct DeviceRow: View {
    let device: AppDeviceSummary
    let revoke: () -> Void

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Text(device.deviceId)
                        .font(.body)
                        .lineLimit(1)
                    if device.currentDevice {
                        Text("This device")
                            .font(.caption.weight(.medium))
                            .foregroundStyle(.secondary)
                    }
                }

                Text(statusText)
                    .font(.caption)
                    .foregroundStyle(device.revoked ? .red : .secondary)
            }

            Spacer(minLength: 12)

            if device.currentDevice {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                    .accessibilityLabel("Current device")
            } else if device.revoked {
                Image(systemName: "xmark.circle.fill")
                    .foregroundStyle(.red)
                    .accessibilityLabel("Revoked")
            } else {
                Button(role: .destructive) {
                    revoke()
                } label: {
                    Label("Revoke", systemImage: "xmark.circle")
                }
                .buttonStyle(.borderless)
                .accessibilityIdentifier("RevokeDeviceButton")
            }
        }
        .accessibilityElement(children: .combine)
    }

    private var statusText: String {
        let rooms = "\(device.roomCount) room\(device.roomCount == 1 ? "" : "s")"
        if device.revoked {
            return "Revoked - \(rooms)"
        }
        if device.active {
            return "Active - \(rooms)"
        }
        return "Inactive - \(rooms)"
    }
}

private extension AppDeviceSummary {
    var listID: String {
        "\(accountId)/\(deviceId)"
    }
}

struct QRCodeView: View {
    let value: String
    private let context = CIContext()
    private let filter = CIFilter.qrCodeGenerator()

    var body: some View {
        if let image = makeImage() {
            Image(uiImage: image)
                .interpolation(.none)
                .resizable()
                .scaledToFit()
        } else {
            Image(systemName: "qrcode")
                .resizable()
                .scaledToFit()
                .foregroundStyle(.secondary)
        }
    }

    private func makeImage() -> UIImage? {
        filter.message = Data(value.utf8)
        guard let output = filter.outputImage else { return nil }
        let scaled = output.transformed(by: CGAffineTransform(scaleX: 10, y: 10))
        guard let cgImage = context.createCGImage(scaled, from: scaled.extent) else { return nil }
        return UIImage(cgImage: cgImage)
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
