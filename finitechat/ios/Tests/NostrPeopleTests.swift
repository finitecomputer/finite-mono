import XCTest
@testable import FiniteChat

final class NostrPeopleTests: XCTestCase {
    func testFollowProfileBuildsDirectChatProfileSummary() throws {
        let pubkey = String(repeating: "a", count: 64)
        let npub = try npubFromAccountId(accountId: pubkey)
        let follow = NostrFollowProfile(
            pubkey: pubkey,
            npub: npub,
            name: "Alice",
            username: "alice",
            about: "available for finite chat",
            pictureURL: "https://example.com/alice.jpg",
            relayHint: "wss://relay.example",
            keyPackageAvailability: .available
        )

        let profile = follow.appProfileSummary

        XCTAssertEqual(profile.accountId, pubkey)
        XCTAssertEqual(profile.npub, npub)
        XCTAssertEqual(profile.displayName, "Alice")
        XCTAssertEqual(profile.about, "available for finite chat")
        XCTAssertEqual(profile.picture, "https://example.com/alice.jpg")
        XCTAssertFalse(profile.stale)
    }

    func testUnknownFollowProfileBuildsStaleProfileSummary() throws {
        let pubkey = String(repeating: "b", count: 64)
        let npub = try npubFromAccountId(accountId: pubkey)
        let follow = NostrFollowProfile(
            pubkey: pubkey,
            npub: npub,
            name: nil,
            username: nil,
            about: nil,
            pictureURL: nil,
            relayHint: nil,
            keyPackageAvailability: .unknown
        )

        let profile = follow.appProfileSummary

        XCTAssertEqual(profile.displayName, follow.shortenedNpub)
        XCTAssertTrue(profile.stale)
    }

    func testKeyPackageAvailabilityUsesHumanReadableStatusText() {
        XCTAssertEqual(
            KeyPackageAvailability.available.userStatusText,
            "Ready to message"
        )
        XCTAssertEqual(
            KeyPackageAvailability.unavailable.userStatusText,
            "Needs Finite Chat open"
        )
        XCTAssertEqual(
            KeyPackageAvailability.unknown.userStatusText,
            "Checking availability"
        )
    }

    func testKeyPackageAvailabilityServiceChunksAndMergesResponses() async throws {
        let ids = [
            String(repeating: "a", count: 64),
            String(repeating: "b", count: 64),
            String(repeating: "c", count: 64),
        ]
        let recorder = KeyPackageAvailabilityChunkRecorder()
        let service = FiniteKeyPackageAvailabilityService(
            chunkSize: 2,
            availabilityLoader: { serverURL, accountIDs in
                XCTAssertEqual(serverURL, "https://chat.example")
                await recorder.record(accountIDs)
                return Dictionary(uniqueKeysWithValues: accountIDs.map { accountID in
                    (accountID, accountID == ids[1] || accountID == ids[2])
                })
            }
        )

        let availability = try await service.fetchAvailability(
            serverURL: "https://chat.example",
            accountIDs: ids
        )

        let chunks = await recorder.recordedChunks()
        XCTAssertEqual(chunks, [[ids[0], ids[1]], [ids[2]]])
        XCTAssertEqual(availability[ids[0]], false)
        XCTAssertEqual(availability[ids[1]], true)
        XCTAssertEqual(availability[ids[2]], true)
    }

    func testPeopleModelAppliesKeyPackageAvailabilityWithoutResortingProfiles() async throws {
        let material = try createNostrIdentity()
        let owner = material.accountId
        let amy = String(repeating: "a", count: 64)
        let zed = String(repeating: "b", count: 64)
        let relayService = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            eventLoader: { _, filter, _, _ in
                if filter.kinds == [3] {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1,
                            kind: 3,
                            tags: [["p", zed], ["p", amy]],
                            content: ""
                        ),
                    ]
                }
                if filter.kinds == [0] {
                    return [
                        NostrRelayEvent(
                            pubkey: zed,
                            createdAt: 1,
                            kind: 0,
                            tags: [],
                            content: #"{"display_name":"Zed"}"#
                        ),
                        NostrRelayEvent(
                            pubkey: amy,
                            createdAt: 1,
                            kind: 0,
                            tags: [],
                            content: #"{"display_name":"Amy"}"#
                        ),
                    ]
                }
                return []
            }
        )
        let availabilityService = FiniteKeyPackageAvailabilityService(
            availabilityLoader: { _, accountIDs in
                Dictionary(uniqueKeysWithValues: accountIDs.map { accountID in
                    (accountID, accountID == zed)
                })
            }
        )
        let model = NostrPeopleModel(
            service: relayService,
            keyPackageAvailabilityService: availabilityService
        )

        await model.refresh(
            identity: AppNostrIdentity(material: material),
            serverURL: "https://chat.example"
        )

        XCTAssertEqual(model.profiles.map(\.displayName), ["Amy", "Zed"])
        XCTAssertEqual(model.profiles.map(\.keyPackageAvailability), [.unavailable, .available])
    }

    func testPeopleModelLoadsFollowsFromAccountIDWithoutNostrIdentity() async throws {
        let owner = String(repeating: "a", count: 64)
        let bob = String(repeating: "b", count: 64)
        let relayService = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            eventLoader: { _, filter, _, _ in
                if filter.kinds == [3],
                   filter.authors == [owner],
                   filter.limit == 1
                {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1,
                            kind: 3,
                            tags: [["p", bob]],
                            content: ""
                        ),
                    ]
                }
                if filter.kinds == [0],
                   filter.authors == [bob]
                {
                    return [
                        NostrRelayEvent(
                            pubkey: bob,
                            createdAt: 1,
                            kind: 0,
                            tags: [],
                            content: #"{"display_name":"Runtime Bob"}"#
                        ),
                    ]
                }
                return []
            }
        )
        let cache = NostrPeopleCache(directory: temporaryCacheDirectory())
        let model = NostrPeopleModel(
            service: relayService,
            keyPackageAvailabilityService: FiniteKeyPackageAvailabilityService(
                availabilityLoader: { _, accountIDs in
                    Dictionary(uniqueKeysWithValues: accountIDs.map { ($0, true) })
                }
            ),
            cache: cache
        )

        await model.loadIfNeeded(
            accountID: "  \(owner.uppercased())  ",
            serverURL: "https://chat.example"
        )

        XCTAssertEqual(model.profiles.map(\.displayName), ["Runtime Bob"])
        XCTAssertEqual(model.profiles.map(\.keyPackageAvailability), [.available])
        XCTAssertEqual(
            model.statusText,
            "Loaded 1 of 1 follows from 1 relays."
        )
    }

    func testPeopleModelRechecksKeyPackageAvailabilityForOneProfile() async throws {
        let material = try createNostrIdentity()
        let owner = material.accountId
        let bob = String(repeating: "b", count: 64)
        let relayService = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            eventLoader: { _, filter, _, _ in
                if filter.kinds == [3] {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1,
                            kind: 3,
                            tags: [["p", bob]],
                            content: ""
                        ),
                    ]
                }
                return []
            }
        )
        let availability = KeyPackageAvailabilitySequence(accountID: bob)
        let availabilityService = FiniteKeyPackageAvailabilityService(
            availabilityLoader: { _, accountIDs in
                await availability.next(for: accountIDs)
            }
        )
        let model = NostrPeopleModel(
            service: relayService,
            keyPackageAvailabilityService: availabilityService
        )

        await model.refresh(
            identity: AppNostrIdentity(material: material),
            serverURL: "https://chat.example"
        )
        XCTAssertEqual(model.profiles[0].keyPackageAvailability, .unavailable)

        let updated = try await model.recheckKeyPackageAvailability(
            for: model.profiles[0],
            serverURL: "https://chat.example"
        )

        XCTAssertEqual(updated.pubkey, bob)
        XCTAssertEqual(updated.keyPackageAvailability, .available)
        XCTAssertEqual(model.profiles[0].pubkey, bob)
        XCTAssertEqual(model.profiles[0].keyPackageAvailability, .available)
    }

    func testPeopleModelShowsCachedProfilesBeforeBackgroundRefresh() async throws {
        let material = try createNostrIdentity()
        let owner = material.accountId
        let bob = String(repeating: "b", count: 64)
        let cache = NostrPeopleCache(directory: temporaryCacheDirectory())
        await cache.save(
            NostrFollowFetchResult(
                profiles: [
                    NostrFollowProfile(
                        pubkey: bob,
                        npub: try npubFromAccountId(accountId: bob),
                        name: "Cached Bob",
                        username: "cachedbob",
                        about: "cached",
                        pictureURL: "https://example.com/cached.jpg",
                        relayHint: nil,
                        keyPackageAvailability: .available
                    ),
                ],
                relayCount: 1,
                followedPubkeyCount: 1
            ),
            accountID: owner,
            serverURL: "https://chat.example"
        )
        let relayService = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            eventLoader: { _, filter, _, _ in
                try await Task.sleep(nanoseconds: 250_000_000)
                if filter.kinds == [3] {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1,
                            kind: 3,
                            tags: [["p", bob]],
                            content: ""
                        ),
                    ]
                }
                if filter.kinds == [0] {
                    return [
                        NostrRelayEvent(
                            pubkey: bob,
                            createdAt: 2,
                            kind: 0,
                            tags: [],
                            content: #"{"display_name":"Fresh Bob"}"#
                        ),
                    ]
                }
                return []
            }
        )
        let availabilityService = FiniteKeyPackageAvailabilityService(
            availabilityLoader: { _, accountIDs in
                Dictionary(uniqueKeysWithValues: accountIDs.map { ($0, true) })
            }
        )
        let model = NostrPeopleModel(
            service: relayService,
            keyPackageAvailabilityService: availabilityService,
            cache: cache
        )

        await model.loadIfNeeded(
            identity: AppNostrIdentity(material: material),
            serverURL: "https://chat.example"
        )

        XCTAssertEqual(model.profiles.map(\.displayName), ["Cached Bob"])
        XCTAssertEqual(model.profiles.map(\.keyPackageAvailability), [.unknown])

        try await Task.sleep(nanoseconds: 1_100_000_000)

        XCTAssertEqual(model.profiles.map(\.displayName), ["Fresh Bob"])
        XCTAssertEqual(model.profiles.map(\.keyPackageAvailability), [.available])
    }

    func testPeopleCacheIsAccountScopedAndAvailabilityNeutral() async throws {
        let owner = String(repeating: "a", count: 64)
        let bob = String(repeating: "b", count: 64)
        let cache = NostrPeopleCache(directory: temporaryCacheDirectory())
        await cache.save(
            NostrFollowFetchResult(
                profiles: [
                    NostrFollowProfile(
                        pubkey: bob,
                        npub: try npubFromAccountId(accountId: bob),
                        name: "Cached Bob",
                        username: "cachedbob",
                        about: nil,
                        pictureURL: "https://example.com/bob.jpg",
                        relayHint: nil,
                        keyPackageAvailability: .available
                    ),
                ],
                relayCount: 1,
                followedPubkeyCount: 1
            ),
            accountID: owner,
            serverURL: "https://chat-one.example"
        )

        let loaded = await cache.load(
            accountID: owner.uppercased(),
            serverURL: "https://chat-two.example"
        )

        XCTAssertEqual(loaded?.profiles.map(\.displayName), ["Cached Bob"])
        XCTAssertEqual(loaded?.profiles.map(\.keyPackageAvailability), [.unknown])
    }

    func testPeopleCacheLoadsSingleProfileMetadata() async throws {
        let owner = String(repeating: "a", count: 64)
        let cache = NostrPeopleCache(directory: temporaryCacheDirectory())
        await cache.saveProfile(
            NostrFollowProfile(
                pubkey: owner,
                npub: try npubFromAccountId(accountId: owner),
                name: "Cached Paul",
                username: "paul",
                about: "cached profile",
                pictureURL: "https://example.com/paul.jpg",
                relayHint: nil,
                keyPackageAvailability: .unknown
            )
        )

        let loaded = await cache.loadProfile(accountID: owner.uppercased())

        XCTAssertEqual(loaded?.pubkey, owner)
        XCTAssertEqual(loaded?.displayName, "Cached Paul")
        XCTAssertEqual(loaded?.username, "paul")
        XCTAssertEqual(loaded?.about, "cached profile")
        XCTAssertEqual(loaded?.pictureURL, "https://example.com/paul.jpg")
        XCTAssertEqual(loaded?.keyPackageAvailability, .available)
    }

    func testPeopleCachePrefersNonEmptyLegacyCacheOverEmptyCurrentCache() async throws {
        let owner = "555ae69c929b24dcd4e0057f052bf3c8f3a85f3fd84a2b8b4516fae5d0dff7d9"
        let bob = "7bc70ce80a898cc4b3e17d0992fa17264df188e1c806b60ed54424e8b41f5686"
        let carol = "7c765d407d3a9d5ea117cb8b8699628560787fc084a0c76afaa449bfbd121d84"
        let serverURL = "https://chat.finite.computer"
        let directory = temporaryCacheDirectory()
        let cache = NostrPeopleCache(directory: directory)
        try writePeopleCacheEnvelope(
            directory: directory,
            filename: "\(owner).json",
            profiles: [],
            relayCount: 13,
            followedPubkeyCount: 0
        )
        try writePeopleCacheEnvelope(
            directory: directory,
            filename: legacyPeopleCacheFilename(accountID: owner, serverURL: serverURL),
            profiles: [
                sparsePeopleProfile(pubkey: bob, npub: try npubFromAccountId(accountId: bob)),
                sparsePeopleProfile(pubkey: carol, npub: try npubFromAccountId(accountId: carol)),
            ],
            relayCount: 9,
            followedPubkeyCount: 467
        )

        let loaded = await cache.load(accountID: owner, serverURL: serverURL)

        XCTAssertEqual(loaded?.profiles.map(\.pubkey), [bob, carol])
        XCTAssertEqual(loaded?.profiles.map(\.keyPackageAvailability), [.unknown, .unknown])
        XCTAssertEqual(loaded?.followedPubkeyCount, 467)
    }

    func testPeopleCachePreserveNonEmptyResultConsidersLegacyCache() async throws {
        let owner = "555ae69c929b24dcd4e0057f052bf3c8f3a85f3fd84a2b8b4516fae5d0dff7d9"
        let bob = "7bc70ce80a898cc4b3e17d0992fa17264df188e1c806b60ed54424e8b41f5686"
        let serverURL = "https://chat.finite.computer"
        let directory = temporaryCacheDirectory()
        let cache = NostrPeopleCache(directory: directory)
        try writePeopleCacheEnvelope(
            directory: directory,
            filename: legacyPeopleCacheFilename(accountID: owner, serverURL: serverURL),
            profiles: [
                sparsePeopleProfile(pubkey: bob, npub: try npubFromAccountId(accountId: bob)),
            ],
            relayCount: 9,
            followedPubkeyCount: 467
        )

        await cache.save(
            NostrFollowFetchResult(
                profiles: [],
                relayCount: 13,
                followedPubkeyCount: 0
            ),
            accountID: owner,
            serverURL: serverURL,
            preserveNonEmptyOnEmptyResult: true
        )
        let loaded = await cache.load(accountID: owner, serverURL: serverURL)

        XCTAssertEqual(loaded?.profiles.map(\.pubkey), [bob])
        XCTAssertEqual(loaded?.followedPubkeyCount, 467)
    }

    func testPeopleCachePreservesNonEmptyListWhenRefreshFindsNoFollows() async throws {
        let owner = String(repeating: "a", count: 64)
        let bob = String(repeating: "b", count: 64)
        let cache = NostrPeopleCache(directory: temporaryCacheDirectory())
        await cache.save(
            NostrFollowFetchResult(
                profiles: [
                    NostrFollowProfile(
                        pubkey: bob,
                        npub: try npubFromAccountId(accountId: bob),
                        name: "Cached Bob",
                        username: nil,
                        about: nil,
                        pictureURL: nil,
                        relayHint: nil,
                        keyPackageAvailability: .unknown
                    ),
                ],
                relayCount: 1,
                followedPubkeyCount: 1
            ),
            accountID: owner,
            serverURL: "https://chat.example"
        )
        await cache.save(
            NostrFollowFetchResult(
                profiles: [],
                relayCount: 5,
                followedPubkeyCount: 0
            ),
            accountID: owner,
            serverURL: "https://chat.example",
            preserveNonEmptyOnEmptyResult: true
        )

        let loaded = await cache.load(accountID: owner, serverURL: "https://chat.example")

        XCTAssertEqual(loaded?.profiles.map(\.displayName), ["Cached Bob"])
    }

    func testPeopleCacheRestoresProfileMetadataAfterRawFollowRefresh() async throws {
        let owner = String(repeating: "a", count: 64)
        let bob = String(repeating: "b", count: 64)
        let cache = NostrPeopleCache(directory: temporaryCacheDirectory())
        await cache.save(
            NostrFollowFetchResult(
                profiles: [
                    NostrFollowProfile(
                        pubkey: bob,
                        npub: try npubFromAccountId(accountId: bob),
                        name: "Cached Bob",
                        username: "cachedbob",
                        about: "cached profile",
                        pictureURL: "https://example.com/cached.jpg",
                        relayHint: "wss://relay.example",
                        keyPackageAvailability: .available
                    ),
                ],
                relayCount: 1,
                followedPubkeyCount: 1
            ),
            accountID: owner,
            serverURL: "https://chat.example"
        )
        await cache.save(
            NostrFollowFetchResult(
                profiles: [
                    NostrFollowProfile(
                        pubkey: bob,
                        npub: try npubFromAccountId(accountId: bob),
                        name: nil,
                        username: nil,
                        about: nil,
                        pictureURL: nil,
                        relayHint: "wss://relay.example",
                        keyPackageAvailability: .unknown
                    ),
                ],
                relayCount: 1,
                followedPubkeyCount: 1
            ),
            accountID: owner,
            serverURL: "https://chat.example"
        )

        let loaded = await cache.load(accountID: owner, serverURL: "https://chat.example")

        XCTAssertEqual(loaded?.profiles.map(\.displayName), ["Cached Bob"])
        XCTAssertEqual(loaded?.profiles.map(\.username), ["cachedbob"])
        XCTAssertEqual(loaded?.profiles.map(\.about), ["cached profile"])
        XCTAssertEqual(loaded?.profiles.map(\.pictureURL), ["https://example.com/cached.jpg"])
        XCTAssertEqual(loaded?.profiles.map(\.keyPackageAvailability), [.unknown])
    }

    func testPeopleCacheMergesPartialProfileMetadataRefresh() async throws {
        let owner = String(repeating: "a", count: 64)
        let bob = String(repeating: "b", count: 64)
        let cache = NostrPeopleCache(directory: temporaryCacheDirectory())
        await cache.save(
            NostrFollowFetchResult(
                profiles: [
                    NostrFollowProfile(
                        pubkey: bob,
                        npub: try npubFromAccountId(accountId: bob),
                        name: "Cached Bob",
                        username: "cachedbob",
                        about: "cached profile",
                        pictureURL: "https://example.com/cached.jpg",
                        relayHint: nil,
                        keyPackageAvailability: .unknown
                    ),
                ],
                relayCount: 1,
                followedPubkeyCount: 1
            ),
            accountID: owner,
            serverURL: "https://chat.example"
        )
        await cache.save(
            NostrFollowFetchResult(
                profiles: [
                    NostrFollowProfile(
                        pubkey: bob,
                        npub: try npubFromAccountId(accountId: bob),
                        name: "Fresh Bob",
                        username: nil,
                        about: nil,
                        pictureURL: nil,
                        relayHint: nil,
                        keyPackageAvailability: .unknown
                    ),
                ],
                relayCount: 1,
                followedPubkeyCount: 1
            ),
            accountID: owner,
            serverURL: "https://chat.example"
        )

        let loaded = await cache.load(accountID: owner, serverURL: "https://chat.example")

        XCTAssertEqual(loaded?.profiles.map(\.displayName), ["Fresh Bob"])
        XCTAssertEqual(loaded?.profiles.map(\.username), ["cachedbob"])
        XCTAssertEqual(loaded?.profiles.map(\.about), ["cached profile"])
        XCTAssertEqual(loaded?.profiles.map(\.pictureURL), ["https://example.com/cached.jpg"])
    }

    func testPeopleModelKeepsVisibleProfilesWhenRefreshFindsNoFollows() async throws {
        let material = try createNostrIdentity()
        let owner = material.accountId
        let bob = String(repeating: "b", count: 64)
        let sequence = FollowRefreshSequence(owner: owner, followed: bob)
        let relayService = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            discoveryRelays: [],
            eventLoader: { _, filter, _, _ in
                await sequence.events(for: filter)
            }
        )
        let availabilityService = FiniteKeyPackageAvailabilityService(
            availabilityLoader: { _, accountIDs in
                Dictionary(uniqueKeysWithValues: accountIDs.map { ($0, true) })
            }
        )
        let model = NostrPeopleModel(
            service: relayService,
            keyPackageAvailabilityService: availabilityService,
            cache: NostrPeopleCache(directory: temporaryCacheDirectory())
        )

        await model.refresh(
            identity: AppNostrIdentity(material: material),
            serverURL: "https://chat.example"
        )

        XCTAssertEqual(model.profiles.map(\.displayName), ["Fresh Bob"])
        XCTAssertEqual(model.profiles.map(\.keyPackageAvailability), [.available])

        await model.refresh(
            identity: AppNostrIdentity(material: material),
            serverURL: "https://chat.example"
        )

        XCTAssertEqual(model.profiles.map(\.displayName), ["Fresh Bob"])
        XCTAssertEqual(model.profiles.map(\.keyPackageAvailability), [.available])
        XCTAssertEqual(model.statusText, "Showing cached people. Refresh found no follows across 1 Nostr relays.")
    }

    func testPeopleModelEnrichesRawRefreshWithCachedProfileMetadata() async throws {
        let material = try createNostrIdentity()
        let owner = material.accountId
        let bob = String(repeating: "b", count: 64)
        let cache = NostrPeopleCache(directory: temporaryCacheDirectory())
        await cache.save(
            NostrFollowFetchResult(
                profiles: [
                    NostrFollowProfile(
                        pubkey: bob,
                        npub: try npubFromAccountId(accountId: bob),
                        name: "Cached Bob",
                        username: "cachedbob",
                        about: "cached profile",
                        pictureURL: "https://example.com/cached.jpg",
                        relayHint: nil,
                        keyPackageAvailability: .unknown
                    ),
                ],
                relayCount: 1,
                followedPubkeyCount: 1
            ),
            accountID: owner,
            serverURL: "https://chat.example"
        )
        let relayService = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            discoveryRelays: [],
            eventLoader: { _, filter, _, _ in
                if filter.kinds == [3] {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1_800_000_050,
                            kind: 3,
                            tags: [["p", bob]],
                            content: ""
                        ),
                    ]
                }
                return []
            }
        )
        let availabilityService = FiniteKeyPackageAvailabilityService(
            availabilityLoader: { _, accountIDs in
                Dictionary(uniqueKeysWithValues: accountIDs.map { ($0, true) })
            }
        )
        let model = NostrPeopleModel(
            service: relayService,
            keyPackageAvailabilityService: availabilityService,
            cache: cache
        )

        await model.refresh(
            identity: AppNostrIdentity(material: material),
            serverURL: "https://chat.example"
        )

        XCTAssertEqual(model.profiles.map(\.displayName), ["Cached Bob"])
        XCTAssertEqual(model.profiles.map(\.username), ["cachedbob"])
        XCTAssertEqual(model.profiles.map(\.about), ["cached profile"])
        XCTAssertEqual(model.profiles.map(\.pictureURL), ["https://example.com/cached.jpg"])
        XCTAssertEqual(model.profiles.map(\.keyPackageAvailability), [.available])
        XCTAssertEqual(model.statusText, "Loaded 1 of 1 follows from 1 relays.")
    }

    func testPeopleModelShowsFollowRowsBeforeMetadataRefreshFinishes() async throws {
        let material = try createNostrIdentity()
        let owner = material.accountId
        let bob = String(repeating: "b", count: 64)
        let metadataGate = MetadataFetchGate()
        let relayService = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            discoveryRelays: [],
            eventLoader: { _, filter, _, _ in
                if filter.kinds == [3] {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1_800_000_060,
                            kind: 3,
                            tags: [["p", bob, "", "Bobby"]],
                            content: ""
                        ),
                    ]
                }
                if filter.kinds == [0] {
                    await metadataGate.enterAndWait()
                    return [
                        NostrRelayEvent(
                            pubkey: bob,
                            createdAt: 1_800_000_061,
                            kind: 0,
                            tags: [],
                            content: #"{"display_name":"Fresh Bob","name":"freshbob"}"#
                        ),
                    ]
                }
                return []
            }
        )
        let cache = NostrPeopleCache(directory: temporaryCacheDirectory())
        let model = NostrPeopleModel(
            service: relayService,
            keyPackageAvailabilityService: FiniteKeyPackageAvailabilityService(
                availabilityLoader: { _, accountIDs in
                    Dictionary(uniqueKeysWithValues: accountIDs.map { ($0, true) })
                }
            ),
            cache: cache
        )

        let refresh = Task {
            await model.refresh(
                identity: AppNostrIdentity(material: material),
                serverURL: "https://chat.example"
            )
        }
        await metadataGate.waitUntilEntered()

        XCTAssertEqual(model.profiles.map(\.displayName), ["Bobby"])
        XCTAssertEqual(model.profiles.map(\.keyPackageAvailability), [.unknown])
        XCTAssertEqual(
            model.statusText,
            "Loaded 1 of 1 follows from 1 relays. Updating profiles..."
        )
        let cachedBeforeMetadata = await cache.load(
            accountID: owner,
            serverURL: "https://chat.example"
        )
        XCTAssertEqual(cachedBeforeMetadata?.profiles.map(\.displayName), ["Bobby"])
        XCTAssertEqual(cachedBeforeMetadata?.profiles.map(\.keyPackageAvailability), [.unknown])

        await metadataGate.release()
        await refresh.value

        XCTAssertEqual(model.profiles.map(\.displayName), ["Fresh Bob"])
        XCTAssertEqual(model.profiles.map(\.username), ["freshbob"])
        XCTAssertEqual(model.profiles.map(\.keyPackageAvailability), [.available])
        XCTAssertEqual(model.statusText, "Loaded 1 of 1 follows from 1 relays.")
    }

    func testPeopleModelNoFollowsStatusNamesAccountAndExpandedRelayCount() async throws {
        let owner = String(repeating: "a", count: 64)
        let npub = try npubFromAccountId(accountId: owner)
        let shortened = "\(npub.prefix(10))...\(npub.suffix(4))"
        let relayService = NostrRelayProfileService(
            relays: ["wss://primary.example"],
            discoveryRelays: ["wss://discovery.example"],
            eventLoader: { _, _, _, _ in [] }
        )
        let model = NostrPeopleModel(
            service: relayService,
            keyPackageAvailabilityService: FiniteKeyPackageAvailabilityService(
                availabilityLoader: { _, _ in [:] }
            ),
            cache: NostrPeopleCache(directory: temporaryCacheDirectory())
        )

        await model.refresh(accountID: owner, serverURL: "https://chat.example")

        XCTAssertTrue(model.profiles.isEmpty)
        XCTAssertEqual(
            model.statusText,
            "No follows found for \(shortened) across 2 Nostr relays."
        )
    }

    func testPeopleModelCachesFetchedProfilesBeforeAvailabilityFinishes() async throws {
        let material = try createNostrIdentity()
        let owner = material.accountId
        let bob = String(repeating: "b", count: 64)
        let cache = NostrPeopleCache(directory: temporaryCacheDirectory())
        let availabilityGate = AvailabilityGate(accountID: bob)
        let relayService = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            eventLoader: { _, filter, _, _ in
                if filter.kinds == [3] {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1,
                            kind: 3,
                            tags: [["p", bob]],
                            content: ""
                        ),
                    ]
                }
                if filter.kinds == [0] {
                    return [
                        NostrRelayEvent(
                            pubkey: bob,
                            createdAt: 2,
                            kind: 0,
                            tags: [],
                            content: #"{"display_name":"Fresh Bob"}"#
                        ),
                    ]
                }
                return []
            }
        )
        let availabilityService = FiniteKeyPackageAvailabilityService(
            availabilityLoader: { _, accountIDs in
                await availabilityGate.enterAndWait(for: accountIDs)
            }
        )
        let model = NostrPeopleModel(
            service: relayService,
            keyPackageAvailabilityService: availabilityService,
            cache: cache
        )

        let refresh = Task {
            await model.refresh(
                identity: AppNostrIdentity(material: material),
                serverURL: "https://chat.example"
            )
        }
        await availabilityGate.waitUntilEntered()

        let cached = await cache.load(
            accountID: owner,
            serverURL: "https://different-chat.example"
        )

        XCTAssertEqual(cached?.profiles.map(\.displayName), ["Fresh Bob"])
        XCTAssertEqual(cached?.profiles.map(\.keyPackageAvailability), [.unknown])

        await availabilityGate.release()
        await refresh.value
        XCTAssertEqual(model.profiles.map(\.keyPackageAvailability), [.available])
    }

    func testPeopleModelUsesCachedProfilesWhenRefreshFails() async throws {
        let material = try createNostrIdentity()
        let owner = material.accountId
        let bob = String(repeating: "b", count: 64)
        let cache = NostrPeopleCache(directory: temporaryCacheDirectory())
        await cache.save(
            NostrFollowFetchResult(
                profiles: [
                    NostrFollowProfile(
                        pubkey: bob,
                        npub: try npubFromAccountId(accountId: bob),
                        name: "Cached Bob",
                        username: nil,
                        about: nil,
                        pictureURL: nil,
                        relayHint: nil,
                        keyPackageAvailability: .unknown
                    ),
                ],
                relayCount: 1,
                followedPubkeyCount: 1
            ),
            accountID: owner,
            serverURL: "https://chat.example"
        )
        let relayService = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            eventLoader: { _, _, _, _ in
                throw URLError(.cannotConnectToHost)
            }
        )
        let model = NostrPeopleModel(
            service: relayService,
            keyPackageAvailabilityService: FiniteKeyPackageAvailabilityService(
                availabilityLoader: { _, _ in [:] }
            ),
            cache: cache
        )

        await model.refresh(
            identity: AppNostrIdentity(material: material),
            serverURL: "https://chat.example"
        )

        XCTAssertEqual(model.profiles.map(\.displayName), ["Cached Bob"])
        XCTAssertTrue(model.statusText?.contains("Showing cached people") == true)
    }

    func testFetchFollowProfilesUsesContactListAndToleratesRelayFailure() async throws {
        let owner = String(repeating: "a", count: 64)
        let bob = String(repeating: "b", count: 64)
        let anonymous = String(repeating: "c", count: 64)

        let service = NostrRelayProfileService(
            relays: ["wss://bad.example", "wss://good.example"],
            eventLoader: { relay, filter, _, _ in
                if relay == "wss://bad.example" {
                    throw URLError(.cannotConnectToHost)
                }

                if filter.kinds == [3],
                   filter.authors == [owner],
                   filter.limit == 1
                {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1_800_000_000,
                            kind: 3,
                            tags: [
                                ["p", anonymous, "", ""],
                                ["p", bob, "wss://relay.example", "Bobby"],
                                ["e", bob],
                            ],
                            content: ""
                        ),
                    ]
                }

                if filter.kinds == [0],
                   Set(filter.authors) == Set([bob, anonymous]),
                   filter.limit == 2
                {
                    return [
                        NostrRelayEvent(
                            pubkey: bob,
                            createdAt: 1_800_000_001,
                            kind: 0,
                            tags: [],
                            content: #"{"name":"bob","display_name":"Bob Miller","about":"hi","picture":"https://example.com/bob.jpg"}"#
                        ),
                    ]
                }

                return []
            }
        )

        let result = try await service.fetchFollowProfiles(forAccountID: owner.uppercased())

        XCTAssertEqual(result.relayCount, 2)
        XCTAssertEqual(result.followedPubkeyCount, 2)
        XCTAssertEqual(result.profiles.map(\.pubkey), [bob, anonymous])
        XCTAssertEqual(result.profiles[0].displayName, "Bob Miller")
        XCTAssertEqual(result.profiles[0].username, "bob")
        XCTAssertEqual(result.profiles[0].about, "hi")
        XCTAssertEqual(result.profiles[0].pictureURL, "https://example.com/bob.jpg")
        XCTAssertEqual(result.profiles[0].relayHint, "wss://relay.example")
        XCTAssertEqual(result.profiles[1].displayName, result.profiles[1].shortenedNpub)
    }

    func testFetchProfileLoadsSignedInKindZeroMetadata() async throws {
        let owner = String(repeating: "a", count: 64)
        let service = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            discoveryRelays: [],
            eventLoader: { _, filter, _, _ in
                if filter.kinds == [0],
                   filter.authors == [owner],
                   filter.limit == 1
                {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1_800_000_100,
                            kind: 0,
                            tags: [],
                            content: #"{"name":"paul","display_name":"Paul","about":"Finite","picture":"https://example.com/paul.jpg"}"#
                        ),
                    ]
                }
                return []
            }
        )

        let profile = await service.fetchProfile(forAccountID: owner.uppercased())

        XCTAssertEqual(profile?.pubkey, owner)
        XCTAssertEqual(profile?.npub, try npubFromAccountId(accountId: owner))
        XCTAssertEqual(profile?.displayName, "Paul")
        XCTAssertEqual(profile?.username, "paul")
        XCTAssertEqual(profile?.about, "Finite")
        XCTAssertEqual(profile?.pictureURL, "https://example.com/paul.jpg")
        XCTAssertEqual(profile?.keyPackageAvailability, .available)
    }

    func testFetchFollowProfileSeedsDoesNotWaitForMetadata() async throws {
        let owner = String(repeating: "a", count: 64)
        let bob = String(repeating: "b", count: 64)
        let filterRecorder = RelayFilterRecorder()
        let service = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            discoveryRelays: [],
            eventLoader: { _, filter, _, _ in
                await filterRecorder.record(filter)
                if filter.kinds == [3] {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1_800_000_062,
                            kind: 3,
                            tags: [["p", bob, "wss://profile.example", "Bobby"]],
                            content: ""
                        ),
                    ]
                }
                return []
            }
        )

        let seeds = try await service.fetchFollowProfileSeeds(forAccountID: owner)

        XCTAssertEqual(seeds.relayCount, 1)
        XCTAssertEqual(seeds.followedPubkeyCount, 1)
        XCTAssertEqual(seeds.metadataRelays, ["wss://relay.example", "wss://profile.example"])
        XCTAssertEqual(seeds.profiles.map(\.pubkey), [bob])
        XCTAssertEqual(seeds.profiles.map(\.displayName), ["Bobby"])
        let sawMetadataFetch = await filterRecorder.sawMetadataFetch
        XCTAssertFalse(sawMetadataFetch)
    }

    func testFetchFollowProfilesUsesNewestContactListEvent() async throws {
        let owner = String(repeating: "1", count: 64)
        let olderFollow = String(repeating: "2", count: 64)
        let newerFollow = String(repeating: "3", count: 64)

        let service = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            eventLoader: { _, filter, _, _ in
                if filter.kinds == [3] {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 10,
                            kind: 3,
                            tags: [["p", olderFollow]],
                            content: ""
                        ),
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 20,
                            kind: 3,
                            tags: [["p", newerFollow, "", "Newer"]],
                            content: ""
                        ),
                    ]
                }
                return []
            }
        )

        let result = try await service.fetchFollowProfiles(forAccountID: owner)

        XCTAssertEqual(result.followedPubkeyCount, 1)
        XCTAssertEqual(result.profiles.map(\.pubkey), [newerFollow])
        XCTAssertEqual(result.profiles[0].displayName, "Newer")
    }

    func testFetchFollowProfilesUsesNip65RelaysForContactListAndMetadata() async throws {
        let owner = String(repeating: "d", count: 64)
        let followed = String(repeating: "e", count: 64)

        let service = NostrRelayProfileService(
            relays: ["wss://bootstrap.example"],
            eventLoader: { relay, filter, _, _ in
                if filter.kinds == [10_002],
                   relay == "wss://bootstrap.example"
                {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1_800_000_010,
                            kind: 10_002,
                            tags: [
                                ["r", "wss://read-only.example", "read"],
                                ["r", "wss://write.example", "write"],
                                ["r", "wss://implicit-read-write.example"],
                            ],
                            content: ""
                        ),
                    ]
                }

                if filter.kinds == [3],
                   relay == "wss://read-only.example"
                {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1_800_000_011,
                            kind: 3,
                            tags: [["p", followed]],
                            content: ""
                        ),
                    ]
                }

                if filter.kinds == [0],
                   relay == "wss://read-only.example"
                {
                    return [
                        NostrRelayEvent(
                            pubkey: followed,
                            createdAt: 1_800_000_012,
                            kind: 0,
                            tags: [],
                            content: #"{"display_name":"Read Relay Follow"}"#
                        ),
                    ]
                }

                return []
            }
        )

        let result = try await service.fetchFollowProfiles(forAccountID: owner)

        XCTAssertEqual(result.relayCount, 4)
        XCTAssertEqual(result.followedPubkeyCount, 1)
        XCTAssertEqual(result.profiles[0].displayName, "Read Relay Follow")
    }

    func testFetchFollowProfilesFallsBackToDiscoveryRelaysForMissingMetadata() async throws {
        let owner = String(repeating: "8", count: 64)
        let followed = String(repeating: "9", count: 64)

        let service = NostrRelayProfileService(
            relays: ["wss://contacts.example"],
            discoveryRelays: ["wss://metadata.example"],
            eventLoader: { relay, filter, _, _ in
                if filter.kinds == [3],
                   relay == "wss://contacts.example"
                {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1_800_000_025,
                            kind: 3,
                            tags: [["p", followed]],
                            content: ""
                        ),
                    ]
                }

                if filter.kinds == [0],
                   relay == "wss://metadata.example"
                {
                    return [
                        NostrRelayEvent(
                            pubkey: followed,
                            createdAt: 1_800_000_026,
                            kind: 0,
                            tags: [],
                            content: #"{"display_name":"Metadata Relay Friend","name":"metadatafriend"}"#
                        ),
                    ]
                }

                return []
            }
        )

        let result = try await service.fetchFollowProfiles(forAccountID: owner)

        XCTAssertEqual(result.followedPubkeyCount, 1)
        XCTAssertEqual(result.profiles[0].displayName, "Metadata Relay Friend")
        XCTAssertEqual(result.profiles[0].username, "metadatafriend")
    }

    func testFetchFollowProfilesFallsBackToDiscoveryRelaysWhenPrimaryMissesContacts() async throws {
        let owner = String(repeating: "6", count: 64)
        let followed = String(repeating: "7", count: 64)

        let service = NostrRelayProfileService(
            relays: ["wss://primary.example"],
            discoveryRelays: ["wss://discovery.example"],
            eventLoader: { relay, filter, _, _ in
                if relay == "wss://primary.example" {
                    throw URLError(.timedOut)
                }

                if filter.kinds == [10_002],
                   relay == "wss://discovery.example"
                {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1_800_000_030,
                            kind: 10_002,
                            tags: [
                                ["r", "wss://contacts.example", "write"],
                                ["r", "wss://profile.example", "read"],
                            ],
                            content: ""
                        ),
                    ]
                }

                if filter.kinds == [3],
                   relay == "wss://contacts.example"
                {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1_800_000_031,
                            kind: 3,
                            tags: [["p", followed, "wss://profile.example", "Fallback Bob"]],
                            content: ""
                        ),
                    ]
                }

                if filter.kinds == [0],
                   relay == "wss://profile.example"
                {
                    return [
                        NostrRelayEvent(
                            pubkey: followed,
                            createdAt: 1_800_000_032,
                            kind: 0,
                            tags: [],
                            content: #"{"display_name":"Discovery Relay Bob","name":"discoverybob"}"#
                        ),
                    ]
                }

                return []
            }
        )

        let result = try await service.fetchFollowProfiles(forAccountID: owner)

        XCTAssertEqual(result.relayCount, 4)
        XCTAssertEqual(result.followedPubkeyCount, 1)
        XCTAssertEqual(result.profiles[0].pubkey, followed)
        XCTAssertEqual(result.profiles[0].displayName, "Discovery Relay Bob")
        XCTAssertEqual(result.profiles[0].username, "discoverybob")
        XCTAssertEqual(result.profiles[0].relayHint, "wss://profile.example")
    }

    func testFetchFollowProfilesUsesContactRelayHintForMetadata() async throws {
        let owner = String(repeating: "4", count: 64)
        let followed = String(repeating: "5", count: 64)

        let service = NostrRelayProfileService(
            relays: ["wss://bootstrap.example"],
            eventLoader: { relay, filter, _, _ in
                if filter.kinds == [3],
                   relay == "wss://bootstrap.example"
                {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1_800_000_020,
                            kind: 3,
                            tags: [["p", followed, "wss://profile.example", "Fallback Name"]],
                            content: ""
                        ),
                    ]
                }

                if filter.kinds == [0],
                   relay == "wss://profile.example"
                {
                    return [
                        NostrRelayEvent(
                            pubkey: followed,
                            createdAt: 1_800_000_021,
                            kind: 0,
                            tags: [],
                            content: #"{"display_name":"Relay Hint Profile","name":"relayhint"}"#
                        ),
                    ]
                }

                return []
            }
        )

        let result = try await service.fetchFollowProfiles(forAccountID: owner)

        XCTAssertEqual(result.followedPubkeyCount, 1)
        XCTAssertEqual(result.profiles[0].displayName, "Relay Hint Profile")
        XCTAssertEqual(result.profiles[0].username, "relayhint")
    }

    func testFetchFollowProfilesFetchesMetadataChunksConcurrently() async throws {
        let owner = String(repeating: "1", count: 64)
        let followed = (0..<81).map { index in
            String(format: "%064x", index + 2)
        }
        let overlapProbe = MetadataChunkOverlapProbe()

        let service = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            discoveryRelays: [],
            eventLoader: { _, filter, _, _ in
                if filter.kinds == [3] {
                    return [
                        NostrRelayEvent(
                            pubkey: owner,
                            createdAt: 1_800_000_050,
                            kind: 3,
                            tags: followed.map { ["p", $0] },
                            content: ""
                        ),
                    ]
                }

                if filter.kinds == [0] {
                    await overlapProbe.recordMetadataChunk()
                    try await Task.sleep(nanoseconds: 150_000_000)
                    guard await overlapProbe.sawOverlappingMetadataChunks else { return [] }
                    return filter.authors.map { pubkey in
                        NostrRelayEvent(
                            pubkey: pubkey,
                            createdAt: 1_800_000_051,
                            kind: 0,
                            tags: [],
                            content: #"{"display_name":"Friend \#(pubkey.suffix(2))"}"#
                        )
                    }
                }

                return []
            }
        )

        let result = try await service.fetchFollowProfiles(forAccountID: owner)

        XCTAssertEqual(result.followedPubkeyCount, 81)
        XCTAssertTrue(
            result.profiles.contains { $0.pubkey == followed[0] && $0.displayName.hasPrefix("Friend") }
        )
        XCTAssertTrue(
            result.profiles.contains { $0.pubkey == followed[80] && $0.displayName.hasPrefix("Friend") }
        )
    }

    func testLiveFollowProfilesFixtureLoadsFromConfiguredRelays() async throws {
#if FINITECHAT_LIVE_NOSTR_TESTS
        let accountID = "4dcfa4f7ab49fb1484623c5f4c271fd0a079691c6d3ea3b1da0221a418638e8e"
        let relays = [
            "wss://relay.primal.net",
            "wss://nos.lol",
            "wss://relay.damus.io",
            "wss://us-east.nostr.pikachat.org",
            "wss://eu.nostr.pikachat.org",
        ]
        let service = NostrRelayProfileService(
            relays: relays,
            timeoutSeconds: 10
        )

        let result = try await service.fetchFollowProfiles(forAccountID: accountID)

        XCTAssertEqual(result.relayCount, relays.count)
        XCTAssertGreaterThanOrEqual(result.followedPubkeyCount, 4)
        XCTAssertGreaterThanOrEqual(result.profiles.count, 4)
        XCTAssertTrue(
            result.profiles.contains { $0.displayName.localizedCaseInsensitiveContains("jack") },
            "expected the live fixture to include Jack"
        )
        XCTAssertTrue(
            result.profiles.contains { $0.displayName.localizedCaseInsensitiveContains("fiatjaf") },
            "expected the live fixture to include fiatjaf"
        )
#else
        throw XCTSkip("Pass OTHER_SWIFT_FLAGS='$(inherited) -D FINITECHAT_LIVE_NOSTR_TESTS' to run the live Nostr relay fixture test.")
#endif
    }

    func testLiveConfiguredAccountIDsLoadFollowsFromConfiguredRelays() async throws {
#if FINITECHAT_LIVE_NOSTR_TESTS
        let environment = ProcessInfo.processInfo.environment
        let rawAccountIDs = environment["FINITECHAT_LIVE_NOSTR_ACCOUNT_IDS"]
            ?? environment["TEST_RUNNER_FINITECHAT_LIVE_NOSTR_ACCOUNT_IDS"]
            ?? ""
        let accountIDs = rawAccountIDs
            .split(separator: ",")
            .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .filter { !$0.isEmpty }
        guard !accountIDs.isEmpty else {
            throw XCTSkip("Set FINITECHAT_LIVE_NOSTR_ACCOUNT_IDS to comma-separated hex pubkeys.")
        }

        let service = NostrRelayProfileService(timeoutSeconds: 10)
        for accountID in accountIDs {
            let result = try await service.fetchFollowProfileSeeds(forAccountID: accountID)
            XCTAssertGreaterThan(
                result.followedPubkeyCount,
                0,
                "expected \(accountID) to have visible follow contacts"
            )
            XCTAssertFalse(
                result.profiles.isEmpty,
                "expected \(accountID) to render at least one follow seed"
            )
        }
#else
        throw XCTSkip("Pass OTHER_SWIFT_FLAGS='$(inherited) -D FINITECHAT_LIVE_NOSTR_TESTS' to run live Nostr relay tests.")
#endif
    }
}

private func temporaryCacheDirectory(
    file: StaticString = #filePath,
    line: UInt = #line
) -> URL {
    let url = FileManager.default.temporaryDirectory
        .appendingPathComponent("FiniteChatTests", isDirectory: true)
        .appendingPathComponent(UUID().uuidString, isDirectory: true)
    do {
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    } catch {
        XCTFail("failed to create temp cache directory: \(error)", file: file, line: line)
    }
    return url
}

private func writePeopleCacheEnvelope(
    directory: URL,
    filename: String,
    profiles: [[String: Any]],
    relayCount: Int,
    followedPubkeyCount: Int,
    file: StaticString = #filePath,
    line: UInt = #line
) throws {
    let envelope: [String: Any] = [
        "profiles": profiles,
        "relayCount": relayCount,
        "followedPubkeyCount": followedPubkeyCount,
        "cachedAt": Date().timeIntervalSinceReferenceDate,
    ]
    let data = try JSONSerialization.data(withJSONObject: envelope, options: [.sortedKeys])
    do {
        try data.write(to: directory.appendingPathComponent(filename), options: [.atomic])
    } catch {
        XCTFail("failed to write people cache fixture: \(error)", file: file, line: line)
        throw error
    }
}

private func sparsePeopleProfile(pubkey: String, npub: String) -> [String: Any] {
    [
        "pubkey": pubkey,
        "npub": npub,
        "keyPackageAvailability": KeyPackageAvailability.unknown.rawValue,
    ]
}

private func legacyPeopleCacheFilename(accountID: String, serverURL: String) -> String {
    let accountKey = accountID.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    let serverKey = serverURL
        .trimmingCharacters(in: .whitespacesAndNewlines)
        .lowercased()
        .utf8
        .map { String(format: "%02x", $0) }
        .joined()
    return "\(accountKey)-\(serverKey).json"
}

private actor KeyPackageAvailabilityChunkRecorder {
    private var chunks: [[String]] = []

    func record(_ chunk: [String]) {
        chunks.append(chunk)
    }

    func recordedChunks() -> [[String]] {
        chunks
    }
}

private actor MetadataChunkOverlapProbe {
    private var metadataChunkCount = 0

    var sawOverlappingMetadataChunks: Bool {
        metadataChunkCount > 1
    }

    func recordMetadataChunk() {
        metadataChunkCount += 1
    }
}

private actor RelayFilterRecorder {
    private var filters: [NostrRelayFilter] = []

    var sawMetadataFetch: Bool {
        filters.contains { $0.kinds == [0] }
    }

    func record(_ filter: NostrRelayFilter) {
        filters.append(filter)
    }
}

private actor MetadataFetchGate {
    private var entered = false
    private var released = false
    private var entryContinuations: [CheckedContinuation<Void, Never>] = []
    private var releaseContinuations: [CheckedContinuation<Void, Never>] = []

    func waitUntilEntered() async {
        if entered { return }
        await withCheckedContinuation { continuation in
            entryContinuations.append(continuation)
        }
    }

    func enterAndWait() async {
        entered = true
        for continuation in entryContinuations {
            continuation.resume()
        }
        entryContinuations.removeAll()
        if !released {
            await withCheckedContinuation { continuation in
                releaseContinuations.append(continuation)
            }
        }
    }

    func release() {
        released = true
        for continuation in releaseContinuations {
            continuation.resume()
        }
        releaseContinuations.removeAll()
    }
}

private actor FollowRefreshSequence {
    private let owner: String
    private let followed: String
    private var contactListFetchCount = 0

    init(owner: String, followed: String) {
        self.owner = owner
        self.followed = followed
    }

    func events(for filter: NostrRelayFilter) -> [NostrRelayEvent] {
        if filter.kinds == [3] {
            contactListFetchCount += 1
            guard contactListFetchCount == 1 else { return [] }
            return [
                NostrRelayEvent(
                    pubkey: owner,
                    createdAt: 1_800_000_040,
                    kind: 3,
                    tags: [["p", followed]],
                    content: ""
                ),
            ]
        }
        if filter.kinds == [0] {
            return [
                NostrRelayEvent(
                    pubkey: followed,
                    createdAt: 1_800_000_041,
                    kind: 0,
                    tags: [],
                    content: #"{"display_name":"Fresh Bob"}"#
                ),
            ]
        }
        return []
    }
}

private actor KeyPackageAvailabilitySequence {
    private let accountID: String
    private var calls = 0

    init(accountID: String) {
        self.accountID = accountID
    }

    func next(for accountIDs: [String]) -> [String: Bool] {
        calls += 1
        return Dictionary(uniqueKeysWithValues: accountIDs.map { accountID in
            (accountID, accountID == self.accountID && calls > 1)
        })
    }
}

private actor AvailabilityGate {
    private let accountID: String
    private var entered = false
    private var released = false
    private var entryContinuations: [CheckedContinuation<Void, Never>] = []
    private var releaseContinuations: [CheckedContinuation<Void, Never>] = []

    init(accountID: String) {
        self.accountID = accountID
    }

    func waitUntilEntered() async {
        if entered { return }
        await withCheckedContinuation { continuation in
            entryContinuations.append(continuation)
        }
    }

    func enterAndWait(for accountIDs: [String]) async -> [String: Bool] {
        entered = true
        for continuation in entryContinuations {
            continuation.resume()
        }
        entryContinuations.removeAll()
        if !released {
            await withCheckedContinuation { continuation in
                releaseContinuations.append(continuation)
            }
        }
        return Dictionary(uniqueKeysWithValues: accountIDs.map { accountID in
            (accountID, accountID == self.accountID)
        })
    }

    func release() {
        released = true
        for continuation in releaseContinuations {
            continuation.resume()
        }
        releaseContinuations.removeAll()
    }
}
