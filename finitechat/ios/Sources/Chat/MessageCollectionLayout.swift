import UIKit

enum MessageCollectionUpdateKind: Equatable {
    case reconfigureOnly
    case tailMutation
    case structural
}

enum MessageCollectionLayout {
    static let jumpButtonSpacing: CGFloat = 12
    static let bottomContentSpacing: CGFloat = 4

    static func shouldPinToBottom(
        isNearBottom: Bool,
        followsBottom: Bool,
        isHoldingInitialBottomPin: Bool
    ) -> Bool {
        isNearBottom || followsBottom || isHoldingInitialBottomPin
    }

    static func nextFollowsBottom(
        current: Bool,
        isNearBottom: Bool,
        isUserScrolling: Bool
    ) -> Bool {
        if isNearBottom {
            return true
        }
        if isUserScrolling {
            return false
        }
        return current
    }

    static func shouldShowJumpButton(isNearBottom: Bool, followsBottom: Bool) -> Bool {
        !isNearBottom && !followsBottom
    }

    static func effectiveContentInset(
        boundsHeight: CGFloat,
        contentHeight: CGFloat,
        topChromeInset: CGFloat,
        bottomInset: CGFloat
    ) -> UIEdgeInsets {
        let effectiveBottomInset = bottomInset + bottomContentSpacing
        let availableHeight = max(0, boundsHeight - topChromeInset - effectiveBottomInset)
        let extraTopInset = max(0, availableHeight - contentHeight)
        return UIEdgeInsets(
            top: extraTopInset,
            left: 0,
            bottom: effectiveBottomInset,
            right: 0
        )
    }

    static func classifyUpdate(oldIDs: [String], newIDs: [String]) -> MessageCollectionUpdateKind {
        guard oldIDs != newIDs else { return .reconfigureOnly }
        if oldIDs.isPrefix(of: newIDs) || newIDs.isPrefix(of: oldIDs) {
            return .tailMutation
        }
        return .structural
    }

    static func isStrictPrepend(oldIDs: [String], newIDs: [String]) -> Bool {
        guard !oldIDs.isEmpty, newIDs.count > oldIDs.count else { return false }
        return Array(newIDs.suffix(oldIDs.count)) == oldIDs
    }

    static func contentOffsetY(
        preserving anchor: ScrollAnchor,
        itemFrame: CGRect,
        minOffsetY: CGFloat,
        maxOffsetY: CGFloat
    ) -> CGFloat {
        let itemPosition: CGFloat
        switch anchor.edge {
        case .top:
            itemPosition = itemFrame.minY
        case .bottom:
            itemPosition = itemFrame.maxY
        }
        return min(
            max(itemPosition - anchor.distanceFromContentOffset, minOffsetY),
            maxOffsetY
        )
    }

    static func isNearBottom(
        contentOffsetY: CGFloat,
        boundsHeight: CGFloat,
        contentHeight: CGFloat,
        topAdjustedInset: CGFloat,
        bottomInset: CGFloat,
        tolerance: CGFloat = 56
    ) -> Bool {
        let minOffsetY = -topAdjustedInset
        let effectiveOffsetY = max(contentOffsetY, minOffsetY)
        let visibleBottom = effectiveOffsetY + boundsHeight - bottomInset
        return visibleBottom >= contentHeight - tolerance
    }

    static func bottomContentOffset(
        contentHeight: CGFloat,
        boundsHeight: CGFloat,
        topAdjustedInset: CGFloat,
        bottomInset: CGFloat
    ) -> CGPoint {
        let minOffsetY = -topAdjustedInset
        let maxOffsetY = max(minOffsetY, contentHeight - boundsHeight + bottomInset)
        return CGPoint(x: 0, y: maxOffsetY)
    }
}

private extension Array where Element: Equatable {
    func isPrefix(of other: [Element]) -> Bool {
        guard count <= other.count else { return false }
        return zip(self, other).allSatisfy(==)
    }
}
