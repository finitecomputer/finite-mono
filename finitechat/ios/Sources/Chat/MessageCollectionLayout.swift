import UIKit

enum MessageCollectionUpdateKind: Equatable {
    case reconfigureOnly
    case tailMutation
    case structural
}

enum MessageCollectionLayout {
    static let jumpButtonSpacing: CGFloat = 12
    static let bottomContentSpacing: CGFloat = 4
    static let chatTopFadeExtension: CGFloat = 32
    static let inlineNavigationBarHeight: CGFloat = 44

    static func chatTopFadeHeight(safeAreaTop: CGFloat) -> CGFloat {
        max(0, safeAreaTop) + inlineNavigationBarHeight + chatTopFadeExtension
    }

    enum ComposerChrome {
        static let dockBottomPadding: CGFloat = 8
        static let iconRowBottomPadding: CGFloat = 10
        static let iconRowHeight: CGFloat = 34
        static let fadeLiftAboveIcons: CGFloat = 44
    }

    enum GroupCreateChrome {
        static let dockTopPadding: CGFloat = 12
        static let dockBottomPadding: CGFloat = 8
        static let buttonHeight: CGFloat = 50
        static let fadeLiftAboveButton: CGFloat = 40
    }

    static func groupCreateFadeHeight(safeAreaBottom: CGFloat) -> CGFloat {
        max(0, safeAreaBottom)
            + GroupCreateChrome.dockTopPadding
            + GroupCreateChrome.buttonHeight
            + GroupCreateChrome.dockBottomPadding
            + GroupCreateChrome.fadeLiftAboveButton
    }

    static func safeZoneFadeHeight(safeAreaBottom: CGFloat) -> CGFloat {
        max(0, safeAreaBottom)
            + ComposerChrome.dockBottomPadding
            + ComposerChrome.iconRowBottomPadding
            + ComposerChrome.iconRowHeight
            + ComposerChrome.fadeLiftAboveIcons
    }

    static func bottomViewportInset(
        keyboardInset: CGFloat,
        accessoryHeight: CGFloat,
        safeAreaBottom: CGFloat = 0
    ) -> CGFloat {
        let keyboard = max(0, keyboardInset)
        let accessory = max(0, accessoryHeight)
        let keyboardVisible = keyboard > safeAreaBottom + 20
        if keyboardVisible {
            return keyboard + accessory
        }
        return accessory
    }

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
