import Foundation
import Security

struct AppNostrIdentity: Codable, Equatable, Sendable {
    let accountSecretHex: String
    let accountID: String
    let npub: String

    init(material: NostrIdentityMaterial) {
        accountSecretHex = material.accountSecretHex
        accountID = material.accountId
        npub = material.npub
    }
}

protocol AppNostrIdentityStoring: AnyObject {
    func load() -> AppNostrIdentity?
    func save(_ identity: AppNostrIdentity) throws
    func clear()
}

enum AppNostrIdentityStoreError: Error {
    case encode
    case keychain(OSStatus)
    case verification
}

final class KeychainNostrIdentityStore: AppNostrIdentityStoring {
    static let productionService = "computer.finite.finitechat.workos-linked-account"
    static let localDevelopmentService = "computer.finite.finitechat.local-device-link-account"
    static let primaryAccount = "primary"

    private let service: String
    private let account: String

    init(
        service: String = productionService,
        account: String = primaryAccount
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

    func save(_ identity: AppNostrIdentity) throws {
        guard let data = try? JSONEncoder().encode(identity) else {
            throw AppNostrIdentityStoreError.encode
        }
        var query = baseQuery()
        let update: [String: Any] = [kSecValueData as String: data]
        let status = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if status != errSecSuccess {
            guard status == errSecItemNotFound else {
                throw AppNostrIdentityStoreError.keychain(status)
            }
            query[kSecValueData as String] = data
            query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            let addStatus = SecItemAdd(query as CFDictionary, nil)
            guard addStatus == errSecSuccess else {
                throw AppNostrIdentityStoreError.keychain(addStatus)
            }
        }
        guard load() == identity else {
            throw AppNostrIdentityStoreError.verification
        }
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

    func save(_ identity: AppNostrIdentity) throws {
        self.identity = identity
    }

    func clear() {
        identity = nil
    }
}
