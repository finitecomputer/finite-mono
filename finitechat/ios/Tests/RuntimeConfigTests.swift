import XCTest
import CoreGraphics
import UIKit
@testable import FiniteChat

final class RuntimeConfigTests: XCTestCase {
    func testExplicitSaveTrimsAndPersistsConfig() throws {
        let url = try temporaryConfigURL()

        try RuntimeConfig(
            serverURL: "  http://127.0.0.1:8787  ",
            deviceID: "  ios  "
        ).save(storageURL: url)

        let persisted = try persistedConfig(at: url)
        XCTAssertEqual(persisted.serverURL, "http://127.0.0.1:8787")
        XCTAssertEqual(persisted.deviceID, "ios")
    }

    func testLaunchOverridesUseStableStoreWithoutRewritingStableConfig() throws {
        let url = try temporaryConfigURL()
        try RuntimeConfig(
            serverURL: "https://persisted.example",
            deviceID: "persisted-device"
        ).save(storageURL: url)

        let loaded = RuntimeConfig.load(
            environment: ["FINITECHAT_SERVER_URL": "https://env.example"],
            args: [
                "FiniteChat",
                "--finitechat-server",
                "https://args.example",
                "--finitechat-device",
                "transient-device",
            ],
            storageURL: url
        )

        XCTAssertEqual(loaded.serverURL, "https://args.example")
        XCTAssertEqual(loaded.deviceID, "transient-device")
        XCTAssertFalse(loaded.usesTransientStore)
        XCTAssertFalse(loaded.persistsRuntimeIdentityUpdates)

        let persisted = try persistedConfig(at: url)
        XCTAssertEqual(persisted.serverURL, "https://persisted.example")
        XCTAssertEqual(persisted.deviceID, "persisted-device")
        XCTAssertFalse(persisted.usesTransientStore)

        let relaunched = RuntimeConfig.load(
            environment: [:],
            args: ["FiniteChat"],
            storageURL: url
        )

        XCTAssertEqual(relaunched.serverURL, "https://persisted.example")
        XCTAssertEqual(relaunched.deviceID, "persisted-device")
        XCTAssertFalse(relaunched.usesTransientStore)
    }

    func testExplicitPersistentLaunchOverridesPersistForManualRelaunch() throws {
        let url = try temporaryConfigURL()
        try RuntimeConfig(
            serverURL: "https://persisted.example",
            deviceID: "persisted-device"
        ).save(storageURL: url)

        let loaded = RuntimeConfig.load(
            environment: ["FINITECHAT_SERVER_URL": "https://env.example"],
            args: [
                "FiniteChat",
                "--finitechat-server",
                "https://args.example",
                "--finitechat-device",
                "persisted-override-device",
                "--finitechat-persist-launch-config",
            ],
            storageURL: url
        )

        XCTAssertEqual(loaded.serverURL, "https://args.example")
        XCTAssertEqual(loaded.deviceID, "persisted-override-device")
        XCTAssertFalse(loaded.usesTransientStore)

        let persisted = try persistedConfig(at: url)
        XCTAssertEqual(persisted.serverURL, "https://args.example")
        XCTAssertEqual(persisted.deviceID, "persisted-override-device")
        XCTAssertFalse(persisted.usesTransientStore)

        let relaunched = RuntimeConfig.load(
            environment: [:],
            args: ["FiniteChat"],
            storageURL: url
        )

        XCTAssertEqual(relaunched.serverURL, "https://args.example")
        XCTAssertEqual(relaunched.deviceID, "persisted-override-device")
        XCTAssertFalse(relaunched.usesTransientStore)
    }

    func testTransientLaunchOverridesDoNotRewritePersistedConfig() throws {
        let url = try temporaryConfigURL()
        try RuntimeConfig(
            serverURL: "https://persisted.example",
            deviceID: "persisted-device"
        ).save(storageURL: url)

        let loaded = RuntimeConfig.load(
            environment: ["FINITECHAT_SERVER_URL": "https://env.example"],
            args: [
                "FiniteChat",
                "--finitechat-server",
                "https://args.example",
                "--finitechat-device",
                "transient-device",
                "--finitechat-transient-config",
            ],
            storageURL: url
        )

        XCTAssertEqual(loaded.serverURL, "https://args.example")
        XCTAssertEqual(loaded.deviceID, "transient-device")
        XCTAssertTrue(loaded.usesTransientStore)

        let persisted = try persistedConfig(at: url)
        XCTAssertEqual(persisted.serverURL, "https://persisted.example")
        XCTAssertEqual(persisted.deviceID, "persisted-device")
        XCTAssertFalse(persisted.usesTransientStore)
    }

    func testLaunchAutomationUsesStableStoreAndDoesNotRewritePersistedConfigByDefault() throws {
        let url = try temporaryConfigURL()
        try RuntimeConfig(
            serverURL: "https://persisted.example",
            deviceID: "persisted-device"
        ).save(storageURL: url)

        let loaded = RuntimeConfig.load(
            environment: [:],
            args: [
                "FiniteChat",
                "--finitechat-server",
                "http://127.0.0.1:1",
                "--finitechat-device",
                "codex-persist-check",
                "--finitechat-auto-join",
                "finite://join?v=1&s=http%3A%2F%2F127.0.0.1%3A1&r=room-main&i=invite-1&t=token&a=npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqgcpfl3",
                "--finitechat-auto-send",
                "probe",
            ],
            storageURL: url
        )

        XCTAssertEqual(loaded.serverURL, "http://127.0.0.1:1")
        XCTAssertEqual(loaded.deviceID, "codex-persist-check")
        XCTAssertFalse(loaded.usesTransientStore)
        XCTAssertFalse(loaded.persistsRuntimeIdentityUpdates)

        let persisted = try persistedConfig(at: url)
        XCTAssertEqual(persisted.serverURL, "https://persisted.example")
        XCTAssertEqual(persisted.deviceID, "persisted-device")
        XCTAssertFalse(persisted.usesTransientStore)

        let relaunched = RuntimeConfig.load(
            environment: [:],
            args: ["FiniteChat"],
            storageURL: url
        )

        XCTAssertEqual(relaunched.serverURL, "https://persisted.example")
        XCTAssertEqual(relaunched.deviceID, "persisted-device")
        XCTAssertFalse(relaunched.usesTransientStore)
    }

    func testExplicitPersistentLocalLaunchAutomationRepairsServerForManualRelaunch() throws {
        let url = try temporaryConfigURL()
        try RuntimeConfig(
            serverURL: "https://persisted.example",
            deviceID: "persisted-device"
        ).save(storageURL: url)

        let loaded = RuntimeConfig.load(
            environment: [:],
            args: [
                "FiniteChat",
                "--finitechat-server",
                "http://127.0.0.1:1",
                "--finitechat-device",
                "codex-persist-check",
                "--finitechat-auto-create-room",
                "Probe",
                "--finitechat-auto-send",
                "probe",
                "--finitechat-persist-launch-config",
            ],
            storageURL: url
        )

        XCTAssertEqual(loaded.serverURL, "http://127.0.0.1:1")
        XCTAssertEqual(loaded.deviceID, "codex-persist-check")
        XCTAssertFalse(loaded.usesTransientStore)

        let persisted = try persistedConfig(at: url)
        XCTAssertEqual(persisted.serverURL, "http://127.0.0.1:1")
        XCTAssertEqual(persisted.deviceID, "codex-persist-check")
        XCTAssertFalse(persisted.usesTransientStore)

        let relaunched = RuntimeConfig.load(
            environment: [:],
            args: ["FiniteChat"],
            storageURL: url
        )

        XCTAssertEqual(relaunched.serverURL, "https://chat.finite.computer")
        XCTAssertEqual(relaunched.deviceID, "codex-persist-check")
        XCTAssertFalse(relaunched.usesTransientStore)

        let repaired = try persistedConfig(at: url)
        XCTAssertEqual(repaired.serverURL, "https://chat.finite.computer")
        XCTAssertEqual(repaired.deviceID, "codex-persist-check")
    }

    func testExplicitTransientLaunchAutomationDoesNotRewritePersistedConfig() throws {
        let url = try temporaryConfigURL()
        try RuntimeConfig(
            serverURL: "https://persisted.example",
            deviceID: "persisted-device"
        ).save(storageURL: url)

        let loaded = RuntimeConfig.load(
            environment: [:],
            args: [
                "FiniteChat",
                "--finitechat-server",
                "http://127.0.0.1:1",
                "--finitechat-device",
                "codex-persist-check",
                "--finitechat-auto-join",
                "finite://join?v=1&s=http%3A%2F%2F127.0.0.1%3A1&r=room-main&i=invite-1&t=token&a=npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqgcpfl3",
                "--finitechat-auto-send",
                "probe",
                "--finitechat-transient-config",
            ],
            storageURL: url
        )

        XCTAssertEqual(loaded.serverURL, "http://127.0.0.1:1")
        XCTAssertEqual(loaded.deviceID, "codex-persist-check")
        XCTAssertTrue(loaded.usesTransientStore)

        let persisted = try persistedConfig(at: url)
        XCTAssertEqual(persisted.serverURL, "https://persisted.example")
        XCTAssertEqual(persisted.deviceID, "persisted-device")
        XCTAssertFalse(persisted.usesTransientStore)
    }

    func testFirstLaunchAutomationOverrideDoesNotSeedStableConfig() throws {
        let url = try temporaryConfigURL()

        let loaded = RuntimeConfig.load(
            environment: [:],
            args: [
                "FiniteChat",
                "--finitechat-server",
                "http://127.0.0.1:1",
                "--finitechat-device",
                "codex-persist-check",
                "--finitechat-auto-create-room",
                "Probe",
            ],
            storageURL: url
        )

        XCTAssertEqual(loaded.serverURL, "http://127.0.0.1:1")
        XCTAssertEqual(loaded.deviceID, "codex-persist-check")
        XCTAssertFalse(loaded.usesTransientStore)
        XCTAssertFalse(loaded.persistsRuntimeIdentityUpdates)
        XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))

        let relaunched = RuntimeConfig.load(
            environment: [:],
            args: ["FiniteChat"],
            storageURL: url
        )

        XCTAssertEqual(relaunched.serverURL, "https://chat.finite.computer")
        assertGeneratedDefaultDeviceID(relaunched.deviceID)
        XCTAssertFalse(relaunched.usesTransientStore)
    }

    func testPersistedDevelopmentServerOnlyConfigRepairsToDefaultServerAndDevice() throws {
        for serverURL in [
            "http://192.168.1.226:8789",
            "http://127.0.0.1:18788",
            "https://10.0.0.3:8789",
            "https://finite-chat.local:8789",
        ] {
            let url = try temporaryConfigURL()
            try Data(#"{"server_url":"\#(serverURL)"}"#.utf8).write(to: url)

            let loaded = RuntimeConfig.load(
                environment: [:],
                args: ["FiniteChat"],
                storageURL: url
            )

            XCTAssertEqual(loaded.serverURL, "https://chat.finite.computer", serverURL)
            assertGeneratedDefaultDeviceID(loaded.deviceID)
            XCTAssertFalse(loaded.usesTransientStore, serverURL)

            let persisted = try persistedConfig(at: url)
            XCTAssertEqual(persisted.serverURL, "https://chat.finite.computer", serverURL)
            XCTAssertEqual(persisted.deviceID, loaded.deviceID, serverURL)
        }
    }

    func testFirstLaunchOverridesDoNotSeedStableConfig() throws {
        let url = try temporaryConfigURL()

        let firstLaunch = RuntimeConfig.load(
            environment: [:],
            args: [
                "FiniteChat",
                "--finitechat-server",
                "http://192.168.1.226:8789",
                "--finitechat-device",
                "qt433",
            ],
            storageURL: url
        )

        XCTAssertEqual(firstLaunch.serverURL, "http://192.168.1.226:8789")
        XCTAssertEqual(firstLaunch.deviceID, "qt433")
        XCTAssertFalse(firstLaunch.usesTransientStore)
        XCTAssertFalse(firstLaunch.persistsRuntimeIdentityUpdates)
        XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))

        let relaunched = RuntimeConfig.load(
            environment: [:],
            args: ["FiniteChat"],
            storageURL: url
        )

        XCTAssertEqual(relaunched.serverURL, "https://chat.finite.computer")
        assertGeneratedDefaultDeviceID(relaunched.deviceID)
        XCTAssertFalse(relaunched.usesTransientStore)
    }

    func testFirstLaunchLocalOverrideWithPersistRepairsServerForRelaunch() throws {
        let url = try temporaryConfigURL()

        let firstLaunch = RuntimeConfig.load(
            environment: [:],
            args: [
                "FiniteChat",
                "--finitechat-server",
                "http://192.168.1.226:8789",
                "--finitechat-device",
                "qt433",
                "--finitechat-persist-launch-config",
            ],
            storageURL: url
        )

        XCTAssertEqual(firstLaunch.serverURL, "http://192.168.1.226:8789")
        XCTAssertEqual(firstLaunch.deviceID, "qt433")
        XCTAssertFalse(firstLaunch.usesTransientStore)
        XCTAssertEqual(try persistedConfig(at: url), firstLaunch)

        let relaunched = RuntimeConfig.load(
            environment: [:],
            args: ["FiniteChat"],
            storageURL: url
        )

        XCTAssertEqual(relaunched.serverURL, "https://chat.finite.computer")
        XCTAssertEqual(relaunched.deviceID, "qt433")

        let repaired = try persistedConfig(at: url)
        XCTAssertEqual(repaired.serverURL, "https://chat.finite.computer")
        XCTAssertEqual(repaired.deviceID, "qt433")
    }

    func testMissingConfigIgnoresLegacyDeviceStore() throws {
        let url = try temporaryConfigURL()
        let supportURL = url.deletingLastPathComponent()
        let deviceStoreURL = supportURL
            .appendingPathComponent("FiniteChat", isDirectory: true)
            .appendingPathComponent("qt433", isDirectory: true)
        try FileManager.default.createDirectory(
            at: deviceStoreURL,
            withIntermediateDirectories: true
        )
        try Data("secret".utf8).write(to: deviceStoreURL.appendingPathComponent("account-secret.hex"))

        let loaded = RuntimeConfig.load(
            environment: [:],
            args: ["FiniteChat"],
            storageURL: url
        )

        XCTAssertEqual(loaded.serverURL, "https://chat.finite.computer")
        assertGeneratedDefaultDeviceID(loaded.deviceID)
        XCTAssertEqual(try persistedConfig(at: url), loaded)
    }

    func testMissingConfigIgnoresLegacyPlaceholderStore() throws {
        let url = try temporaryConfigURL()
        let supportURL = url.deletingLastPathComponent()
        let deviceStoreURL = supportURL
            .appendingPathComponent("FiniteChat", isDirectory: true)
            .appendingPathComponent("qt433", isDirectory: true)
        try FileManager.default.createDirectory(
            at: deviceStoreURL,
            withIntermediateDirectories: true
        )
        try Data().write(to: deviceStoreURL.appendingPathComponent("client.sqlite3"))

        let loaded = RuntimeConfig.load(
            environment: [:],
            args: ["FiniteChat"],
            storageURL: url
        )

        XCTAssertEqual(loaded.serverURL, "https://chat.finite.computer")
        assertGeneratedDefaultDeviceID(loaded.deviceID)
        XCTAssertEqual(try persistedConfig(at: url), loaded)
    }

    func testPersistedDevelopmentServerRepairDoesNotRecoverLegacyDeviceStore() throws {
        let url = try temporaryConfigURL()
        try RuntimeConfig(
            serverURL: "http://192.168.1.226:8789",
            deviceID: "ios"
        ).save(storageURL: url)

        let supportURL = url.deletingLastPathComponent()
        let dataRoot = supportURL.appendingPathComponent("FiniteChat", isDirectory: true)
        let emptyStoreURL = dataRoot.appendingPathComponent("ios", isDirectory: true)
        let initializedStoreURL = dataRoot.appendingPathComponent("qt433", isDirectory: true)
        try FileManager.default.createDirectory(
            at: emptyStoreURL,
            withIntermediateDirectories: true
        )
        try FileManager.default.createDirectory(
            at: initializedStoreURL,
            withIntermediateDirectories: true
        )
        try Data().write(to: emptyStoreURL.appendingPathComponent("client.sqlite3"))
        try Data("secret".utf8)
            .write(to: initializedStoreURL.appendingPathComponent("account-secret.hex"))

        let loaded = RuntimeConfig.load(
            environment: [:],
            args: ["FiniteChat"],
            storageURL: url
        )

        XCTAssertEqual(loaded.serverURL, "https://chat.finite.computer")
        XCTAssertEqual(loaded.deviceID, "ios")
        XCTAssertEqual(try persistedConfig(at: url), loaded)
    }

    func testMissingConfigIgnoresMultipleLegacyDeviceStores() throws {
        let url = try temporaryConfigURL()
        let supportURL = url.deletingLastPathComponent()
        let dataRoot = supportURL.appendingPathComponent("FiniteChat", isDirectory: true)
        for deviceID in ["alice", "bob"] {
            let storeURL = dataRoot.appendingPathComponent(deviceID, isDirectory: true)
            try FileManager.default.createDirectory(
                at: storeURL,
                withIntermediateDirectories: true
            )
            try Data("secret".utf8).write(to: storeURL.appendingPathComponent("account-secret.hex"))
        }

        let loaded = RuntimeConfig.load(
            environment: [:],
            args: ["FiniteChat"],
            storageURL: url
        )

        XCTAssertEqual(loaded.serverURL, "https://chat.finite.computer")
        assertGeneratedDefaultDeviceID(loaded.deviceID)
        XCTAssertEqual(try persistedConfig(at: url), loaded)
    }

    func testRuntimeDataStoreIgnoresLegacyStoreAndCreatesStableStore() throws {
        let supportURL = try temporarySupportURL()
        let legacyStoreURL = supportURL
            .appendingPathComponent("FiniteChat", isDirectory: true)
            .appendingPathComponent("qt433", isDirectory: true)
        try FileManager.default.createDirectory(
            at: legacyStoreURL,
            withIntermediateDirectories: true
        )
        try Data("secret".utf8)
            .write(to: legacyStoreURL.appendingPathComponent("account-secret.hex"))
        try Data("sqlite".utf8)
            .write(to: legacyStoreURL.appendingPathComponent("client.sqlite3"))

        let dataDir = try RuntimeDataStore.dataDir(
            deviceID: "qt433",
            applicationSupportURL: supportURL
        )
        let stableURL = URL(fileURLWithPath: dataDir)

        XCTAssertEqual(stableURL.lastPathComponent, "FiniteChatStore")
        XCTAssertTrue(FileManager.default.fileExists(atPath: stableURL.path))
        XCTAssertFalse(FileManager.default.fileExists(
            atPath: stableURL.appendingPathComponent("account-secret.hex").path
        ))
        XCTAssertFalse(FileManager.default.fileExists(
            atPath: stableURL.appendingPathComponent("client.sqlite3").path
        ))
        XCTAssertTrue(FileManager.default.fileExists(
            atPath: legacyStoreURL.appendingPathComponent("account-secret.hex").path
        ))
    }

    func testRuntimeDataStoreKeepsExistingStableStore() throws {
        let supportURL = try temporarySupportURL()
        let stableStoreURL = supportURL.appendingPathComponent("FiniteChatStore", isDirectory: true)
        try FileManager.default.createDirectory(
            at: stableStoreURL,
            withIntermediateDirectories: true
        )
        try Data("stable".utf8)
            .write(to: stableStoreURL.appendingPathComponent("account-secret.hex"))

        let legacyStoreURL = supportURL
            .appendingPathComponent("FiniteChat", isDirectory: true)
            .appendingPathComponent("qt433", isDirectory: true)
        try FileManager.default.createDirectory(
            at: legacyStoreURL,
            withIntermediateDirectories: true
        )
        try Data("legacy".utf8)
            .write(to: legacyStoreURL.appendingPathComponent("account-secret.hex"))

        let dataDir = try RuntimeDataStore.dataDir(
            deviceID: "qt433",
            applicationSupportURL: supportURL
        )
        let selectedURL = URL(fileURLWithPath: dataDir)

        XCTAssertEqual(selectedURL, stableStoreURL)
        XCTAssertEqual(
            try Data(contentsOf: stableStoreURL.appendingPathComponent("account-secret.hex")),
            Data("stable".utf8)
        )
    }

    func testRuntimeDataStoreUsesIsolatedTransientStore() throws {
        let supportURL = try temporarySupportURL()
        let stableStoreURL = supportURL.appendingPathComponent("FiniteChatStore", isDirectory: true)
        try FileManager.default.createDirectory(
            at: stableStoreURL,
            withIntermediateDirectories: true
        )
        try Data("stable".utf8)
            .write(to: stableStoreURL.appendingPathComponent("account-secret.hex"))

        let dataDir = try RuntimeDataStore.dataDir(
            deviceID: "codex/persist-check",
            applicationSupportURL: supportURL,
            transient: true
        )
        let transientURL = URL(fileURLWithPath: dataDir)

        XCTAssertEqual(transientURL.lastPathComponent, "codex-persist-check")
        XCTAssertEqual(transientURL.deletingLastPathComponent().lastPathComponent, "FiniteChatTransient")
        XCTAssertTrue(FileManager.default.fileExists(atPath: transientURL.path))
        XCTAssertEqual(
            try Data(contentsOf: stableStoreURL.appendingPathComponent("account-secret.hex")),
            Data("stable".utf8)
        )
    }

    private func temporaryConfigURL() throws -> URL {
        try temporarySupportURL().appendingPathComponent("finitechat_config.json")
    }

    private func temporarySupportURL() throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true
        )
        return directory
    }

    private func persistedConfig(at url: URL) throws -> RuntimeConfig {
        let data = try Data(contentsOf: url)
        return try JSONDecoder().decode(RuntimeConfig.self, from: data)
    }

    private func assertGeneratedDefaultDeviceID(
        _ deviceID: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertTrue(deviceID.hasPrefix("ios-"), file: file, line: line)
        XCTAssertEqual(deviceID.count, 16, file: file, line: line)
    }
}

final class ChatTimelineActivityTests: XCTestCase {
    func testActivityRowHasStableNonDurableIdentity() {
        let row = ChatTimelineRow.activity(
            ChatTimelineActivity(
                kind: .working,
                members: [
                    appTypingMember(
                        roomID: "room-main",
                        deviceID: "hermes-agent",
                        displayName: "Hermes",
                        activityKind: "working"
                    ),
                ]
            )
        )

        XCTAssertEqual(row.id, "activity-working")
        XCTAssertNil(row.oldestMessageID)
    }

    func testGroupedMessageRowCanBeFoundByAnyContainedMessageID() {
        let first = chatMessage(id: "message-1", seq: 1, text: "first")
        let second = chatMessage(id: "message-2", seq: 2, text: "second")

        let rows = ChatTimeline.rows(messages: [first, second])

        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(
            ChatTimeline.rowID(containingMessageID: "message-1", rows: rows),
            "group-message-1-message-2-2"
        )
        XCTAssertEqual(
            ChatTimeline.rowID(containingMessageID: "message-2", rows: rows),
            "group-message-1-message-2-2"
        )
        XCTAssertNil(ChatTimeline.rowID(containingMessageID: "missing-message", rows: rows))
    }

    func testMessageGroupUsesCachedProfilePicture() {
        let message = chatMessage(id: "message-1", seq: 1, text: "hello")
        let rows = ChatTimeline.rows(
            messages: [message],
            profiles: [
                AppProfileSummary(
                    accountId: "alice-account",
                    npub: "npub1alice",
                    displayName: "Alice",
                    about: nil,
                    picture: "https://example.invalid/alice.png",
                    stale: false,
                    isAgent: false
                ),
            ]
        )

        guard case .messageGroup(let group) = rows.first else {
            XCTFail("expected message group")
            return
        }
        XCTAssertEqual(group.senderPictureURL, "https://example.invalid/alice.png")
    }

    func testRoomProjectionIncludesActivityOnlyRoom() {
        let projections = ChatTimeline.roomProjections(
            messages: [],
            typingMembers: [
                appTypingMember(
                    roomID: "room-empty",
                    deviceID: "hermes-agent",
                    displayName: "Hermes",
                    activityKind: "working"
                ),
            ]
        )

        let rows = projections["room-empty"]?.rows
        XCTAssertEqual(rows?.count, 1)
        guard case .activity(let activity) = rows?.first else {
            XCTFail("expected activity marker")
            return
        }
        XCTAssertEqual(activity.kind, .working)
        XCTAssertEqual(activity.label, "Hermes is working")
        XCTAssertNil(activity.primaryPictureURL)
        XCTAssertTrue(projections["room-empty"]?.messages.isEmpty ?? false)
    }

    func testStrongestActivityKindWinsForSameRoom() {
        let rows = ChatTimeline.rows(
            messages: [],
            typingMembers: [
                appTypingMember(
                    roomID: "room-main",
                    deviceID: "alice-ios",
                    displayName: "Alice",
                    activityKind: "typing"
                ),
                appTypingMember(
                    roomID: "room-main",
                    deviceID: "hermes-agent",
                    displayName: "Hermes",
                    activityKind: "working"
                ),
            ]
        )

        XCTAssertEqual(rows.count, 1)
        guard case .activity(let activity) = rows.first else {
            XCTFail("expected activity marker")
            return
        }
        XCTAssertEqual(activity.kind, .working)
        XCTAssertEqual(activity.label, "Hermes is working")
    }

    func testActivityUsesMemberPicture() {
        let rows = ChatTimeline.rows(
            messages: [],
            typingMembers: [
                appTypingMember(
                    roomID: "room-main",
                    deviceID: "hermes-agent",
                    displayName: "Hermes",
                    picture: "https://example.invalid/agent.png",
                    activityKind: "working"
                ),
            ]
        )

        guard case .activity(let activity) = rows.first else {
            XCTFail("expected activity marker")
            return
        }
        XCTAssertEqual(activity.primaryPictureURL, "https://example.invalid/agent.png")
    }


    private func appTypingMember(
        roomID: String,
        accountID: String = "alice-account",
        deviceID: String,
        displayName: String,
        picture: String? = nil,
        activityKind: String
    ) -> AppTypingMember {
        AppTypingMember(
            roomId: roomID,
            accountId: accountID,
            deviceId: deviceID,
            displayName: displayName,
            picture: picture,
            npub: nil,
            activityKind: activityKind
        )
    }

    private func chatMessage(id: String, seq: UInt64, text: String) -> ChatMessage {
        ChatMessage(
            roomId: "room-main",
            seq: seq,
            messageId: id,
            conversationId: nil,
            senderAccountId: "alice-account",
            senderDeviceId: "alice-ios",
            senderDisplayName: "Alice",
            senderNpub: nil,
            text: text,
            displayContent: text,
            richTextJson: "",
            payload: Data(text.utf8),
            replyToMessageId: nil,
            isMine: false,
            outboundDelivery: nil,
            reactions: [],
            media: [],
            readReceipt: nil,
            poll: nil,
            timestampUnixSeconds: 1_700_000_000 + seq,
            displayTimestamp: "now"
        )
    }
}

@MainActor
final class AppModelPersistenceTests: XCTestCase {
    func testMyProfileUsesSignedInProfileNotActiveScannedProfile() async throws {
        var state = savedChatState()
        let ownProfile = AppProfileSummary(
            accountId: state.identity.accountId,
            npub: "npub1paul",
            displayName: "Paul",
            about: nil,
            picture: "https://example.invalid/paul.png",
            stale: false,
            isAgent: false
        )
        let scannedProfile = AppProfileSummary(
            accountId: "bob-account",
            npub: "npub1bob",
            displayName: "Bob",
            about: nil,
            picture: "https://example.invalid/bob.png",
            stale: false,
            isAgent: false
        )
        state.profiles = [scannedProfile, ownProfile]
        state.activeProfileId = scannedProfile.accountId
        let runtime = FakeFiniteChatRuntime(
            initialState: state,
            startRuntimeState: state
        )
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()
        try await waitForActions(runtime, [.startRuntime])

        XCTAssertEqual(model.activeProfile?.displayName, "Bob")
        XCTAssertEqual(model.myProfile?.displayName, "Paul")
        XCTAssertEqual(model.myProfile?.picture, "https://example.invalid/paul.png")
    }

    func testMyProfileHydratesFromSignedInNostrMetadata() async throws {
        let material = try createNostrIdentity()
        var state = emptyChatState(deviceID: "qt433")
        state.identity = Identity(
            accountId: material.accountId,
            deviceId: "qt433",
            accountSecretHex: material.accountSecretHex
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: state,
            startRuntimeState: state
        )
        let relayService = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            discoveryRelays: [],
            eventLoader: { _, filter, _, _ in
                if filter.kinds == [0],
                   filter.authors == [material.accountId]
                {
                    return [
                        NostrRelayEvent(
                            pubkey: material.accountId,
                            createdAt: 1_800_000_100,
                            kind: 0,
                            tags: [],
                            content: #"{"name":"paul","display_name":"Relay Paul","about":"from nostr","picture":"https://example.com/relay-paul.jpg"}"#
                        ),
                    ]
                }
                return []
            }
        )
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "qt433"
            ),
            applicationSupportURL: try temporarySupportURL(),
            requiresNostrLogin: true,
            nostrIdentityStore: MemoryNostrIdentityStore(identity: AppNostrIdentity(material: material)),
            nostrProfileService: relayService,
            nostrPeopleCache: NostrPeopleCache(directory: try temporarySupportURL()),
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()
        try await waitForActions(runtime, [.startRuntime])
        try await waitUntil {
            model.myProfile?.displayName == "Relay Paul"
        }

        XCTAssertEqual(model.myProfile?.about, "from nostr")
        XCTAssertEqual(model.myProfile?.picture, "https://example.com/relay-paul.jpg")
    }

    func testSavedMyProfileWinsOverRelayedNostrMetadata() async throws {
        let material = try createNostrIdentity()
        var state = emptyChatState(deviceID: "qt433")
        state.identity = Identity(
            accountId: material.accountId,
            deviceId: "qt433",
            accountSecretHex: material.accountSecretHex
        )
        state.profiles = [
            AppProfileSummary(
                accountId: material.accountId,
                npub: material.npub,
                displayName: "Finite Paul",
                about: nil,
                picture: nil,
                stale: false,
                isAgent: false
            ),
        ]
        let runtime = FakeFiniteChatRuntime(
            initialState: state,
            startRuntimeState: state
        )
        let relayService = NostrRelayProfileService(
            relays: ["wss://relay.example"],
            discoveryRelays: [],
            eventLoader: { _, filter, _, _ in
                if filter.kinds == [0],
                   filter.authors == [material.accountId]
                {
                    return [
                        NostrRelayEvent(
                            pubkey: material.accountId,
                            createdAt: 1_800_000_100,
                            kind: 0,
                            tags: [],
                            content: #"{"display_name":"Relay Paul","picture":"https://example.com/relay-paul.jpg"}"#
                        ),
                    ]
                }
                return []
            }
        )
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "qt433"
            ),
            applicationSupportURL: try temporarySupportURL(),
            requiresNostrLogin: true,
            nostrIdentityStore: MemoryNostrIdentityStore(identity: AppNostrIdentity(material: material)),
            nostrProfileService: relayService,
            nostrPeopleCache: NostrPeopleCache(directory: try temporarySupportURL()),
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()
        try await waitForActions(runtime, [.startRuntime])
        try await waitUntil {
            model.relayedMyProfile?.displayName == "Relay Paul"
        }

        XCTAssertEqual(model.myProfile?.displayName, "Finite Paul")
        XCTAssertNil(model.myProfile?.picture)
    }

    func testForceCloseStyleRelaunchUsesSameStableStoreAndKeepsSavedProjection() async throws {
        let supportURL = try temporarySupportURL()
        let configURL = supportURL.appendingPathComponent("finitechat_config.json")
        let config = RuntimeConfig(
            serverURL: "http://192.168.1.226:8789",
            deviceID: "qt433"
        )
        try config.save(storageURL: configURL)

        let savedState = savedChatState()
        let offlineState = savedChatState(
            status: "offline",
            toast: "Showing saved chats. Connection will retry."
        )
        var openedOptions: [OpenOptions] = []

        let firstRuntime = FakeFiniteChatRuntime(
            initialState: savedState,
            startRuntimeState: offlineState
        )
        let firstLaunch = AppModel(
            config: config,
            applicationSupportURL: supportURL,
            configStorageURL: configURL,
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { options in
            openedOptions.append(options)
            return firstRuntime
        }

        firstLaunch.start()
        try await waitForActions(firstRuntime, [.startRuntime])

        XCTAssertEqual(firstLaunch.rooms.map(\.roomId), ["room-main"])
        XCTAssertEqual(firstLaunch.selectedRoom?.roomId, "room-main")
        XCTAssertEqual(firstLaunch.selectedRoomMessages.map(\.text), ["saved before force close"])
        XCTAssertEqual(firstLaunch.chatProjections["room-main"]?.messages.map(\.text), [
            "saved before force close",
        ])
        XCTAssertEqual(firstLaunch.runtimeStorePath, openedOptions[0].dataDir)
        XCTAssertEqual(firstLaunch.state?.status, "offline")
        XCTAssertNil(firstLaunch.userNoticeText)
        XCTAssertEqual(firstLaunch.developerRuntimeStatus, "offline")
        XCTAssertEqual(firstRuntime.dispatchedActions, [.startRuntime])

        let relaunchConfig = RuntimeConfig.load(
            environment: [:],
            args: ["FiniteChat"],
            storageURL: configURL
        )
        let secondRuntime = FakeFiniteChatRuntime(
            initialState: savedState,
            startRuntimeState: offlineState
        )
        let relaunch = AppModel(
            config: relaunchConfig,
            applicationSupportURL: supportURL,
            configStorageURL: configURL,
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { options in
            openedOptions.append(options)
            return secondRuntime
        }

        relaunch.start()
        try await waitForActions(secondRuntime, [.startRuntime])

        XCTAssertEqual(openedOptions.count, 2)
        XCTAssertEqual(openedOptions[0].serverUrl, "http://192.168.1.226:8789")
        XCTAssertEqual(openedOptions[1].serverUrl, "https://chat.finite.computer")
        XCTAssertEqual(openedOptions[0].deviceId, "qt433")
        XCTAssertEqual(openedOptions[1].deviceId, "qt433")
        XCTAssertEqual(openedOptions[0].dataDir, openedOptions[1].dataDir)
        XCTAssertEqual(
            URL(fileURLWithPath: openedOptions[1].dataDir).lastPathComponent,
            "FiniteChatStore"
        )
        XCTAssertEqual(relaunch.runtimeStorePath, openedOptions[1].dataDir)
        XCTAssertEqual(relaunch.rooms.map(\.roomId), ["room-main"])
        XCTAssertEqual(relaunch.selectedRoomMessages.map(\.text), ["saved before force close"])
        XCTAssertEqual(relaunch.state?.status, "offline")
        XCTAssertEqual(
            relaunch.state?.toast,
            "Showing saved chats. Connection will retry."
        )
        XCTAssertNil(relaunch.userNoticeText)
        XCTAssertEqual(relaunch.developerRuntimeStatus, "offline")
        XCTAssertEqual(
            relaunch.developerPersistenceSummary,
            "1 room(s), selected room-main, 1 selected message(s), 1 projected message(s)"
        )
        XCTAssertNil(relaunch.errorText)
        XCTAssertEqual(secondRuntime.dispatchedActions, [.startRuntime])
    }

    func testForegroundStartRetriesRuntimeAndRefreshesVisibleState() async throws {
        let supportURL = try temporarySupportURL()
        let savedState = savedChatState(
            status: "offline",
            toast: "Showing saved chats. Connection will retry."
        )
        let readyState = savedChatState(status: "ready", toast: nil)
        let runtime = FakeFiniteChatRuntime(
            initialState: savedState,
            startRuntimeStates: [savedState, readyState]
        )
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            applicationSupportURL: supportURL,
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()
        XCTAssertEqual(model.state?.status, "offline")
        model.startFromForeground()

        try await waitUntil {
            model.state?.status == "ready"
                && runtime.dispatchedActions == [.startRuntime, .startRuntime]
        }
        XCTAssertNil(model.developerErrorText)
        XCTAssertNil(model.userNoticeText)
    }

    func testForegroundStartRunsLaunchAutomationAfterRuntimeReady() async throws {
        let supportURL = try temporarySupportURL()
        let readyState = savedChatState(status: "ready", toast: nil)
        let runtime = FakeFiniteChatRuntime(
            initialState: readyState,
            startRuntimeState: readyState
        )
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            applicationSupportURL: supportURL,
            args: [
                "FiniteChat",
                "--finitechat-auto-send",
                "foreground launch automation message",
            ],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.startFromForeground()

        try await waitUntil {
            runtime.dispatchedActions.contains {
                if case .sendMessage = $0 { return true }
                return false
            }
        }
        XCTAssertEqual(
            Array(runtime.dispatchedActions.prefix(2)),
            [
                .startRuntime,
                .sendMessage(
                    roomId: "room-main",
                    text: "foreground launch automation message"
                ),
            ]
        )
        XCTAssertNil(model.developerErrorText)
    }

    func testProductHarnessDeliveredTranscriptPresentationHasNoNormalOfflineBanner() throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let transcriptState = productHarnessDeliveredTranscriptState()
        let runtime = FakeFiniteChatRuntime(
            initialState: transcriptState,
            startRuntimeState: transcriptState
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()

        let notice = NoticeBarPresentation(text: model.userNoticeText)
        XCTAssertNil(model.userNoticeText)
        XCTAssertNil(notice.visibleText)
        XCTAssertFalse(
            notice.visibleText?.localizedCaseInsensitiveContains("offline") ?? false
        )
        XCTAssertFalse(
            notice.visibleText?.localizedCaseInsensitiveContains("reconnecting") ?? false
        )

        let projection = try XCTUnwrap(model.chatProjections["room-main"])
        XCTAssertEqual(projection.rows.count, 1)
        XCTAssertEqual(
            projection.messages.map(\.text),
            ["online product harness message", "offline product harness message"]
        )
        XCTAssertEqual(model.selectedRoomMessages, projection.messages)

        let descriptors = projection.messages.map(ChatMessageBubbleAccessibilityDescriptor.init)
        XCTAssertEqual(
            descriptors.map(\.label),
            [
                "online product harness message, 2:32 PM, Delivered",
                "offline product harness message, 2:32 PM, Delivered",
            ]
        )
        XCTAssertEqual(descriptors.map(\.value), ["two checks", "two checks"])
        XCTAssertEqual(
            descriptors.map(\.identifier),
            ["ChatMessageBubble-message-online", "ChatMessageBubble-message-offline"]
        )
    }

    func testRawRuntimeDiagnosticsStayOutOfNormalChatSurfaces() throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            throw RawDiagnosticError(
                description: "HTTP runtime transport failed: server returned 404 Not Found"
            )
        }

        model.start()

        XCTAssertNil(model.userNoticeText)
        XCTAssertEqual(model.roomListEmptyDescription, "Open Settings to check connection.")
        XCTAssertEqual(
            model.developerErrorText,
            "HTTP runtime transport failed: server returned 404 Not Found"
        )
    }

    func testDeveloperDiagnosticsExportRedactsTransportDetails() throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let longHex = String(repeating: "a", count: 64)
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            throw RawDiagnosticError(
                description: "HTTP failed at http://127.0.0.1:8787 with /Users/alice/private-store/\(longHex)"
            )
        }

        model.start()

        let export = model.developerDiagnosticsExport
        XCTAssertNil(model.userNoticeText)
        XCTAssertTrue(export.contains("event=open.failed"))
        XCTAssertTrue(export.contains("[url]"))
        XCTAssertTrue(export.contains("[path]"))
        XCTAssertFalse(export.contains("http://127.0.0.1:8787"))
        XCTAssertFalse(export.contains("/Users/alice"))
        XCTAssertFalse(export.contains(longHex))
    }

    func testDeveloperDiagnosticsDoNotIncludeMessageTextAndStayBounded() throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: savedChatState(),
            startRuntimeState: savedChatState()
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()
        for index in 0..<240 {
            model.outboundText = "secret message body \(index)"
            XCTAssertTrue(model.send())
        }

        let export = model.developerDiagnosticsExport
        XCTAssertLessThanOrEqual(model.developerDiagnostics.count, 200)
        XCTAssertTrue(export.contains("event=send_message.requested"))
        XCTAssertFalse(export.contains("secret message body"))
        XCTAssertFalse(export.contains("saved before force close"))
    }

    func testInjectedApplicationSupportKeepsRuntimeIdentityConfigLocal() throws {
        let supportURL = try temporarySupportURL()
        let localConfigURL = supportURL.appendingPathComponent("finitechat_config.json")
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: savedChatState(),
            startRuntimeState: savedChatState()
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: supportURL,
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()

        XCTAssertEqual(try persistedConfig(at: localConfigURL).serverURL, "http://127.0.0.1:1")
        XCTAssertEqual(try persistedConfig(at: localConfigURL).deviceID, "qt433")
    }

    func testProductHarnessLaunchArgumentUsesExplicitSupportForConfigAndStore() throws {
        let harnessRoot = try temporarySupportURL()
            .appendingPathComponent(".state", isDirectory: true)
            .appendingPathComponent("product-harness", isDirectory: true)
            .appendingPathComponent("ios", isDirectory: true)
            .appendingPathComponent("text-offline", isDirectory: true)
            .appendingPathComponent("simulator-a", isDirectory: true)
        try FileManager.default.createDirectory(
            at: harnessRoot,
            withIntermediateDirectories: true
        )
        try RuntimeConfig(
            serverURL: "http://127.0.0.1:8787",
            deviceID: "simulator-a"
        ).save(storageURL: harnessRoot.appendingPathComponent("finitechat_config.json"))

        var openedOptions: [OpenOptions] = []
        let runtime = FakeFiniteChatRuntime(
            initialState: emptyChatState(deviceID: "simulator-a"),
            startRuntimeState: emptyChatState(deviceID: "simulator-a")
        )
        let model = AppModel(
            args: [
                "FiniteChat",
                "--finitechat-product-harness-root",
                harnessRoot.path,
                "--finitechat-server",
                "http://127.0.0.1:8787",
                "--finitechat-device",
                "simulator-a",
            ],
            startsUpdateLoop: false
        ) { options in
            openedOptions.append(options)
            return runtime
        }

        model.start()

        XCTAssertEqual(openedOptions.count, 1)
        XCTAssertEqual(openedOptions[0].serverUrl, "http://127.0.0.1:8787")
        XCTAssertEqual(openedOptions[0].deviceId, "simulator-a")
        XCTAssertEqual(
            URL(fileURLWithPath: openedOptions[0].dataDir),
            harnessRoot.appendingPathComponent("FiniteChatStore", isDirectory: true)
        )
        XCTAssertEqual(model.runtimeStorePath, openedOptions[0].dataDir)
        XCTAssertNil(model.developerErrorText)
    }

    func testInvalidProductHarnessLaunchArgumentDoesNotOpenDefaultStore() throws {
        var openedOptions: [OpenOptions] = []
        let model = AppModel(
            args: [
                "FiniteChat",
                "--finitechat-product-harness-root",
                "relative/path",
            ],
            startsUpdateLoop: false
        ) { options in
            openedOptions.append(options)
            return FakeFiniteChatRuntime(
                initialState: self.savedChatState(),
                startRuntimeState: self.savedChatState()
            )
        }

        model.start()

        XCTAssertTrue(openedOptions.isEmpty)
        XCTAssertEqual(
            model.developerErrorText,
            "--finitechat-product-harness-root must be an absolute path"
        )
        XCTAssertNil(model.runtimeStorePath)
    }

    func testNostrNsecSignInOpensRuntimeWithAccountSecret() async throws {
        let material = try createNostrIdentity()
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        var state = emptyChatState()
        state.identity = Identity(
            accountId: material.accountId,
            deviceId: "qt433",
            accountSecretHex: material.accountSecretHex
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: state,
            startRuntimeState: state
        )
        let identityStore = MemoryNostrIdentityStore()
        var openedOptions: [OpenOptions] = []
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            requiresNostrLogin: true,
            nostrIdentityStore: identityStore,
            startsUpdateLoop: false
        ) { options in
            openedOptions.append(options)
            return runtime
        }

        XCTAssertTrue(model.requiresNostrLogin)

        XCTAssertTrue(model.signInWithNsec(material.nsec))

        XCTAssertFalse(model.requiresNostrLogin)
        XCTAssertEqual(model.nostrIdentity?.accountID, material.accountId)
        XCTAssertEqual(model.activeAccountID, material.accountId)
        XCTAssertEqual(model.nostrIdentity?.npub, material.npub)
        XCTAssertEqual(identityStore.load()?.nsec, material.nsec)
        XCTAssertEqual(openedOptions.count, 1)
        XCTAssertEqual(openedOptions[0].accountSecretHex, material.accountSecretHex)
        try await waitForActions(runtime, [.startRuntime])
    }

    func testExistingStableRuntimeStoreAutoRecoversWithoutLoginPrompt() async throws {
        let material = try createNostrIdentity()
        let config = RuntimeConfig(
            serverURL: "https://chat.finite.computer",
            deviceID: "ios"
        )
        let supportURL = try temporarySupportURL()
        let storeURL = supportURL.appendingPathComponent("FiniteChatStore", isDirectory: true)
        try FileManager.default.createDirectory(
            at: storeURL,
            withIntermediateDirectories: true
        )
        try Data(material.accountSecretHex.utf8)
            .write(to: storeURL.appendingPathComponent("account-secret.hex"))
        try Data().write(to: storeURL.appendingPathComponent("client.sqlite3"))

        var state = emptyChatState(deviceID: "ios")
        state.identity = Identity(
            accountId: material.accountId,
            deviceId: "ios",
            accountSecretHex: material.accountSecretHex
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: state,
            startRuntimeState: state
        )
        let identityStore = MemoryNostrIdentityStore()
        var openedOptions: [OpenOptions] = []
        let model = AppModel(
            config: config,
            applicationSupportURL: supportURL,
            requiresNostrLogin: true,
            nostrIdentityStore: identityStore,
            startsUpdateLoop: false
        ) { options in
            openedOptions.append(options)
            return runtime
        }

        XCTAssertFalse(model.requiresNostrLogin)
        XCTAssertTrue(model.canRecoverRuntimeIdentity)

        model.start()

        XCTAssertFalse(model.requiresNostrLogin)
        XCTAssertFalse(model.canRecoverRuntimeIdentity)
        XCTAssertEqual(openedOptions.count, 1)
        XCTAssertNil(openedOptions[0].accountSecretHex)
        XCTAssertEqual(model.nostrIdentity?.accountID, material.accountId)
        XCTAssertEqual(model.activeAccountID, material.accountId)
        XCTAssertEqual(identityStore.load()?.nsec, material.nsec)
        try await waitForActions(runtime, [.startRuntime])
    }

    func testExplicitExistingDeviceAccountRecoveryRestoresKeychainIdentity() async throws {
        let material = try createNostrIdentity()
        let config = RuntimeConfig(
            serverURL: "https://chat.finite.computer",
            deviceID: "ios"
        )
        let supportURL = try temporarySupportURL()
        let storeURL = supportURL.appendingPathComponent("FiniteChatStore", isDirectory: true)
        try FileManager.default.createDirectory(
            at: storeURL,
            withIntermediateDirectories: true
        )
        try Data(material.accountSecretHex.utf8)
            .write(to: storeURL.appendingPathComponent("account-secret.hex"))
        try Data().write(to: storeURL.appendingPathComponent("client.sqlite3"))

        var state = emptyChatState(deviceID: "ios")
        state.identity = Identity(
            accountId: material.accountId,
            deviceId: "ios",
            accountSecretHex: material.accountSecretHex
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: state,
            startRuntimeState: state
        )
        let identityStore = MemoryNostrIdentityStore()
        var openedOptions: [OpenOptions] = []
        let model = AppModel(
            config: config,
            applicationSupportURL: supportURL,
            requiresNostrLogin: true,
            nostrIdentityStore: identityStore,
            startsUpdateLoop: false
        ) { options in
            openedOptions.append(options)
            return runtime
        }

        XCTAssertFalse(model.requiresNostrLogin)
        XCTAssertTrue(model.canRecoverRuntimeIdentity)

        XCTAssertTrue(model.recoverExistingDeviceAccount())

        XCTAssertFalse(model.requiresNostrLogin)
        XCTAssertFalse(model.canRecoverRuntimeIdentity)
        XCTAssertEqual(openedOptions.count, 1)
        XCTAssertNil(openedOptions[0].accountSecretHex)
        XCTAssertEqual(model.nostrIdentity?.accountID, material.accountId)
        XCTAssertEqual(model.activeAccountID, material.accountId)
        XCTAssertEqual(identityStore.load()?.nsec, material.nsec)
        try await waitForActions(runtime, [.startRuntime])
    }

    func testPushTokenReceivedBeforeNostrLoginRegistersAfterSignIn() async throws {
        let material = try createNostrIdentity()
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        var state = emptyChatState()
        state.identity = Identity(
            accountId: material.accountId,
            deviceId: "qt433",
            accountSecretHex: material.accountSecretHex
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: state,
            startRuntimeState: state
        )
        let identityStore = MemoryNostrIdentityStore()
        var openedOptions: [OpenOptions] = []
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            requiresNostrLogin: true,
            nostrIdentityStore: identityStore,
            startsUpdateLoop: false
        ) { options in
            openedOptions.append(options)
            return runtime
        }

        model.registerPushToken("  00010f10ff  ")

        XCTAssertTrue(openedOptions.isEmpty)
        XCTAssertTrue(runtime.dispatchedActions.isEmpty)

        XCTAssertTrue(model.signInWithNsec(material.nsec))

        XCTAssertEqual(openedOptions.count, 1)
        try await waitForActions(
            runtime,
            [.startRuntime, .setPushToken(token: "00010f10ff")]
        )
    }

    func testSignOutDeletesSavedNostrIdentityAndReturnsToLoginGate() throws {
        let material = try createNostrIdentity()
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        var state = emptyChatState()
        state.identity = Identity(
            accountId: material.accountId,
            deviceId: "qt433",
            accountSecretHex: material.accountSecretHex
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: state,
            startRuntimeState: state
        )
        let identityStore = MemoryNostrIdentityStore(identity: AppNostrIdentity(material: material))
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            requiresNostrLogin: true,
            nostrIdentityStore: identityStore,
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        XCTAssertFalse(model.requiresNostrLogin)
        model.start()
        XCTAssertNotNil(model.state)

        model.signOutAndDeleteEverything()

        XCTAssertTrue(model.requiresNostrLogin)
        XCTAssertNil(model.nostrIdentity)
        XCTAssertNil(identityStore.load())
        XCTAssertNil(model.state)
        XCTAssertNil(model.runtimeStorePath)
        XCTAssertEqual(model.serverURL, "https://chat.finite.computer")
        assertGeneratedDefaultDeviceID(model.deviceID)
    }

    func testSignOutRemovesPushTokenBeforeClearingRuntime() async throws {
        let material = try createNostrIdentity()
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        var state = emptyChatState()
        state.identity = Identity(
            accountId: material.accountId,
            deviceId: "qt433",
            accountSecretHex: material.accountSecretHex
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: state,
            startRuntimeState: state
        )
        let identityStore = MemoryNostrIdentityStore(identity: AppNostrIdentity(material: material))
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            requiresNostrLogin: true,
            nostrIdentityStore: identityStore,
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()
        model.registerPushToken("apns-token")
        try await waitForActions(
            runtime,
            [.startRuntime, .setPushToken(token: "apns-token")]
        )

        model.signOutAndDeleteEverything()

        try await waitForActions(
            runtime,
            [.startRuntime, .setPushToken(token: "apns-token"), .removePushToken]
        )
        XCTAssertNil(identityStore.load())
    }

    func testAttachmentCaptionOverrideDispatchesCaptionWithoutClearingComposerDraft() async throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: savedChatState(),
            startRuntimeState: savedChatState()
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }
        model.start()
        model.outboundText = "typed draft"

        let attachment = OutboundAttachment(
            filename: "voice_1725000123.m4a",
            mimeType: VoiceRecordingAttachment.mimeType,
            kind: .voiceNote,
            bytes: Data([0x00, 0x01])
        )
        model.sendAttachments(
            roomID: "room-main",
            attachments: [attachment],
            captionOverride: "  hello from transcript  "
        )

        try await waitUntil {
            runtime.dispatchedActions.count >= 2
        }

        guard case .sendAttachments(
            let roomID,
            let attachments,
            let caption,
            let replyToMessageID
        ) = runtime.dispatchedActions.last else {
            return XCTFail("expected sendAttachments action")
        }
        XCTAssertEqual(roomID, "room-main")
        XCTAssertEqual(attachments, [attachment])
        XCTAssertEqual(caption, "hello from transcript")
        XCTAssertNil(replyToMessageID)
        XCTAssertEqual(model.outboundText, "typed draft")
    }

    func testLaunchAutomationSendsSyntheticAttachmentThroughRustAction() async throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: savedChatState(),
            startRuntimeState: savedChatState()
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: [
                "FiniteChat",
                "--finitechat-auto-send-attachment-text",
                "offline attachment proof",
            ],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()

        try await waitUntil {
            runtime.dispatchedActions.count >= 2
        }
        guard case .sendAttachments(
            let roomID,
            let attachments,
            let caption,
            let replyToMessageID
        ) = runtime.dispatchedActions.last else {
            return XCTFail("expected launch automation sendAttachments action")
        }
        XCTAssertEqual(roomID, "room-main")
        XCTAssertEqual(caption, "")
        XCTAssertNil(replyToMessageID)
        XCTAssertEqual(attachments.count, 1)
        XCTAssertEqual(attachments[0].filename, "launch-automation.txt")
        XCTAssertEqual(attachments[0].mimeType, "text/plain")
        XCTAssertEqual(attachments[0].kind, .file)
        XCTAssertEqual(attachments[0].bytes, Data("offline attachment proof".utf8))
    }

    func testLaunchAutomationStartsProfileChatAndSendsThroughNewRoom() async throws {
        let bobAccountID = String(repeating: "b", count: 64)
        let bobNpub = try npubFromAccountId(accountId: bobAccountID)
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: emptyChatState(),
            startRuntimeState: emptyChatState()
        ) { action, currentState in
            var state = currentState
            switch action {
            case .startProfileChat(_, let displayName):
                let room = AppRoomSummary(
                    roomId: "room-bob",
                    displayName: displayName,
                    picture: nil,
                    state: .connected,
                    status: "connected",
                    userStatusText: "Connected",
                    lastMessagePreview: "",
                    unreadCount: 0,
                    canLoadOlder: false,
                    isAgentChat: false
                )
                state.rooms = [room]
                state.selectedRoomId = room.roomId
                state.status = "chat created"
            case .openRoom(let roomID):
                state.selectedRoomId = roomID
            case .sendMessage(let roomID, let text):
                state.messages.append(ChatMessage(
                    roomId: roomID,
                    seq: 1,
                    messageId: "message-bob",
                    conversationId: nil,
                    senderAccountId: "alice-account",
                    senderDeviceId: "qt433",
                    senderDisplayName: "qt433",
                    senderNpub: nil,
                    text: text,
                    displayContent: text,
                    richTextJson: "",
                    payload: Data(text.utf8),
                    replyToMessageId: nil,
                    isMine: true,
                    outboundDelivery: OutboundDelivery(
                        localSend: .sent,
                        serverDelivery: .delivered
                    ),
                    reactions: [],
                    media: [],
                    readReceipt: nil,
                    poll: nil,
                    timestampUnixSeconds: 1_700_000_000,
                    displayTimestamp: "now"
                ))
            default:
                break
            }
            return state
        }
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: [
                "FiniteChat",
                "--finitechat-auto-start-profile-chat-npub",
                bobNpub,
                "--finitechat-auto-send",
                "hello from profile automation",
            ],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()

        try await waitUntil {
            runtime.dispatchedActions.contains {
                if case .sendMessage = $0 { return true }
                return false
            }
        }
        XCTAssertEqual(
            runtime.dispatchedActions,
            [
                .startRuntime,
                .startProfileChat(
                    profile: AppProfileSummary(
                        accountId: bobAccountID,
                        npub: bobNpub,
                        displayName: shortenedDisplayNpub(bobNpub),
                        about: nil,
                        picture: nil,
                        stale: true,
                        isAgent: false
                    ),
                    displayName: "Chat with \(shortenedDisplayNpub(bobNpub))"
                ),
                .sendMessage(
                    roomId: "room-bob",
                    text: "hello from profile automation"
                ),
            ]
        )
        XCTAssertEqual(model.outboundText, "")
    }

    func testLaunchAutomationSendsAttachmentFileThroughRustAction() async throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: savedChatState(),
            startRuntimeState: savedChatState()
        )
        let supportURL = try temporarySupportURL()
        let imageURL = supportURL.appendingPathComponent("launch-image.png")
        let imageBytes = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        try imageBytes.write(to: imageURL)
        let model = AppModel(
            config: config,
            applicationSupportURL: supportURL,
            args: [
                "FiniteChat",
                "--finitechat-auto-send-attachment-file",
                imageURL.path,
                "--finitechat-auto-send-attachment-caption",
                "image caption",
            ],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()

        try await waitUntil {
            runtime.dispatchedActions.count >= 2
        }
        guard case .sendAttachments(
            let roomID,
            let attachments,
            let caption,
            let replyToMessageID
        ) = runtime.dispatchedActions.last else {
            return XCTFail("expected launch automation sendAttachments action")
        }
        XCTAssertEqual(roomID, "room-main")
        XCTAssertEqual(caption, "image caption")
        XCTAssertNil(replyToMessageID)
        XCTAssertEqual(attachments.count, 1)
        XCTAssertEqual(attachments[0].filename, "launch-image.png")
        XCTAssertEqual(attachments[0].mimeType, "image/png")
        XCTAssertEqual(attachments[0].kind, .image)
        XCTAssertEqual(attachments[0].bytes, imageBytes)
    }

    func testLaunchAutomationSendsBase64AttachmentThroughRustAction() async throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: savedChatState(),
            startRuntimeState: savedChatState()
        )
        let imageBytes = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: [
                "FiniteChat",
                "--finitechat-auto-send-attachment-base64",
                imageBytes.base64EncodedString(),
                "--finitechat-auto-send-attachment-filename",
                "launch-image.png",
                "--finitechat-auto-send-attachment-mime-type",
                "image/png",
                "--finitechat-auto-send-attachment-caption",
                "image caption",
            ],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()

        try await waitUntil {
            runtime.dispatchedActions.count >= 2
        }
        guard case .sendAttachments(
            let roomID,
            let attachments,
            let caption,
            let replyToMessageID
        ) = runtime.dispatchedActions.last else {
            return XCTFail("expected launch automation sendAttachments action")
        }
        XCTAssertEqual(roomID, "room-main")
        XCTAssertEqual(caption, "image caption")
        XCTAssertNil(replyToMessageID)
        XCTAssertEqual(attachments.count, 1)
        XCTAssertEqual(attachments[0].filename, "launch-image.png")
        XCTAssertEqual(attachments[0].mimeType, "image/png")
        XCTAssertEqual(attachments[0].kind, .image)
        XCTAssertEqual(attachments[0].bytes, imageBytes)
    }

    func testDownloadAttachmentDispatchesRustBeginBeforeBlockingDownload() async throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        var savedState = savedChatState()
        let attachment = ChatMediaAttachment(
            attachmentId: "attachment-1",
            url: "http://blob.example/sha256",
            mimeType: "image/jpeg",
            filename: "photo.jpg",
            kind: .image,
            width: nil,
            height: nil,
            localPath: nil,
            uploadProgressPerMille: nil,
            downloadProgressPerMille: nil
        )
        var message = savedState.messages[0]
        message.media = [attachment]
        savedState.messages = [message]
        let runtime = FakeFiniteChatRuntime(
            initialState: savedState,
            startRuntimeState: savedState
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }
        model.start()

        model.downloadAttachment(roomID: "room-main", message: message, attachment: attachment)

        try await waitUntil {
            runtime.dispatchedActions.count >= 3
        }

        guard case .beginDownloadAttachment(
            let beginRoomID,
            let beginMessageID,
            let beginAttachmentID
        ) = runtime.dispatchedActions[1] else {
            return XCTFail("expected beginDownloadAttachment before blocking download")
        }
        XCTAssertEqual(beginRoomID, "room-main")
        XCTAssertEqual(beginMessageID, "message-1")
        XCTAssertEqual(beginAttachmentID, "attachment-1")

        guard case .downloadAttachment(
            let downloadRoomID,
            let downloadMessageID,
            let downloadAttachmentID
        ) = runtime.dispatchedActions[2] else {
            return XCTFail("expected downloadAttachment after beginDownloadAttachment")
        }
        XCTAssertEqual(downloadRoomID, "room-main")
        XCTAssertEqual(downloadMessageID, "message-1")
        XCTAssertEqual(downloadAttachmentID, "attachment-1")
    }

    func testDownloadAttachmentDispatchesOnlyForExplicitCacheMissCandidate() async throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let savedState = savedChatState()
        let runtime = FakeFiniteChatRuntime(
            initialState: savedState,
            startRuntimeState: savedState
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }
        model.start()
        let message = savedState.messages[0]

        func attachment(
            url: String? = "http://blob.example/sha256",
            localPath: String? = nil,
            uploadProgressPerMille: UInt32? = nil,
            downloadProgressPerMille: UInt32? = nil
        ) -> ChatMediaAttachment {
            ChatMediaAttachment(
                attachmentId: UUID().uuidString,
                url: url,
                mimeType: "image/jpeg",
                filename: "photo.jpg",
                kind: .image,
                width: nil,
                height: nil,
                localPath: localPath,
                uploadProgressPerMille: uploadProgressPerMille,
                downloadProgressPerMille: downloadProgressPerMille
            )
        }

        model.downloadAttachment(
            roomID: "room-main",
            message: message,
            attachment: attachment(localPath: "/tmp/photo.jpg")
        )
        model.downloadAttachment(
            roomID: "room-main",
            message: message,
            attachment: attachment(url: nil)
        )
        model.downloadAttachment(
            roomID: "room-main",
            message: message,
            attachment: attachment(uploadProgressPerMille: 0)
        )
        model.downloadAttachment(
            roomID: "room-main",
            message: message,
            attachment: attachment(downloadProgressPerMille: 0)
        )

        try await waitForActions(runtime, [.startRuntime])
    }

    func testOfflineNoticeIsSuppressedWhenThereAreNoSavedChats() throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let offlineEmpty = emptyChatState(
            status: "offline",
            toast: "Showing saved chats. Connection will retry."
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: offlineEmpty,
            startRuntimeState: offlineEmpty
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()

        XCTAssertNil(model.userNoticeText)
        XCTAssertEqual(model.roomListEmptyDescription, "No chats yet")
        XCTAssertEqual(model.developerRuntimeStatus, "offline")
    }

    func testQueuedOfflineTextStateDoesNotRequestNormalSurfaceOfflineBanner() throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        var queuedState = savedChatState(
            status: "offline",
            toast: "Showing saved chats. Connection will retry."
        )
        queuedState.messages[0].outboundDelivery = OutboundDelivery(
            localSend: .sent,
            serverDelivery: .undelivered
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: queuedState,
            startRuntimeState: queuedState
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()
        XCTAssertNil(model.userNoticeText)
        let presentation = NoticeBarPresentation(text: model.userNoticeText)
        XCTAssertNil(presentation.visibleText)
        XCTAssertFalse(
            presentation.visibleText?.localizedCaseInsensitiveContains("offline") ?? false
        )
        XCTAssertFalse(
            presentation.visibleText?.localizedCaseInsensitiveContains("reconnecting") ?? false
        )
    }

    func testConnectedSavedRoomCanSendWhileRuntimeStatusIsOffline() async throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let offlineConnectedState = savedChatState(
            status: "offline",
            toast: "Showing saved chats. Connection will retry."
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: offlineConnectedState,
            startRuntimeState: offlineConnectedState
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()
        model.outboundText = "send through local membership"

        XCTAssertEqual(model.selectedRoom?.state, .connected)
        XCTAssertEqual(model.developerRuntimeStatus, "offline")
        XCTAssertNil(model.userNoticeText)
        XCTAssertTrue(model.canSend)
        XCTAssertTrue(model.send())
        XCTAssertEqual(model.outboundText, "")

        try await waitUntil {
            runtime.dispatchedActions.contains {
                if case .sendMessage = $0 { return true }
                return false
            }
        }
        guard case .sendMessage(
            let roomID,
            let text
        ) = runtime.dispatchedActions.last else {
            return XCTFail("expected sendMessage for connected local room")
        }
        XCTAssertEqual(roomID, "room-main")
        XCTAssertEqual(text, "send through local membership")
    }

    func testSendUsesExplicitRoomIDInsteadOfSelectedRoom() async throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        var state = savedChatState()
        state.rooms.append(AppRoomSummary(
            roomId: "room-secondary",
            displayName: "Secondary Room",
            picture: nil,
            state: .connected,
            status: "connected",
            userStatusText: "Connected",
            lastMessagePreview: "",
            unreadCount: 0,
            canLoadOlder: false,
            isAgentChat: false
        ))
        let runtime = FakeFiniteChatRuntime(
            initialState: state,
            startRuntimeState: state
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()
        XCTAssertEqual(model.selectedRoom?.roomId, "room-main")

        model.outboundText = "send to the visible thread only"
        XCTAssertTrue(model.send(roomID: "room-secondary"))

        try await waitUntil {
            runtime.dispatchedActions.contains {
                if case .sendMessage = $0 { return true }
                return false
            }
        }
        guard case .sendMessage(
            let roomID,
            let text
        ) = runtime.dispatchedActions.last else {
            return XCTFail("expected sendMessage for explicit room")
        }
        XCTAssertEqual(roomID, "room-secondary")
        XCTAssertEqual(text, "send to the visible thread only")
    }

    func testRuntimeDispatchesAreFifoAcrossStartupAndUserActions() async throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let startRuntimeEntered = expectation(description: "start runtime dispatch entered")
        let releaseStartRuntime = DispatchSemaphore(value: 0)
        let runtime = FakeFiniteChatRuntime(
            initialState: savedChatState(),
            startRuntimeState: savedChatState()
        )
        runtime.setDispatchStartHook { action in
            if action == .startRuntime {
                startRuntimeEntered.fulfill()
                _ = releaseStartRuntime.wait(timeout: .now() + 5)
            }
        }
        defer {
            runtime.setDispatchStartHook(nil)
            releaseStartRuntime.signal()
        }
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()
        await fulfillment(of: [startRuntimeEntered], timeout: 1)

        model.outboundText = "queued behind startup"
        XCTAssertTrue(model.send())
        try await Task.sleep(nanoseconds: 100_000_000)
        XCTAssertEqual(
            runtime.dispatchedActions,
            [],
            "user actions must not overtake the startup command"
        )

        runtime.setDispatchStartHook(nil)
        releaseStartRuntime.signal()
        try await waitUntil {
            runtime.dispatchedActions == [
                .startRuntime,
                .sendMessage(roomId: "room-main", text: "queued behind startup"),
            ]
        }
    }

    func testBackgroundSendDoesNotWaitForRuntimeDispatch() async throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let sendStarted = expectation(description: "send runtime dispatch started")
        let releaseSend = DispatchSemaphore(value: 0)
        let runtime = FakeFiniteChatRuntime(
            initialState: savedChatState(),
            startRuntimeState: savedChatState()
        ) { action, current in
            if case .sendMessage = action {
                sendStarted.fulfill()
                _ = releaseSend.wait(timeout: .now() + 5)
            }
            return current
        }
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()
        try await waitUntil {
            model.developerDiagnosticsExport.contains("event=start.succeeded")
        }
        let startedAt = Date()

        XCTAssertTrue(model.send(roomID: "room-main", text: "slow send"))
        XCTAssertLessThan(Date().timeIntervalSince(startedAt), 0.2)
        let optimistic = try XCTUnwrap(model.selectedRoomMessages.last)
        XCTAssertEqual(optimistic.text, "slow send")
        XCTAssertEqual(optimistic.outboundDelivery?.localSend, .sending)
        XCTAssertEqual(optimistic.outboundDelivery?.serverDelivery, .undelivered)

        await fulfillment(of: [sendStarted], timeout: 1)
        releaseSend.signal()

        try await waitUntil {
            runtime.dispatchedActions.contains(.sendMessage(roomId: "room-main", text: "slow send"))
        }
    }

    func testOpeningRoomMarksItRead() async throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        var state = savedChatState()
        state.rooms[0].unreadCount = 1
        let runtime = FakeFiniteChatRuntime(
            initialState: state,
            startRuntimeState: state
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()
        let room = try XCTUnwrap(model.rooms.first)
        model.openRoom(room)

        try await waitUntil {
            runtime.dispatchedActions.count >= 3
        }

        let actions = runtime.dispatchedActions
        XCTAssertTrue(actions.contains(.startRuntime))
        let openIndex = try XCTUnwrap(actions.firstIndex(of: .openRoom(roomId: "room-main")))
        let markReadIndex = try XCTUnwrap(actions.firstIndex(of: .markRoomRead(roomId: "room-main")))
        XCTAssertLessThan(openIndex, markReadIndex)
    }

    func testNoticeBarPresentationExposesStableAccessibilityOnlyWhenVisible() {
        let visible = NoticeBarPresentation(text: " Network problem ")
        XCTAssertEqual(visible.visibleText, "Network problem")
        XCTAssertEqual(visible.accessibilityIdentifier, "NoticeBar")

        let hidden = NoticeBarPresentation(text: "   ")
        XCTAssertNil(hidden.visibleText)
    }

    func testUnavailableSavedRoomKeepsCachedMessagesButCannotSend() async throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let unavailableState = savedChatState(
            roomState: .unavailableOnDevice,
            roomStatus: "room is not available on this device"
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: unavailableState,
            startRuntimeState: unavailableState
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()
        model.outboundText = "this should not send yet"
        let selectedRoom = try XCTUnwrap(model.selectedRoom)

        XCTAssertEqual(model.rooms.map(\.roomId), ["room-main"])
        XCTAssertEqual(selectedRoom.state, .unavailableOnDevice)
        XCTAssertEqual(selectedRoom.status, "room is not available on this device")
        XCTAssertEqual(selectedRoom.userStatusText, "Unavailable on this device")
        XCTAssertEqual(model.selectedRoomMessages.map(\.text), ["saved before force close"])
        XCTAssertEqual(model.chatProjections["room-main"]?.messages.map(\.text), [
            "saved before force close",
        ])
        XCTAssertFalse(model.canSend)
        XCTAssertFalse(model.send())
        XCTAssertFalse(model.createInvite(for: selectedRoom))
        XCTAssertFalse(model.sendPoll(
            roomID: selectedRoom.roomId,
            question: "Should not leave Swift?",
            options: ["Yes", "No"]
        ))
        XCTAssertFalse(model.sendAttachment(
            roomID: selectedRoom.roomId,
            fileURL: URL(fileURLWithPath: "/tmp/finitechat-unavailable.txt")
        ))
        XCTAssertFalse(model.sendAttachments(
            roomID: selectedRoom.roomId,
            attachments: [
                OutboundAttachment(
                    filename: "blocked.txt",
                    mimeType: "text/plain",
                    kind: .file,
                    bytes: Data("blocked".utf8)
                ),
            ],
            captionOverride: "blocked"
        ))
        XCTAssertEqual(model.outboundText, "this should not send yet")
        try await waitForActions(runtime, [.startRuntime])
        XCTAssertNil(model.errorText)
    }

    func testScanningExistingInviteRoomSurfacesWhereUserLanded() async throws {
        let config = RuntimeConfig(
            serverURL: "https://chat.finite.computer",
            deviceID: "qt433"
        )
        let readyState = savedChatState()
        let scannedState = savedChatState(
            status: "join requested",
            roomState: .waitingForApproval,
            roomStatus: "waiting for room admission",
            flow: AppFlowState(
                noticeText: "Access requested for Main Room. Waiting for the agent to approve this device.",
                noticeBusy: false,
                scanInFlight: false,
                inviteJoinSubmissionRoomId: nil,
                scanResult: .room,
                imageUploadUrl: nil
            )
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: readyState,
            startRuntimeState: readyState
        ) { action, current in
            if case .scanTarget = action {
                return scannedState
            }
            return current
        }
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }

        model.start()
        model.scanDraft = "finite://join?v=1&s=https%3A%2F%2Fchat.finite.computer&r=room-main&i=invite-1&t=token&a=npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqgcpfl3"

        var scanResult: AppScanTargetResult?
        XCTAssertTrue(model.scanTarget { result in
            scanResult = result
        })

        try await waitUntil {
            scanResult != nil
        }

        guard case .room(let room) = scanResult else {
            return XCTFail("scan should resolve to the selected room")
        }

        XCTAssertEqual(room.roomId, "room-main")
        XCTAssertEqual(model.state?.selectedRoomId, "room-main")
        XCTAssertEqual(model.scanDraft, "")
        XCTAssertEqual(
            model.userNoticeText,
            "Access requested for Main Room. Waiting for the agent to approve this device."
        )
    }

    func testPendingRoomPresentationHidesLowLevelWelcomeErrors() {
        let room = AppRoomSummary(
            roomId: "room-main",
            displayName: "Main Room",
            picture: nil,
            state: .waitingForApproval,
            status: "client error: this device has no accepted Welcome for room 'room-main'",
            userStatusText: "Waiting for approval",
            lastMessagePreview: "",
            unreadCount: 0,
            canLoadOlder: false,
            isAgentChat: false
        )

        XCTAssertNil(PendingRoomPresentation(room: room).detailText)
    }

    func testRetryMessageDispatchesOnlyForFailedLocalOutbound() async throws {
        let config = RuntimeConfig(
            serverURL: "http://127.0.0.1:1",
            deviceID: "qt433"
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: savedChatState(),
            startRuntimeState: savedChatState()
        )
        let model = AppModel(
            config: config,
            applicationSupportURL: try temporarySupportURL(),
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { _ in
            runtime
        }
        model.start()
        try await waitForActions(runtime, [.startRuntime])

        func message(
            _ id: String,
            isMine: Bool = true,
            outboundDelivery: OutboundDelivery?
        ) -> ChatMessage {
            ChatMessage(
                roomId: "room-main",
                seq: 1,
                messageId: id,
                conversationId: nil,
                senderAccountId: isMine ? "alice-account" : "bob-account",
                senderDeviceId: isMine ? "qt433" : "bob-ios",
                senderDisplayName: isMine ? "qt433" : "Bob",
                senderNpub: nil,
                text: "retry candidate",
                displayContent: "retry candidate",
                richTextJson: "",
                payload: Data("retry candidate".utf8),
                replyToMessageId: nil,
                isMine: isMine,
                outboundDelivery: outboundDelivery,
                reactions: [],
                media: [],
                readReceipt: nil,
                poll: nil,
                timestampUnixSeconds: 1_700_000_000,
                displayTimestamp: "now"
            )
        }

        XCTAssertFalse(model.retry(message(
            "message-inbound-failed",
            isMine: false,
            outboundDelivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .failed(reason: "malformed nonlocal state")
            )
        )))
        XCTAssertFalse(model.retry(message(
            "message-undelivered",
            outboundDelivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .undelivered
            )
        )))
        XCTAssertFalse(model.retry(message(
            "message-delivered",
            outboundDelivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .delivered
            )
        )))
        XCTAssertEqual(runtime.dispatchedActions, [.startRuntime])

        XCTAssertTrue(model.retry(message(
            "message-failed",
            outboundDelivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .failed(reason: "server rejected send")
            )
        )))
        try await waitUntil {
            runtime.dispatchedActions.count >= 2
        }
        guard case .retryMessage(let roomID, let messageID) = runtime.dispatchedActions.last else {
            return XCTFail("expected retryMessage action")
        }
        XCTAssertEqual(roomID, "room-main")
        XCTAssertEqual(messageID, "message-failed")
    }

    func testExplicitTransientDiagnosticLaunchKeepsStableRelaunchOnSavedIdentity() throws {
        let supportURL = try temporarySupportURL()
        let configURL = supportURL.appendingPathComponent("finitechat_config.json")
        try RuntimeConfig(
            serverURL: "https://persisted.example",
            deviceID: "qt433"
        ).save(storageURL: configURL)

        let diagnosticArgs = [
            "FiniteChat",
            "--finitechat-server",
            "http://127.0.0.1:1",
            "--finitechat-device",
            "diagnostics-visual",
            "--finitechat-transient-config",
        ]
        let diagnosticConfig = RuntimeConfig.load(
            environment: [:],
            args: diagnosticArgs,
            storageURL: configURL
        )
        var openedOptions: [OpenOptions] = []
        let diagnosticRuntime = FakeFiniteChatRuntime(
            initialState: emptyChatState(deviceID: "diagnostics-visual"),
            startRuntimeState: emptyChatState(deviceID: "diagnostics-visual")
        )
        let diagnosticLaunch = AppModel(
            config: diagnosticConfig,
            applicationSupportURL: supportURL,
            configStorageURL: configURL,
            args: diagnosticArgs,
            startsUpdateLoop: false
        ) { options in
            openedOptions.append(options)
            return diagnosticRuntime
        }

        diagnosticLaunch.start()

        XCTAssertEqual(openedOptions.count, 1)
        XCTAssertEqual(openedOptions[0].serverUrl, "http://127.0.0.1:1")
        XCTAssertEqual(openedOptions[0].deviceId, "diagnostics-visual")
        let diagnosticStore = URL(fileURLWithPath: openedOptions[0].dataDir)
        XCTAssertEqual(diagnosticStore.lastPathComponent, "diagnostics-visual")
        XCTAssertEqual(diagnosticStore.deletingLastPathComponent().lastPathComponent, "FiniteChatTransient")
        XCTAssertEqual(diagnosticLaunch.runtimeStorePath, openedOptions[0].dataDir)

        let relaunchConfig = RuntimeConfig.load(
            environment: [:],
            args: ["FiniteChat"],
            storageURL: configURL
        )
        let relaunchRuntime = FakeFiniteChatRuntime(
            initialState: savedChatState(),
            startRuntimeState: savedChatState()
        )
        let relaunch = AppModel(
            config: relaunchConfig,
            applicationSupportURL: supportURL,
            configStorageURL: configURL,
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { options in
            openedOptions.append(options)
            return relaunchRuntime
        }

        relaunch.start()

        XCTAssertEqual(openedOptions.count, 2)
        XCTAssertEqual(openedOptions[1].serverUrl, "https://persisted.example")
        XCTAssertEqual(openedOptions[1].deviceId, "qt433")
        XCTAssertEqual(
            URL(fileURLWithPath: openedOptions[1].dataDir).lastPathComponent,
            "FiniteChatStore"
        )
        XCTAssertEqual(relaunch.runtimeStorePath, openedOptions[1].dataDir)
        XCTAssertEqual(relaunch.selectedRoomMessages.map(\.text), ["saved before force close"])
    }

    func testStableLaunchOverrideDoesNotPersistResolvedChatIdentityWithoutExplicitPersist() throws {
        let supportURL = try temporarySupportURL()
        let configURL = supportURL.appendingPathComponent("finitechat_config.json")
        try RuntimeConfig(
            serverURL: "https://persisted.example",
            deviceID: "ios"
        ).save(storageURL: configURL)

        let launchArgs = [
            "FiniteChat",
            "--finitechat-server",
            "http://192.168.1.226:8789",
            "--finitechat-device",
            "qt433",
        ]
        let launchConfig = RuntimeConfig.load(
            environment: [:],
            args: launchArgs,
            storageURL: configURL
        )
        var openedOptions: [OpenOptions] = []
        let launchRuntime = FakeFiniteChatRuntime(
            initialState: savedChatState(),
            startRuntimeState: savedChatState()
        )
        let firstLaunch = AppModel(
            config: launchConfig,
            applicationSupportURL: supportURL,
            configStorageURL: configURL,
            args: launchArgs,
            startsUpdateLoop: false
        ) { options in
            openedOptions.append(options)
            return launchRuntime
        }

        firstLaunch.start()

        XCTAssertEqual(openedOptions.count, 1)
        XCTAssertEqual(openedOptions[0].serverUrl, "http://192.168.1.226:8789")
        XCTAssertEqual(openedOptions[0].deviceId, "qt433")
        XCTAssertEqual(
            URL(fileURLWithPath: openedOptions[0].dataDir).lastPathComponent,
            "FiniteChatStore"
        )
        XCTAssertEqual(firstLaunch.deviceID, "qt433")
        XCTAssertEqual(firstLaunch.selectedRoomMessages.map(\.text), ["saved before force close"])

        let persistedAfterLaunch = try persistedConfig(at: configURL)
        XCTAssertEqual(persistedAfterLaunch.serverURL, "https://persisted.example")
        XCTAssertEqual(persistedAfterLaunch.deviceID, "ios")

        let relaunchConfig = RuntimeConfig.load(
            environment: [:],
            args: ["FiniteChat"],
            storageURL: configURL
        )
        let relaunchRuntime = FakeFiniteChatRuntime(
            initialState: savedChatState(),
            startRuntimeState: savedChatState()
        )
        let relaunch = AppModel(
            config: relaunchConfig,
            applicationSupportURL: supportURL,
            configStorageURL: configURL,
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { options in
            openedOptions.append(options)
            return relaunchRuntime
        }

        relaunch.start()

        XCTAssertEqual(openedOptions.count, 2)
        XCTAssertEqual(openedOptions[1].serverUrl, "https://persisted.example")
        XCTAssertEqual(openedOptions[1].deviceId, "ios")
        XCTAssertEqual(
            URL(fileURLWithPath: openedOptions[1].dataDir).lastPathComponent,
            "FiniteChatStore"
        )
        XCTAssertEqual(relaunch.selectedRoomMessages.map(\.text), ["saved before force close"])

        let persistedAfterRelaunch = try persistedConfig(at: configURL)
        XCTAssertEqual(persistedAfterRelaunch.serverURL, "https://persisted.example")
        XCTAssertEqual(persistedAfterRelaunch.deviceID, "qt433")
    }

    func testUseDefaultServerRepairsPersistedDevelopmentServerWithoutDeletingState() throws {
        let supportURL = try temporarySupportURL()
        let configURL = supportURL.appendingPathComponent("finitechat_config.json")
        let launchConfig = RuntimeConfig(
            serverURL: "http://192.168.1.226:8789",
            deviceID: "ios"
        )
        try launchConfig.save(storageURL: configURL)

        var openedOptions: [OpenOptions] = []
        let model = AppModel(
            config: launchConfig,
            applicationSupportURL: supportURL,
            configStorageURL: configURL,
            args: ["FiniteChat"],
            startsUpdateLoop: false
        ) { options in
            openedOptions.append(options)
            return FakeFiniteChatRuntime(
                initialState: self.savedChatState(),
                startRuntimeState: self.savedChatState()
            )
        }

        model.start()
        model.useDefaultServer()

        XCTAssertEqual(openedOptions.count, 2)
        XCTAssertEqual(openedOptions[0].serverUrl, "http://192.168.1.226:8789")
        XCTAssertEqual(openedOptions[1].serverUrl, "https://chat.finite.computer")
        XCTAssertEqual(model.serverURL, "https://chat.finite.computer")
        XCTAssertEqual(model.selectedRoomMessages.map(\.text), ["saved before force close"])

        let persisted = try persistedConfig(at: configURL)
        XCTAssertEqual(persisted.serverURL, "https://chat.finite.computer")
        XCTAssertEqual(persisted.deviceID, "qt433")
    }

    @MainActor
    func testStartProfileChatDispatchesDirectProfileChatAction() async throws {
        let profile = AppProfileSummary(
            accountId: "bob-account",
            npub: "npub1bob",
            displayName: "Bob",
            about: nil,
            picture: nil,
            stale: false,
        isAgent: false
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: emptyChatState(),
            startRuntimeState: emptyChatState()
        ) { action, currentState in
            var state = currentState
            switch action {
            case .startProfileChat(_, let displayName):
                let room = AppRoomSummary(
                    roomId: "room-bob",
                    displayName: displayName,
                    picture: nil,
                    state: .connected,
                    status: "connected",
                    userStatusText: "Connected",
                    lastMessagePreview: "",
                    unreadCount: 0,
                    canLoadOlder: false,
                    isAgentChat: false
                )
                state.rooms = [room]
                state.selectedRoomId = room.roomId
                state.status = "chat created"
            default:
                break
            }
            return state
        }
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in runtime }

        model.start()

        XCTAssertTrue(model.startProfileChat(for: profile))
        let expectedActions: [AppAction] = [
            .startRuntime,
            .startProfileChat(profile: profile, displayName: "Chat with Bob"),
        ]
        try await waitUntil {
            runtime.dispatchedActions == expectedActions
                && model.selectedRoom?.roomId == "room-bob"
        }
        XCTAssertEqual(runtime.dispatchedActions, expectedActions)
        XCTAssertEqual(model.selectedRoom?.roomId, "room-bob")
        XCTAssertNil(model.state?.activeInvite)
    }

    @MainActor
    func testStartProfileChatTreatsOpenedExistingDirectRoomAsSuccess() async throws {
        let profile = AppProfileSummary(
            accountId: "bob-account",
            npub: "npub1bob",
            displayName: "Bob",
            about: nil,
            picture: nil,
            stale: false,
        isAgent: false
        )
        let existingRoom = AppRoomSummary(
            roomId: "room-bob",
            displayName: "Chat with Bob",
            picture: nil,
            state: .connected,
            status: "connected",
            userStatusText: "Connected",
            lastMessagePreview: "",
            unreadCount: 0,
            canLoadOlder: false,
            isAgentChat: false
        )
        var existingState = emptyChatState()
        existingState.rooms = [existingRoom]
        let runtime = FakeFiniteChatRuntime(
            initialState: existingState,
            startRuntimeState: existingState
        ) { action, currentState in
            var state = currentState
            if case .startProfileChat = action {
                state.selectedRoomId = "room-bob"
                state.status = "chat opened"
                state.toast = nil
            }
            return state
        }
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in runtime }

        model.start()

        XCTAssertTrue(model.startProfileChat(for: profile))
        let expectedActions: [AppAction] = [
            .startRuntime,
            .startProfileChat(profile: profile, displayName: "Chat with Bob"),
        ]
        try await waitUntil {
            runtime.dispatchedActions == expectedActions
                && model.selectedRoom?.roomId == "room-bob"
        }
        XCTAssertEqual(runtime.dispatchedActions, expectedActions)
        XCTAssertEqual(model.rooms.count, 1)
        XCTAssertEqual(model.selectedRoom?.roomId, "room-bob")
        XCTAssertNil(model.developerErrorText)
    }

    @MainActor
    func testStartProfileChatFailureKeepsNoticeVisibleWithoutRooms() async throws {
        let profile = AppProfileSummary(
            accountId: "bob-account",
            npub: "npub1bob",
            displayName: "Bob",
            about: nil,
            picture: nil,
            stale: false,
        isAgent: false
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: emptyChatState(),
            startRuntimeState: emptyChatState()
        ) { action, currentState in
            var state = currentState
            if case .startProfileChat = action {
                state.status = "chat unavailable"
                state.toast = "Ask them to open Finite Chat, then try again"
            }
            return state
        }
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in runtime }

        model.start()

        XCTAssertTrue(model.startProfileChat(for: profile))
        try await waitUntil {
            runtime.dispatchedActions == [
                .startRuntime,
                .startProfileChat(profile: profile, displayName: "Chat with Bob"),
            ]
                && model.userNoticeText == "Ask them to open Finite Chat, then try again"
        }
        XCTAssertTrue(model.rooms.isEmpty)
        XCTAssertEqual(
            model.userNoticeText,
            "Ask them to open Finite Chat, then try again"
        )
        XCTAssertEqual(model.actionNoticeText, model.userNoticeText)
    }

    @MainActor
    func testStartNewChatWithOneProfileStartsDirectChatWithoutRoomName() async throws {
        let bob = AppProfileSummary(
            accountId: "bob-account",
            npub: "npub1bob",
            displayName: "Bob",
            about: nil,
            picture: nil,
            stale: false,
        isAgent: false
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: emptyChatState(),
            startRuntimeState: emptyChatState()
        ) { action, currentState in
            var state = currentState
            if case .startProfileChat(_, let displayName) = action {
                let room = AppRoomSummary(
                    roomId: "room-bob",
                    displayName: displayName,
                    picture: nil,
                    state: .connected,
                    status: "connected",
                    userStatusText: "Connected",
                    lastMessagePreview: "",
                    unreadCount: 0,
                    canLoadOlder: false,
                    isAgentChat: false
                )
                state.rooms = [room]
                state.selectedRoomId = room.roomId
                state.status = "chat created"
            }
            return state
        }
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in runtime }

        model.start()

        XCTAssertTrue(model.startNewChat(named: "", with: [bob]))
        let expectedActions: [AppAction] = [
            .startRuntime,
            .startProfileChat(profile: bob, displayName: "Chat with Bob"),
        ]
        try await waitUntil {
            runtime.dispatchedActions == expectedActions
                && model.selectedRoom?.roomId == "room-bob"
        }
        XCTAssertEqual(
            runtime.dispatchedActions,
            [
                .startRuntime,
                .startProfileChat(profile: bob, displayName: "Chat with Bob"),
            ]
        )
        XCTAssertEqual(model.selectedRoom?.roomId, "room-bob")
    }

    func testStartGroupChatDispatchesBackedGroupAction() async throws {
        let bob = AppProfileSummary(
            accountId: "bob-account",
            npub: "npub1bob",
            displayName: "Bob",
            about: nil,
            picture: nil,
            stale: false,
        isAgent: false
        )
        let carol = AppProfileSummary(
            accountId: "carol-account",
            npub: "npub1carol",
            displayName: "Carol",
            about: nil,
            picture: nil,
            stale: false,
        isAgent: false
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: emptyChatState(),
            startRuntimeState: emptyChatState()
        ) { action, currentState in
            var state = currentState
            switch action {
            case .startGroupChat(_, let displayName):
                let room = AppRoomSummary(
                    roomId: "room-group",
                    displayName: displayName,
                    picture: nil,
                    state: .connected,
                    status: "connected",
                    userStatusText: "Connected",
                    lastMessagePreview: "",
                    unreadCount: 0,
                    canLoadOlder: false,
                    isAgentChat: false
                )
                state.rooms = [room]
                state.selectedRoomId = room.roomId
                state.status = "chat created"
            default:
                break
            }
            return state
        }
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in runtime }

        model.start()

        XCTAssertTrue(model.startNewChat(named: "Weekend plans", with: [bob, carol]))
        let expectedActions: [AppAction] = [
            .startRuntime,
            .startGroupChat(
                profiles: [bob, carol],
                displayName: "Weekend plans"
            ),
        ]
        try await waitUntil {
            runtime.dispatchedActions == expectedActions
                && model.selectedRoom?.roomId == "room-group"
        }
        XCTAssertEqual(runtime.dispatchedActions, expectedActions)
        XCTAssertEqual(model.selectedRoom?.roomId, "room-group")
    }

    func testStartNewChatWithMultipleProfilesStartsNamedGroup() async throws {
        let bob = AppProfileSummary(
            accountId: "bob-account",
            npub: "npub1bob",
            displayName: "Bob",
            about: nil,
            picture: nil,
            stale: false,
        isAgent: false
        )
        let carol = AppProfileSummary(
            accountId: "carol-account",
            npub: "npub1carol",
            displayName: "Carol",
            about: nil,
            picture: nil,
            stale: false,
        isAgent: false
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: emptyChatState(),
            startRuntimeState: emptyChatState()
        ) { action, currentState in
            var state = currentState
            if case .startGroupChat(_, let displayName) = action {
                let room = AppRoomSummary(
                    roomId: "room-group",
                    displayName: displayName,
                    picture: nil,
                    state: .connected,
                    status: "connected",
                    userStatusText: "Connected",
                    lastMessagePreview: "",
                    unreadCount: 0,
                    canLoadOlder: false,
                    isAgentChat: false
                )
                state.rooms = [room]
                state.selectedRoomId = room.roomId
                state.status = "chat created"
            }
            return state
        }
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in runtime }

        model.start()

        XCTAssertTrue(model.startNewChat(named: "Weekend plans", with: [bob, carol]))
        let expectedActions: [AppAction] = [
            .startRuntime,
            .startGroupChat(
                profiles: [bob, carol],
                displayName: "Weekend plans"
            ),
        ]
        try await waitUntil {
            runtime.dispatchedActions == expectedActions
                && model.selectedRoom?.roomId == "room-group"
        }
        XCTAssertEqual(runtime.dispatchedActions, expectedActions)
        XCTAssertEqual(model.selectedRoom?.roomId, "room-group")
    }

    func testStartGroupChatFailureKeepsNoticeVisibleWithoutRooms() async throws {
        let bob = AppProfileSummary(
            accountId: "bob-account",
            npub: "npub1bob",
            displayName: "Bob",
            about: nil,
            picture: nil,
            stale: false,
        isAgent: false
        )
        let carol = AppProfileSummary(
            accountId: "carol-account",
            npub: "npub1carol",
            displayName: "Carol",
            about: nil,
            picture: nil,
            stale: false,
        isAgent: false
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: emptyChatState(),
            startRuntimeState: emptyChatState()
        ) { action, currentState in
            var state = currentState
            if case .startGroupChat = action {
                state.status = "chat unavailable"
                state.toast = "Ask everyone to open Finite Chat, then try again"
            }
            return state
        }
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in runtime }

        model.start()

        XCTAssertTrue(model.startNewChat(named: "Weekend plans", with: [bob, carol]))
        try await waitUntil {
            runtime.dispatchedActions == [
                .startRuntime,
                .startGroupChat(
                    profiles: [bob, carol],
                    displayName: "Weekend plans"
                ),
            ]
                && model.userNoticeText == "Ask everyone to open Finite Chat, then try again"
        }
        XCTAssertTrue(model.rooms.isEmpty)
        XCTAssertEqual(
            model.userNoticeText,
            "Ask everyone to open Finite Chat, then try again"
        )
        XCTAssertEqual(model.actionNoticeText, model.userNoticeText)
    }

    func testAddRoomMembersDispatchesBackedMemberAction() async throws {
        let bob = AppProfileSummary(
            accountId: "bob-account",
            npub: "npub1bob",
            displayName: "Bob",
            about: nil,
            picture: nil,
            stale: false,
        isAgent: false
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: savedChatState(),
            startRuntimeState: savedChatState()
        ) { action, currentState in
            var state = currentState
            if case .addRoomMembers(let roomID, _) = action {
                state.selectedRoomId = roomID
                state.status = "people added"
            }
            return state
        }
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in runtime }

        model.start()
        let room = model.rooms[0]

        XCTAssertTrue(model.addMembers(to: room, profiles: [bob]))
        let expectedActions: [AppAction] = [
            .startRuntime,
            .addRoomMembers(roomId: "room-main", profiles: [bob]),
        ]
        try await waitUntil {
            runtime.dispatchedActions == expectedActions
                && model.state?.status == "people added"
        }
        XCTAssertEqual(runtime.dispatchedActions, expectedActions)
        XCTAssertEqual(model.selectedRoom?.roomId, "room-main")
    }

    func testAddRoomMembersFailureKeepsNoticeVisibleAndExistingRoom() async throws {
        let bob = AppProfileSummary(
            accountId: "bob-account",
            npub: "npub1bob",
            displayName: "Bob",
            about: nil,
            picture: nil,
            stale: false,
        isAgent: false
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: savedChatState(),
            startRuntimeState: savedChatState()
        ) { action, currentState in
            var state = currentState
            if case .addRoomMembers = action {
                state.status = "chat unavailable"
                state.toast = "Ask everyone to open Finite Chat, then try again"
            }
            return state
        }
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in runtime }

        model.start()
        let room = model.rooms[0]

        XCTAssertTrue(model.addMembers(to: room, profiles: [bob]))
        try await waitUntil {
            runtime.dispatchedActions == [
                .startRuntime,
                .addRoomMembers(roomId: "room-main", profiles: [bob]),
            ]
                && model.userNoticeText == "Ask everyone to open Finite Chat, then try again"
        }
        XCTAssertEqual(model.rooms.count, 1)
        XCTAssertEqual(model.rooms[0].roomId, "room-main")
        XCTAssertEqual(
            model.userNoticeText,
            "Ask everyone to open Finite Chat, then try again"
        )
        XCTAssertEqual(model.actionNoticeText, model.userNoticeText)
    }

    @MainActor
    func testScanTargetResultKeepsFailedProfileCodeVisible() async throws {
        let runtime = FakeFiniteChatRuntime(
            initialState: emptyChatState(),
            startRuntimeState: emptyChatState()
        ) { action, currentState in
            var state = currentState
            if case .scanTarget = action {
                state.status = "profile unavailable"
                state.toast = "Profile unavailable"
                state.flow = AppFlowState(
                    noticeText: "Profile unavailable",
                    noticeBusy: false,
                    scanInFlight: false,
                    inviteJoinSubmissionRoomId: nil,
                    scanResult: .unavailable,
                    imageUploadUrl: nil
                )
            }
            return state
        }
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in runtime }

        model.start()
        model.scanDraft = "npub1bob"

        var result: AppScanTargetResult?
        XCTAssertTrue(model.scanTarget { scanResult in
            result = scanResult
        })

        try await waitUntil {
            result != nil
        }

        if case .unavailable = result {
            // Expected.
        } else {
            XCTFail("expected unavailable scan result")
        }
        XCTAssertEqual(model.scanDraft, "npub1bob")
        XCTAssertEqual(model.userNoticeText, "Profile unavailable")
    }

    @MainActor
    func testScanTargetResultReturnsProfileWhenLookupFallsBackToPlaceholder() async throws {
        let profile = AppProfileSummary(
            accountId: "bob-account",
            npub: "npub1bob",
            displayName: "bob-acc…ount",
            about: nil,
            picture: nil,
            stale: true,
        isAgent: false
        )
        let runtime = FakeFiniteChatRuntime(
            initialState: emptyChatState(),
            startRuntimeState: emptyChatState()
        ) { action, currentState in
            var state = currentState
            if case .scanTarget = action {
                state.activeProfileId = profile.accountId
                state.profiles = [profile]
                state.status = "profile details unavailable"
                state.toast = "Profile details unavailable; you can still start a chat"
                state.flow = AppFlowState(
                    noticeText: "Profile opened.",
                    noticeBusy: false,
                    scanInFlight: false,
                    inviteJoinSubmissionRoomId: nil,
                    scanResult: .profile,
                    imageUploadUrl: nil
                )
            }
            return state
        }
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in runtime }

        model.start()
        model.scanDraft = "npub1bob"

        var result: AppScanTargetResult?
        XCTAssertTrue(model.scanTarget { scanResult in
            result = scanResult
        })

        try await waitUntil {
            result != nil
        }

        guard case .profile(let scanned) = result else {
            return XCTFail("expected scanned profile result")
        }
        XCTAssertEqual(scanned.accountId, "bob-account")
        XCTAssertTrue(scanned.stale)
        XCTAssertEqual(model.scanDraft, "")
        XCTAssertEqual(
            model.userNoticeText,
            "Profile details unavailable; you can still start a chat"
        )
    }

    @MainActor
    func testScannedProfileCodeBuildsDirectChatTargetWithoutLookup() async throws {
        let bob = try createNostrIdentity()
        let runtime = FakeFiniteChatRuntime(
            initialState: emptyChatState(),
            startRuntimeState: emptyChatState()
        ) { action, currentState in
            var state = currentState
            if case .startProfileChat(let dispatchedProfile, let displayName) = action {
                state.rooms = [
                    AppRoomSummary(
                        roomId: "room-bob",
                        displayName: displayName,
                        picture: nil,
                        state: .connected,
                        status: "connected",
                        userStatusText: "Connected",
                        lastMessagePreview: "",
                        unreadCount: 0,
                        canLoadOlder: false,
                        isAgentChat: false
                    ),
                ]
                state.selectedRoomId = "room-bob"
                state.status = "chat created"
                XCTAssertEqual(dispatchedProfile.accountId, bob.accountId)
            }
            return state
        }
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in runtime }

        model.start()

        let profile = try profileSummaryFromScannedProfileCode("nostr:\(bob.npub)", model: model)

        XCTAssertEqual(profile.accountId, bob.accountId)
        XCTAssertEqual(profile.npub, bob.npub)
        XCTAssertTrue(profile.stale)
        XCTAssertTrue(model.startProfileChat(for: profile))
        let expectedActions: [AppAction] = [
            .startRuntime,
            .startProfileChat(
                profile: profile,
                displayName: "Chat with \(profile.displayName)"
            ),
        ]
        try await waitUntil {
            runtime.dispatchedActions == expectedActions
                && model.selectedRoom?.roomId == "room-bob"
        }
        XCTAssertEqual(runtime.dispatchedActions, expectedActions)
    }

    @MainActor
    func testScannedHexProfileCodeBuildsDirectChatTargetWithoutLookup() throws {
        let bob = try createNostrIdentity()
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in
            FakeFiniteChatRuntime(
                initialState: self.emptyChatState(),
                startRuntimeState: self.emptyChatState()
            )
        }

        model.start()

        let profile = try profileSummaryFromScannedProfileCode(
            bob.accountId.uppercased(),
            model: model
        )

        XCTAssertEqual(profile.accountId, bob.accountId)
        XCTAssertEqual(profile.npub, bob.npub)
        XCTAssertEqual(profile.displayName, shortenedDisplayNpub(bob.npub))
        XCTAssertTrue(profile.stale)
    }

    @MainActor
    func testScannedNprofileCodeBuildsDirectChatTargetWithoutLookup() async throws {
        let bobAccountID = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        let bobNpub = try npubFromAccountId(accountId: bobAccountID)
        let bobNprofile = "nprofile1qqsqzg69v7y6hn00qy352euf40x77qfrg4ncn27dauqjx3t83x4ummcs22eux"
        let runtime = FakeFiniteChatRuntime(
            initialState: emptyChatState(),
            startRuntimeState: emptyChatState()
        ) { action, currentState in
            var state = currentState
            if case .startProfileChat(let dispatchedProfile, let displayName) = action {
                XCTAssertEqual(dispatchedProfile.accountId, bobAccountID)
                state.rooms = [
                    AppRoomSummary(
                        roomId: "room-bob",
                        displayName: displayName,
                        picture: nil,
                        state: .connected,
                        status: "connected",
                        userStatusText: "Connected",
                        lastMessagePreview: "",
                        unreadCount: 0,
                        canLoadOlder: false,
                        isAgentChat: false
                    ),
                ]
                state.selectedRoomId = "room-bob"
                state.status = "chat created"
            }
            return state
        }
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in runtime }

        model.start()

        let profile = try profileSummaryFromScannedProfileCode("nostr:\(bobNprofile)", model: model)

        XCTAssertEqual(profile.accountId, bobAccountID)
        XCTAssertEqual(profile.npub, bobNpub)
        XCTAssertEqual(profile.displayName, shortenedDisplayNpub(bobNpub))
        XCTAssertTrue(model.startProfileChat(for: profile))
        let expectedActions: [AppAction] = [
            .startRuntime,
            .startProfileChat(
                profile: profile,
                displayName: "Chat with \(profile.displayName)"
            ),
        ]
        try await waitUntil {
            runtime.dispatchedActions == expectedActions
                && model.selectedRoom?.roomId == "room-bob"
        }
        XCTAssertEqual(runtime.dispatchedActions, expectedActions)
    }

    @MainActor
    func testScannedProfileUrlBuildsDirectChatTargetWithoutLookup() throws {
        let bob = try createNostrIdentity()
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in
            FakeFiniteChatRuntime(
                initialState: self.emptyChatState(),
                startRuntimeState: self.emptyChatState()
            )
        }

        model.start()

        let profile = try profileSummaryFromScannedProfileCode(
            "https://finite.computer/profile?npub=\(bob.npub)",
            model: model
        )

        XCTAssertEqual(profile.accountId, bob.accountId)
        XCTAssertEqual(profile.npub, bob.npub)
        XCTAssertTrue(profile.stale)
    }

    @MainActor
    func testScannedProfileUrlNprofileBuildsDirectChatTargetWithoutLookup() throws {
        let bobAccountID = "2222222222222222222222222222222222222222222222222222222222222222"
        let bobNpub = try npubFromAccountId(accountId: bobAccountID)
        let bobNprofile = "nprofile1qqszyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygsmjs029"
        let model = AppModel(
            config: RuntimeConfig(
                serverURL: "https://chat.finite.computer",
                deviceID: "alice-phone"
            ),
            startsUpdateLoop: false
        ) { _ in
            FakeFiniteChatRuntime(
                initialState: self.emptyChatState(),
                startRuntimeState: self.emptyChatState()
            )
        }

        model.start()

        let profile = try profileSummaryFromScannedProfileCode(
            "https://finite.computer/profile?nprofile=\(bobNprofile)",
            model: model
        )

        XCTAssertEqual(profile.accountId, bobAccountID)
        XCTAssertEqual(profile.npub, bobNpub)
        XCTAssertTrue(profile.stale)
    }

    func testFiniteJoinCodeIsReservedForInviteHandlingEvenWhenItCarriesInviterNpub() throws {
        let inviter = try createNostrIdentity()
        let inviteCode = "finite://join?v=1&s=https%3A%2F%2Fchat.finite.computer&r=room-main&i=invite-1&t=token&a=\(inviter.npub)"

        XCTAssertTrue(isFiniteChatInviteCode(inviteCode))
        XCTAssertEqual(try profileNpub(from: inviteCode), inviter.npub)
    }

    private func savedChatState(
        status: String = "ready",
        toast: String? = nil,
        roomState: AppRoomState = .connected,
        roomStatus: String = "connected",
        flow: AppFlowState = AppFlowState(
            noticeText: nil,
            noticeBusy: false,
            scanInFlight: false,
            inviteJoinSubmissionRoomId: nil,
            scanResult: .none,
            imageUploadUrl: nil
        )
    ) -> AppState {
        let identity = Identity(
            accountId: "alice-account",
            deviceId: "qt433",
            accountSecretHex: String(repeating: "0", count: 64)
        )
        let room = AppRoomSummary(
            roomId: "room-main",
            displayName: "Main Room",
            picture: nil,
            state: roomState,
            status: roomStatus,
            userStatusText: roomUserStatusText(state: roomState, status: roomStatus),
            lastMessagePreview: "saved before force close",
            unreadCount: 0,
            canLoadOlder: false,
            isAgentChat: false
        )
        let message = ChatMessage(
            roomId: "room-main",
            seq: 1,
            messageId: "message-1",
            conversationId: nil,
            senderAccountId: "alice-account",
            senderDeviceId: "qt433",
            senderDisplayName: "qt433",
            senderNpub: nil,
            text: "saved before force close",
            displayContent: "saved before force close",
            richTextJson: "",
            payload: Data("saved before force close".utf8),
            replyToMessageId: nil,
            isMine: true,
            outboundDelivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .delivered
            ),
            reactions: [],
            media: [],
            readReceipt: nil,
            poll: nil,
            timestampUnixSeconds: 1_700_000_000,
            displayTimestamp: "now"
        )
        return AppState(
            rev: 1,
            identity: identity,
            rooms: [room],
            selectedRoomId: "room-main",
            activeInvite: nil,
            activeProfileId: nil,
            status: status,
            toast: toast,
            messages: [message],
            mediaGallery: nil,
            roomDetails: nil,
            profiles: [],
            devices: [],
            typingMembers: [],
            flow: flow
        )
    }

    private func productHarnessDeliveredTranscriptState() -> AppState {
        var state = savedChatState(
            status: "offline",
            toast: "Showing saved chats. Connection will retry."
        )
        state.rooms[0].lastMessagePreview = "offline product harness message"
        state.messages = [
            productHarnessMessage(
                id: "message-online",
                seq: 1,
                text: "online product harness message"
            ),
            productHarnessMessage(
                id: "message-offline",
                seq: 2,
                text: "offline product harness message"
            ),
        ]
        return state
    }

    private func productHarnessMessage(
        id: String,
        seq: UInt64,
        text: String
    ) -> ChatMessage {
        ChatMessage(
            roomId: "room-main",
            seq: seq,
            messageId: id,
            conversationId: nil,
            senderAccountId: "alice-account",
            senderDeviceId: "qt433",
            senderDisplayName: "qt433",
            senderNpub: nil,
            text: text,
            displayContent: text,
            richTextJson: "",
            payload: Data(text.utf8),
            replyToMessageId: nil,
            isMine: true,
            outboundDelivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .delivered
            ),
            reactions: [],
            media: [],
            readReceipt: ChatReadReceiptSummary(
                deliveredCount: 1,
                readCount: 0,
                displayText: "Delivered"
            ),
            poll: nil,
            timestampUnixSeconds: 1_700_000_000 + seq,
            displayTimestamp: "2:32 PM"
        )
    }

    private func emptyChatState(
        deviceID: String = "qt433",
        status: String = "ready",
        toast: String? = nil,
        flow: AppFlowState = AppFlowState(
            noticeText: nil,
            noticeBusy: false,
            scanInFlight: false,
            inviteJoinSubmissionRoomId: nil,
            scanResult: .none,
            imageUploadUrl: nil
        )
    ) -> AppState {
        AppState(
            rev: 1,
            identity: Identity(
                accountId: "alice-account",
                deviceId: deviceID,
                accountSecretHex: String(repeating: "0", count: 64)
            ),
            rooms: [],
            selectedRoomId: nil,
            activeInvite: nil,
            activeProfileId: nil,
            status: status,
            toast: toast,
            messages: [],
            mediaGallery: nil,
            roomDetails: nil,
            profiles: [],
            devices: [],
            typingMembers: [],
            flow: flow
        )
    }

    private func roomUserStatusText(state: AppRoomState, status: String) -> String {
        switch state {
        case .connected:
            return "Connected"
        case .waitingForApproval:
            return "Waiting for approval"
        case .joining:
            return "Joining"
        case .unavailableOnDevice:
            return "Unavailable on this device"
        }
    }

    private func temporarySupportURL() throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true
        )
        return directory
    }

    private func persistedConfig(at url: URL) throws -> RuntimeConfig {
        let data = try Data(contentsOf: url)
        return try JSONDecoder().decode(RuntimeConfig.self, from: data)
    }

    private func assertGeneratedDefaultDeviceID(
        _ deviceID: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertTrue(deviceID.hasPrefix("ios-"), file: file, line: line)
        XCTAssertEqual(deviceID.count, 16, file: file, line: line)
    }
}

final class ReactionEmojiCatalogTests: XCTestCase {
    func testEmptySearchShowsRecentSectionFirst() {
        let sections = ReactionEmojiCatalog.filteredSections(searchText: "")

        XCTAssertEqual(sections.first?.title, "Recent")
        XCTAssertEqual(
            Array(sections.first?.emojis.map(\.emoji).prefix(6) ?? []),
            ["❤️", "👍", "👎", "😂", "😮", "😢"]
        )
    }

    func testSearchMatchesEmojiNameAndKeywordWithoutDuplicates() {
        let rocketMatches = ReactionEmojiCatalog.filteredSections(searchText: "rocket")
        XCTAssertEqual(rocketMatches.map(\.title), ["Results"])
        XCTAssertEqual(rocketMatches.first?.emojis.map(\.emoji), ["🚀"])

        let agreeMatches = ReactionEmojiCatalog.filteredSections(searchText: "agree")
            .flatMap(\.emojis)
            .map(\.emoji)
        XCTAssertEqual(agreeMatches.count, Set(agreeMatches).count)
        XCTAssertTrue(agreeMatches.contains("👍"))
        XCTAssertTrue(agreeMatches.contains("🤝"))
    }

    func testWhitespaceOnlySearchUsesDefaultSections() {
        XCTAssertEqual(
            ReactionEmojiCatalog.filteredSections(searchText: "   "),
            ReactionEmojiCatalog.sections
        )
    }

    func testUnknownSearchReturnsNoSections() {
        XCTAssertTrue(ReactionEmojiCatalog.filteredSections(searchText: "not-an-emoji").isEmpty)
    }
}

final class OutboundDeliveryAccessibilityTests: XCTestCase {
    func testDeliveredBubbleProjectsTranscriptCheckmarkAccessibility() {
        let message = chatMessage(
            id: "message-delivered",
            text: "online product harness message",
            displayTimestamp: "2:32 PM",
            outboundDelivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .delivered
            )
        )

        let descriptor = ChatMessageBubbleAccessibilityDescriptor(message: message)

        XCTAssertEqual(
            descriptor.label,
            "online product harness message, 2:32 PM, Delivered"
        )
        XCTAssertEqual(descriptor.value, "two checks")
        XCTAssertEqual(descriptor.identifier, "ChatMessageBubble-message-delivered")
    }

    func testUndeliveredBubbleProjectsOneCheckAccessibility() {
        let message = chatMessage(
            id: "message-local",
            text: "offline product harness message",
            displayTimestamp: "2:33 PM",
            outboundDelivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .undelivered
            )
        )

        let descriptor = ChatMessageBubbleAccessibilityDescriptor(message: message)

        XCTAssertEqual(
            descriptor.label,
            "offline product harness message, 2:33 PM, Sent locally"
        )
        XCTAssertEqual(descriptor.value, "one check")
        XCTAssertEqual(descriptor.identifier, "ChatMessageBubble-message-local")
    }

    func testReadBubbleProjectsFilledCheckmarkAccessibility() {
        let message = chatMessage(
            id: "message-read",
            text: "read product harness message",
            displayTimestamp: "2:34 PM",
            outboundDelivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .delivered
            ),
            readReceipt: ChatReadReceiptSummary(
                deliveredCount: 1,
                readCount: 1,
                displayText: "Read"
            )
        )

        let descriptor = ChatMessageBubbleAccessibilityDescriptor(message: message)

        XCTAssertEqual(descriptor.label, "read product harness message, 2:34 PM, Read")
        XCTAssertEqual(descriptor.value, "two filled checks")
        XCTAssertEqual(descriptor.identifier, "ChatMessageBubble-message-read")
    }

    func testFailedBubbleProjectsRetryAccessibility() {
        let message = chatMessage(
            id: "message-failed",
            text: "failed product harness message",
            displayTimestamp: "2:35 PM",
            outboundDelivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .failed(reason: "server rejected send")
            )
        )

        let descriptor = ChatMessageBubbleAccessibilityDescriptor(message: message)

        XCTAssertEqual(descriptor.label, "failed product harness message, 2:35 PM, Not sent")
        XCTAssertEqual(descriptor.value, "retry required")
        XCTAssertEqual(descriptor.identifier, "ChatMessageBubble-message-failed")
    }

    func testInboundBubbleDoesNotInventOutboundDeliveryAccessibility() {
        let message = chatMessage(
            id: "message-inbound",
            text: "peer product harness message",
            displayTimestamp: "2:36 PM",
            isMine: false,
            outboundDelivery: nil
        )

        let descriptor = ChatMessageBubbleAccessibilityDescriptor(message: message)

        XCTAssertEqual(descriptor.label, "peer product harness message, 2:36 PM")
        XCTAssertEqual(descriptor.value, "")
        XCTAssertEqual(descriptor.identifier, "ChatMessageBubble-message-inbound")
    }

    func testUndeliveredMessageProjectsOneCheckDescriptor() {
        let descriptor = OutboundDeliveryAccessibilityDescriptor(
            messageID: "message-local",
            delivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .undelivered
            ),
            readReceipt: nil
        )

        XCTAssertEqual(descriptor.state, "sent-undelivered")
        XCTAssertEqual(descriptor.label, "Sent locally")
        XCTAssertEqual(descriptor.value, "one check")
        XCTAssertEqual(
            descriptor.identifier,
            "OutboundDeliveryMark-sent-undelivered-message-local"
        )
    }

    func testDeliveredMessageProjectsTwoCheckDescriptor() {
        let descriptor = OutboundDeliveryAccessibilityDescriptor(
            messageID: "message-delivered",
            delivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .delivered
            ),
            readReceipt: ChatReadReceiptSummary(
                deliveredCount: 1,
                readCount: 0,
                displayText: "Delivered"
            )
        )

        XCTAssertEqual(descriptor.state, "delivered-unread")
        XCTAssertEqual(descriptor.label, "Delivered")
        XCTAssertEqual(descriptor.value, "two checks")
        XCTAssertEqual(
            descriptor.identifier,
            "OutboundDeliveryMark-delivered-unread-message-delivered"
        )
    }

    func testReadMessageProjectsFilledTwoCheckDescriptor() {
        let descriptor = OutboundDeliveryAccessibilityDescriptor(
            messageID: "message-read",
            delivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .delivered
            ),
            readReceipt: ChatReadReceiptSummary(
                deliveredCount: 1,
                readCount: 1,
                displayText: "Read"
            )
        )

        XCTAssertEqual(descriptor.state, "delivered-read")
        XCTAssertEqual(descriptor.label, "Read")
        XCTAssertEqual(descriptor.value, "two filled checks")
        XCTAssertEqual(
            descriptor.identifier,
            "OutboundDeliveryMark-delivered-read-message-read"
        )
    }

    func testFailedMessageProjectsRetryDescriptor() {
        let descriptor = OutboundDeliveryAccessibilityDescriptor(
            messageID: "message-failed",
            delivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .failed(reason: "server rejected send")
            ),
            readReceipt: nil
        )

        XCTAssertEqual(descriptor.state, "failed")
        XCTAssertEqual(descriptor.label, "Not sent")
        XCTAssertEqual(descriptor.value, "retry required")
        XCTAssertEqual(descriptor.identifier, "OutboundDeliveryMark-failed-message-failed")
    }

    func testFailedLocalMessageShowsExplicitRetryAffordance() {
        let message = chatMessage(
            id: "message-failed",
            text: "failed product harness message",
            displayTimestamp: "2:35 PM",
            outboundDelivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .failed(reason: "server rejected send")
            )
        )

        let presentation = MessageRetryPresentation(message: message)

        XCTAssertTrue(presentation.isVisible)
        XCTAssertEqual(presentation.accessibilityIdentifier, "RetryMessageButton-message-failed")
    }

    func testRetryAffordanceDoesNotAppearForInboundOrUndeliveredMessages() {
        let inbound = chatMessage(
            id: "message-inbound",
            text: "peer product harness message",
            displayTimestamp: "2:36 PM",
            isMine: false,
            outboundDelivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .failed(reason: "malformed nonlocal state")
            )
        )
        let undelivered = chatMessage(
            id: "message-local",
            text: "offline product harness message",
            displayTimestamp: "2:33 PM",
            outboundDelivery: OutboundDelivery(
                localSend: .sent,
                serverDelivery: .undelivered
            )
        )

        XCTAssertFalse(MessageRetryPresentation(message: inbound).isVisible)
        XCTAssertFalse(MessageRetryPresentation(message: undelivered).isVisible)
    }

    private func chatMessage(
        id: String,
        text: String,
        displayTimestamp: String,
        isMine: Bool = true,
        outboundDelivery: OutboundDelivery?,
        readReceipt: ChatReadReceiptSummary? = nil
    ) -> ChatMessage {
        ChatMessage(
            roomId: "room-main",
            seq: 1,
            messageId: id,
            conversationId: nil,
            senderAccountId: isMine ? "alice-account" : "bob-account",
            senderDeviceId: isMine ? "alice-ios" : "bob-ios",
            senderDisplayName: isMine ? "Alice" : "Bob",
            senderNpub: nil,
            text: text,
            displayContent: text,
            richTextJson: "",
            payload: Data(text.utf8),
            replyToMessageId: nil,
            isMine: isMine,
            outboundDelivery: outboundDelivery,
            reactions: [],
            media: [],
            readReceipt: readReceipt,
            poll: nil,
            timestampUnixSeconds: 1_700_000_000,
            displayTimestamp: displayTimestamp
        )
    }
}

final class ChatMediaGalleryItemIdentityTests: XCTestCase {
    func testGeneratedGalleryItemUsesRustStableItemIdForSwiftIdentity() {
        let item = ChatMediaGalleryItem(
            itemId: "room-main|message-1|image-1",
            roomId: "room-main",
            messageId: "message-1",
            attachmentId: "image-1",
            attachment: ChatMediaAttachment(
                attachmentId: "image-1",
                url: "https://example.invalid/image-1",
                mimeType: "image/jpeg",
                filename: "image-1.jpg",
                kind: .image,
                width: nil,
                height: nil,
                localPath: nil,
                uploadProgressPerMille: nil,
                downloadProgressPerMille: nil
            ),
            senderDisplayName: "Alice",
            senderNpub: nil,
            timestampUnixSeconds: 1_700_000_000,
            displayTimestamp: "now"
        )

        XCTAssertEqual(item.id, "room-main|message-1|image-1")
    }
}

@MainActor
final class PushNotificationManagerTests: XCTestCase {
    func testDeviceTokenHexEncodingLowercasesAndPadsBytes() {
        let token = PushNotificationManager.hexToken(from: Data([0x00, 0x01, 0x0f, 0x10, 0xff]))

        XCTAssertEqual(token, "00010f10ff")
    }
}

final class AttachmentTransferPresentationTests: XCTestCase {
    func testZeroProgressMeansCoarseInFlightNotDeterminateProgress() {
        XCTAssertNil(attachmentDeterminateTransferProgress(nil))
        XCTAssertNil(attachmentDeterminateTransferProgress(0))
        XCTAssertEqual(attachmentDeterminateTransferProgress(1), 0.001)
        XCTAssertEqual(attachmentDeterminateTransferProgress(1_000), 1.0)
        XCTAssertEqual(attachmentDeterminateTransferProgress(5_000), 1.0)
    }

    func testDeliveredCacheMissIsExplicitDownloadCandidate() {
        XCTAssertTrue(attachmentCanDownload(remoteAttachment()))
        XCTAssertFalse(attachmentCanDownload(remoteAttachment(localPath: "/tmp/photo.jpg")))
        XCTAssertFalse(attachmentCanDownload(remoteAttachment(uploadProgressPerMille: 0)))
        XCTAssertFalse(attachmentCanDownload(remoteAttachment(downloadProgressPerMille: 0)))
    }

    private func remoteAttachment(
        localPath: String? = nil,
        uploadProgressPerMille: UInt32? = nil,
        downloadProgressPerMille: UInt32? = nil
    ) -> ChatMediaAttachment {
        ChatMediaAttachment(
            attachmentId: "image-1",
            url: "https://example.invalid/image-1",
            mimeType: "image/jpeg",
            filename: "image-1.jpg",
            kind: .image,
            width: nil,
            height: nil,
            localPath: localPath,
            uploadProgressPerMille: uploadProgressPerMille,
            downloadProgressPerMille: downloadProgressPerMille
        )
    }
}

final class SaveMediaActionTests: XCTestCase {
    func testSaveableImageAttachmentURLsIncludesOnlyLocalImages() {
        let message = chatMessage(media: [
            mediaAttachment(id: "image-local", kind: .image, localPath: "/tmp/photo.jpg"),
            mediaAttachment(id: "video-local", kind: .video, localPath: "/tmp/video.mov"),
            mediaAttachment(id: "image-remote", kind: .image, localPath: nil),
            mediaAttachment(id: "image-blank", kind: .image, localPath: "   "),
            mediaAttachment(id: "file-local", kind: .file, localPath: "/tmp/file.pdf"),
        ])

        XCTAssertEqual(
            saveableImageAttachmentURLs(in: message).map(\.path),
            ["/tmp/photo.jpg"]
        )
    }

    func testSaveableImageAttachmentURLsPreservesMultipleImagesInMessageOrder() {
        let message = chatMessage(media: [
            mediaAttachment(id: "first", kind: .image, localPath: "/tmp/first.jpg"),
            mediaAttachment(id: "second", kind: .image, localPath: "/tmp/second.png"),
        ])

        XCTAssertEqual(
            saveableImageAttachmentURLs(in: message).map(\.path),
            ["/tmp/first.jpg", "/tmp/second.png"]
        )
    }

    func testSaveMediaActionTitleMatchesImageCount() {
        XCTAssertNil(saveMediaActionTitle(imageCount: 0))
        XCTAssertEqual(saveMediaActionTitle(imageCount: 1), "Save Photo")
        XCTAssertEqual(saveMediaActionTitle(imageCount: 2), "Save Photos")
    }

    func testPhotoLibraryAddUsageDescriptionIsPresent() {
        let value = Bundle.main.object(
            forInfoDictionaryKey: "NSPhotoLibraryAddUsageDescription"
        ) as? String

        XCTAssertEqual(
            value,
            "Finite Chat saves photos from chats to your photo library when you choose Save Photo."
        )
    }

    private func chatMessage(media: [ChatMediaAttachment]) -> ChatMessage {
        ChatMessage(
            roomId: "room-main",
            seq: 1,
            messageId: "message-1",
            conversationId: nil,
            senderAccountId: "alice-account",
            senderDeviceId: "alice-ios",
            senderDisplayName: "Alice",
            senderNpub: nil,
            text: "photo",
            displayContent: "photo",
            richTextJson: "",
            payload: Data("photo".utf8),
            replyToMessageId: nil,
            isMine: false,
            outboundDelivery: nil,
            reactions: [],
            media: media,
            readReceipt: nil,
            poll: nil,
            timestampUnixSeconds: 1_700_000_000,
            displayTimestamp: "now"
        )
    }

    private func mediaAttachment(
        id: String,
        kind: ChatMediaKind,
        localPath: String?
    ) -> ChatMediaAttachment {
        ChatMediaAttachment(
            attachmentId: id,
            url: nil,
            mimeType: kind == .image ? "image/jpeg" : "application/octet-stream",
            filename: "\(id).dat",
            kind: kind,
            width: nil,
            height: nil,
            localPath: localPath,
            uploadProgressPerMille: nil,
            downloadProgressPerMille: nil
        )
    }
}

private struct RawDiagnosticError: Error, CustomStringConvertible {
    let description: String
}

private final class FakeFiniteChatRuntime: FiniteChatRuntimeProtocol, @unchecked Sendable {
    private let lock = NSRecursiveLock()
    private var currentState: AppState
    private var startRuntimeStates: [AppState]
    private let dispatchOverride: ((AppAction, AppState) -> AppState)?
    private var dispatchedActionsStorage: [AppAction] = []
    private var reconciler: AppReconciler?
    private var stateReadHook: (() -> Void)?
    private var dispatchStartHook: ((AppAction) -> Void)?

    var dispatchedActions: [AppAction] {
        withLock {
            dispatchedActionsStorage
        }
    }

    init(
        initialState: AppState,
        startRuntimeState: AppState,
        dispatchOverride: ((AppAction, AppState) -> AppState)? = nil
    ) {
        self.currentState = initialState
        self.startRuntimeStates = [startRuntimeState]
        self.dispatchOverride = dispatchOverride
    }

    init(
        initialState: AppState,
        startRuntimeStates: [AppState],
        dispatchOverride: ((AppAction, AppState) -> AppState)? = nil
    ) {
        currentState = initialState
        self.startRuntimeStates = startRuntimeStates.isEmpty ? [initialState] : startRuntimeStates
        self.dispatchOverride = dispatchOverride
    }

    func state() throws -> AppState {
        let hook = withLock {
            stateReadHook
        }
        hook?()
        return withLock {
            currentState
        }
    }

    func setStateReadHook(_ hook: (() -> Void)?) {
        withLock {
            stateReadHook = hook
        }
    }

    func setDispatchStartHook(_ hook: ((AppAction) -> Void)?) {
        withLock {
            dispatchStartHook = hook
        }
    }

    func dispatch(action: AppAction) throws {
        _ = try dispatchAndWait(action: action)
    }

    func dispatchAndWait(action: AppAction) throws -> AppState {
        let hook = withLock {
            dispatchStartHook
        }
        hook?(action)
        let updatedState = withLock {
            dispatchedActionsStorage.append(action)
            let previousRev = currentState.rev
            if action == .startRuntime {
                if startRuntimeStates.count > 1 {
                    currentState = startRuntimeStates.removeFirst()
                } else if let startRuntimeState = startRuntimeStates.first {
                    currentState = startRuntimeState
                }
            } else if let dispatchOverride {
                currentState = dispatchOverride(action, currentState)
            }
            if currentState.rev <= previousRev {
                currentState.rev = previousRev + 1
            }
            return currentState
        }
        publish(updatedState)
        return updatedState
    }

    func waitForUpdate(timeoutMillis: UInt64) throws -> AppState {
        let updatedState = withLock {
            let previousRev = currentState.rev
            if startRuntimeStates.count > 1 {
                currentState = startRuntimeStates.removeFirst()
            } else if let startRuntimeState = startRuntimeStates.first {
                currentState = startRuntimeState
            }
            if currentState.rev <= previousRev {
                currentState.rev = previousRev + 1
            }
            return currentState
        }
        publish(updatedState)
        return updatedState
    }

    func listenForUpdates(reconciler: AppReconciler) {
        let current = withLock {
            self.reconciler = reconciler
            return currentState
        }
        reconciler.reconcile(update: .fullState(current))
    }

    private func publish(_ state: AppState) {
        let listener = withLock {
            reconciler
        }
        listener?.reconcile(update: .fullState(state))
    }

    private func withLock<T>(_ operation: () throws -> T) rethrows -> T {
        lock.lock()
        defer { lock.unlock() }
        return try operation()
    }
}

final class MessageCollectionLayoutTests: XCTestCase {
    func testJumpButtonSpacingMatchesKeyboardChromeGap() {
        XCTAssertEqual(MessageCollectionLayout.jumpButtonSpacing, 12)
    }

    func testEffectiveContentInsetAccountsForAccessoryInset() {
        let inset = MessageCollectionLayout.effectiveContentInset(
            boundsHeight: 600,
            contentHeight: 180,
            topChromeInset: 44,
            bottomInset: 72
        )

        XCTAssertEqual(inset.top, 300)
        XCTAssertEqual(inset.bottom, 76)
    }

    func testGroupCreateFadeHeightReachesFloatingButton() {
        let chrome = MessageCollectionLayout.GroupCreateChrome.self
        let expected = chrome.dockTopPadding
            + chrome.buttonHeight
            + chrome.dockBottomPadding
            + chrome.fadeLiftAboveButton
        XCTAssertEqual(
            MessageCollectionLayout.groupCreateFadeHeight(safeAreaBottom: 34),
            34 + expected
        )
    }

    func testSafeZoneFadeHeightReachesComposerIcons() {
        let chrome = MessageCollectionLayout.ComposerChrome.self
        let expected = chrome.dockBottomPadding
            + chrome.iconRowBottomPadding
            + chrome.iconRowHeight
            + chrome.fadeLiftAboveIcons
        XCTAssertEqual(
            MessageCollectionLayout.safeZoneFadeHeight(safeAreaBottom: 34),
            34 + expected
        )
        XCTAssertEqual(
            MessageCollectionLayout.safeZoneFadeHeight(safeAreaBottom: 0),
            expected
        )
    }

    func testBottomViewportInsetUsesOccupiedKeyboardOrAccessoryHeight() {
        XCTAssertEqual(
            MessageCollectionLayout.bottomViewportInset(
                keyboardInset: 330,
                accessoryHeight: 58,
                safeAreaBottom: 34
            ),
            388
        )
        XCTAssertEqual(
            MessageCollectionLayout.bottomViewportInset(
                keyboardInset: -12,
                accessoryHeight: 58
            ),
            58
        )
        XCTAssertEqual(
            MessageCollectionLayout.bottomViewportInset(
                keyboardInset: 34,
                accessoryHeight: 58,
                safeAreaBottom: 34
            ),
            58
        )
    }

    func testBottomPinSurvivesKeyboardGeometryTransition() {
        XCTAssertTrue(
            MessageCollectionLayout.shouldPinToBottom(
                isNearBottom: false,
                followsBottom: true,
                isHoldingInitialBottomPin: false
            )
        )
        XCTAssertTrue(
            MessageCollectionLayout.shouldPinToBottom(
                isNearBottom: false,
                followsBottom: false,
                isHoldingInitialBottomPin: true
            )
        )
        XCTAssertFalse(
            MessageCollectionLayout.shouldPinToBottom(
                isNearBottom: false,
                followsBottom: false,
                isHoldingInitialBottomPin: false
            )
        )
    }

    func testFollowsBottomOnlyTurnsOffForUserScroll() {
        XCTAssertTrue(
            MessageCollectionLayout.nextFollowsBottom(
                current: true,
                isNearBottom: false,
                isUserScrolling: false
            )
        )
        XCTAssertFalse(
            MessageCollectionLayout.nextFollowsBottom(
                current: true,
                isNearBottom: false,
                isUserScrolling: true
            )
        )
        XCTAssertTrue(
            MessageCollectionLayout.nextFollowsBottom(
                current: false,
                isNearBottom: true,
                isUserScrolling: false
            )
        )
        XCTAssertFalse(
            MessageCollectionLayout.shouldShowJumpButton(
                isNearBottom: false,
                followsBottom: true
            )
        )
        XCTAssertTrue(
            MessageCollectionLayout.shouldShowJumpButton(
                isNearBottom: false,
                followsBottom: false
            )
        )
    }

    func testNearBottomUsesVisibleViewportBottom() {
        XCTAssertTrue(
            MessageCollectionLayout.isNearBottom(
                contentOffsetY: 900,
                boundsHeight: 500,
                contentHeight: 1300,
                topAdjustedInset: 30,
                bottomInset: 106
            )
        )
        XCTAssertFalse(
            MessageCollectionLayout.isNearBottom(
                contentOffsetY: 700,
                boundsHeight: 500,
                contentHeight: 1300,
                topAdjustedInset: 30,
                bottomInset: 106
            )
        )
    }

    func testBottomContentOffsetUsesHostOwnedBottomInset() {
        let offset = MessageCollectionLayout.bottomContentOffset(
            contentHeight: 1300,
            boundsHeight: 500,
            topAdjustedInset: 30,
            bottomInset: 72
        )
        XCTAssertEqual(offset, CGPoint(x: 0, y: 872))
    }

    func testUpdateClassificationUsesTailMutationForAppendAndTrim() {
        XCTAssertEqual(
            MessageCollectionLayout.classifyUpdate(
                oldIDs: ["a", "b"],
                newIDs: ["a", "b", "c"]
            ),
            .tailMutation
        )
        XCTAssertEqual(
            MessageCollectionLayout.classifyUpdate(
                oldIDs: ["a", "b", "c"],
                newIDs: ["a", "b"]
            ),
            .tailMutation
        )
    }

    func testUpdateClassificationTreatsReshapesAsStructural() {
        XCTAssertEqual(
            MessageCollectionLayout.classifyUpdate(
                oldIDs: ["row-1", "row-2"],
                newIDs: ["row-0", "row-2"]
            ),
            .structural
        )
        XCTAssertEqual(
            MessageCollectionLayout.classifyUpdate(
                oldIDs: ["row-1", "row-2"],
                newIDs: ["row-1", "row-2"]
            ),
            .reconfigureOnly
        )
    }
}

final class ChatComposerAccessibilityTests: XCTestCase {
    @MainActor
    func testComposerTextViewHasStableAccessibilityTarget() {
        let textView = PastableTextView()

        XCTAssertEqual(textView.accessibilityLabel, "Message")
        XCTAssertEqual(textView.accessibilityIdentifier, "ComposerMessageField")
    }
}

final class StagedComposerAttachmentTests: XCTestCase {
    func testFileURLStagesOutboundAttachmentMetadataAndBytes() throws {
        let directory = try temporaryDirectory()
        let url = directory.appendingPathComponent("sample.png")
        let bytes = Data([0x89, 0x50, 0x4E, 0x47])
        try bytes.write(to: url)

        let staged = try StagedComposerAttachment(fileURL: url)
        let outbound = staged.outboundAttachment

        XCTAssertEqual(staged.filename, "sample.png")
        XCTAssertEqual(staged.mimeType, "image/png")
        XCTAssertEqual(staged.kind, .image)
        XCTAssertEqual(outbound.filename, "sample.png")
        XCTAssertEqual(outbound.mimeType, "image/png")
        XCTAssertEqual(outbound.kind, .image)
        XCTAssertEqual(outbound.bytes, bytes)
    }

    func testFileURLRejectsProtocolOversizedAttachment() throws {
        let directory = try temporaryDirectory()
        let url = directory.appendingPathComponent("too-large.bin")
        try Data(count: maxComposerAttachmentBytes + 1).write(to: url)

        XCTAssertThrowsError(try StagedComposerAttachment(fileURL: url)) { error in
            guard case ComposerAttachmentError.tooLarge(let filename) = error else {
                return XCTFail("Unexpected error: \(error)")
            }
            XCTAssertEqual(filename, "too-large.bin")
        }
    }

    func testPastedImageStagesAsOutboundAttachment() throws {
        let bytes = Data([0x47, 0x49, 0x46, 0x38])

        let staged = try StagedComposerAttachment(
            pastedData: bytes,
            mimeType: "image/gif"
        )
        let outbound = staged.outboundAttachment

        XCTAssertTrue(staged.filename.hasPrefix("pasted-"))
        XCTAssertTrue(staged.filename.hasSuffix(".gif"))
        XCTAssertEqual(staged.mimeType, "image/gif")
        XCTAssertEqual(staged.kind, .image)
        XCTAssertEqual(outbound.mimeType, "image/gif")
        XCTAssertEqual(outbound.kind, .image)
        XCTAssertEqual(outbound.bytes, bytes)
    }

    private func temporaryDirectory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true
        )
        return directory
    }
}

final class ImageUploadPayloadTests: XCTestCase {
    func testUploadPayloadDownscalesAndReencodesImagesForProfileAndRoomMetadata() throws {
        let sourceData = try makeSolidImageData(width: 2400, height: 1200)

        let payload = try ImageUploadPayload(sourceData: sourceData)
        let decoded = try XCTUnwrap(UIImage(data: payload.data))
        let largestSide = max(decoded.size.width, decoded.size.height)

        XCTAssertEqual(payload.mimeType, "image/jpeg")
        XCTAssertLessThanOrEqual(largestSide, 1024)
        XCTAssertLessThanOrEqual(payload.data.count, maxImageUploadBytes)
    }

    func testUploadPayloadRejectsUnreadableBytesBeforeUpload() {
        XCTAssertThrowsError(try ImageUploadPayload(sourceData: Data("not an image".utf8))) { error in
            guard case ImageUploadError.unreadableImage = error else {
                return XCTFail("Unexpected error: \(error)")
            }
        }
    }

    private func makeSolidImageData(width: CGFloat, height: CGFloat) throws -> Data {
        let renderer = UIGraphicsImageRenderer(size: CGSize(width: width, height: height))
        let image = renderer.image { context in
            UIColor.systemBlue.setFill()
            context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        }
        return try XCTUnwrap(image.pngData())
    }
}

final class VoiceMessageTests: XCTestCase {
    func testVoiceRecordingAttachmentUsesProtocolVoiceKind() throws {
        let bytes = Data([0x00, 0x01, 0x02])
        let now = Date(timeIntervalSince1970: 1_725_000_123)

        let attachment = try VoiceRecordingAttachment.outboundAttachment(data: bytes, now: now)

        XCTAssertEqual(attachment.filename, "voice_1725000123.m4a")
        XCTAssertEqual(attachment.mimeType, "audio/mp4")
        XCTAssertEqual(attachment.kind, .voiceNote)
        XCTAssertEqual(attachment.bytes, bytes)
    }

    func testVoiceRecordingAttachmentRejectsOversizeBeforeDispatch() {
        let now = Date(timeIntervalSince1970: 1_725_000_123)

        XCTAssertThrowsError(try VoiceRecordingAttachment.outboundAttachment(
            data: Data(count: maxComposerAttachmentBytes + 1),
            now: now
        )) { error in
            guard case ComposerAttachmentError.tooLarge(let filename) = error else {
                return XCTFail("Unexpected error: \(error)")
            }
            XCTAssertEqual(filename, "voice_1725000123.m4a")
        }
    }

    func testVoiceDurationFormattingUsesMonospacedClockShape() {
        XCTAssertEqual(formattedDuration(0), "0:00")
        XCTAssertEqual(formattedDuration(65.9), "1:05")
        XCTAssertEqual(formattedDuration(3_605), "60:05")
    }

    func testVoiceRecordingCaptionTrimsTranscript() {
        XCTAssertEqual(
            voiceRecordingCaption(VoiceRecordingState(
                phase: .recording,
                durationSecs: 1,
                levels: [],
                transcript: "  Hello from speech  \n"
            )),
            "Hello from speech"
        )
        XCTAssertEqual(
            voiceRecordingCaption(VoiceRecordingState(
                phase: .paused,
                durationSecs: 1,
                levels: [],
                transcript: " \n "
            )),
            ""
        )
        XCTAssertEqual(voiceRecordingCaption(nil), "")
    }
}

private func waitUntil(
    timeout: TimeInterval = 2,
    condition: @escaping @MainActor () -> Bool
) async throws {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        if await condition() {
            return
        }
        try await Task.sleep(nanoseconds: 10_000_000)
    }
    XCTFail("timed out waiting for condition")
}

@MainActor
private func waitForActions(
    _ runtime: FakeFiniteChatRuntime,
    _ expected: [AppAction],
    timeout: TimeInterval = 2
) async throws {
    try await waitUntil(timeout: timeout) {
        runtime.dispatchedActions == expected
    }
}
