import Foundation
import SwiftUI

enum KeyPackageAvailability: String, Codable, Equatable, Sendable {
    case unknown
    case available
    case unavailable

    var userStatusText: String {
        switch self {
        case .available:
            return "Ready to message"
        case .unavailable:
            return "Needs Finite Chat open"
        case .unknown:
            return "Checking availability"
        }
    }
}

struct NostrFollowProfile: Codable, Identifiable, Equatable, Sendable {
    let pubkey: String
    let npub: String
    let name: String?
    let username: String?
    let about: String?
    let pictureURL: String?
    let relayHint: String?
    let keyPackageAvailability: KeyPackageAvailability
    let isAgent: Bool

    private enum CodingKeys: String, CodingKey {
        case pubkey
        case npub
        case name
        case username
        case about
        case pictureURL
        case relayHint
        case keyPackageAvailability
        case isAgent
    }

    init(
        pubkey: String,
        npub: String,
        name: String?,
        username: String?,
        about: String?,
        pictureURL: String?,
        relayHint: String?,
        keyPackageAvailability: KeyPackageAvailability = .unknown,
        isAgent: Bool = false
    ) {
        self.pubkey = pubkey
        self.npub = npub
        self.name = name
        self.username = username
        self.about = about
        self.pictureURL = pictureURL
        self.relayHint = relayHint
        self.keyPackageAvailability = keyPackageAvailability
        self.isAgent = isAgent
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        pubkey = try container.decode(String.self, forKey: .pubkey)
        npub = try container.decode(String.self, forKey: .npub)
        name = try container.decodeIfPresent(String.self, forKey: .name)
        username = try container.decodeIfPresent(String.self, forKey: .username)
        about = try container.decodeIfPresent(String.self, forKey: .about)
        pictureURL = try container.decodeIfPresent(String.self, forKey: .pictureURL)
        relayHint = try container.decodeIfPresent(String.self, forKey: .relayHint)
        keyPackageAvailability = try container.decodeIfPresent(KeyPackageAvailability.self, forKey: .keyPackageAvailability) ?? .unknown
        isAgent = try container.decodeIfPresent(Bool.self, forKey: .isAgent) ?? false
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(pubkey, forKey: .pubkey)
        try container.encode(npub, forKey: .npub)
        try container.encodeIfPresent(name, forKey: .name)
        try container.encodeIfPresent(username, forKey: .username)
        try container.encodeIfPresent(about, forKey: .about)
        try container.encodeIfPresent(pictureURL, forKey: .pictureURL)
        try container.encodeIfPresent(relayHint, forKey: .relayHint)
        try container.encode(keyPackageAvailability, forKey: .keyPackageAvailability)
        try container.encode(isAgent, forKey: .isAgent)
    }

    var id: String { pubkey }

    var displayName: String {
        for candidate in [name, username] {
            if let value = candidate?.trimmingCharacters(in: .whitespacesAndNewlines),
               !value.isEmpty
            {
                return value
            }
        }
        return shortenedNpub
    }

    var hasProfileName: Bool {
        for candidate in [name, username] {
            if let value = candidate?.trimmingCharacters(in: .whitespacesAndNewlines),
               !value.isEmpty
            {
                return true
            }
        }
        return false
    }

    var hasProfileMetadata: Bool {
        if isAgent {
            return true
        }
        for candidate in [name, username, about, pictureURL] {
            if let value = candidate?.trimmingCharacters(in: .whitespacesAndNewlines),
               !value.isEmpty
            {
                return true
            }
        }
        return false
    }

    var shortenedNpub: String {
        guard npub.count > 18 else { return npub }
        return "\(npub.prefix(10))...\(npub.suffix(4))"
    }

    func withCachedMetadata(_ metadata: NostrCachedProfileMetadata?) -> NostrFollowProfile {
        guard let metadata else { return self }
        return NostrFollowProfile(
            pubkey: pubkey,
            npub: npub,
            name: name.nostrPreferredValue(over: metadata.name),
            username: username.nostrPreferredValue(over: metadata.username),
            about: about.nostrPreferredValue(over: metadata.about),
            pictureURL: pictureURL.nostrPreferredValue(over: metadata.pictureURL),
            relayHint: relayHint,
            keyPackageAvailability: keyPackageAvailability,
            isAgent: isAgent || metadata.isAgent
        )
    }

    func withKeyPackageAvailability(_ availability: KeyPackageAvailability) -> NostrFollowProfile {
        NostrFollowProfile(
            pubkey: pubkey,
            npub: npub,
            name: name,
            username: username,
            about: about,
            pictureURL: pictureURL,
            relayHint: relayHint,
            keyPackageAvailability: availability,
            isAgent: isAgent
        )
    }

    var appProfileSummary: AppProfileSummary {
        AppProfileSummary(
            accountId: pubkey,
            npub: npub,
            displayName: displayName,
            about: about,
            picture: pictureURL,
            stale: keyPackageAvailability == .unknown,
            isAgent: isAgent
        )
    }
}

struct NostrFollowFetchResult: Equatable, Sendable {
    let profiles: [NostrFollowProfile]
    let relayCount: Int
    let followedPubkeyCount: Int
}

struct NostrFollowSeedResult: Equatable, Sendable {
    let profiles: [NostrFollowProfile]
    let metadataRelays: [String]
    let relayCount: Int
    let followedPubkeyCount: Int

    var fetchResult: NostrFollowFetchResult {
        NostrFollowFetchResult(
            profiles: profiles,
            relayCount: relayCount,
            followedPubkeyCount: followedPubkeyCount
        )
    }
}

actor NostrPeopleCache: Sendable {
    static let shared = NostrPeopleCache()

    private let directory: URL
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()
    private let profileMetadataFileName = "profile-metadata.json"

    init(directory: URL? = nil) {
        if let directory {
            self.directory = directory
        } else {
            let root = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
                .first ?? FileManager.default.temporaryDirectory
            self.directory = root
                .appendingPathComponent("FiniteChat", isDirectory: true)
                .appendingPathComponent("PeopleCache", isDirectory: true)
        }
    }

    func load(accountID: String, serverURL: String) -> NostrFollowFetchResult? {
        let metadata = loadProfileMetadata()
        var firstResult: NostrFollowFetchResult?
        for url in cacheURLs(accountID: accountID, serverURL: serverURL) {
            guard let result = load(from: url) else { continue }
            if firstResult == nil {
                firstResult = result
            }
            if !result.profiles.isEmpty {
                return enrich(result, metadata: metadata)
            }
        }
        return firstResult.map { enrich($0, metadata: metadata) }
    }

    func loadProfile(accountID: String) -> NostrFollowProfile? {
        let key = accountCacheKey(accountID)
        guard let metadata = loadProfileMetadata()[key],
              let npub = try? npubFromAccountId(accountId: key)
        else {
            return nil
        }
        return NostrFollowProfile(
            pubkey: key,
            npub: npub,
            name: metadata.name,
            username: metadata.username,
            about: metadata.about,
            pictureURL: metadata.pictureURL,
            relayHint: nil,
            keyPackageAvailability: .available,
            isAgent: metadata.isAgent
        )
    }

    func enrich(_ result: NostrFollowFetchResult) -> NostrFollowFetchResult {
        enrich(result, metadata: loadProfileMetadata())
    }

    func saveProfile(_ profile: NostrFollowProfile) {
        saveProfileMetadata(from: [profile])
    }

    func save(
        _ result: NostrFollowFetchResult,
        accountID: String,
        serverURL: String,
        preserveNonEmptyOnEmptyResult: Bool = false
    ) {
        do {
            try FileManager.default.createDirectory(
                at: directory,
                withIntermediateDirectories: true
            )
            if preserveNonEmptyOnEmptyResult,
               result.profiles.isEmpty,
               hasNonEmptyCachedProfiles(accountID: accountID, serverURL: serverURL)
            {
                return
            }
            let envelope = NostrPeopleCacheEnvelope(
                profiles: result.profiles,
                relayCount: result.relayCount,
                followedPubkeyCount: result.followedPubkeyCount,
                cachedAt: Date()
            )
            let data = try encoder.encode(envelope)
            try data.write(
                to: cacheURL(accountID: accountID),
                options: [.atomic]
            )
            saveProfileMetadata(from: result.profiles)
        } catch {
            // Cache failures must not block the People surface.
        }
    }

    private func load(from url: URL) -> NostrFollowFetchResult? {
        guard let data = try? Data(contentsOf: url),
              let envelope = try? decoder.decode(NostrPeopleCacheEnvelope.self, from: data)
        else {
            return nil
        }
        return NostrFollowFetchResult(
            profiles: envelope.profiles.map { $0.withKeyPackageAvailability(.unknown) },
            relayCount: envelope.relayCount,
            followedPubkeyCount: envelope.followedPubkeyCount
        )
    }

    private func hasNonEmptyCachedProfiles(accountID: String, serverURL: String) -> Bool {
        cacheURLs(accountID: accountID, serverURL: serverURL).contains { url in
            guard let existing = load(from: url) else { return false }
            return !existing.profiles.isEmpty
        }
    }

    private func enrich(
        _ result: NostrFollowFetchResult,
        metadata: [String: NostrCachedProfileMetadata]
    ) -> NostrFollowFetchResult {
        NostrFollowFetchResult(
            profiles: result.profiles.map { profile in
                let key = profile.pubkey.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
                return profile.withCachedMetadata(metadata[key])
            },
            relayCount: result.relayCount,
            followedPubkeyCount: result.followedPubkeyCount
        )
    }

    private func loadProfileMetadata() -> [String: NostrCachedProfileMetadata] {
        guard let data = try? Data(contentsOf: profileMetadataURL()),
              let envelope = try? decoder.decode(NostrProfileMetadataCacheEnvelope.self, from: data)
        else {
            return [:]
        }
        var result: [String: NostrCachedProfileMetadata] = [:]
        for profile in envelope.profiles {
            let key = profile.pubkey.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            guard !key.isEmpty else { continue }
            result[key] = profile.withPubkey(key)
        }
        return result
    }

    private func saveProfileMetadata(from profiles: [NostrFollowProfile]) {
        var metadata = loadProfileMetadata()
        let now = Date()
        var changed = false
        for profile in profiles {
            let key = profile.pubkey.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            guard !key.isEmpty else { continue }
            if profile.hasProfileMetadata {
                let existing = metadata[key]
                metadata[key] = NostrCachedProfileMetadata(
                    pubkey: key,
                    name: profile.name.nostrPreferredValue(over: existing?.name),
                    username: profile.username.nostrPreferredValue(over: existing?.username),
                    about: profile.about.nostrPreferredValue(over: existing?.about),
                    pictureURL: profile.pictureURL.nostrPreferredValue(over: existing?.pictureURL),
                    isAgent: profile.isAgent || (existing?.isAgent == true),
                    cachedAt: now
                )
                changed = true
            } else if metadata[key] == nil {
                metadata[key] = NostrCachedProfileMetadata(
                    pubkey: key,
                    name: nil,
                    username: nil,
                    about: nil,
                    pictureURL: nil,
                    isAgent: false,
                    cachedAt: now
                )
                changed = true
            }
        }
        guard changed else { return }
        do {
            try FileManager.default.createDirectory(
                at: directory,
                withIntermediateDirectories: true
            )
            let envelope = NostrProfileMetadataCacheEnvelope(
                profiles: metadata.values.sorted { $0.pubkey < $1.pubkey }
            )
            let data = try encoder.encode(envelope)
            try data.write(to: profileMetadataURL(), options: [.atomic])
        } catch {
            // Profile metadata cache failures must not block People rendering.
        }
    }

    private func cacheURL(accountID: String) -> URL {
        directory.appendingPathComponent("\(accountCacheKey(accountID)).json")
    }

    private func cacheURLs(accountID: String, serverURL: String) -> [URL] {
        [
            cacheURL(accountID: accountID),
            legacyCacheURL(accountID: accountID, serverURL: serverURL),
        ]
    }

    private func profileMetadataURL() -> URL {
        directory.appendingPathComponent(profileMetadataFileName)
    }

    private func legacyCacheURL(accountID: String, serverURL: String) -> URL {
        let accountKey = accountCacheKey(accountID)
        let serverKey = serverURL
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
            .utf8
            .map { String(format: "%02x", $0) }
            .joined()
        return directory.appendingPathComponent("\(accountKey)-\(serverKey).json")
    }

    private func accountCacheKey(_ accountID: String) -> String {
        let accountKey = accountID.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return accountKey.isEmpty ? "unknown" : accountKey
    }
}

private struct NostrPeopleCacheEnvelope: Codable {
    let profiles: [NostrFollowProfile]
    let relayCount: Int
    let followedPubkeyCount: Int
    let cachedAt: Date
}

struct NostrCachedProfileMetadata: Codable, Equatable, Sendable {
    let pubkey: String
    let name: String?
    let username: String?
    let about: String?
    let pictureURL: String?
    let isAgent: Bool
    let cachedAt: Date

    private enum CodingKeys: String, CodingKey {
        case pubkey
        case name
        case username
        case about
        case pictureURL
        case isAgent
        case cachedAt
    }

    init(
        pubkey: String,
        name: String?,
        username: String?,
        about: String?,
        pictureURL: String?,
        isAgent: Bool = false,
        cachedAt: Date
    ) {
        self.pubkey = pubkey
        self.name = name
        self.username = username
        self.about = about
        self.pictureURL = pictureURL
        self.isAgent = isAgent
        self.cachedAt = cachedAt
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        pubkey = try container.decode(String.self, forKey: .pubkey)
        name = try container.decodeIfPresent(String.self, forKey: .name)
        username = try container.decodeIfPresent(String.self, forKey: .username)
        about = try container.decodeIfPresent(String.self, forKey: .about)
        pictureURL = try container.decodeIfPresent(String.self, forKey: .pictureURL)
        isAgent = try container.decodeIfPresent(Bool.self, forKey: .isAgent) ?? false
        cachedAt = try container.decode(Date.self, forKey: .cachedAt)
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(pubkey, forKey: .pubkey)
        try container.encodeIfPresent(name, forKey: .name)
        try container.encodeIfPresent(username, forKey: .username)
        try container.encodeIfPresent(about, forKey: .about)
        try container.encodeIfPresent(pictureURL, forKey: .pictureURL)
        try container.encode(isAgent, forKey: .isAgent)
        try container.encode(cachedAt, forKey: .cachedAt)
    }

    func withPubkey(_ pubkey: String) -> NostrCachedProfileMetadata {
        NostrCachedProfileMetadata(
            pubkey: pubkey,
            name: name,
            username: username,
            about: about,
            pictureURL: pictureURL,
            isAgent: isAgent,
            cachedAt: cachedAt
        )
    }
}

private struct NostrProfileMetadataCacheEnvelope: Codable {
    let profiles: [NostrCachedProfileMetadata]
}

private extension Optional where Wrapped == String {
    var nostrNormalizedProfileValue: String? {
        guard let value = self?.trimmingCharacters(in: .whitespacesAndNewlines),
              !value.isEmpty
        else {
            return nil
        }
        return value
    }

    func nostrPreferredValue(over fallback: String?) -> String? {
        nostrNormalizedProfileValue ?? fallback.nostrNormalizedProfileValue
    }
}

final class NostrPeopleModel: ObservableObject {
    @Published private(set) var profiles: [NostrFollowProfile] = []
    @Published private(set) var isLoading = false
    @Published private(set) var isCheckingKeyPackageAvailability = false
    @Published private(set) var statusText: String?

    private let service: NostrRelayProfileService
    private let keyPackageAvailabilityService: FiniteKeyPackageAvailabilityService
    private let cache: NostrPeopleCache?
    private var lastLoadedAccountID: String?
    private var lastLoadedServerURL: String?

    init(
        service: NostrRelayProfileService = NostrRelayProfileService(),
        keyPackageAvailabilityService: FiniteKeyPackageAvailabilityService = FiniteKeyPackageAvailabilityService(),
        cache: NostrPeopleCache? = .shared
    ) {
        self.service = service
        self.keyPackageAvailabilityService = keyPackageAvailabilityService
        self.cache = cache
    }

    @MainActor
    func loadIfNeeded(identity: AppNostrIdentity?, serverURL: String) async {
        await loadIfNeeded(accountID: identity?.accountID, serverURL: serverURL)
    }

    @MainActor
    func loadIfNeeded(accountID: String?, serverURL: String) async {
        guard let accountID = normalizedAccountID(accountID) else {
            profiles = []
            statusText = nil
            lastLoadedAccountID = nil
            lastLoadedServerURL = nil
            return
        }
        guard lastLoadedAccountID != accountID
            || lastLoadedServerURL != serverURL
            || profiles.isEmpty
        else { return }
        if let cached = await cache?.load(accountID: accountID, serverURL: serverURL) {
            applyCached(cached, accountID: accountID, serverURL: serverURL)
            Task { [weak self] in
                await self?.refresh(accountID: accountID, serverURL: serverURL)
            }
            return
        }
        await refresh(accountID: accountID, serverURL: serverURL)
    }

    @MainActor
    func refresh(identity: AppNostrIdentity?, serverURL: String) async {
        await refresh(accountID: identity?.accountID, serverURL: serverURL)
    }

    @MainActor
    func refresh(accountID: String?, serverURL: String) async {
        guard let accountID = normalizedAccountID(accountID) else { return }
        lastLoadedAccountID = accountID
        lastLoadedServerURL = serverURL
        isLoading = true
        defer { isLoading = false }
        statusText = nil
        do {
            let seedResult = try await service.fetchFollowProfileSeeds(forAccountID: accountID)
            let cachedSeedResult = await cache?.enrich(seedResult.fetchResult) ?? seedResult.fetchResult
            guard isCurrent(accountID: accountID, serverURL: serverURL) else { return }
            if cachedSeedResult.profiles.isEmpty {
                await handleEmptyRefreshResult(
                    cachedSeedResult,
                    accountID: accountID,
                    serverURL: serverURL
                )
                return
            }

            profiles = cachedSeedResult.profiles
            statusText = "Loaded \(cachedSeedResult.profiles.count) of \(cachedSeedResult.followedPubkeyCount) follows from \(cachedSeedResult.relayCount) relays. Updating profiles..."
            await cache?.save(
                cachedSeedResult,
                accountID: accountID,
                serverURL: serverURL,
                preserveNonEmptyOnEmptyResult: true
            )

            let fetchedResult = await service.enrichFollowProfileSeeds(seedResult)
            let result = await cache?.enrich(fetchedResult) ?? fetchedResult
            guard isCurrent(accountID: accountID, serverURL: serverURL) else { return }
            profiles = result.profiles
            await cache?.save(
                result,
                accountID: accountID,
                serverURL: serverURL,
                preserveNonEmptyOnEmptyResult: true
            )

            await refreshKeyPackageAvailability(serverURL: serverURL)
            await cache?.save(
                NostrFollowFetchResult(
                    profiles: profiles,
                    relayCount: result.relayCount,
                    followedPubkeyCount: result.followedPubkeyCount
                ),
                accountID: accountID,
                serverURL: serverURL,
                preserveNonEmptyOnEmptyResult: true
            )
            if result.followedPubkeyCount == 0 {
                statusText = "No follows found for \(accountDisplay(accountID)) across \(result.relayCount) Nostr relays."
            } else {
                statusText = "Loaded \(result.profiles.count) of \(result.followedPubkeyCount) follows from \(result.relayCount) relays."
            }
        } catch is CancellationError {
            return
        } catch {
            guard isCurrent(accountID: accountID, serverURL: serverURL) else { return }
            if profiles.isEmpty,
               let cached = await cache?.load(accountID: accountID, serverURL: serverURL)
            {
                applyCached(cached, accountID: accountID, serverURL: serverURL)
                statusText = "Showing cached people. Refresh failed: \(error.localizedDescription)"
            } else if profiles.isEmpty {
                profiles = []
                statusText = "Could not load follows: \(error.localizedDescription)"
            } else {
                statusText = "Could not refresh people: \(error.localizedDescription)"
            }
        }
    }

    @MainActor
    private func handleEmptyRefreshResult(
        _ result: NostrFollowFetchResult,
        accountID: String,
        serverURL: String
    ) async {
        if !profiles.isEmpty {
            await cache?.save(
                result,
                accountID: accountID,
                serverURL: serverURL,
                preserveNonEmptyOnEmptyResult: true
            )
            await refreshKeyPackageAvailability(serverURL: serverURL)
            await cache?.save(
                NostrFollowFetchResult(
                    profiles: profiles,
                    relayCount: result.relayCount,
                    followedPubkeyCount: max(result.followedPubkeyCount, profiles.count)
                ),
                accountID: accountID,
                serverURL: serverURL,
                preserveNonEmptyOnEmptyResult: true
            )
            statusText = "Showing cached people. Refresh found no follows across \(result.relayCount) Nostr relays."
            return
        }

        profiles = []
        await cache?.save(
            result,
            accountID: accountID,
            serverURL: serverURL,
            preserveNonEmptyOnEmptyResult: true
        )
        statusText = "No follows found for \(accountDisplay(accountID)) across \(result.relayCount) Nostr relays."
    }

    @MainActor
    private func applyCached(
        _ result: NostrFollowFetchResult,
        accountID: String,
        serverURL: String
    ) {
        lastLoadedAccountID = accountID
        lastLoadedServerURL = serverURL
        profiles = result.profiles
        statusText = result.followedPubkeyCount == 0
            ? "No cached follows found."
            : "Showing cached people. Refreshing..."
    }

    @MainActor
    private func isCurrent(accountID: String, serverURL: String) -> Bool {
        lastLoadedAccountID == accountID && lastLoadedServerURL == serverURL
    }

    private func normalizedAccountID(_ accountID: String?) -> String? {
        let trimmed = accountID?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard let trimmed, !trimmed.isEmpty else { return nil }
        return trimmed
    }

    private func accountDisplay(_ accountID: String) -> String {
        if let npub = try? npubFromAccountId(accountId: accountID) {
            return shortenedNpub(npub)
        }
        guard accountID.count > 12 else { return accountID }
        return "\(accountID.prefix(8))...\(accountID.suffix(4))"
    }

    private func shortenedNpub(_ npub: String) -> String {
        guard npub.count > 18 else { return npub }
        return "\(npub.prefix(10))...\(npub.suffix(4))"
    }

    @MainActor
    func recheckKeyPackageAvailability(
        for profile: NostrFollowProfile,
        serverURL: String
    ) async throws -> NostrFollowProfile {
        let availability = try await keyPackageAvailabilityService.fetchAvailability(
            serverURL: serverURL,
            accountIDs: [profile.pubkey]
        )
        let nextAvailability: KeyPackageAvailability = availability[profile.pubkey] == true
            ? .available
            : .unavailable
        let updated = profile.withKeyPackageAvailability(nextAvailability)
        if let index = profiles.firstIndex(where: { $0.pubkey == profile.pubkey }) {
            profiles[index] = profiles[index].withKeyPackageAvailability(nextAvailability)
            return profiles[index]
        }
        return updated
    }

    @MainActor
    private func refreshKeyPackageAvailability(serverURL: String) async {
        let accountIDs = profiles.map(\.pubkey)
        guard !accountIDs.isEmpty else { return }
        isCheckingKeyPackageAvailability = true
        defer { isCheckingKeyPackageAvailability = false }
        do {
            let availability = try await keyPackageAvailabilityService.fetchAvailability(
                serverURL: serverURL,
                accountIDs: accountIDs
            )
            profiles = profiles.map { profile in
                let nextAvailability: KeyPackageAvailability = availability[profile.pubkey] == true
                    ? .available
                    : .unavailable
                return profile.withKeyPackageAvailability(nextAvailability)
            }
        } catch {
            // Leave rows in the neutral unknown state. A failed availability
            // check is not evidence that a person lacks KeyPackages.
        }
    }
}

final class FiniteKeyPackageAvailabilityService: Sendable {
    typealias AvailabilityLoader = @Sendable (
        _ serverURL: String,
        _ accountIDs: [String]
    ) async throws -> [String: Bool]

    private let chunkSize: Int
    private let availabilityLoader: AvailabilityLoader

    init(
        chunkSize: Int = 100,
        availabilityLoader: @escaping AvailabilityLoader = { serverURL, accountIDs in
            try await FiniteKeyPackageAvailabilityService.fetchAvailabilityFromServer(
                serverURL: serverURL,
                accountIDs: accountIDs
            )
        }
    ) {
        self.chunkSize = max(1, chunkSize)
        self.availabilityLoader = availabilityLoader
    }

    func fetchAvailability(serverURL: String, accountIDs: [String]) async throws -> [String: Bool] {
        let normalized = accountIDs.compactMap { accountID -> String? in
            let trimmed = accountID.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            return trimmed.isEmpty ? nil : trimmed
        }
        guard !normalized.isEmpty else { return [:] }
        var availability: [String: Bool] = [:]
        for chunk in normalized.chunked(into: chunkSize) {
            let chunkAvailability = try await availabilityLoader(serverURL, chunk)
            availability.merge(chunkAvailability) { _, new in new }
        }
        return availability
    }

    private static func fetchAvailabilityFromServer(
        serverURL: String,
        accountIDs: [String]
    ) async throws -> [String: Bool] {
        let trimmedServerURL = serverURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let baseURL = URL(string: trimmedServerURL) else {
            throw KeyPackageAvailabilityError.invalidServerURL
        }
        let url = baseURL.appendingPathComponent("key-packages/availability")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.timeoutInterval = 10
        request.setValue("application/json", forHTTPHeaderField: "content-type")
        request.httpBody = try JSONEncoder().encode(
            KeyPackageAvailabilityRequestBody(accountIDs: accountIDs)
        )
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse,
              (200..<300).contains(httpResponse.statusCode)
        else {
            throw KeyPackageAvailabilityError.serverRejected
        }
        let decoded = try JSONDecoder().decode(KeyPackageAvailabilityResponseBody.self, from: data)
        return Dictionary(uniqueKeysWithValues: decoded.accounts.map { account in
            (account.accountID, account.available)
        })
    }
}

private enum KeyPackageAvailabilityError: Error {
    case invalidServerURL
    case serverRejected
}

private struct KeyPackageAvailabilityRequestBody: Encodable {
    let accountIDs: [String]

    enum CodingKeys: String, CodingKey {
        case accountIDs = "account_ids"
    }
}

private struct KeyPackageAvailabilityResponseBody: Decodable {
    let accounts: [KeyPackageAvailabilityAccountBody]
}

private struct KeyPackageAvailabilityAccountBody: Decodable {
    let accountID: String
    let available: Bool

    enum CodingKeys: String, CodingKey {
        case accountID = "account_id"
        case available
    }
}

final class NostrRelayProfileService: Sendable {
    typealias EventLoader = @Sendable (
        _ relay: String,
        _ filter: NostrRelayFilter,
        _ subscriptionPrefix: String,
        _ timeoutNanoseconds: UInt64
    ) async throws -> [NostrRelayEvent]

    static let pikaProfileRelays = [
        "wss://relay.primal.net",
        "wss://nos.lol",
        "wss://relay.damus.io",
        "wss://offchain.pub",
    ]

    static let profileDiscoveryRelays = [
        "wss://purplepag.es",
        "wss://nostr.wine",
        "wss://nostr.mom",
        "wss://relay.current.fyi",
        "wss://relayable.org",
        "wss://relay.nostr.wirednet.jp",
        "wss://nostr-pub.wellorder.net",
        "wss://nostr-01.yakihonne.com",
        "wss://nostr-02.yakihonne.com",
    ]

    private let relays: [String]
    private let discoveryRelays: [String]
    private let timeoutNanoseconds: UInt64
    private let eventLoader: EventLoader

    init(
        relays: [String] = NostrRelayProfileService.pikaProfileRelays,
        discoveryRelays: [String] = NostrRelayProfileService.profileDiscoveryRelays,
        timeoutSeconds: Double = 5,
        eventLoader: @escaping EventLoader = { relay, filter, subscriptionPrefix, timeoutNanoseconds in
            try await NostrRelayProfileService.fetchEventsFromRelay(
                from: relay,
                filter: filter,
                subscriptionPrefix: subscriptionPrefix,
                timeoutNanoseconds: timeoutNanoseconds
            )
        }
    ) {
        self.relays = Self.mergedRelays(relays)
        self.discoveryRelays = Self.mergedRelays(discoveryRelays)
        timeoutNanoseconds = UInt64(max(timeoutSeconds, 1) * 1_000_000_000)
        self.eventLoader = eventLoader
    }

    func fetchFollowProfiles(forAccountID accountID: String) async throws -> NostrFollowFetchResult {
        let seeds = try await fetchFollowProfileSeeds(forAccountID: accountID)
        return await enrichFollowProfileSeeds(seeds)
    }

    func fetchProfile(forAccountID accountID: String) async -> NostrFollowProfile? {
        let normalizedAccountID = accountID.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard Self.isHexPubkey(normalizedAccountID),
              let npub = try? npubFromAccountId(accountId: normalizedAccountID)
        else {
            return nil
        }
        let primaryRelays = await contactListRelays(
            forAccountID: normalizedAccountID,
            bootstrapRelays: relays
        )
        let metadata = await fetchMetadataWithDiscoveryFallback(
            forPubkeys: [normalizedAccountID],
            primaryRelays: primaryRelays
        )
        guard let profile = metadata[normalizedAccountID] else { return nil }
        return NostrFollowProfile(
            pubkey: normalizedAccountID,
            npub: npub,
            name: profile.displayName ?? profile.name,
            username: profile.name,
            about: profile.about,
            pictureURL: profile.pictureURL,
            relayHint: nil,
            keyPackageAvailability: .available,
            isAgent: profile.isAgent
        )
    }

    func fetchFollowProfileSeeds(forAccountID accountID: String) async throws -> NostrFollowSeedResult {
        let primaryContactRelays = await contactListRelays(
            forAccountID: accountID,
            bootstrapRelays: relays
        )
        let primaryContactResult = await fetchContacts(
            forAccountID: accountID,
            relays: primaryContactRelays
        )
        let (contactRelays, contactResult) = await selectedContactFetch(
            forAccountID: accountID,
            primaryRelays: primaryContactRelays,
            primaryResult: primaryContactResult
        )
        if contactResult.allRelayAttemptsFailed {
            throw NostrPeopleFetchError.allContactRelaysFailed
        }
        let followed = contactResult.contacts.values.sorted { left, right in
            left.pubkey < right.pubkey
        }
        guard !followed.isEmpty else {
            return NostrFollowSeedResult(
                profiles: [],
                metadataRelays: contactRelays,
                relayCount: contactRelays.count,
                followedPubkeyCount: 0
            )
        }
        let primaryMetadataRelays = Self.mergedRelays(contactRelays + followed.compactMap(\.relayHint))
        let profiles = Self.followProfiles(from: followed, metadata: [:])
        return NostrFollowSeedResult(
            profiles: profiles,
            metadataRelays: primaryMetadataRelays,
            relayCount: contactRelays.count,
            followedPubkeyCount: followed.count
        )
    }

    func enrichFollowProfileSeeds(_ seeds: NostrFollowSeedResult) async -> NostrFollowFetchResult {
        let followedPubkeys = seeds.profiles.map(\.pubkey)
        guard !followedPubkeys.isEmpty else {
            return seeds.fetchResult
        }
        let metadata = await fetchMetadataWithDiscoveryFallback(
            forPubkeys: followedPubkeys,
            primaryRelays: seeds.metadataRelays
        )
        let profiles = Self.followProfiles(from: seeds.profiles, metadata: metadata)
        return NostrFollowFetchResult(
            profiles: profiles,
            relayCount: seeds.relayCount,
            followedPubkeyCount: seeds.followedPubkeyCount
        )
    }

    private static func followProfiles(
        from contacts: [NostrContact],
        metadata: [String: NostrProfileMetadata]
    ) -> [NostrFollowProfile] {
        let profiles = contacts.compactMap { contact -> NostrFollowProfile? in
            guard let npub = try? npubFromAccountId(accountId: contact.pubkey) else { return nil }
            let profile = metadata[contact.pubkey]
            return NostrFollowProfile(
                pubkey: contact.pubkey,
                npub: npub,
                name: profile?.displayName ?? profile?.name ?? contact.petname,
                username: profile?.name,
                about: profile?.about,
                pictureURL: profile?.pictureURL,
                relayHint: contact.relayHint,
                isAgent: profile?.isAgent ?? false
            )
        }
        return sortedFollowProfiles(profiles)
    }

    private static func followProfiles(
        from seeds: [NostrFollowProfile],
        metadata: [String: NostrProfileMetadata]
    ) -> [NostrFollowProfile] {
        let profiles = seeds.map { seed in
            let profile = metadata[seed.pubkey]
            return NostrFollowProfile(
                pubkey: seed.pubkey,
                npub: seed.npub,
                name: profile?.displayName ?? profile?.name ?? seed.name,
                username: profile?.name ?? seed.username,
                about: profile?.about ?? seed.about,
                pictureURL: profile?.pictureURL ?? seed.pictureURL,
                relayHint: seed.relayHint,
                keyPackageAvailability: seed.keyPackageAvailability,
                isAgent: (profile?.isAgent ?? false) || seed.isAgent
            )
        }
        return sortedFollowProfiles(profiles)
    }

    private static func sortedFollowProfiles(_ profiles: [NostrFollowProfile]) -> [NostrFollowProfile] {
        profiles.sorted { left, right in
            let leftNamed = left.hasProfileName
            let rightNamed = right.hasProfileName
            if leftNamed != rightNamed {
                return leftNamed
            }
            return left.displayName.localizedCaseInsensitiveCompare(right.displayName) == .orderedAscending
        }
    }

    private func selectedContactFetch(
        forAccountID accountID: String,
        primaryRelays: [String],
        primaryResult: NostrContactFetchResult
    ) async -> ([String], NostrContactFetchResult) {
        guard (primaryResult.contacts.isEmpty || primaryResult.allRelayAttemptsFailed),
              !discoveryRelays.isEmpty
        else {
            return (primaryRelays, primaryResult)
        }
        let expandedBootstrapRelays = Self.mergedRelays(primaryRelays + discoveryRelays)
        guard expandedBootstrapRelays.count > primaryRelays.count else {
            return (primaryRelays, primaryResult)
        }
        let expandedContactRelays = await contactListRelays(
            forAccountID: accountID,
            bootstrapRelays: expandedBootstrapRelays
        )
        let expandedResult = await fetchContacts(
            forAccountID: accountID,
            relays: expandedContactRelays
        )
        if !expandedResult.contacts.isEmpty
            || (primaryResult.contacts.isEmpty && !expandedResult.allRelayAttemptsFailed)
            || primaryResult.allRelayAttemptsFailed
        {
            return (expandedContactRelays, expandedResult)
        }
        return (primaryRelays, primaryResult)
    }

    private func contactListRelays(
        forAccountID accountID: String,
        bootstrapRelays: [String]
    ) async -> [String] {
        let normalizedAccountID = accountID.lowercased()
        let filter = NostrRelayFilter(kinds: [10_002], authors: [normalizedAccountID], limit: 1)
        let batch = await fetchEvents(
            filter: filter,
            subscriptionPrefix: "finite-relays",
            relays: bootstrapRelays
        )
        guard let latest = batch.events
            .filter({ $0.kind == 10_002 && $0.pubkey.lowercased() == normalizedAccountID })
            .max(by: { $0.createdAt < $1.createdAt })
        else {
            return bootstrapRelays
        }

        let advertisedRelays = latest.tags.compactMap { tag -> String? in
            guard tag.count >= 2, tag[0] == "r" else { return nil }
            let marker = tag.count >= 3
                ? tag[2].trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
                : ""
            guard marker.isEmpty || marker == "write" || marker == "read" else { return nil }
            return tag[1].nostrNonEmptyTrimmed
        }
        return Self.mergedRelays(bootstrapRelays + advertisedRelays)
    }

    private func fetchContacts(forAccountID accountID: String, relays: [String]) async -> NostrContactFetchResult {
        let normalizedAccountID = accountID.lowercased()
        let filter = NostrRelayFilter(kinds: [3], authors: [normalizedAccountID], limit: 1)
        let batch = await fetchEvents(
            filter: filter,
            subscriptionPrefix: "finite-contacts",
            relays: relays
        )
        guard let latest = batch.events
            .filter({ $0.kind == 3 && $0.pubkey.lowercased() == normalizedAccountID })
            .max(by: { $0.createdAt < $1.createdAt })
        else {
            return NostrContactFetchResult(
                contacts: [:],
                allRelayAttemptsFailed: batch.allAttemptsFailed
            )
        }
        var contacts: [String: NostrContact] = [:]
        for tag in latest.tags {
            guard tag.count >= 2, tag[0] == "p", Self.isHexPubkey(tag[1]) else { continue }
            let pubkey = tag[1].lowercased()
            let relayHint = tag.count >= 3 ? tag[2].nostrNonEmptyTrimmed : nil
            let petname = tag.count >= 4 ? tag[3].nostrNonEmptyTrimmed : nil
            contacts[pubkey] = NostrContact(pubkey: pubkey, relayHint: relayHint, petname: petname)
        }
        return NostrContactFetchResult(
            contacts: contacts,
            allRelayAttemptsFailed: batch.allAttemptsFailed
        )
    }

    private func fetchMetadataWithDiscoveryFallback(
        forPubkeys pubkeys: [String],
        primaryRelays: [String]
    ) async -> [String: NostrProfileMetadata] {
        var metadata = await fetchMetadata(forPubkeys: pubkeys, relays: primaryRelays)
        let missingPubkeys = pubkeys.filter { metadata[$0] == nil }
        guard !missingPubkeys.isEmpty else { return metadata }

        let fallbackRelays = Self.mergedRelays(discoveryRelays.filter { relay in
            !primaryRelays.contains { $0.caseInsensitiveCompare(relay) == .orderedSame }
        })
        guard !fallbackRelays.isEmpty else { return metadata }

        let fallbackMetadata = await fetchMetadata(
            forPubkeys: missingPubkeys,
            relays: fallbackRelays
        )
        metadata.merge(fallbackMetadata) { current, fallback in
            current.createdAt >= fallback.createdAt ? current : fallback
        }
        return metadata
    }

    private func fetchMetadata(forPubkeys pubkeys: [String], relays: [String]) async -> [String: NostrProfileMetadata] {
        await withTaskGroup(of: [String: NostrProfileMetadata].self) { group in
            for chunk in pubkeys.chunked(into: 80) {
                group.addTask {
                    await self.fetchMetadataChunk(forPubkeys: chunk, relays: relays)
                }
            }

            var metadata: [String: NostrProfileMetadata] = [:]
            for await chunkMetadata in group {
                metadata.merge(chunkMetadata) { current, next in
                    current.createdAt >= next.createdAt ? current : next
                }
            }
            return metadata
        }
    }

    private func fetchMetadataChunk(forPubkeys pubkeys: [String], relays: [String]) async -> [String: NostrProfileMetadata] {
        var metadata: [String: NostrProfileMetadata] = [:]
        let filter = NostrRelayFilter(kinds: [0], authors: pubkeys, limit: pubkeys.count)
        let batch = await fetchEvents(
            filter: filter,
            subscriptionPrefix: "finite-profiles",
            relays: relays
        )
        for event in batch.events where event.kind == 0 {
            guard Self.isHexPubkey(event.pubkey) else { continue }
            let pubkey = event.pubkey.lowercased()
            guard metadata[pubkey]?.createdAt ?? 0 <= event.createdAt else { continue }
            metadata[pubkey] = NostrProfileMetadata(event: event)
        }
        return metadata
    }

    private func fetchEvents(
        filter: NostrRelayFilter,
        subscriptionPrefix: String,
        relays: [String]
    ) async -> NostrRelayEventBatch {
        await withTaskGroup(of: NostrRelayAttemptResult.self) { group in
            for relay in relays {
                group.addTask {
                    do {
                        return NostrRelayAttemptResult(
                            events: try await self.eventLoader(
                                relay,
                                filter,
                                subscriptionPrefix,
                                self.timeoutNanoseconds
                            ),
                            failed: false
                        )
                    } catch {
                        return NostrRelayAttemptResult(events: [], failed: true)
                    }
                }
            }
            var events: [NostrRelayEvent] = []
            var attemptedRelayCount = 0
            var failedRelayCount = 0
            for await relayResult in group {
                attemptedRelayCount += 1
                if relayResult.failed {
                    failedRelayCount += 1
                }
                events.append(contentsOf: relayResult.events)
            }
            return NostrRelayEventBatch(
                events: events,
                attemptedRelayCount: attemptedRelayCount,
                failedRelayCount: failedRelayCount
            )
        }
    }

    private static func mergedRelays(_ values: [String]) -> [String] {
        var seen: Set<String> = []
        var merged: [String] = []
        for value in values {
            guard let relay = value.nostrNonEmptyTrimmed else { continue }
            let key = relay.lowercased()
            guard !seen.contains(key), URL(string: relay) != nil else { continue }
            seen.insert(key)
            merged.append(relay)
        }
        return merged
    }

    private static func fetchEventsFromRelay(
        from relay: String,
        filter: NostrRelayFilter,
        subscriptionPrefix: String,
        timeoutNanoseconds: UInt64
    ) async throws -> [NostrRelayEvent] {
        guard let url = URL(string: relay) else { return [] }
        let configuration = URLSessionConfiguration.ephemeral
        let timeoutSeconds = max(1, Double(timeoutNanoseconds) / 1_000_000_000)
        configuration.timeoutIntervalForRequest = timeoutSeconds
        configuration.timeoutIntervalForResource = timeoutSeconds
        let session = URLSession(configuration: configuration)
        let task = session.webSocketTask(with: url)
        let subscriptionID = "\(subscriptionPrefix)-\(UUID().uuidString)"
        let filterData = try JSONEncoder().encode(filter)
        let filterObject = try JSONSerialization.jsonObject(with: filterData)
        let payload: [Any] = ["REQ", subscriptionID, filterObject]
        let data = try JSONSerialization.data(withJSONObject: payload)
        guard let message = String(data: data, encoding: .utf8) else { return [] }

        task.resume()
        defer {
            task.cancel(with: .goingAway, reason: nil)
            session.invalidateAndCancel()
        }
        try await sendWithTimeout(
            task,
            message: message,
            timeoutNanoseconds: timeoutNanoseconds
        )

        var events: [NostrRelayEvent] = []
        while !Task.isCancelled {
            let received: URLSessionWebSocketTask.Message
            do {
                received = try await receiveWithTimeout(task, timeoutNanoseconds: timeoutNanoseconds)
            } catch is NostrRelayTimeout {
                break
            }
            let text: String?
            switch received {
            case .string(let value):
                text = value
            case .data(let data):
                text = String(data: data, encoding: .utf8)
            @unknown default:
                text = nil
            }
            guard let text else { continue }
            let parsed = Self.parseRelayMessage(text)
            if parsed.eoseSubscriptionID == subscriptionID {
                break
            }
            if let event = parsed.event, parsed.subscriptionID == subscriptionID {
                events.append(event)
            }
        }
        return events
    }

    private static func sendWithTimeout(
        _ task: URLSessionWebSocketTask,
        message: String,
        timeoutNanoseconds: UInt64
    ) async throws {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                let resume = NostrRelayContinuation<Void>(continuation)
                let timeout = relayTimeoutWorkItem(
                    task: task,
                    timeoutNanoseconds: timeoutNanoseconds,
                    resume: resume
                )
                task.send(.string(message)) { error in
                    timeout.cancel()
                    if let error {
                        resume.resume(throwing: error)
                    } else {
                        resume.resume(returning: ())
                    }
                }
            }
        } onCancel: {
            task.cancel(with: .goingAway, reason: nil)
        }
    }

    private static func receiveWithTimeout(
        _ task: URLSessionWebSocketTask,
        timeoutNanoseconds: UInt64
    ) async throws -> URLSessionWebSocketTask.Message {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                let resume = NostrRelayContinuation<URLSessionWebSocketTask.Message>(continuation)
                let timeout = relayTimeoutWorkItem(
                    task: task,
                    timeoutNanoseconds: timeoutNanoseconds,
                    resume: resume
                )
                task.receive { result in
                    timeout.cancel()
                    switch result {
                    case .success(let message):
                        resume.resume(returning: message)
                    case .failure(let error):
                        resume.resume(throwing: error)
                    }
                }
            }
        } onCancel: {
            task.cancel(with: .goingAway, reason: nil)
        }
    }

    private static func relayTimeoutWorkItem<Value>(
        task: URLSessionWebSocketTask,
        timeoutNanoseconds: UInt64,
        resume: NostrRelayContinuation<Value>
    ) -> NostrRelayTimeoutHandle {
        let timeoutSeconds = Double(timeoutNanoseconds) / 1_000_000_000
        let timeout = DispatchWorkItem {
            task.cancel(with: .goingAway, reason: nil)
            resume.resume(throwing: NostrRelayTimeout())
        }
        DispatchQueue.global(qos: .utility).asyncAfter(
            deadline: .now() + timeoutSeconds,
            execute: timeout
        )
        return NostrRelayTimeoutHandle(workItem: timeout)
    }

    private static func parseRelayMessage(_ text: String) -> NostrRelayMessage {
        guard let root = try? JSONSerialization.jsonObject(with: Data(text.utf8)),
              let array = root as? [Any],
              let kind = array.first as? String
        else {
            return NostrRelayMessage()
        }
        if kind == "EOSE", array.count >= 2 {
            return NostrRelayMessage(eoseSubscriptionID: array[1] as? String)
        }
        guard kind == "EVENT",
              array.count >= 3,
              let subscriptionID = array[1] as? String,
              let eventObject = array[2] as? [String: Any]
        else {
            return NostrRelayMessage()
        }
        return NostrRelayMessage(
            subscriptionID: subscriptionID,
            event: NostrRelayEvent(object: eventObject)
        )
    }

    private static func isHexPubkey(_ value: String) -> Bool {
        let hexCharacters = Set("0123456789abcdefABCDEF")
        return value.count == 64 && value.allSatisfy { character in
            hexCharacters.contains(character)
        }
    }
}

private enum NostrPeopleFetchError: LocalizedError, Sendable {
    case allContactRelaysFailed

    var errorDescription: String? {
        switch self {
        case .allContactRelaysFailed:
            return "All contact-list relay requests failed."
        }
    }
}

private struct NostrContactFetchResult: Sendable {
    let contacts: [String: NostrContact]
    let allRelayAttemptsFailed: Bool
}

private struct NostrRelayEventBatch: Sendable {
    let events: [NostrRelayEvent]
    let attemptedRelayCount: Int
    let failedRelayCount: Int

    var allAttemptsFailed: Bool {
        attemptedRelayCount > 0 && failedRelayCount == attemptedRelayCount
    }
}

private struct NostrRelayAttemptResult: Sendable {
    let events: [NostrRelayEvent]
    let failed: Bool
}

struct NostrRelayFilter: Encodable, Sendable {
    let kinds: [Int]
    let authors: [String]
    let limit: Int?
}

private struct NostrRelayTimeout: Error, Sendable {}

private final class NostrRelayTimeoutHandle: @unchecked Sendable {
    private let workItem: DispatchWorkItem

    init(workItem: DispatchWorkItem) {
        self.workItem = workItem
    }

    func cancel() {
        workItem.cancel()
    }
}

private final class NostrRelayContinuation<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Value, Error>?

    init(_ continuation: CheckedContinuation<Value, Error>) {
        self.continuation = continuation
    }

    func resume(returning value: Value) {
        takeContinuation()?.resume(returning: value)
    }

    func resume(throwing error: Error) {
        takeContinuation()?.resume(throwing: error)
    }

    private func takeContinuation() -> CheckedContinuation<Value, Error>? {
        lock.lock()
        defer { lock.unlock() }
        let current = continuation
        continuation = nil
        return current
    }
}

private struct NostrRelayMessage: Sendable {
    var subscriptionID: String?
    var eoseSubscriptionID: String?
    var event: NostrRelayEvent?
}

struct NostrRelayEvent: Sendable {
    let pubkey: String
    let createdAt: Int
    let kind: Int
    let tags: [[String]]
    let content: String

    init(
        pubkey: String,
        createdAt: Int,
        kind: Int,
        tags: [[String]],
        content: String
    ) {
        self.pubkey = pubkey
        self.createdAt = createdAt
        self.kind = kind
        self.tags = tags
        self.content = content
    }

    init?(object: [String: Any]) {
        guard let pubkey = object["pubkey"] as? String,
              let createdAt = object["created_at"] as? Int,
              let kind = object["kind"] as? Int,
              let content = object["content"] as? String
        else {
            return nil
        }
        self.pubkey = pubkey
        self.createdAt = createdAt
        self.kind = kind
        self.content = content
        tags = (object["tags"] as? [[Any]])?.map { tag in
            tag.compactMap { $0 as? String }
        } ?? []
    }
}

private struct NostrContact: Sendable {
    let pubkey: String
    let relayHint: String?
    let petname: String?
}

private struct NostrProfileMetadata: Sendable {
    let createdAt: Int
    let name: String?
    let displayName: String?
    let about: String?
    let pictureURL: String?
    let isAgent: Bool

    init(event: NostrRelayEvent) {
        createdAt = event.createdAt
        let object = (try? JSONSerialization.jsonObject(with: Data(event.content.utf8))) as? [String: Any]
        name = object?["name"] as? String
        displayName = (object?["display_name"] as? String) ?? (object?["displayName"] as? String)
        about = object?["about"] as? String
        pictureURL = (object?["picture"] as? String) ?? (object?["picture_url"] as? String)
        let finiteRole = ((object?["finite_role"] as? String) ?? (object?["finiteRole"] as? String))?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        isAgent = Self.boolValue(object?["bot"]) == true || finiteRole == "agent"
    }

    private static func boolValue(_ value: Any?) -> Bool? {
        if let bool = value as? Bool {
            return bool
        }
        if let string = value as? String {
            switch string.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
            case "true", "1", "yes":
                return true
            case "false", "0", "no":
                return false
            default:
                return nil
            }
        }
        return nil
    }
}

private extension Array {
    func chunked(into size: Int) -> [[Element]] {
        guard size > 0 else { return [self] }
        var chunks: [[Element]] = []
        var index = startIndex
        while index < endIndex {
            let next = Swift.min(index + size, endIndex)
            chunks.append(Array(self[index..<next]))
            index = next
        }
        return chunks
    }
}

private extension String {
    var nostrNonEmptyTrimmed: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}
