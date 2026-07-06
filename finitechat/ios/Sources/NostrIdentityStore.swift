import Foundation
import Security

struct AppNostrIdentity: Codable, Equatable, Sendable {
    let accountSecretHex: String
    let accountID: String
    let npub: String
    let nsec: String

    init(material: NostrIdentityMaterial) {
        accountSecretHex = material.accountSecretHex
        accountID = material.accountId
        npub = material.npub
        nsec = material.nsec
    }
}

protocol AppNostrIdentityStoring: AnyObject {
    func load() -> AppNostrIdentity?
    func save(_ identity: AppNostrIdentity)
    func clear()
}

final class KeychainNostrIdentityStore: AppNostrIdentityStoring {
    private let service: String
    private let account: String

    init(
        service: String = "computer.finite.finitechat.nostr-identity",
        account: String = "primary"
    ) {
        self.service = service
        self.account = account
    }

    func load() -> AppNostrIdentity? {
        var query = baseQuery()
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne

        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess,
              let data = item as? Data
        else {
            return nil
        }
        return try? JSONDecoder().decode(AppNostrIdentity.self, from: data)
    }

    func save(_ identity: AppNostrIdentity) {
        guard let data = try? JSONEncoder().encode(identity) else { return }
        var query = baseQuery()
        let update: [String: Any] = [kSecValueData as String: data]
        let status = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if status == errSecSuccess { return }

        query[kSecValueData as String] = data
        query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(query as CFDictionary, nil)
    }

    func clear() {
        SecItemDelete(baseQuery() as CFDictionary)
    }

    private func baseQuery() -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }
}

final class MemoryNostrIdentityStore: AppNostrIdentityStoring {
    private var identity: AppNostrIdentity?

    init(identity: AppNostrIdentity? = nil) {
        self.identity = identity
    }

    func load() -> AppNostrIdentity? {
        identity
    }

    func save(_ identity: AppNostrIdentity) {
        self.identity = identity
    }

    func clear() {
        identity = nil
    }
}
