import Foundation

struct ChatTimelineMessageGroup: Identifiable, Equatable {
    let senderAccountId: String
    let senderDeviceId: String
    let senderDisplayName: String
    let senderNpub: String?
    let senderPictureURL: String?
    let isMine: Bool
    var messages: [ChatMessage]

    var id: String {
        guard let firstMessageId = messages.first?.messageId else {
            return "\(senderAccountId)/\(senderDeviceId)"
        }
        let lastMessageId = messages.last?.messageId ?? firstMessageId
        return "\(firstMessageId)-\(lastMessageId)-\(messages.count)"
    }
}

struct ChatRoomProjection: Equatable {
    let roomID: String
    let messages: [ChatMessage]
    let rows: [ChatTimelineRow]
    let messagesById: [String: ChatMessage]

    static func empty(roomID: String) -> ChatRoomProjection {
        ChatRoomProjection(
            roomID: roomID,
            messages: [],
            rows: [],
            messagesById: [:]
        )
    }
}

enum ChatActivityKind: Equatable {
    case working
    case thinking
    case typing
    case other(String)

    init(rawValue: String) {
        let normalized = rawValue.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        switch normalized {
        case "working":
            self = .working
        case "thinking":
            self = .thinking
        case "typing":
            self = .typing
        case "":
            self = .other("active")
        default:
            self = .other(normalized)
        }
    }

    var id: String {
        switch self {
        case .working:
            return "working"
        case .thinking:
            return "thinking"
        case .typing:
            return "typing"
        case .other(let rawValue):
            return rawValue
        }
    }

    var priority: Int {
        switch self {
        case .working:
            return 0
        case .thinking:
            return 1
        case .typing:
            return 2
        case .other:
            return 3
        }
    }

    var verb: String {
        switch self {
        case .working:
            return "working"
        case .thinking:
            return "thinking"
        case .typing:
            return "typing"
        case .other:
            return "active"
        }
    }
}

struct ChatTimelineActivity: Identifiable, Equatable {
    let kind: ChatActivityKind
    let members: [AppTypingMember]

    init(kind: ChatActivityKind, members: [AppTypingMember]) {
        self.kind = kind
        self.members = Self.orderedMembers(members)
    }

    init?(members: [AppTypingMember]) {
        let ordered = Self.orderedMembers(members)
        guard let selectedKind = ordered
            .map({ ChatActivityKind(rawValue: $0.activityKind) })
            .min(by: { $0.priority < $1.priority })
        else {
            return nil
        }
        self.kind = selectedKind
        self.members = ordered.filter { ChatActivityKind(rawValue: $0.activityKind) == selectedKind }
    }

    var id: String {
        "activity-\(kind.id)"
    }

    var primaryMember: AppTypingMember? {
        members.first
    }

    var primaryDisplayName: String {
        let trimmed = primaryMember?.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        if let trimmed, !trimmed.isEmpty {
            return trimmed
        }
        return primaryMember?.deviceId ?? "Someone"
    }

    var primarySubtitle: String {
        primaryMember?.npub ?? primaryMember?.accountId ?? kind.id
    }

    var primaryPictureURL: String? {
        primaryMember?.picture
    }

    var label: String {
        if members.count <= 1 {
            return "\(primaryDisplayName) is \(kind.verb)"
        }
        return "\(primaryDisplayName) and \(members.count - 1) others are \(kind.verb)"
    }

    private static func orderedMembers(_ members: [AppTypingMember]) -> [AppTypingMember] {
        members.sorted {
            let leftName = $0.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
            let rightName = $1.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
            if leftName != rightName { return leftName < rightName }
            if $0.accountId != $1.accountId { return $0.accountId < $1.accountId }
            return $0.deviceId < $1.deviceId
        }
    }
}

enum ChatTimelineRow: Identifiable, Equatable {
    case messageGroup(ChatTimelineMessageGroup)
    case activity(ChatTimelineActivity)

    var id: String {
        switch self {
        case .messageGroup(let group):
            "group-\(group.id)"
        case .activity(let activity):
            activity.id
        }
    }

    var oldestMessageID: String? {
        switch self {
        case .messageGroup(let group):
            group.messages.first?.messageId
        case .activity:
            nil
        }
    }

    func containsMessage(_ messageID: String) -> Bool {
        switch self {
        case .messageGroup(let group):
            group.messages.contains { $0.messageId == messageID }
        case .activity:
            false
        }
    }
}

enum ChatTimeline {
    static func roomProjections(
        messages: [ChatMessage],
        typingMembers: [AppTypingMember] = [],
        profiles: [AppProfileSummary] = []
    ) -> [String: ChatRoomProjection] {
        guard !messages.isEmpty || !typingMembers.isEmpty else { return [:] }
        let profilesByAccountID = profilesByAccountID(profiles)

        var messagesByRoom: [String: [ChatMessage]] = [:]
        messagesByRoom.reserveCapacity(8)
        for message in messages {
            messagesByRoom[message.roomId, default: []].append(message)
        }

        var liveMembersByRoom: [String: [AppTypingMember]] = [:]
        liveMembersByRoom.reserveCapacity(typingMembers.count)
        for member in typingMembers {
            liveMembersByRoom[member.roomId, default: []].append(member)
        }

        let roomIDs = Set(messagesByRoom.keys).union(liveMembersByRoom.keys)
        var projections: [String: ChatRoomProjection] = [:]
        projections.reserveCapacity(roomIDs.count)
        for roomID in roomIDs {
            let roomMessages = messagesByRoom[roomID] ?? []
            let ordered = orderedMessages(roomMessages)
            projections[roomID] = ChatRoomProjection(
                roomID: roomID,
                messages: ordered,
                rows: rows(
                    orderedMessages: ordered,
                    typingMembers: liveMembersByRoom[roomID] ?? [],
                    profilesByAccountID: profilesByAccountID
                ),
                messagesById: messagesById(ordered)
            )
        }
        return projections
    }

    static func rows(
        messages: [ChatMessage],
        typingMembers: [AppTypingMember] = [],
        profiles: [AppProfileSummary] = []
    ) -> [ChatTimelineRow] {
        let profilesByAccountID = profilesByAccountID(profiles)
        return rows(
            orderedMessages: orderedMessages(messages),
            typingMembers: typingMembers,
            profilesByAccountID: profilesByAccountID
        )
    }

    static func messagesById(_ messages: [ChatMessage]) -> [String: ChatMessage] {
        Dictionary(uniqueKeysWithValues: messages.map { ($0.messageId, $0) })
    }

    static func rowID(containingMessageID messageID: String, rows: [ChatTimelineRow]) -> String? {
        for row in rows {
            if row.containsMessage(messageID) {
                return row.id
            }
        }
        return nil
    }

    private static func orderedMessages(_ messages: [ChatMessage]) -> [ChatMessage] {
        messages.sorted {
            if $0.seq == $1.seq {
                return $0.messageId < $1.messageId
            }
            return $0.seq < $1.seq
        }
    }

    private static func rows(
        orderedMessages ordered: [ChatMessage],
        typingMembers: [AppTypingMember],
        profilesByAccountID: [String: AppProfileSummary]
    ) -> [ChatTimelineRow] {
        var rows: [ChatTimelineRow] = []
        rows.reserveCapacity(ordered.count + (typingMembers.isEmpty ? 0 : 1))

        for message in ordered {
            if let lastIndex = rows.indices.last,
               case .messageGroup(var group) = rows[lastIndex],
               group.senderAccountId == message.senderAccountId,
               group.senderDeviceId == message.senderDeviceId,
               group.isMine == message.isMine,
               message.replyToMessageId == nil
            {
                group.messages.append(message)
                rows[lastIndex] = .messageGroup(group)
                continue
            }

            rows.append(
                .messageGroup(
                    ChatTimelineMessageGroup(
                        senderAccountId: message.senderAccountId,
                        senderDeviceId: message.senderDeviceId,
                        senderDisplayName: message.senderDisplayName,
                        senderNpub: message.senderNpub,
                        senderPictureURL: profilesByAccountID[message.senderAccountId]?.picture,
                        isMine: message.isMine,
                        messages: [message]
                    )
                )
            )
        }

        if let activity = ChatTimelineActivity(members: typingMembers) {
            rows.append(.activity(activity))
        }

        return rows
    }

    private static func profilesByAccountID(_ profiles: [AppProfileSummary]) -> [String: AppProfileSummary] {
        var result: [String: AppProfileSummary] = [:]
        result.reserveCapacity(profiles.count)
        for profile in profiles {
            result[profile.accountId] = profile
        }
        return result
    }
}
