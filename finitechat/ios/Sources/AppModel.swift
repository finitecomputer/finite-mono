import Foundation
import OSLog
import SwiftUI
import UIKit
import UniformTypeIdentifiers

private enum AppAccountLinkError: LocalizedError {
    case authKitNotConfigured
    case invalidAuthorizationURL

    var errorDescription: String? {
        switch self {
        case .authKitNotConfigured:
            "AuthKit is not configured"
        case .invalidAuthorizationURL:
            "AuthKit returned an invalid authorization URL"
        }
    }
}

private enum AccountLinkStage: String {
    case startingAuthentication = "starting_authentication"
    case presentingAuthentication = "presenting_authentication"
    case exchangingAuthentication = "exchanging_authentication"
    case creatingDeviceLink = "creating_device_link"
    case approvingDeviceLink = "approving_device_link"
    case claimingAccount = "claiming_account"
    case storingAccount = "storing_account"
    case acknowledgingAccount = "acknowledging_account"
    case waitingForAgent = "waiting_for_agent"
    case startingRuntime = "starting_runtime"
}

struct RuntimeConfig: Codable, Equatable {
    let serverURL: String
    let dashboardURL: String
    let deviceID: String
    let workosClientID: String?
    let usesTransientStore: Bool
    let persistsRuntimeIdentityUpdates: Bool

    static let defaultServerURL = "https://chat.finite.computer"
    static let defaultDashboardURL = "https://finite.computer"
    private static let generatedDeviceIDPrefix = "ios-"
    private static let transientConfigArgument = "--finitechat-transient-config"
    private static let transientConfigEnvironmentKey = "FINITECHAT_TRANSIENT_CONFIG"
    private static let persistLaunchConfigArgument = "--finitechat-persist-launch-config"
    private static let persistLaunchConfigEnvironmentKey = "FINITECHAT_PERSIST_LAUNCH_CONFIG"

    enum CodingKeys: String, CodingKey {
        case serverURL = "server_url"
        case dashboardURL = "dashboard_url"
        case deviceID = "device_id"
    }

    init(
        serverURL: String,
        dashboardURL: String = defaultDashboardURL,
        deviceID: String,
        workosClientID: String? = nil,
        usesTransientStore: Bool = false,
        persistsRuntimeIdentityUpdates: Bool = true
    ) {
        self.serverURL = serverURL
        self.dashboardURL = dashboardURL
        self.deviceID = deviceID
        self.workosClientID = Self.normalizedWorkOSClientID(workosClientID)
        self.usesTransientStore = usesTransientStore
        self.persistsRuntimeIdentityUpdates = persistsRuntimeIdentityUpdates
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        serverURL = try container.decode(String.self, forKey: .serverURL)
        dashboardURL = try container.decodeIfPresent(String.self, forKey: .dashboardURL)
            ?? Self.defaultDashboardURL
        deviceID = try container.decode(String.self, forKey: .deviceID)
        workosClientID = nil
        usesTransientStore = false
        persistsRuntimeIdentityUpdates = true
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(serverURL, forKey: .serverURL)
        try container.encode(dashboardURL, forKey: .dashboardURL)
        try container.encode(deviceID, forKey: .deviceID)
    }

    static func load(
        environment: [String: String] = ProcessInfo.processInfo.environment,
        args: [String] = CommandLine.arguments,
        bundleInfo: [String: Any] = Bundle.main.infoDictionary ?? [:],
        storageURL: URL? = nil,
        allowsDevelopmentOverrides: Bool = developmentOverridesEnabled
    ) -> RuntimeConfig {
        let serverURL = argumentValue("--finitechat-server", in: args)
            ?? environmentValue("FINITECHAT_SERVER_URL", in: environment)
        let requestedDashboardURL = argumentValue("--finitechat-dashboard", in: args)
            ?? environmentValue("FINITECHAT_DASHBOARD_URL", in: environment)
        let dashboardURL = allowsDevelopmentOverrides ? requestedDashboardURL : nil
        let deviceID = argumentValue("--finitechat-device", in: args)
            ?? environmentValue("FINITECHAT_DEVICE_ID", in: environment)
        let bundledWorkOSClientID = normalizedWorkOSClientID(
            bundleInfo["WorkOSClientID"] as? String
        )
        let persisted = loadPersisted(storageURL: storageURL)
        let fallback = RuntimeConfig(
            serverURL: persisted.serverURL ?? defaultServerURL,
            dashboardURL: allowsDevelopmentOverrides
                ? (persisted.dashboardURL ?? defaultDashboardURL)
                : defaultDashboardURL,
            deviceID: persisted.deviceID ?? generatedDefaultDeviceID()
        )
        let hostedUnitTest = storageURL == nil && environment["XCTestConfigurationFilePath"] != nil
        let persistLaunchOverride = argumentFlag(persistLaunchConfigArgument, in: args)
            || truthyEnvironmentValue(persistLaunchConfigEnvironmentKey, in: environment)
        let hasLaunchOverride = serverURL != nil || dashboardURL != nil || deviceID != nil
        let transientOverride = argumentFlag(transientConfigArgument, in: args)
            || truthyEnvironmentValue(transientConfigEnvironmentKey, in: environment)
            || hostedUnitTest
        let shouldPersistResolvedIdentity = !transientOverride
            && (!hasLaunchOverride || persistLaunchOverride)
        let config = RuntimeConfig(
            serverURL: serverURL ?? fallback.serverURL,
            dashboardURL: dashboardURL ?? fallback.dashboardURL,
            deviceID: deviceID ?? fallback.deviceID,
            workosClientID: bundledWorkOSClientID,
            usesTransientStore: transientOverride,
            persistsRuntimeIdentityUpdates: shouldPersistResolvedIdentity
        )
        let shouldPersistFallbackRepair = !hasLaunchOverride
            && (
                persisted.serverURL != config.serverURL
                    || persisted.dashboardURL != config.dashboardURL
                    || persisted.deviceID != config.deviceID
            )
        // Runtime identity is product state. Launch values are test/developer
        // inputs unless the caller explicitly opts into persisting them.
        if !transientOverride
            && (
                shouldPersistFallbackRepair
                    || persistLaunchOverride
            )
        {
            try? config.save(storageURL: storageURL)
        }
        return config
    }

    func save(storageURL: URL? = nil) throws {
        let config = RuntimeConfig(
            serverURL: serverURL.trimmingCharacters(in: .whitespacesAndNewlines),
            dashboardURL: dashboardURL.trimmingCharacters(in: .whitespacesAndNewlines),
            deviceID: deviceID.trimmingCharacters(in: .whitespacesAndNewlines),
            workosClientID: workosClientID
        )
        guard !config.serverURL.isEmpty,
              !config.dashboardURL.isEmpty,
              !config.deviceID.isEmpty
        else {
            throw ConfigError.emptyValue
        }
        let data = try JSONEncoder().encode(config)
        let url = try storageURL ?? Self.configURL()
        try data.write(to: url, options: .atomic)
    }

    private static func loadPersisted(storageURL: URL?) -> PersistedRuntimeConfig {
        guard let url = storageURL ?? (try? configURL()),
              let data = try? Data(contentsOf: url),
              let config = try? JSONDecoder().decode(PersistedRuntimeConfig.self, from: data)
        else {
            return PersistedRuntimeConfig()
        }
        return config.normalized()
    }

    fileprivate static func configURL() throws -> URL {
        let support = try FileManager.default.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )
        return support.appendingPathComponent("finitechat_config.json")
    }

    private static func generatedDefaultDeviceID() -> String {
        let normalized = UUID().uuidString
            .lowercased()
            .filter { $0.isLetter || $0.isNumber }
        let suffix = normalized.prefix(12)
        return "\(generatedDeviceIDPrefix)\(suffix)"
    }

    private static func normalizedWorkOSClientID(_ value: String?) -> String? {
        guard let value = value?.trimmingCharacters(in: .whitespacesAndNewlines),
              value.hasPrefix("client_"),
              value != "client_replace_me",
              value.count <= 256,
              !value.contains(where: \.isNewline)
        else {
            return nil
        }
        return value
    }

    enum ConfigError: Error {
        case emptyValue
    }

    static var developmentOverridesEnabled: Bool {
#if DEBUG && targetEnvironment(simulator)
        true
#else
        false
#endif
    }

    var usesLocalDeviceLinkEnvironment: Bool {
        Self.isLoopbackHTTPURL(serverURL) && Self.isLoopbackHTTPURL(dashboardURL)
    }

    var requiresWorkOSAuthentication: Bool {
        !usesLocalDeviceLinkEnvironment
    }

    var hasCompatibleDeviceLinkOrigins: Bool {
        usesLocalDeviceLinkEnvironment
            || (
                serverURL == Self.defaultServerURL
                    && dashboardURL == Self.defaultDashboardURL
            )
    }

    var accountIdentityKeychainService: String {
        usesLocalDeviceLinkEnvironment
            ? KeychainNostrIdentityStore.localDevelopmentService
            : KeychainNostrIdentityStore.productionService
    }

    var accountIdentityKeychainAccount: String {
        usesLocalDeviceLinkEnvironment
            ? deviceID
            : KeychainNostrIdentityStore.primaryAccount
    }

    private static func isLoopbackHTTPURL(_ value: String) -> Bool {
        guard let components = URLComponents(string: value),
              components.scheme?.lowercased() == "http",
              let host = components.host?.lowercased()
        else {
            return false
        }
        if host == "localhost" || host == "::1" {
            return true
        }
        let parts = host.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 4 else { return false }
        let octets = parts.compactMap { UInt8($0) }
        return octets.count == 4 && octets[0] == 127
    }

    private static func environmentValue(
        _ key: String,
        in environment: [String: String]
    ) -> String? {
        let value = environment[key]?.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let value, !value.isEmpty else { return nil }
        return value
    }

    private static func argumentValue(_ name: String, in args: [String]) -> String? {
        guard let index = args.firstIndex(of: name) else {
            return nil
        }
        let valueIndex = args.index(after: index)
        guard valueIndex < args.endIndex else {
            return nil
        }
        let value = args[valueIndex].trimmingCharacters(in: .whitespacesAndNewlines)
        return value.isEmpty ? nil : value
    }

    private static func argumentFlag(_ name: String, in args: [String]) -> Bool {
        args.contains(name)
    }

    private static func truthyEnvironmentValue(
        _ key: String,
        in environment: [String: String]
    ) -> Bool {
        guard let value = environment[key]?.trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased(),
            !value.isEmpty
        else {
            return false
        }
        return !["0", "false", "no", "off"].contains(value)
    }
}

private struct PersistedRuntimeConfig: Codable, Equatable {
    var serverURL: String?
    var dashboardURL: String?
    var deviceID: String?

    enum CodingKeys: String, CodingKey {
        case serverURL = "server_url"
        case dashboardURL = "dashboard_url"
        case deviceID = "device_id"
    }

    init(
        serverURL: String? = nil,
        dashboardURL: String? = nil,
        deviceID: String? = nil
    ) {
        self.serverURL = serverURL
        self.dashboardURL = dashboardURL
        self.deviceID = deviceID
    }

    func normalized() -> PersistedRuntimeConfig {
        PersistedRuntimeConfig(
            serverURL: normalizedPersistedServerURL(serverURL),
            dashboardURL: normalizedPersistedServerURL(dashboardURL),
            deviceID: normalizedNonEmpty(deviceID)
        )
    }

    private func normalizedNonEmpty(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let trimmed, !trimmed.isEmpty else { return nil }
        return trimmed
    }

    private func normalizedPersistedServerURL(_ value: String?) -> String? {
        guard let trimmed = normalizedNonEmpty(value),
              let components = URLComponents(string: trimmed),
              components.scheme?.lowercased() == "https",
              let host = components.host?.lowercased(),
              !host.isEmpty,
              !Self.isDevelopmentHost(host)
        else {
            return nil
        }
        return trimmed
    }

    private static func isDevelopmentHost(_ host: String) -> Bool {
        if host == "localhost"
            || host == "0.0.0.0"
            || host == "::1"
            || host.hasSuffix(".local")
        {
            return true
        }

        if host.hasPrefix("127.") {
            return true
        }

        let octets = host.split(separator: ".").compactMap { Int($0) }
        guard octets.count == 4 else { return false }
        if octets[0] == 10 || (octets[0] == 192 && octets[1] == 168) {
            return true
        }
        if octets[0] == 172 && (16...31).contains(octets[1]) {
            return true
        }
        if octets[0] == 169 && octets[1] == 254 {
            return true
        }
        return false
    }
}

typealias AppRuntimeFactory = (OpenOptions) throws -> any FiniteChatRuntimeProtocol

extension AppRoomSummary {
    var isWaitingForWelcome: Bool {
        state == .waitingForApproval
            && status.localizedCaseInsensitiveContains("waiting for room admission")
    }
}

struct RuntimeDataStore {
    private static let currentDataDirectoryName = "FiniteChatStore"
    private static let transientDataRootDirectoryName = "FiniteChatTransient"

    static func dataDir(
        deviceID: String,
        applicationSupportURL: URL? = nil,
        transient: Bool = false
    ) throws -> String {
        let supportURL: URL
        if let applicationSupportURL {
            supportURL = applicationSupportURL
        } else {
            supportURL = try defaultApplicationSupportURL(create: true)
        }
        if transient {
            let transientStoreURL = supportURL
                .appendingPathComponent(transientDataRootDirectoryName, isDirectory: true)
                .appendingPathComponent(safeDeviceDirectoryName(deviceID), isDirectory: true)
            try FileManager.default.createDirectory(
                at: transientStoreURL,
                withIntermediateDirectories: true
            )
            return transientStoreURL.path
        }
        let currentStoreURL = supportURL.appendingPathComponent(
            currentDataDirectoryName,
            isDirectory: true
        )
        try FileManager.default.createDirectory(
            at: currentStoreURL,
            withIntermediateDirectories: true
        )
        return currentStoreURL.path
    }

    static func deleteDataDir(
        deviceID: String,
        applicationSupportURL: URL? = nil,
        transient: Bool = false
    ) throws {
        let path = try dataDir(
            deviceID: deviceID,
            applicationSupportURL: applicationSupportURL,
            transient: transient
        )
        let url = URL(fileURLWithPath: path, isDirectory: true)
        if FileManager.default.fileExists(atPath: url.path) {
            try FileManager.default.removeItem(at: url)
        }
    }

    static func hasRecoverableStableStore(applicationSupportURL: URL? = nil) -> Bool {
        let supportURL: URL
        if let applicationSupportURL {
            supportURL = applicationSupportURL
        } else if let defaultURL = try? defaultApplicationSupportURL(create: false) {
            supportURL = defaultURL
        } else {
            return false
        }
        let storeURL = supportURL.appendingPathComponent(
            currentDataDirectoryName,
            isDirectory: true
        )
        return FileManager.default.fileExists(
            atPath: storeURL.appendingPathComponent("account-secret.hex").path
        ) && FileManager.default.fileExists(
            atPath: storeURL.appendingPathComponent("client.sqlite3").path
        )
    }

    private static func safeDeviceDirectoryName(_ deviceID: String) -> String {
        deviceID
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "/", with: "-")
    }

    private static func defaultApplicationSupportURL(create: Bool) throws -> URL {
        try FileManager.default.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: create
        )
    }
}

private struct ProductHarnessSupportResolution {
    let url: URL?
    let error: String?
}

private struct AppLaunchConfigurationError: Error, CustomStringConvertible {
    let message: String

    var description: String {
        message
    }
}

struct DeveloperDiagnosticEntry: Identifiable, Equatable {
    let id: Int
    let timestampUnixSeconds: Int64
    let category: String
    let event: String
    let details: [String: String]

    static func exportText(_ entries: [DeveloperDiagnosticEntry]) -> String {
        var lines = [
            "Finite Chat diagnostics",
            "redaction=urls,paths,long-hex",
            "event_count=\(entries.count)",
        ]
        for entry in entries {
            let details = entry.details
                .sorted { $0.key < $1.key }
                .map { "\($0.key)=\($0.value)" }
                .joined(separator: " ")
            if details.isEmpty {
                lines.append(
                    "seq=\(entry.id) ts=\(entry.timestampUnixSeconds) category=\(entry.category) event=\(entry.event)"
                )
            } else {
                lines.append(
                    "seq=\(entry.id) ts=\(entry.timestampUnixSeconds) category=\(entry.category) event=\(entry.event) \(details)"
                )
            }
        }
        return lines.joined(separator: "\n")
    }
}

private struct DiagnosticActionSummary {
    let category: String
    let name: String
    let details: [String: String]
}

@MainActor
final class AppModel: ObservableObject, AppReconciler {
    private static let developerDiagnosticsLimit = 200
    private static let optimisticSequenceBase = UInt64.max - 1_000_000
    private static let accountLinkLogger = Logger(
        subsystem: "computer.finite.finitechat",
        category: "AccountLink"
    )
#if DEBUG
    private static let debugDiagnosticsQueue = DispatchQueue(
        label: "computer.finite.finitechat.debug-diagnostics",
        qos: .utility
    )
#endif
    private static let diagnosticRedactions: [(regex: NSRegularExpression, replacement: String)] = [
        (#"https?://[^\s\)"]+"#, "[url]"),
        (#"file://[^\s\)"]+"#, "[url]"),
        (#"/(?:Users|private|var|tmp|Volumes)/[^\s]+"#, "[path]"),
        (#"\b[0-9a-fA-F]{32,}\b"#, "[hex]"),
    ].compactMap { pattern, replacement in
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return nil }
        return (regex, replacement)
    }
    private static let optimisticTimestampFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.timeStyle = .short
        formatter.dateStyle = .none
        return formatter
    }()

    @Published var serverURL: String
    @Published private(set) var dashboardURL: String
    @Published var deviceID: String
    @Published private(set) var state: AppState? {
        didSet {
            rebuildChatProjections()
            if let state {
                appendStateDiagnostic(state, event: "state.projected")
            }
        }
    }
    @Published private(set) var chatProjections: [String: ChatRoomProjection] = [:]
    @Published var errorText: String?
    @Published var outboundText: String = ""
    @Published private(set) var runtimeStorePath: String?
    @Published private(set) var developerDiagnostics: [DeveloperDiagnosticEntry] = []
    @Published private(set) var nostrIdentity: AppNostrIdentity?
    @Published private(set) var requiresNostrLogin: Bool
    @Published private(set) var canRecoverRuntimeIdentity: Bool
    @Published private(set) var accountLinkPhase: AccountLinkPhase = .ready

    private var runtime: (any FiniteChatRuntimeProtocol)?
    private var openKey = ""
    private var foregroundStartKey: String?
    private let usesTransientStore: Bool
    private var pendingOptimisticMessages: [String: ChatMessage] = [:]
    private var optimisticMessageCounter: UInt64 = 0
    private let persistsRuntimeIdentityUpdates: Bool
    private let applicationSupportURL: URL?
    private let configStorageURL: URL?
    private let args: [String]
    private let runtimeFactory: AppRuntimeFactory
    private let startsUpdateLoop: Bool
    private let nostrIdentityStore: AppNostrIdentityStoring
    private let webAuthenticationPresenter: any WebAuthenticationPresenting
    private let workosClientID: String?
    private var updateTask: Task<Void, Never>?
    private var launchAutomationTask: Task<Void, Never>?
    private var postSendCatchUpTask: Task<Void, Never>?
    private var runtimeDispatchTail: Task<Void, Never>?
    private var lastAppliedRuntimeRev: UInt64 = 0
    private var attachmentDownloadsInFlight = Set<String>()
    private var messageRetriesInFlight = Set<String>()
    private var lastTypingIntentByRoom: [String: Bool] = [:]
    private var pendingPushToken: String?
    private var pushTokenRegistrationInFlight: String?
    private var didRunLaunchAutomation = false
    private let launchConfigurationError: String?
    private var accountLinkTask: Task<Void, Never>?

    deinit {
        updateTask?.cancel()
        launchAutomationTask?.cancel()
        postSendCatchUpTask?.cancel()
        runtimeDispatchTail?.cancel()
        accountLinkTask?.cancel()
    }

    init(
        config: RuntimeConfig? = nil,
        applicationSupportURL: URL? = nil,
        configStorageURL: URL? = nil,
        args: [String] = CommandLine.arguments,
        requiresNostrLogin: Bool = false,
        nostrIdentityStore: AppNostrIdentityStoring? = nil,
        webAuthenticationPresenter: (any WebAuthenticationPresenting)? = nil,
        startsUpdateLoop: Bool = true,
        runtimeFactory: @escaping AppRuntimeFactory = { options in
            try FiniteChatRuntime.open(options: options)
        }
    ) {
        let productHarnessSupport = Self.productHarnessApplicationSupportURL(args: args)
        let resolvedApplicationSupportURL = applicationSupportURL ?? productHarnessSupport.url
        let resolvedConfigStorageURL = configStorageURL
            ?? resolvedApplicationSupportURL?.appendingPathComponent("finitechat_config.json")
            ?? (try? RuntimeConfig.configURL())
        let resolvedConfig = config ?? RuntimeConfig.load(
            args: args,
            storageURL: resolvedConfigStorageURL
        )
        serverURL = resolvedConfig.serverURL
        dashboardURL = resolvedConfig.dashboardURL
        deviceID = resolvedConfig.deviceID
        workosClientID = resolvedConfig.workosClientID
        usesTransientStore = resolvedConfig.usesTransientStore
        persistsRuntimeIdentityUpdates = resolvedConfig.persistsRuntimeIdentityUpdates
        self.applicationSupportURL = resolvedApplicationSupportURL
        self.configStorageURL = configStorageURL
            ?? resolvedConfigStorageURL
        self.args = args
        self.runtimeFactory = runtimeFactory
        self.startsUpdateLoop = startsUpdateLoop
        self.webAuthenticationPresenter = webAuthenticationPresenter
            ?? NativeWebAuthenticationPresenter()
        let resolvedNostrIdentityStore = nostrIdentityStore
            ?? KeychainNostrIdentityStore(
                service: resolvedConfig.accountIdentityKeychainService,
                account: resolvedConfig.accountIdentityKeychainAccount
        )
        self.nostrIdentityStore = resolvedNostrIdentityStore
        let storedNostrIdentity = resolvedNostrIdentityStore.load()
        let hasRecoverableRuntimeIdentity = !usesTransientStore
            && storedNostrIdentity == nil
            && RuntimeDataStore.hasRecoverableStableStore(
                applicationSupportURL: resolvedApplicationSupportURL
            )
        nostrIdentity = storedNostrIdentity
        self.requiresNostrLogin = requiresNostrLogin
            && storedNostrIdentity == nil
            && !hasRecoverableRuntimeIdentity
            && !Self.hasLaunchAutomation(args: args)
        canRecoverRuntimeIdentity = hasRecoverableRuntimeIdentity
        launchConfigurationError = productHarnessSupport.error
        appendDiagnostic(
            category: "persistence",
            event: "app.configured",
            details: [
                "store_mode": usesTransientStore ? "transient" : "stable",
                "has_explicit_support_root": resolvedApplicationSupportURL == nil ? "false" : "true",
                "has_recoverable_runtime_identity": hasRecoverableRuntimeIdentity ? "true" : "false",
                "has_launch_configuration_error": launchConfigurationError == nil ? "false" : "true",
                "requires_nostr_login": self.requiresNostrLogin ? "true" : "false",
            ]
        )
    }

    nonisolated func reconcile(update: AppUpdate) {
        Task { @MainActor [weak self] in
            self?.applyRuntimeUpdate(update)
        }
    }

    private func applyRuntimeUpdate(_ update: AppUpdate) {
        switch update {
        case .fullState(let nextState):
            applyRuntimeSnapshot(nextState)
        }
    }

    private func applyRuntimeSnapshot(_ nextState: AppState) {
        let performanceInterval = FinitePerformance.begin(
            "Apply runtime snapshot",
            warningBudgetMilliseconds: 16.67
        )
        defer { FinitePerformance.end(performanceInterval) }
        if nextState.rev < lastAppliedRuntimeRev {
            return
        }
        if nextState.rev == lastAppliedRuntimeRev, state == nextState {
            return
        }
        lastAppliedRuntimeRev = nextState.rev
        state = nextState
    }

    var rooms: [AppRoomSummary] {
        state?.rooms ?? []
    }

    var availableAgents: [AppRoomSummary] {
        rooms
            .filter { $0.state == .connected && $0.isAgentChat }
            .sorted {
                $0.displayName.localizedCaseInsensitiveCompare($1.displayName) == .orderedAscending
            }
    }

    var pairedAgent: AppRoomSummary? {
        guard let roomID = state?.pairedAgent?.canonicalRoomId else { return nil }
        return availableAgents.first { $0.roomId == roomID }
    }

    var selectedRoom: AppRoomSummary? {
        guard let state else { return nil }
        if let selected = state.selectedRoomId,
           let room = state.rooms.first(where: { $0.roomId == selected })
        {
            return room
        }
        return state.rooms.first
    }

    var selectedRoomMessages: [ChatMessage] {
        guard let roomId = selectedRoom?.roomId else { return [] }
        return projection(for: roomId).messages
    }

    func topics(for roomID: String) -> [AppTopicSummary] {
        state?.topics.filter { $0.roomId == roomID && !$0.archived } ?? []
    }

    func selectedTopic(in roomID: String) -> AppTopicSummary? {
        let roomTopics = topics(for: roomID)
        if let selectedTopicID = state?.selectedTopicId,
           let topic = roomTopics.first(where: { $0.topicId == selectedTopicID })
        {
            return topic
        }
        return roomTopics.first
    }

    func selectedChat(in topic: AppTopicSummary) -> AppChatSummary? {
        let visibleChats = topic.chats.filter { !$0.archived }
        if let selectedChatID = state?.selectedChatId,
           let chat = visibleChats.first(where: { $0.chatId == selectedChatID })
        {
            return chat
        }
        if let activeChatID = topic.activeChatId,
           let chat = visibleChats.first(where: { $0.chatId == activeChatID })
        {
            return chat
        }
        return visibleChats.first
    }

    func selectedChatRoute(for roomID: String) -> (topicID: String, chatID: String)? {
        guard let topic = selectedTopic(in: roomID),
              let chat = selectedChat(in: topic)
        else {
            return nil
        }
        return (topic.topicId, chat.chatId)
    }

    func compositionRoute(for roomID: String, replyTo message: ChatMessage? = nil) -> (topicID: String, chatID: String)? {
        if let message,
           let topicID = message.conversationId,
           let chatID = message.chatId
        {
            return (topicID, chatID)
        }
        return selectedChatRoute(for: roomID)
    }

    var roomListEmptyDescription: String {
        if developerErrorText != nil {
            return "Open Settings to check connection."
        }
        if state == nil {
            return "Opening chats..."
        }
        return "No chats yet"
    }

    var userNoticeText: String? {
        let toast = state?.toast?.nonEmptyTrimmed
        let flowNotice = state?.flow.noticeText?.nonEmptyTrimmed
        if toast == "Showing saved chats. Connection will retry." {
            return flowNotice
        }
        return toast ?? flowNotice
    }

    var actionNoticeText: String? {
        userNoticeText ?? developerErrorText
    }

    var developerErrorText: String? {
        errorText?.nonEmptyTrimmed
    }

    var developerRuntimeStatus: String? {
        state?.status.nonEmptyTrimmed
    }

    var developerPersistenceSummary: String {
        let roomCount = rooms.count
        let selectedRoomID = state?.selectedRoomId?.nonEmptyTrimmed ?? "none"
        let selectedMessages = selectedRoomMessages.count
        let projectedMessages = state?.messages.count ?? 0
        return "\(roomCount) room(s), selected \(selectedRoomID), \(selectedMessages) selected message(s), \(projectedMessages) projected message(s)"
    }

    var developerDiagnosticsExport: String {
        DeveloperDiagnosticEntry.exportText(developerDiagnostics)
    }

    var developerDiagnosticsPreview: [DeveloperDiagnosticEntry] {
        Array(developerDiagnostics.suffix(8))
    }

    var myNpub: String? {
        if let npub = nostrIdentity?.npub {
            return npub
        }
        guard let accountID = state?.identity.accountId.nonEmptyTrimmed else {
            return nil
        }
        return try? npubFromAccountId(accountId: accountID)
    }

    func beginAccountLink() {
        guard requiresNostrLogin, accountLinkPhase == .ready else { return }
        errorText = nil
        accountLinkPhase = .authenticating
        Self.accountLinkLogger.notice("requested")
        appendDiagnostic(category: "account_link", event: "requested")
        let serverURL = serverURL
        let dashboardURL = dashboardURL
        let deviceID = deviceID
        let runtimeConfig = RuntimeConfig(
            serverURL: serverURL,
            dashboardURL: dashboardURL,
            deviceID: deviceID,
            workosClientID: workosClientID
        )
        guard runtimeConfig.hasCompatibleDeviceLinkOrigins else {
            accountLinkPhase = .ready
            errorText = "Local account linking requires both the chat server and dashboard to use loopback URLs."
            return
        }
        accountLinkTask?.cancel()
        accountLinkTask = Task { [weak self] in
            var stage = AccountLinkStage.startingAuthentication
            do {
                guard let self else { return }
                let authKit: NativeAuthKitSession?
                if runtimeConfig.requiresWorkOSAuthentication {
                    guard let workosClientID = runtimeConfig.workosClientID else {
                        throw AppAccountLinkError.authKitNotConfigured
                    }
                    let session = try await Task.detached(priority: .userInitiated) {
                        try NativeAuthKitSession.start(clientId: workosClientID)
                    }.value
                    guard let authorizationURL = URL(
                        string: session.authorizationUrl()
                    ) else {
                        throw AppAccountLinkError.invalidAuthorizationURL
                    }
                    stage = .presentingAuthentication
                    self.appendAccountLinkProgress(stage)
                    let callbackURL = try await self.webAuthenticationPresenter.authenticate(
                        url: authorizationURL
                    )
                    stage = .exchangingAuthentication
                    self.appendAccountLinkProgress(stage)
                    try await Task.detached(priority: .userInitiated) {
                        try session.complete(callbackUrl: callbackURL.absoluteString)
                    }.value
                    authKit = session
                } else {
                    authKit = nil
                }
                guard !Task.isCancelled else { return }
                stage = .creatingDeviceLink
                self.appendAccountLinkProgress(stage)
                let link = try await Task.detached(priority: .userInitiated) {
                    try NativeDeviceLinkSession.create(
                        serverUrl: serverURL,
                        dashboardUrl: dashboardURL,
                        targetDeviceId: deviceID
                    )
                }.value
                guard !Task.isCancelled else {
                    link.release()
                    return
                }
                stage = .approvingDeviceLink
                self.appendAccountLinkProgress(stage)
                if let authKit {
                    try await Task.detached(priority: .userInitiated) {
                        try link.approveWithAuthkit(authkit: authKit)
                    }.value
                } else {
                    try await Task.detached(priority: .userInitiated) {
                        try link.approveAuthenticatedAccount(accessToken: nil)
                    }.value
                }
                guard !Task.isCancelled else {
                    link.release()
                    return
                }
                self.accountLinkPhase = .waiting
                stage = .claimingAccount
                self.appendAccountLinkProgress(stage)
                let accountSecret = try await Task.detached(priority: .userInitiated) {
                    try link.claimAccountSecret()
                }.value
                stage = .storingAccount
                self.appendAccountLinkProgress(stage)
                let material = try nostrIdentityFromAccountSecretHex(
                    accountSecretHex: accountSecret
                )
                try self.applyNostrIdentity(
                    AppNostrIdentity(material: material),
                    resetStore: true
                )
                stage = .acknowledgingAccount
                self.appendAccountLinkProgress(stage)
                try await Task.detached(priority: .utility) {
                    try link.acknowledgeStored()
                }.value
                stage = .waitingForAgent
                self.appendAccountLinkProgress(stage)
                stage = .startingRuntime
                self.appendAccountLinkProgress(stage)
                self.startFromForeground()
                self.accountLinkPhase = .ready
                Self.accountLinkLogger.notice("succeeded")
                self.appendDiagnostic(category: "account_link", event: "succeeded")
                self.finishAccountLinkInBackground(link: link, authKit: authKit)
            } catch let error as WebAuthenticationPresentationError
                where error == .canceled
            {
                self?.accountLinkPhase = .ready
                self?.errorText = nil
                Self.accountLinkLogger.notice(
                    "canceled stage=\(stage.rawValue, privacy: .public)"
                )
                self?.appendDiagnostic(
                    category: "account_link",
                    event: "canceled",
                    details: ["stage": stage.rawValue]
                )
            } catch is CancellationError {
                self?.accountLinkPhase = .ready
                self?.errorText = nil
                Self.accountLinkLogger.notice(
                    "task canceled stage=\(stage.rawValue, privacy: .public)"
                )
                self?.appendDiagnostic(
                    category: "account_link",
                    event: "canceled",
                    details: ["stage": stage.rawValue]
                )
            } catch {
                self?.accountLinkPhase = .ready
                self?.errorText = Self.accountLinkErrorText(error, stage: stage)
                var details = self?.diagnosticErrorDetails(error) ?? [:]
                details["stage"] = stage.rawValue
                if stage == .exchangingAuthentication {
                    let reason = Self.redactedDiagnosticValue(
                        String(describing: error)
                    )
                    Self.accountLinkLogger.error(
                        "failed stage=\(stage.rawValue, privacy: .public) type=\(String(describing: type(of: error)), privacy: .public) reason=\(reason, privacy: .public)"
                    )
                } else {
                    Self.accountLinkLogger.error(
                        "failed stage=\(stage.rawValue, privacy: .public) type=\(String(describing: type(of: error)), privacy: .public)"
                    )
                }
                self?.appendDiagnostic(
                    category: "account_link",
                    event: "failed",
                    details: details
                )
            }
        }
    }

    private func finishAccountLinkInBackground(
        link: NativeDeviceLinkSession,
        authKit: NativeAuthKitSession?
    ) {
        appendDiagnostic(category: "account_link", event: "finalization.requested")
        Task { [weak self, link, authKit] in
            do {
                try await Task.detached(priority: .utility) {
                    if let authKit {
                        try link.waitUntilReadyWithAuthkit(authkit: authKit)
                    } else {
                        try link.waitUntilReady(accessToken: nil)
                    }
                }.value
                self?.appendDiagnostic(
                    category: "account_link",
                    event: "finalization.succeeded"
                )
            } catch {
                guard let self else { return }
                self.appendDiagnostic(
                    category: "account_link",
                    event: "finalization.failed",
                    details: self.diagnosticErrorDetails(error)
                )
            }
        }
    }

    private func appendAccountLinkProgress(_ stage: AccountLinkStage) {
        Self.accountLinkLogger.info(
            "progress stage=\(stage.rawValue, privacy: .public)"
        )
        appendDiagnostic(
            category: "account_link",
            event: "progress",
            details: ["stage": stage.rawValue]
        )
    }

    private static func accountLinkErrorText(
        _ error: Error,
        stage: AccountLinkStage
    ) -> String {
        if let linkError = error as? AppAccountLinkError {
            switch linkError {
            case .authKitNotConfigured:
                return "Secure sign in is not configured in this build."
            case .invalidAuthorizationURL:
                return "Secure sign in could not start. Please try again."
            }
        }
        if let presentationError = error as? WebAuthenticationPresentationError {
            switch presentationError {
            case .canceled:
                return ""
            case .couldNotStart:
                return "Secure sign in could not open. Please try again."
            case .missingCallback:
                return "Sign in did not return a valid response. Please try again."
            case .invalidPresentationContext:
                return "Secure sign in could not be presented. Please try again."
            case .failed:
                return "Sign in could not return to Finite. Please try again."
            }
        }
        let description = String(describing: error)
        if description.localizedCaseInsensitiveContains("was denied") {
            return "Sign in was denied."
        }
        if description.localizedCaseInsensitiveContains("expired") {
            return "That secure link expired. Start a new one."
        }
        if description.localizedCaseInsensitiveContains("temporarily unavailable") {
            return "Secure sign in is temporarily unavailable. Try again in a moment."
        }
        switch stage {
        case .startingAuthentication:
            return "Secure sign in could not start. Please try again."
        case .presentingAuthentication:
            return "Sign in could not return to Finite. Please try again."
        case .exchangingAuthentication:
            return "Finite could not verify that sign in. Please try again."
        case .creatingDeviceLink, .approvingDeviceLink:
            return "You’re signed in, but this iPhone could not start linking. Please try again."
        case .claimingAccount, .storingAccount, .acknowledgingAccount,
             .waitingForAgent, .startingRuntime:
            return "This iPhone could not finish linking to your agent. Please try again."
        }
    }

    @discardableResult
    func recoverExistingDeviceAccount() -> Bool {
        guard canRecoverRuntimeIdentity else { return false }
        appendDiagnostic(category: "persistence", event: "recover_existing_account.requested")
        requiresNostrLogin = false
        start()
        if nostrIdentity != nil {
            canRecoverRuntimeIdentity = false
            appendDiagnostic(category: "persistence", event: "recover_existing_account.succeeded")
            return true
        }
        closeRuntime()
        requiresNostrLogin = true
        errorText = "Existing device account could not be recovered."
        appendDiagnostic(category: "persistence", event: "recover_existing_account.failed")
        return false
    }

    func signOutAndDeleteEverything() {
        appendDiagnostic(category: "persistence", event: "signout.delete_all.requested")
        let runtimeForPushCleanup = runtime
        runtimeDispatchTail?.cancel()
        runtimeDispatchTail = nil
        if let runtimeForPushCleanup {
            removePushTokenDuringSignOut(runtime: runtimeForPushCleanup)
        }
        pendingPushToken = nil
        nostrIdentityStore.clear()
        closeRuntime()
        try? RuntimeDataStore.deleteDataDir(
            deviceID: deviceID,
            applicationSupportURL: applicationSupportURL,
            transient: usesTransientStore
        )
        if let configStorageURL {
            try? FileManager.default.removeItem(at: configStorageURL)
        }
        let resetConfig = RuntimeConfig.load(args: args, storageURL: configStorageURL)
        serverURL = resetConfig.serverURL
        dashboardURL = resetConfig.dashboardURL
        deviceID = resetConfig.deviceID
        nostrIdentity = nil
        requiresNostrLogin = true
        canRecoverRuntimeIdentity = false
        errorText = nil
    }

    func useDefaultServer() {
        let defaultServerURL = RuntimeConfig.defaultServerURL
        guard serverURL != defaultServerURL else { return }
        appendDiagnostic(
            category: "persistence",
            event: "server.reset_default.requested",
            details: ["from": Self.redactedDiagnosticValue(serverURL)]
        )
        closeRuntime()
        serverURL = defaultServerURL
        do {
            try RuntimeConfig(
                serverURL: serverURL,
                dashboardURL: dashboardURL,
                deviceID: deviceID
            ).save(
                storageURL: configStorageURL
            )
            appendDiagnostic(category: "persistence", event: "server.reset_default.succeeded")
            errorText = nil
        } catch {
            appendDiagnostic(
                category: "persistence",
                event: "server.reset_default.failed",
                details: diagnosticErrorDetails(error)
            )
            errorText = String(describing: error)
        }
        start()
    }

    var canSend: Bool {
        guard let selectedRoom else { return false }
        return canSend(roomID: selectedRoom.roomId, text: outboundText)
    }

    func canSend(roomID: String, text: String) -> Bool {
        roomAllowsComposition(roomID)
            && !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func roomAllowsComposition(_ roomID: String) -> Bool {
        state?.rooms.first(where: { $0.roomId == roomID })?.state == .connected
    }

    private static func messageAllowsRetry(_ message: ChatMessage) -> Bool {
        guard message.isMine,
              let serverDelivery = message.outboundDelivery?.serverDelivery,
              case .failed = serverDelivery
        else {
            return false
        }
        return true
    }

    func start() {
        appendDiagnostic(category: "runtime", event: "start.requested")
        do {
            let runtime = try currentRuntime()
            let runtimeKey = openKey
            applyRuntimeSnapshot(try readRuntimeState(runtime))
            enqueueRuntimeDispatch(
                .startRuntime,
                runtime: runtime,
                runtimeKey: runtimeKey,
                priority: .utility,
                onSuccess: { [weak self] _ in
                    guard let self else { return }
                    self.appendDiagnostic(category: "runtime", event: "start.succeeded")
                    self.errorText = nil
                    self.restartUpdateLoopIfEnabled()
                    self.flushPendingPushTokenIfPossible()
                    self.runLaunchAutomationIfRequested()
                },
                onFailure: { [weak self] error in
                    guard let self else { return }
                    self.appendDiagnostic(
                        category: "runtime",
                        event: "start.failed",
                        details: self.diagnosticErrorDetails(error)
                    )
                    self.errorText = String(describing: error)
                }
            )
        } catch {
            appendDiagnostic(
                category: "runtime",
                event: "open.failed",
                details: diagnosticErrorDetails(error)
            )
            errorText = String(describing: error)
        }
        restartUpdateLoopIfEnabled()
    }
    func startFromForeground() {
        guard foregroundStartKey == nil else {
            appendDiagnostic(category: "runtime", event: "foreground_start.coalesced")
            return
        }
        appendDiagnostic(category: "runtime", event: "foreground_start.requested")
        do {
            let runtime = try currentRuntime()
            let runtimeKey = openKey
            applyRuntimeSnapshot(try readRuntimeState(runtime))
            foregroundStartKey = runtimeKey
            enqueueRuntimeDispatch(
                .startRuntime,
                runtime: runtime,
                runtimeKey: runtimeKey,
                priority: .utility,
                onSuccess: { [weak self] _ in
                    guard let self else { return }
                    if self.foregroundStartKey == runtimeKey {
                        self.foregroundStartKey = nil
                    }
                    self.appendDiagnostic(category: "runtime", event: "foreground_start.succeeded")
                    self.errorText = nil
                    self.restartUpdateLoopIfEnabled()
                    self.flushPendingPushTokenIfPossible()
                    self.runLaunchAutomationIfRequested()
                },
                onFailure: { [weak self] error in
                    guard let self else { return }
                    if self.foregroundStartKey == runtimeKey {
                        self.foregroundStartKey = nil
                    }
                    self.appendDiagnostic(
                        category: "runtime",
                        event: "foreground_start.failed",
                        details: self.diagnosticErrorDetails(error)
                    )
                    self.errorText = String(describing: error)
                }
            )
        } catch {
            appendDiagnostic(
                category: "runtime",
                event: "foreground_open.failed",
                details: diagnosticErrorDetails(error)
            )
            errorText = String(describing: error)
        }
        restartUpdateLoopIfEnabled()
    }

    func registerPushToken(_ token: String) {
        let token = token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !token.isEmpty else { return }
        pendingPushToken = token
        appendDiagnostic(
            category: "push",
            event: "token.received",
            details: ["token_bytes": "\(token.count / 2)"]
        )
        flushPendingPushTokenIfPossible()
    }

    func notePushRegistrationFailed(_ error: Error) {
        appendDiagnostic(
            category: "push",
            event: "apns.registration.failed",
            details: diagnosticErrorDetails(error)
        )
    }

    func handleRemotePushWake(
        userInfo: [AnyHashable: Any],
        completion: @escaping (UIBackgroundFetchResult) -> Void
    ) {
        appendDiagnostic(
            category: "push",
            event: "wake.received",
            details: pushWakeDiagnosticDetails(userInfo)
        )
        Task { [weak self] in
            guard let self else {
                completion(.noData)
                return
            }
            do {
                let runtime = try currentRuntime()
                let runtimeKey = openKey
                enqueueRuntimeDispatch(
                    .startRuntime,
                    runtime: runtime,
                    runtimeKey: runtimeKey,
                    priority: .utility,
                    onSuccess: { [weak self] _ in
                        guard let self else {
                            completion(.noData)
                            return
                        }
                        self.errorText = nil
                        self.appendDiagnostic(category: "push", event: "wake.sync.succeeded")
                        self.restartUpdateLoopIfEnabled()
                        completion(.newData)
                    },
                    onFailure: { [weak self] error in
                        guard let self else {
                            completion(.failed)
                            return
                        }
                        self.appendDiagnostic(
                            category: "push",
                            event: "wake.sync.failed",
                            details: self.diagnosticErrorDetails(error)
                        )
                        completion(.failed)
                    }
                )
            } catch {
                appendDiagnostic(
                    category: "push",
                    event: "wake.sync.failed",
                    details: diagnosticErrorDetails(error)
                )
                completion(.failed)
            }
        }
    }

    func openRoom(_ room: AppRoomSummary) {
        dispatchInBackground(.openRoom(roomId: room.roomId)) { [weak self] in
            self?.markRoomRead(room)
        }
    }

    func openTopic(_ topic: AppTopicSummary) {
        dispatchInBackground(.openTopic(roomId: topic.roomId, topicId: topic.topicId))
    }

    func openChat(_ chat: AppChatSummary, in topic: AppTopicSummary) {
        dispatchInBackground(.openChat(
            roomId: topic.roomId,
            topicId: topic.topicId,
            chatId: chat.chatId
        ))
    }

    @discardableResult
    func openChat(
        roomID: String,
        topicID: String,
        chatID: String,
        onOpened: (@MainActor () -> Void)? = nil
    ) -> Bool {
        dispatchInBackground(
            .openChat(roomId: roomID, topicId: topicID, chatId: chatID),
            onSuccess: onOpened
        )
    }

    @discardableResult
    func pairAgent(_ room: AppRoomSummary, onPaired: (@MainActor () -> Void)? = nil) -> Bool {
        dispatchInBackground(
            .pairAgent(roomId: room.roomId),
            onSuccess: onPaired
        )
    }

    @discardableResult
    func startHomeChat(
        text rawText: String,
        intentKey: String,
        onStarted: (@MainActor () -> Void)? = nil,
        onFailure: (@MainActor () -> Void)? = nil
    ) -> Bool {
        let text = rawText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, pairedAgent != nil else { return false }
        return dispatchInBackground(
            .startHomeChat(
                text: text,
                intentKey: intentKey
            ),
            onSuccess: onStarted,
            onFailure: { _ in onFailure?() }
        )
    }

    @discardableResult
    func createHomeChat(
        intentKey: String,
        onCreated: (@MainActor () -> Void)? = nil,
        onFailure: (@MainActor () -> Void)? = nil
    ) -> Bool {
        guard pairedAgent != nil else { return false }
        return dispatchInBackground(
            .startHomeChat(
                text: nil,
                intentKey: intentKey
            ),
            onSuccess: onCreated,
            onFailure: { _ in onFailure?() }
        )
    }

    @discardableResult
    func renameChat(
        roomID: String,
        topicID: String,
        chatID: String,
        title rawTitle: String,
        onRenamed: (@MainActor () -> Void)? = nil,
        onFailure: (@MainActor () -> Void)? = nil
    ) -> Bool {
        let title = rawTitle.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty else { return false }
        return dispatchInBackground(
            .renameChat(
                roomId: roomID,
                topicId: topicID,
                chatId: chatID,
                title: title
            ),
            onSuccess: onRenamed,
            onFailure: { _ in onFailure?() }
        )
    }

    @discardableResult
    func archiveChat(
        roomID: String,
        topicID: String,
        chatID: String,
        onArchived: (@MainActor () -> Void)? = nil,
        onFailure: (@MainActor () -> Void)? = nil
    ) -> Bool {
        dispatchInBackground(
            .setChatArchived(
                roomId: roomID,
                topicId: topicID,
                chatId: chatID,
                archived: true
            ),
            onSuccess: onArchived,
            onFailure: { _ in onFailure?() }
        )
    }

    @discardableResult
    func createTopic(
        roomID: String,
        title rawTitle: String,
        onCreated: (@MainActor () -> Void)? = nil,
        onFailure: (@MainActor () -> Void)? = nil
    ) -> Bool {
        let title = rawTitle.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty else { return false }
        return dispatchInBackground(
            .createTopic(roomId: roomID, title: title),
            onSuccess: onCreated,
            onFailure: { _ in onFailure?() }
        )
    }

    @discardableResult
    func startTopicChat(
        roomID: String,
        topicID: String,
        onStarted: (@MainActor () -> Void)? = nil,
        onFailure: (@MainActor () -> Void)? = nil
    ) -> Bool {
        dispatchInBackground(
            .startTopicChat(roomId: roomID, topicId: topicID, reason: nil),
            onSuccess: onStarted,
            onFailure: { _ in onFailure?() }
        )
    }

    func projection(for roomID: String) -> ChatRoomProjection {
        chatProjections[roomID] ?? .empty(roomID: roomID)
    }

    @discardableResult
    func retry(_ message: ChatMessage) -> Bool {
        guard Self.messageAllowsRetry(message) else { return false }
        let key = "\(message.roomId)|\(message.messageId)"
        guard !messageRetriesInFlight.contains(key) else { return false }
        messageRetriesInFlight.insert(key)
        let runtime: any FiniteChatRuntimeProtocol
        let runtimeKey: String
        do {
            runtime = try currentRuntime()
            runtimeKey = openKey
        } catch {
            messageRetriesInFlight.remove(key)
            errorText = String(describing: error)
            return false
        }
        let action = AppAction.retryMessage(
            roomId: message.roomId,
            messageId: message.messageId
        )
        enqueueRuntimeDispatch(
            action,
            runtime: runtime,
            runtimeKey: runtimeKey,
            priority: .userInitiated,
            onSuccess: { [weak self] nextState in
                guard let self else { return }
                self.messageRetriesInFlight.remove(key)
                self.applyRuntimeSnapshot(nextState)
                self.errorText = nil
                self.restartUpdateLoopIfEnabled()
            },
            onFailure: { [weak self] error in
                guard let self else { return }
                self.messageRetriesInFlight.remove(key)
                self.errorText = String(describing: error)
            }
        )
        return true
    }

    @discardableResult
    func send(roomID: String, replyTo message: ChatMessage? = nil) -> Bool {
        let sent = send(roomID: roomID, text: outboundText, replyTo: message)
        if sent {
            outboundText = ""
        }
        return sent
    }

    @discardableResult
    func send(roomID: String, text rawText: String, replyTo message: ChatMessage? = nil) -> Bool {
        guard roomAllowsComposition(roomID) else { return false }
        let text = rawText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return false }
        let selectedRoute = compositionRoute(for: roomID, replyTo: message)
        let optimisticConversationID = message?.conversationId ?? selectedRoute?.topicID
        let optimisticChatID = message?.chatId ?? selectedRoute?.chatID
        let optimisticMessageID = installOptimisticMessage(
            roomID: roomID,
            text: text,
            replyToMessageID: message?.messageId,
            conversationID: optimisticConversationID,
            chatID: optimisticChatID
        )
        let action: AppAction
        if let message {
            if let selectedRoute {
                action = .sendChatReply(
                    roomId: roomID,
                    topicId: selectedRoute.topicID,
                    chatId: selectedRoute.chatID,
                    text: text,
                    replyToMessageId: message.messageId
                )
            } else {
                action = .sendReply(
                    roomId: roomID,
                    text: text,
                    replyToMessageId: message.messageId
                )
            }
        } else if let selectedRoute {
            action = .sendChatMessage(
                roomId: roomID,
                topicId: selectedRoute.topicID,
                chatId: selectedRoute.chatID,
                text: text
            )
        } else {
            action = .sendMessage(roomId: roomID, text: text)
        }
        let dispatched = dispatchInBackground(action) { [weak self] in
            if let optimisticMessageID {
                self?.removeOptimisticMessage(id: optimisticMessageID)
            }
            self?.schedulePostSendCatchUp()
        } onFailure: { [weak self] error in
            guard let optimisticMessageID else { return }
            self?.markOptimisticMessageFailed(
                id: optimisticMessageID,
                reason: String(describing: error)
            )
        }
        if !dispatched, let optimisticMessageID {
            removeOptimisticMessage(id: optimisticMessageID)
        }
        return dispatched
    }

    @discardableResult
    func send(replyTo message: ChatMessage? = nil) -> Bool {
        guard let roomID = selectedRoom?.roomId else { return false }
        return send(roomID: roomID, replyTo: message)
    }

    @discardableResult
    func sendAttachment(
        roomID: String,
        fileURL: URL,
        replyTo message: ChatMessage? = nil,
        onSuccess: (@MainActor () -> Void)? = nil
    ) -> Bool {
        guard roomAllowsComposition(roomID) else { return false }
        let caption = outboundText.trimmingCharacters(in: .whitespacesAndNewlines)
        let selectedRoute = compositionRoute(for: roomID, replyTo: message)
        outboundText = ""
        Task { [weak self] in
            guard let self else { return }
            do {
                let attachment = try await Task.detached(priority: .userInitiated) {
                    try Self.loadAttachment(from: fileURL)
                }.value
                let runtime = try currentRuntime()
                let runtimeKey = openKey
                let action: AppAction
                if let selectedRoute {
                    action = .sendChatAttachment(
                        roomId: roomID,
                        topicId: selectedRoute.topicID,
                        chatId: selectedRoute.chatID,
                        filename: attachment.filename,
                        mimeType: attachment.mimeType,
                        kind: attachment.kind,
                        bytes: attachment.data,
                        caption: caption,
                        replyToMessageId: message?.messageId
                    )
                } else {
                    action = .sendAttachment(
                        roomId: roomID,
                        filename: attachment.filename,
                        mimeType: attachment.mimeType,
                        kind: attachment.kind,
                        bytes: attachment.data,
                        caption: caption,
                        replyToMessageId: message?.messageId
                    )
                }
                enqueueRuntimeDispatch(
                    action,
                    runtime: runtime,
                    runtimeKey: runtimeKey,
                    priority: .userInitiated,
                    onSuccess: { [weak self] nextState in
                        guard let self else { return }
                        self.applyRuntimeSnapshot(nextState)
                        self.errorText = nil
                        onSuccess?()
                        self.restartUpdateLoopIfEnabled()
                        self.schedulePostSendCatchUp()
                    },
                    onFailure: { [weak self] error in
                        self?.errorText = String(describing: error)
                    }
                )
            } catch {
                errorText = String(describing: error)
            }
        }
        return true
    }

    @discardableResult
    func sendAttachments(
        roomID: String,
        attachments: [OutboundAttachment],
        replyTo message: ChatMessage? = nil,
        captionOverride: String? = nil,
        onSuccess: (@MainActor () -> Void)? = nil
    ) -> Bool {
        guard roomAllowsComposition(roomID) else { return false }
        guard !attachments.isEmpty else { return false }
        let caption = (captionOverride ?? outboundText)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let selectedRoute = compositionRoute(for: roomID, replyTo: message)
        Task { [weak self] in
            guard let self else { return }
            do {
                let runtime = try currentRuntime()
                let runtimeKey = openKey
                let action: AppAction
                if let selectedRoute {
                    action = .sendChatAttachments(
                        roomId: roomID,
                        topicId: selectedRoute.topicID,
                        chatId: selectedRoute.chatID,
                        attachments: attachments,
                        caption: caption,
                        replyToMessageId: message?.messageId
                    )
                } else {
                    action = .sendAttachments(
                        roomId: roomID,
                        attachments: attachments,
                        caption: caption,
                        replyToMessageId: message?.messageId
                    )
                }
                enqueueRuntimeDispatch(
                    action,
                    runtime: runtime,
                    runtimeKey: runtimeKey,
                    priority: .userInitiated,
                    onSuccess: { [weak self] nextState in
                        guard let self else { return }
                        self.applyRuntimeSnapshot(nextState)
                        if captionOverride == nil {
                            self.outboundText = ""
                        }
                        self.errorText = nil
                        onSuccess?()
                        self.restartUpdateLoopIfEnabled()
                        self.schedulePostSendCatchUp()
                    },
                    onFailure: { [weak self] error in
                        self?.errorText = String(describing: error)
                    }
                )
            } catch {
                errorText = String(describing: error)
            }
        }
        return true
    }

    @discardableResult
    func sendPoll(roomID: String, question: String, options: [String]) -> Bool {
        guard roomAllowsComposition(roomID) else { return false }
        let trimmedQuestion = question.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedOptions = options
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        guard !trimmedQuestion.isEmpty, trimmedOptions.count >= 2 else { return false }
        if let selectedRoute = selectedChatRoute(for: roomID) {
            return dispatchInBackground(.sendChatPoll(
                roomId: roomID,
                topicId: selectedRoute.topicID,
                chatId: selectedRoute.chatID,
                question: trimmedQuestion,
                options: trimmedOptions
            ))
        }
        return dispatchInBackground(.sendPoll(
            roomId: roomID,
            question: trimmedQuestion,
            options: trimmedOptions
        ))
    }

    func votePoll(message: ChatMessage, option: ChatPollOption) {
        dispatchInBackground(.votePoll(
            roomId: message.roomId,
            messageId: message.messageId,
            optionId: option.optionId
        ))
    }

    func downloadAttachment(roomID: String, message: ChatMessage, attachment: ChatMediaAttachment) {
        downloadAttachment(roomID: roomID, messageID: message.messageId, attachment: attachment)
    }

    func downloadAttachment(roomID: String, messageID: String, attachment: ChatMediaAttachment) {
        guard attachmentCanDownload(attachment) else { return }

        let key = "\(roomID)|\(messageID)|\(attachment.attachmentId)"
        guard !attachmentDownloadsInFlight.contains(key) else { return }
        attachmentDownloadsInFlight.insert(key)

        let runtime: any FiniteChatRuntimeProtocol
        let runtimeKey: String
        do {
            runtime = try currentRuntime()
            runtimeKey = openKey
        } catch {
            attachmentDownloadsInFlight.remove(key)
            errorText = String(describing: error)
            return
        }

        enqueueRuntimeDispatch(
            .beginDownloadAttachment(
                roomId: roomID,
                messageId: messageID,
                attachmentId: attachment.attachmentId
            ),
            runtime: runtime,
            runtimeKey: runtimeKey,
            priority: .utility,
            onSuccess: { [weak self] beginState in
                guard let self else { return }
                self.applyRuntimeSnapshot(beginState)
                self.errorText = nil
                self.restartUpdateLoopIfEnabled()

                self.enqueueRuntimeDispatch(
                    .downloadAttachment(
                        roomId: roomID,
                        messageId: messageID,
                        attachmentId: attachment.attachmentId
                    ),
                    runtime: runtime,
                    runtimeKey: runtimeKey,
                    priority: .utility,
                    onSuccess: { [weak self] nextState in
                        guard let self else { return }
                        self.attachmentDownloadsInFlight.remove(key)
                        self.applyRuntimeSnapshot(nextState)
                        self.errorText = nil
                        self.restartUpdateLoopIfEnabled()
                    },
                    onFailure: { [weak self] error in
                        guard let self else { return }
                        self.attachmentDownloadsInFlight.remove(key)
                        self.errorText = String(describing: error)
                    }
                )
            },
            onFailure: { [weak self] error in
                guard let self else { return }
                self.attachmentDownloadsInFlight.remove(key)
                self.errorText = String(describing: error)
            }
        )
    }

    func loadOlderMessages(roomID: String, beforeMessageID: String) {
        dispatchInBackground(
            .loadOlderMessages(
                roomId: roomID,
                beforeMessageId: beforeMessageID,
                limit: 50
            ),
            priority: .utility
        )
    }

    func react(to message: ChatMessage, emoji: String) {
        dispatchInBackground(
            .reactToMessage(
                roomId: message.roomId,
                messageId: message.messageId,
                emoji: emoji
            ),
            priority: .utility
        )
    }

    func markRoomRead(_ room: AppRoomSummary) {
        dispatchInBackground(.markRoomRead(roomId: room.roomId), priority: .utility)
    }

    func setTyping(roomID: String, isTyping: Bool) {
        let previousIntent = lastTypingIntentByRoom[roomID] ?? false
        guard previousIntent != isTyping else { return }
        lastTypingIntentByRoom[roomID] = isTyping
        dispatchInBackground(
            .setTyping(roomId: roomID, isTyping: isTyping),
            priority: .utility,
            showsError: false,
            restartsUpdateLoop: false
        )
    }

    private func applyNostrIdentity(
        _ identity: AppNostrIdentity,
        resetStore: Bool
    ) throws {
        closeRuntime()
        if resetStore {
            try? RuntimeDataStore.deleteDataDir(
                deviceID: deviceID,
                applicationSupportURL: applicationSupportURL,
                transient: usesTransientStore
            )
        }
        try nostrIdentityStore.save(identity)
        nostrIdentity = identity
        requiresNostrLogin = false
        canRecoverRuntimeIdentity = false
        appendDiagnostic(category: "persistence", event: "nostr_identity.applied")
        start()
    }

    private func flushPendingPushTokenIfPossible() {
        guard let token = pendingPushToken else { return }
        guard pushTokenRegistrationInFlight != token else { return }
        pushTokenRegistrationInFlight = token
        appendDiagnostic(category: "push", event: "token.register.requested")
        dispatchInBackground(
            .setPushToken(token: token),
            priority: .utility,
            showsError: false
        ) { [weak self] in
            guard let self else { return }
            if self.pendingPushToken == token {
                self.pendingPushToken = nil
            }
            if self.pushTokenRegistrationInFlight == token {
                self.pushTokenRegistrationInFlight = nil
            }
            self.appendDiagnostic(category: "push", event: "token.register.succeeded")
        } onFailure: { [weak self] _ in
            guard let self else { return }
            if self.pushTokenRegistrationInFlight == token {
                self.pushTokenRegistrationInFlight = nil
            }
        }
    }

    private func removePushTokenIfPossible() {
        guard runtime != nil else { return }
        appendDiagnostic(category: "push", event: "token.remove.requested")
        dispatchInBackground(
            .removePushToken,
            priority: .utility,
            showsError: false
        ) { [weak self] in
            self?.appendDiagnostic(category: "push", event: "token.remove.succeeded")
        }
    }

    private func removePushTokenDuringSignOut(runtime: any FiniteChatRuntimeProtocol) {
        appendDiagnostic(category: "push", event: "token.remove.requested")
        Task.detached(priority: .utility) { [runtime] in
            _ = try? runtime.dispatchAndWait(action: .removePushToken)
        }
    }

    private func startUpdateLoop() {
        updateTask?.cancel()
        guard let runtime else { return }
        let runtimeKey = openKey
        updateTask = Task { [weak self, runtime, runtimeKey] in
            while !Task.isCancelled {
                do {
                    try? await Task.sleep(nanoseconds: 2_000_000_000)
                    guard !Task.isCancelled else { return }
                    let nextState = try await Task.detached(priority: .background) {
                        try runtime.waitForUpdate(timeoutMillis: 30_000)
                    }.value
                    guard !Task.isCancelled else { return }
                    guard let self, self.openKey == runtimeKey else { return }
                    self.applyRuntimeSnapshot(nextState)
                    self.appendDiagnostic(category: "runtime", event: "update.polled")
                    self.errorText = nil
                } catch {
                    guard !Task.isCancelled else { return }
                    guard let self, self.openKey == runtimeKey else { return }
                    self.appendDiagnostic(
                        category: "runtime",
                        event: "update.failed",
                        details: self.diagnosticErrorDetails(error)
                    )
                    let description = String(describing: error)
                    if !Self.isTransientUpdateHintError(description) {
                        self.errorText = description
                    }
                    try? await Task.sleep(nanoseconds: 1_000_000_000)
                }
            }
        }
    }

    private static func isTransientUpdateHintError(_ description: String) -> Bool {
        description.contains("SSE hint stream")
            || description.contains("sync hint stream")
            || description.contains("Sync hint stream")
    }

    @discardableResult
    private func enqueueRuntimeDispatch(
        _ action: AppAction,
        runtime: any FiniteChatRuntimeProtocol,
        runtimeKey: String,
        priority: TaskPriority,
        onSuccess: @escaping @MainActor (AppState) -> Void,
        onFailure: @escaping @MainActor (Error) -> Void
    ) -> Bool {
        let previousDispatch = runtimeDispatchTail
        let dispatchTask = Task { @MainActor [weak self, runtime, runtimeKey, previousDispatch] in
            await previousDispatch?.value
            guard !Task.isCancelled, let self else { return }
            do {
                let nextState = try await Task.detached(priority: priority) {
                    let performanceInterval = FinitePerformance.begin("Rust runtime dispatch")
                    defer { FinitePerformance.end(performanceInterval) }
                    return try runtime.dispatchAndWait(action: action)
                }.value
                guard !Task.isCancelled, self.openKey == runtimeKey else { return }
                self.applyRuntimeSnapshot(nextState)
                onSuccess(nextState)
            } catch {
                guard !Task.isCancelled, self.openKey == runtimeKey else { return }
                onFailure(error)
            }
        }
        runtimeDispatchTail = dispatchTask
        return true
    }

    @discardableResult
    private func dispatchInBackground(
        _ action: AppAction,
        priority: TaskPriority = .userInitiated,
        showsError: Bool = true,
        restartsUpdateLoop: Bool = true,
        onSuccess: (@MainActor () -> Void)? = nil,
        onFailure: (@MainActor (Error) -> Void)? = nil
    ) -> Bool {
        let diagnostic = diagnosticAction(action)
        appendDiagnostic(
            category: diagnostic.category,
            event: "\(diagnostic.name).requested",
            details: diagnostic.details
        )
        let runtime: any FiniteChatRuntimeProtocol
        let runtimeKey: String
        do {
            runtime = try currentRuntime()
            runtimeKey = openKey
        } catch {
            appendDiagnostic(
                category: diagnostic.category,
                event: "\(diagnostic.name).failed",
                details: diagnosticErrorDetails(error)
            )
            if showsError {
                errorText = String(describing: error)
            }
            return false
        }

        return enqueueRuntimeDispatch(
            action,
            runtime: runtime,
            runtimeKey: runtimeKey,
            priority: priority,
            onSuccess: { [weak self] nextState in
                guard let self else { return }
                self.applyRuntimeSnapshot(nextState)
                self.errorText = nil
                self.appendDiagnostic(
                    category: diagnostic.category,
                    event: "\(diagnostic.name).succeeded",
                    details: diagnostic.details
                )
                if restartsUpdateLoop {
                    self.restartUpdateLoopIfEnabled()
                }
                onSuccess?()
            },
            onFailure: { [weak self] error in
                guard let self else { return }
                self.appendDiagnostic(
                    category: diagnostic.category,
                    event: "\(diagnostic.name).failed",
                    details: self.diagnosticErrorDetails(error)
                )
                if showsError {
                    self.errorText = String(describing: error)
                }
                onFailure?(error)
            }
        )
    }

    private func restartUpdateLoopIfEnabled() {
        if startsUpdateLoop {
            startUpdateLoop()
        }
    }

    private func schedulePostSendCatchUp() {
        postSendCatchUpTask?.cancel()
        guard runtime != nil else { return }
        let runtimeKey = openKey
        postSendCatchUpTask = Task { [weak self, runtimeKey] in
            for delay in [1_000_000_000, 3_000_000_000, 6_000_000_000, 12_000_000_000] as [UInt64] {
                try? await Task.sleep(nanoseconds: delay)
                guard !Task.isCancelled else { return }
                guard let self, self.openKey == runtimeKey else { return }
                do {
                    let runtime = try currentRuntime()
                    let nextState = try await Task.detached(priority: .utility) {
                        try runtime.dispatchAndWait(action: .startRuntime)
                    }.value
                    guard !Task.isCancelled, openKey == runtimeKey else { return }
                    applyRuntimeSnapshot(nextState)
                    errorText = nil
                    appendDiagnostic(category: "runtime", event: "post_send_catchup.succeeded")
                    restartUpdateLoopIfEnabled()
                } catch {
                    appendDiagnostic(
                        category: "runtime",
                        event: "post_send_catchup.failed",
                        details: diagnosticErrorDetails(error)
                    )
                }
            }
        }
    }

    private func currentRuntime() throws -> any FiniteChatRuntimeProtocol {
        if let launchConfigurationError {
            throw AppLaunchConfigurationError(message: launchConfigurationError)
        }
        if requiresNostrLogin {
            throw AppLaunchConfigurationError(message: "Create or sign in to a Nostr account first.")
        }
        let accountSecretHex = nostrIdentity?.accountSecretHex
        let key = "\(serverURL)|\(deviceID)|\(accountSecretHex ?? "stored")"
        if let runtime, openKey == key {
            return runtime
        }
        let performanceInterval = FinitePerformance.begin(
            "Open Rust runtime",
            warningBudgetMilliseconds: 16.67
        )
        defer { FinitePerformance.end(performanceInterval) }
        let dataDir = try RuntimeDataStore.dataDir(
            deviceID: deviceID,
            applicationSupportURL: applicationSupportURL,
            transient: usesTransientStore
        )
        runtimeStorePath = dataDir
        appendDiagnostic(
            category: "persistence",
            event: "store.resolved",
            details: [
                "store": redactedPathSummary(dataDir),
                "mode": usesTransientStore ? "transient" : "stable",
            ]
        )
        let opened = try runtimeFactory(
            OpenOptions(
                dataDir: dataDir,
                serverUrl: serverURL,
                deviceId: deviceID,
                accountSecretHex: accountSecretHex,
                nowUnixSeconds: nil
            )
        )
        let openedState = try readRuntimeState(opened)
        applyRuntimeSnapshot(openedState)
        syncNostrIdentityFromRuntime(openedState.identity)
        let resolvedDeviceID = openedState.identity.deviceId
        if resolvedDeviceID != deviceID {
            appendDiagnostic(
                category: "runtime",
                event: "identity.resolved",
                details: ["device_changed": "true"]
            )
            deviceID = resolvedDeviceID
        }
        if !usesTransientStore && persistsRuntimeIdentityUpdates {
            try? RuntimeConfig(
                serverURL: serverURL,
                dashboardURL: dashboardURL,
                deviceID: resolvedDeviceID
            ).save(
                storageURL: configStorageURL
            )
        }
        runtime = opened
        let resolvedAccountSecretHex = nostrIdentity?.accountSecretHex ?? accountSecretHex
        openKey = "\(serverURL)|\(deviceID)|\(resolvedAccountSecretHex ?? "stored")"
        opened.listenForUpdates(reconciler: self)
        appendDiagnostic(category: "runtime", event: "open.succeeded")
        return opened
    }

    private func readRuntimeState(
        _ runtime: any FiniteChatRuntimeProtocol
    ) throws -> AppState {
        let performanceInterval = FinitePerformance.begin(
            "Read Rust runtime state",
            warningBudgetMilliseconds: 8
        )
        defer { FinitePerformance.end(performanceInterval) }
        return try runtime.state()
    }

    private func syncNostrIdentityFromRuntime(_ identity: Identity) {
        guard nostrIdentity == nil else { return }
        guard let material = try? nostrIdentityFromAccountSecretHex(
            accountSecretHex: identity.accountSecretHex
        ) else {
            return
        }
        let appIdentity = AppNostrIdentity(material: material)
        try? nostrIdentityStore.save(appIdentity)
        nostrIdentity = appIdentity
        canRecoverRuntimeIdentity = false
    }

    private func closeRuntime() {
        updateTask?.cancel()
        launchAutomationTask?.cancel()
        postSendCatchUpTask?.cancel()
        updateTask = nil
        launchAutomationTask = nil
        postSendCatchUpTask = nil
        runtimeDispatchTail?.cancel()
        runtimeDispatchTail = nil
        foregroundStartKey = nil
        attachmentDownloadsInFlight.removeAll()
        messageRetriesInFlight.removeAll()
        lastTypingIntentByRoom.removeAll()
        pushTokenRegistrationInFlight = nil
        runtime = nil
        openKey = ""
        state = nil
        lastAppliedRuntimeRev = 0
        runtimeStorePath = nil
        pendingOptimisticMessages = [:]
        optimisticMessageCounter = 0
        chatProjections = [:]
    }

    private func rebuildChatProjections() {
        let performanceInterval = FinitePerformance.begin(
            "Rebuild chat projections",
            warningBudgetMilliseconds: 8
        )
        defer { FinitePerformance.end(performanceInterval) }
        guard let state else {
            chatProjections = [:]
            return
        }
        pruneConfirmedOptimisticMessages(confirmedMessages: state.messages)
        let messages = state.messages + pendingOptimisticMessages.values
        chatProjections = ChatTimeline.roomProjections(
            messages: messages,
            typingMembers: state.typingMembers,
            profiles: state.profiles
        )
    }

    private func installOptimisticMessage(
        roomID: String,
        text: String,
        replyToMessageID: String?,
        conversationID: String? = nil,
        chatID: String? = nil
    ) -> String? {
        guard let state else { return nil }
        optimisticMessageCounter &+= 1
        let messageID = "optimistic-\(String(format: "%020llu", optimisticMessageCounter))-\(UUID().uuidString)"
        let timestamp = Date()
        let timestampSeconds = UInt64(max(0, timestamp.timeIntervalSince1970))
        let sequenceOffset = optimisticMessageCounter % 1_000_000
        let message = ChatMessage(
            roomId: roomID,
            seq: Self.optimisticSequenceBase + sequenceOffset,
            messageId: messageID,
            conversationId: conversationID,
            chatId: chatID,
            senderAccountId: state.identity.accountId,
            senderDeviceId: state.identity.deviceId,
            senderDisplayName: state.identity.deviceId,
            senderNpub: myNpub,
            text: text,
            displayContent: text,
            richTextJson: "",
            kind: .message,
            status: .complete,
            finalDelivery: true,
            editOfMessageId: nil,
            payload: Data(text.utf8),
            replyToMessageId: replyToMessageID,
            isMine: true,
            outboundDelivery: OutboundDelivery(
                localSend: .sending,
                serverDelivery: .undelivered
            ),
            reactions: [],
            media: [],
            readReceipt: nil,
            poll: nil,
            timestampUnixSeconds: timestampSeconds,
            displayTimestamp: Self.optimisticTimestampFormatter.string(from: timestamp)
        )
        pendingOptimisticMessages[messageID] = message
        rebuildChatProjections()
        return messageID
    }

    private func removeOptimisticMessage(id: String) {
        guard pendingOptimisticMessages.removeValue(forKey: id) != nil else { return }
        rebuildChatProjections()
    }

    private func markOptimisticMessageFailed(id: String, reason: String) {
        guard var message = pendingOptimisticMessages[id] else { return }
        message.outboundDelivery = OutboundDelivery(
            localSend: .sent,
            serverDelivery: .failed(reason: reason)
        )
        pendingOptimisticMessages[id] = message
        rebuildChatProjections()
    }

    private func pruneConfirmedOptimisticMessages(confirmedMessages: [ChatMessage]) {
        guard !pendingOptimisticMessages.isEmpty else { return }
        pendingOptimisticMessages = pendingOptimisticMessages.filter { _, pending in
            !confirmedMessages.contains { confirmed in
                confirmed.isMine
                    && confirmed.roomId == pending.roomId
                    && confirmed.text == pending.text
                    && confirmed.replyToMessageId == pending.replyToMessageId
                    && confirmed.messageId != pending.messageId
                    && confirmed.timestampUnixSeconds + 30 >= pending.timestampUnixSeconds
            }
        }
    }

    private func appendStateDiagnostic(_ state: AppState, event: String) {
        let outboundMessages = state.messages.compactMap(\.outboundDelivery)
        let projectedChats = state.topics.flatMap(\.chats)
        let projectedHistoryMessages = projectedChats.reduce(UInt64(0)) {
            $0 + UInt64($1.messageCount)
        }
        var undelivered = 0
        var delivered = 0
        var failed = 0
        for delivery in outboundMessages {
            switch delivery.serverDelivery {
            case .undelivered:
                undelivered += 1
            case .delivered:
                delivered += 1
            case .failed:
                failed += 1
            }
        }
        let roomStates = Dictionary(grouping: state.rooms, by: \.state)
            .mapValues(\.count)
        appendDiagnostic(
            category: "runtime",
            event: event,
            details: [
                "rev": "\(state.rev)",
                "status": Self.redactedDiagnosticValue(state.status),
                "rooms": "\(state.rooms.count)",
                "connected_rooms": "\(roomStates[.connected] ?? 0)",
                "unavailable_rooms": "\(roomStates[.unavailableOnDevice] ?? 0)",
                "selected_room": state.selectedRoomId.map(Self.redactedDiagnosticValue) ?? "none",
                "messages": "\(state.messages.count)",
                "topics": "\(state.topics.count)",
                "chats": "\(projectedChats.count)",
                "history_messages": "\(projectedHistoryMessages)",
                "outbound": "\(outboundMessages.count)",
                "undelivered": "\(undelivered)",
                "delivered": "\(delivered)",
                "failed": "\(failed)",
                "profiles": "\(state.profiles.count)",
                "devices": "\(state.devices.count)",
            ]
        )
    }

    private func appendDiagnostic(
        category: String,
        event: String,
        details: [String: String] = [:]
    ) {
        let sanitizedDetails = details.reduce(into: [String: String]()) { output, item in
            output[item.key] = Self.redactedDiagnosticValue(item.value)
        }
        developerDiagnostics.append(DeveloperDiagnosticEntry(
            id: (developerDiagnostics.last?.id ?? 0) + 1,
            timestampUnixSeconds: Int64(Date().timeIntervalSince1970),
            category: Self.redactedDiagnosticValue(category),
            event: Self.redactedDiagnosticValue(event),
            details: sanitizedDetails
        ))
#if DEBUG
        if let entry = developerDiagnostics.last {
            persistDebugDiagnostic(entry)
        }
#endif
        if developerDiagnostics.count > Self.developerDiagnosticsLimit {
            developerDiagnostics.removeFirst(
                developerDiagnostics.count - Self.developerDiagnosticsLimit
            )
        }
    }

#if DEBUG
    private func persistDebugDiagnostic(_ entry: DeveloperDiagnosticEntry) {
        let details = entry.details
            .sorted { $0.key < $1.key }
            .map { "\($0.key)=\($0.value)" }
            .joined(separator: " ")
        let line: String
        if details.isEmpty {
            line = "seq=\(entry.id) ts=\(entry.timestampUnixSeconds) category=\(entry.category) event=\(entry.event)\n"
        } else {
            line = "seq=\(entry.id) ts=\(entry.timestampUnixSeconds) category=\(entry.category) event=\(entry.event) \(details)\n"
        }
        guard let data = line.data(using: .utf8) else { return }
        let configuredSupportURL = applicationSupportURL
        Self.debugDiagnosticsQueue.async {
            let supportURL: URL
            if let configuredSupportURL {
                supportURL = configuredSupportURL
            } else if let defaultURL = try? FileManager.default.url(
                for: .applicationSupportDirectory,
                in: .userDomainMask,
                appropriateFor: nil,
                create: true
            ) {
                supportURL = defaultURL
            } else {
                return
            }
            let url = supportURL.appendingPathComponent("finitechat_debug_diagnostics.log")
            Self.appendDebugDiagnostic(data, to: url)
        }
    }

    nonisolated private static func appendDebugDiagnostic(_ data: Data, to url: URL) {
        if FileManager.default.fileExists(atPath: url.path),
           let handle = try? FileHandle(forWritingTo: url)
        {
            _ = try? handle.seekToEnd()
            try? handle.write(contentsOf: data)
            try? handle.close()
        } else {
            try? data.write(to: url, options: .atomic)
        }
    }
#endif

    private func diagnosticErrorDetails(_ error: Error) -> [String: String] {
        diagnosticErrorDetails(String(describing: error))
    }

    private func diagnosticErrorDetails(_ errorText: String?) -> [String: String] {
        guard let errorText = errorText?.trimmingCharacters(in: .whitespacesAndNewlines),
              !errorText.isEmpty
        else {
            return [:]
        }
        return ["error": Self.redactedDiagnosticValue(errorText)]
    }

    private func diagnosticAction(_ action: AppAction) -> DiagnosticActionSummary {
        switch action {
        case .startRuntime:
            return DiagnosticActionSummary(category: "runtime", name: "start_runtime", details: [:])
        case .stopRuntime:
            return DiagnosticActionSummary(category: "runtime", name: "stop_runtime", details: [:])
        case .openRoom(let roomId):
            return DiagnosticActionSummary(
                category: "runtime",
                name: "open_room",
                details: ["room": roomId]
            )
        case .openTopic(let roomId, let topicId):
            return DiagnosticActionSummary(
                category: "runtime",
                name: "open_topic",
                details: ["room": roomId, "topic": topicId]
            )
        case .openChat(let roomId, let topicId, let chatId):
            return DiagnosticActionSummary(
                category: "runtime",
                name: "open_chat",
                details: ["room": roomId, "topic": topicId, "chat": chatId]
            )
        case .pairAgent(let roomId):
            return DiagnosticActionSummary(
                category: "runtime",
                name: "pair_agent",
                details: ["room": roomId]
            )
        case .startHomeChat:
            return DiagnosticActionSummary(
                category: "transport",
                name: "start_home_chat",
                details: [:]
            )
        case .renameChat(let roomId, let topicId, let chatId, _):
            return DiagnosticActionSummary(
                category: "transport",
                name: "rename_chat",
                details: ["room": roomId, "topic": topicId, "chat": chatId]
            )
        case .setChatArchived(let roomId, let topicId, let chatId, let archived):
            return DiagnosticActionSummary(
                category: "transport",
                name: archived ? "archive_chat" : "restore_chat",
                details: ["room": roomId, "topic": topicId, "chat": chatId]
            )
        case .createTopic(let roomId, _):
            return DiagnosticActionSummary(
                category: "transport",
                name: "create_topic",
                details: ["room": roomId]
            )
        case .startTopicChat(let roomId, let topicId, _):
            return DiagnosticActionSummary(
                category: "transport",
                name: "start_topic_chat",
                details: ["room": roomId, "topic": topicId]
            )
        case .sendMessage(let roomId, _):
            return DiagnosticActionSummary(
                category: "transport",
                name: "send_message",
                details: ["room": roomId]
            )
        case .sendTopicMessage(let roomId, let topicId, _):
            return DiagnosticActionSummary(
                category: "transport",
                name: "send_topic_message",
                details: ["room": roomId, "topic": topicId]
            )
        case .sendChatMessage(let roomId, let topicId, let chatId, _):
            return DiagnosticActionSummary(
                category: "transport",
                name: "send_chat_message",
                details: ["room": roomId, "topic": topicId, "chat": chatId]
            )
        case .sendReply(let roomId, _, let replyToMessageId):
            return DiagnosticActionSummary(
                category: "transport",
                name: "send_reply",
                details: ["room": roomId, "reply_to": replyToMessageId]
            )
        case .sendChatReply(let roomId, let topicId, let chatId, _, let replyToMessageId):
            return DiagnosticActionSummary(
                category: "transport",
                name: "send_chat_reply",
                details: [
                    "room": roomId,
                    "topic": topicId,
                    "chat": chatId,
                    "reply_to": replyToMessageId,
                ]
            )
        case .sendAttachment(let roomId, _, _, _, _, let caption, let replyToMessageId):
            var details = [
                "room": roomId,
                "attachment_count": "1",
                "has_caption": caption.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    ? "false" : "true",
            ]
            if let replyToMessageId {
                details["reply_to"] = replyToMessageId
            }
            return DiagnosticActionSummary(
                category: "transport",
                name: "send_attachment",
                details: details
            )
        case .sendChatAttachment(let roomId, let topicId, let chatId, _, _, _, _, let caption, let replyToMessageId):
            var details = [
                "room": roomId,
                "topic": topicId,
                "chat": chatId,
                "attachment_count": "1",
                "has_caption": caption.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    ? "false" : "true",
            ]
            if let replyToMessageId {
                details["reply_to"] = replyToMessageId
            }
            return DiagnosticActionSummary(
                category: "transport",
                name: "send_chat_attachment",
                details: details
            )
        case .sendAttachments(let roomId, let attachments, let caption, let replyToMessageId):
            var details = [
                "room": roomId,
                "attachment_count": "\(attachments.count)",
                "has_caption": caption.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    ? "false" : "true",
            ]
            if let replyToMessageId {
                details["reply_to"] = replyToMessageId
            }
            return DiagnosticActionSummary(
                category: "transport",
                name: "send_attachments",
                details: details
            )
        case .sendChatAttachments(let roomId, let topicId, let chatId, let attachments, let caption, let replyToMessageId):
            var details = [
                "room": roomId,
                "topic": topicId,
                "chat": chatId,
                "attachment_count": "\(attachments.count)",
                "has_caption": caption.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    ? "false" : "true",
            ]
            if let replyToMessageId {
                details["reply_to"] = replyToMessageId
            }
            return DiagnosticActionSummary(
                category: "transport",
                name: "send_chat_attachments",
                details: details
            )
        case .sendPoll(let roomId, _, let options):
            return DiagnosticActionSummary(
                category: "transport",
                name: "send_poll",
                details: ["room": roomId, "option_count": "\(options.count)"]
            )
        case .sendChatPoll(let roomId, let topicId, let chatId, _, let options):
            return DiagnosticActionSummary(
                category: "transport",
                name: "send_chat_poll",
                details: [
                    "room": roomId,
                    "topic": topicId,
                    "chat": chatId,
                    "option_count": "\(options.count)",
                ]
            )
        case .votePoll(let roomId, let messageId, let optionId):
            return DiagnosticActionSummary(
                category: "transport",
                name: "vote_poll",
                details: ["room": roomId, "message": messageId, "option": optionId]
            )
        case .downloadAttachment(let roomId, let messageId, let attachmentId):
            return DiagnosticActionSummary(
                category: "transport",
                name: "download_attachment",
                details: ["room": roomId, "message": messageId, "attachment": attachmentId]
            )
        case .beginDownloadAttachment(let roomId, let messageId, let attachmentId):
            return DiagnosticActionSummary(
                category: "transport",
                name: "begin_download_attachment",
                details: ["room": roomId, "message": messageId, "attachment": attachmentId]
            )
        case .loadOlderMessages(let roomId, let beforeMessageId, let limit):
            return DiagnosticActionSummary(
                category: "runtime",
                name: "load_older_messages",
                details: [
                    "room": roomId,
                    "before": beforeMessageId,
                    "limit": "\(limit)",
                ]
            )
        case .reactToMessage(let roomId, let messageId, _):
            return DiagnosticActionSummary(
                category: "transport",
                name: "react_to_message",
                details: ["room": roomId, "message": messageId]
            )
        case .markRoomRead(let roomId):
            return DiagnosticActionSummary(
                category: "runtime",
                name: "mark_room_read",
                details: ["room": roomId]
            )
        case .retryMessage(let roomId, let messageId):
            return DiagnosticActionSummary(
                category: "transport",
                name: "retry_message",
                details: ["room": roomId, "message": messageId]
            )
        case .setTyping(let roomId, let isTyping):
            return DiagnosticActionSummary(
                category: "transport",
                name: "set_typing",
                details: ["room": roomId, "typing": isTyping ? "true" : "false"]
            )
        case .setPushToken:
            return DiagnosticActionSummary(
                category: "push",
                name: "set_push_token",
                details: [:]
            )
        case .removePushToken:
            return DiagnosticActionSummary(
                category: "push",
                name: "remove_push_token",
                details: [:]
            )
        default:
            return DiagnosticActionSummary(
                category: "runtime",
                name: "unsupported_ios_action",
                details: [:]
            )
        }
    }

    private func pushWakeDiagnosticDetails(_ userInfo: [AnyHashable: Any]) -> [String: String] {
        var details = [String: String]()
        if let roomID = userInfo["room_id"] as? String {
            details["room"] = roomID
        }
        if let seq = userInfo["seq"] {
            details["seq"] = "\(seq)"
        }
        return details
    }

    private func redactedPathSummary(_ path: String) -> String {
        let components = URL(fileURLWithPath: path).standardizedFileURL.pathComponents
        let suffix = components.suffix(2).joined(separator: "/")
        return suffix.isEmpty ? "[path]" : "[path:\(suffix)]"
    }

    private static func redactedDiagnosticValue(_ value: String) -> String {
        var output = value.trimmingCharacters(in: .whitespacesAndNewlines)
        for redaction in diagnosticRedactions {
            let range = NSRange(output.startIndex..<output.endIndex, in: output)
            output = redaction.regex.stringByReplacingMatches(
                in: output,
                options: [],
                range: range,
                withTemplate: redaction.replacement
            )
        }
        if output.count > 240 {
            output = String(output.prefix(237)) + "..."
        }
        return output
    }

    private func runLaunchAutomationIfRequested() {
        guard !didRunLaunchAutomation else { return }
        let outbound = Self.argumentValue("--finitechat-auto-send", in: args)
        let attachmentText = Self.argumentValue(
            "--finitechat-auto-send-attachment-text",
            in: args
        )
        let attachmentFile = Self.argumentValue(
            "--finitechat-auto-send-attachment-file",
            in: args
        )
        let attachmentBase64 = Self.argumentValue(
            "--finitechat-auto-send-attachment-base64",
            in: args
        )
        let attachmentFilename = Self.argumentValue(
            "--finitechat-auto-send-attachment-filename",
            in: args
        ) ?? "launch-automation.bin"
        let attachmentMimeType = Self.argumentValue(
            "--finitechat-auto-send-attachment-mime-type",
            in: args
        ) ?? "application/octet-stream"
        let attachmentCaption = Self.argumentValue(
            "--finitechat-auto-send-attachment-caption",
            in: args
        )
        guard outbound != nil
            || attachmentText != nil
            || attachmentFile != nil
            || attachmentBase64 != nil
        else {
            return
        }

        didRunLaunchAutomation = true
        deviceID = Self.argumentValue("--finitechat-device", in: args) ?? deviceID
        serverURL = Self.argumentValue("--finitechat-server", in: args) ?? serverURL
        let requestedRoomID = Self.argumentValue("--finitechat-room", in: args)

        launchAutomationTask = Task {
            let roomID = requestedRoomID ?? self.state?.selectedRoomId
            if let outbound, !outbound.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                await self.sendLaunchAutomationMessage(roomID: roomID, text: outbound)
            }
            if let attachmentText,
               !attachmentText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            {
                await self.sendLaunchAutomationAttachment(roomID: roomID, text: attachmentText)
            }
            if let attachmentFile,
               !attachmentFile.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            {
                await self.sendLaunchAutomationAttachmentFile(
                    roomID: roomID,
                    path: attachmentFile,
                    caption: attachmentCaption ?? ""
                )
            }
            if let attachmentBase64,
               !attachmentBase64.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            {
                await self.sendLaunchAutomationAttachmentBase64(
                    roomID: roomID,
                    base64: attachmentBase64,
                    filename: attachmentFilename,
                    mimeType: attachmentMimeType,
                    caption: attachmentCaption ?? ""
                )
            }
        }
    }

    private func launchAutomationRoom(roomID: String?) -> AppRoomSummary? {
        guard let state else { return nil }
        if let roomID {
            return state.rooms.first { $0.roomId == roomID }
        }
        return selectedRoom
    }

    private func sendLaunchAutomationMessage(roomID: String?, text: String) async {
        let deadline = Date().addingTimeInterval(90)
        while !Task.isCancelled, Date() < deadline {
            if let room = launchAutomationRoom(roomID: roomID), room.state == .connected {
                send(roomID: room.roomId, text: text)
                return
            }
            try? await Task.sleep(nanoseconds: 500_000_000)
        }
        outboundText = text
        errorText = "Launch automation timed out waiting for the room to connect"
    }

    private func sendLaunchAutomationAttachmentBase64(
        roomID: String?,
        base64: String,
        filename: String,
        mimeType: String,
        caption: String
    ) async {
        let deadline = Date().addingTimeInterval(90)
        while !Task.isCancelled, Date() < deadline {
            if let room = launchAutomationRoom(roomID: roomID), room.state == .connected {
                let normalized = base64.trimmingCharacters(in: .whitespacesAndNewlines)
                guard let data = Data(base64Encoded: normalized) else {
                    errorText = "Launch automation attachment base64 was invalid"
                    return
                }
                let cleanedFilename = filename.trimmingCharacters(in: .whitespacesAndNewlines)
                let finalFilename = cleanedFilename.isEmpty ? "launch-automation.bin" : cleanedFilename
                let cleanedMimeType = mimeType.trimmingCharacters(in: .whitespacesAndNewlines)
                let finalMimeType = cleanedMimeType.isEmpty
                    ? "application/octet-stream"
                    : cleanedMimeType
                let type = UTType(filenameExtension: URL(fileURLWithPath: finalFilename).pathExtension)
                let attachment = OutboundAttachment(
                    filename: finalFilename,
                    mimeType: finalMimeType,
                    kind: Self.chatMediaKind(for: type),
                    bytes: data
                )
                sendAttachments(
                    roomID: room.roomId,
                    attachments: [attachment],
                    captionOverride: caption
                )
                return
            }
            try? await Task.sleep(nanoseconds: 500_000_000)
        }
        errorText = "Launch automation timed out waiting for the room to connect"
    }

    private func sendLaunchAutomationAttachmentFile(
        roomID: String?,
        path: String,
        caption: String
    ) async {
        let fileURL = URL(fileURLWithPath: path).standardizedFileURL
        let deadline = Date().addingTimeInterval(90)
        while !Task.isCancelled, Date() < deadline {
            if let room = launchAutomationRoom(roomID: roomID), room.state == .connected {
                do {
                    let prepared = try await Task.detached(priority: .userInitiated) {
                        try Self.loadAttachment(from: fileURL)
                    }.value
                    let attachment = OutboundAttachment(
                        filename: prepared.filename,
                        mimeType: prepared.mimeType,
                        kind: prepared.kind,
                        bytes: prepared.data
                    )
                    sendAttachments(
                        roomID: room.roomId,
                        attachments: [attachment],
                        captionOverride: caption
                    )
                } catch {
                    errorText = String(describing: error)
                }
                return
            }
            try? await Task.sleep(nanoseconds: 500_000_000)
        }
        errorText = "Launch automation timed out waiting for the room to connect"
    }

    private func sendLaunchAutomationAttachment(roomID: String?, text: String) async {
        let deadline = Date().addingTimeInterval(90)
        while !Task.isCancelled, Date() < deadline {
            if let room = launchAutomationRoom(roomID: roomID), room.state == .connected {
                let attachment = OutboundAttachment(
                    filename: "launch-automation.txt",
                    mimeType: "text/plain",
                    kind: .file,
                    bytes: Data(text.utf8)
                )
                sendAttachments(
                    roomID: room.roomId,
                    attachments: [attachment],
                    captionOverride: ""
                )
                return
            }
            try? await Task.sleep(nanoseconds: 500_000_000)
        }
        errorText = "Launch automation timed out waiting for the room to connect"
    }

    private static func argumentValue(_ name: String, in args: [String]) -> String? {
        guard let index = args.firstIndex(of: name) else {
            return nil
        }
        let valueIndex = args.index(after: index)
        guard valueIndex < args.endIndex else {
            return nil
        }
        return args[valueIndex]
    }

    private static func hasLaunchAutomation(args: [String]) -> Bool {
        [
            "--finitechat-auto-send",
            "--finitechat-auto-send-attachment-text",
            "--finitechat-auto-send-attachment-file",
            "--finitechat-auto-send-attachment-base64",
        ].contains { args.contains($0) }
    }

    private static func productHarnessApplicationSupportURL(
        args: [String]
    ) -> ProductHarnessSupportResolution {
        let argument = "--finitechat-product-harness-root"
        guard let rawValue = argumentValue(argument, in: args) else {
            return ProductHarnessSupportResolution(url: nil, error: nil)
        }
        let url = URL(fileURLWithPath: rawValue).standardizedFileURL
        guard url.path == rawValue || rawValue.hasPrefix("/") else {
            return ProductHarnessSupportResolution(
                url: nil,
                error: "\(argument) must be an absolute path"
            )
        }
        guard url.isFileURL, url.path.hasPrefix("/") else {
            return ProductHarnessSupportResolution(
                url: nil,
                error: "\(argument) must be an absolute file path"
            )
        }
        if let defaultSupport = try? FileManager.default.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: false
        ).standardizedFileURL,
            url == defaultSupport
        {
            return ProductHarnessSupportResolution(
                url: nil,
                error: "\(argument) must not be the default Application Support path"
            )
        }
        return ProductHarnessSupportResolution(url: url, error: nil)
    }

    nonisolated private static func loadAttachment(from url: URL) throws -> PreparedAttachment {
        let didStartAccessing = url.startAccessingSecurityScopedResource()
        defer {
            if didStartAccessing {
                url.stopAccessingSecurityScopedResource()
            }
        }

        let data = try Data(contentsOf: url)
        let filename = url.lastPathComponent.isEmpty ? "attachment" : url.lastPathComponent
        let type = UTType(filenameExtension: url.pathExtension)
        return PreparedAttachment(
            data: data,
            filename: filename,
            mimeType: type?.preferredMIMEType ?? "application/octet-stream",
            kind: chatMediaKind(for: type)
        )
    }

    nonisolated private static func chatMediaKind(for type: UTType?) -> ChatMediaKind {
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
}

private extension String {
    var nonEmptyTrimmed: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}

private struct PreparedAttachment: Sendable {
    let data: Data
    let filename: String
    let mimeType: String
    let kind: ChatMediaKind
}
