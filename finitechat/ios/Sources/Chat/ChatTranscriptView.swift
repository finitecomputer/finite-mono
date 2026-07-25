import SwiftUI
import UIKit

struct ChatTranscriptView: UIViewControllerRepresentable {
    struct ContentState: Equatable {
        let rows: [ChatTimelineRow]
        let messagesById: [String: ChatMessage]

        var messageIDs: [String] {
            rows.flatMap { row -> [String] in
                switch row {
                case .messageGroup(let group):
                    group.messages.map(\.messageId)
                case .activity:
                    []
                }
            }
        }
    }

    let roomID: String
    let rows: [ChatTimelineRow]
    let messagesById: [String: ChatMessage]
    let onReact: (ChatMessage, String) -> Void
    let onDownloadAttachment: (ChatMessage, ChatMediaAttachment) -> Void
    let onOpenAttachment: (ChatMessage, ChatMediaAttachment) -> Void
    let onVotePoll: (ChatMessage, ChatPollOption) -> Void
    let onRetryMessage: (ChatMessage) -> Void
    let onLongPressMessage: (ChatMessage, CGRect) -> Void
    let onOpenURL: (URL) -> OpenURLAction.Result
    var canLoadOlder = false
    var onLoadOlderMessages: ((String) -> Void)?
    @Binding var followsBottom: Bool

    private var contentState: ContentState {
        ContentState(rows: rows, messagesById: messagesById)
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeUIViewController(context: Context) -> ChatTranscriptHostController {
        let viewController = ChatTranscriptHostController(layout: Self.makeLayout())
        let collectionView = viewController.collectionView
        collectionView.backgroundColor = .clear
        collectionView.contentInsetAdjustmentBehavior = .automatic
        collectionView.alwaysBounceVertical = true
        collectionView.alwaysBounceHorizontal = false
        collectionView.keyboardDismissMode = .interactive
        collectionView.delegate = context.coordinator
        collectionView.showsVerticalScrollIndicator = true
        collectionView.onBoundsSizeChange = { [weak coordinator = context.coordinator] _ in
            coordinator?.handleViewportGeometryChange()
        }
        collectionView.onContentSizeChange = { [weak coordinator = context.coordinator] _ in
            coordinator?.handleContentSizeChange()
        }
        let longPressGesture = UILongPressGestureRecognizer(
            target: context.coordinator,
            action: #selector(Coordinator.handleCollectionLongPress(_:))
        )
        longPressGesture.minimumPressDuration = 0.3
        longPressGesture.cancelsTouchesInView = false
        collectionView.addGestureRecognizer(longPressGesture)
        viewController.onViewportGeometryChange = { [weak coordinator = context.coordinator] in
            coordinator?.handleViewportGeometryChange()
        }
        viewController.onWillDisappear = { [weak coordinator = context.coordinator] in
            coordinator?.persistCurrentScrollPosition()
        }
        viewController.onJumpToBottomTap = { [weak coordinator = context.coordinator] in
            coordinator?.handleJumpButtonTap()
        }

        context.coordinator.collectionView = collectionView
        context.coordinator.viewController = viewController
        context.coordinator.lastContentState = contentState

        let registration = UICollectionView.CellRegistration<UICollectionViewCell, String> {
            [weak coordinator = context.coordinator] cell, _, itemID in
            guard let coordinator, let row = coordinator.rowsByID[itemID] else { return }
            var background = UIBackgroundConfiguration.clear()
            background.backgroundColor = .clear
            cell.backgroundConfiguration = background
            cell.contentConfiguration = UIHostingConfiguration {
                ChatTimelineRowView(
                    row: row,
                    messagesById: coordinator.parent.messagesById,
                    messageFrameRegistry: coordinator.messageFrameRegistry,
                    highlightedMessageID: coordinator.highlightedMessageID,
                    onReact: coordinator.parent.onReact,
                    onDownloadAttachment: coordinator.parent.onDownloadAttachment,
                    onOpenAttachment: coordinator.parent.onOpenAttachment,
                    onVotePoll: coordinator.parent.onVotePoll,
                    onRetryMessage: coordinator.parent.onRetryMessage,
                    onJumpToMessage: { messageID in
                        coordinator.jumpToMessage(messageID)
                    },
                    onLongPressMessage: coordinator.parent.onLongPressMessage
                )
                .environment(\.openURL, OpenURLAction { url in
                    coordinator.parent.onOpenURL(url)
                })
            }
            .minSize(width: 0, height: 0)
            .margins(.all, 0)
        }

        let dataSource = UICollectionViewDiffableDataSource<Int, String>(collectionView: collectionView) {
            collectionView, indexPath, itemID in
            collectionView.dequeueConfiguredReusableCell(
                using: registration,
                for: indexPath,
                item: itemID
            )
        }
        context.coordinator.dataSource = dataSource

        viewController.setJumpButtonVisible(!followsBottom, animated: false)
        context.coordinator.applyViewportInsetsIfNeeded()
        context.coordinator.applyRows(rows, animated: false) {
            context.coordinator.markInitialRowsApplied()
        }

        return viewController
    }

    func updateUIViewController(
        _ viewController: ChatTranscriptHostController,
        context: Context
    ) {
        let performanceInterval = FinitePerformance.begin(
            "Update chat transcript host",
            warningBudgetMilliseconds: 16.67
        )
        defer { FinitePerformance.end(performanceInterval) }
        let coordinator = context.coordinator
        coordinator.parent = self
        coordinator.collectionView = viewController.collectionView
        coordinator.viewController = viewController

        let wasNearBottom = coordinator.isNearBottom()
        let shouldPinToBottom = MessageCollectionLayout.shouldPinToBottom(
            isNearBottom: wasNearBottom,
            followsBottom: followsBottom,
            isHoldingInitialBottomPin: coordinator.isHoldingInitialBottomPin
        )
        let newIDs = rows.map(\.id)
        let updateKind = MessageCollectionLayout.classifyUpdate(
            oldIDs: coordinator.currentIDs,
            newIDs: newIDs
        )
        let isPrepending = coordinator.lastContentState.map {
            MessageCollectionLayout.isStrictPrepend(
                oldIDs: $0.messageIDs,
                newIDs: contentState.messageIDs
            )
        } ?? false
        let anchor = shouldPinToBottom
            ? nil
            : coordinator.captureTopAnchor(edge: isPrepending ? .bottom : .top)
        let contentChanged = coordinator.lastContentState != contentState
        coordinator.lastContentState = contentState

        viewController.setJumpButtonVisible(!followsBottom, animated: true)

        let viewportChanged = coordinator.applyViewportInsetsIfNeeded()

        let completion = {
            if shouldPinToBottom {
                coordinator.scrollToBottom(animated: updateKind == .tailMutation)
            } else if let anchor {
                coordinator.restore(anchor: anchor)
            }
        }

        switch updateKind {
        case .reconfigureOnly:
            let refreshed = contentChanged
                ? coordinator.reconfigureVisibleRows(with: rows, completion: completion)
                : false
            if !refreshed && viewportChanged {
                completion()
            }
        case .tailMutation, .structural:
            coordinator.applyRows(
                rows,
                animated: shouldPinToBottom && updateKind == .tailMutation,
                completion: completion
            )
        }
    }

    static func dismantleUIViewController(
        _ viewController: ChatTranscriptHostController,
        coordinator: Coordinator
    ) {
        coordinator.persistCurrentScrollPosition()
    }

    private static func makeLayout() -> UICollectionViewLayout {
        let itemSize = NSCollectionLayoutSize(
            widthDimension: .fractionalWidth(1),
            heightDimension: .estimated(56)
        )
        let item = NSCollectionLayoutItem(layoutSize: itemSize)
        let group = NSCollectionLayoutGroup.vertical(layoutSize: itemSize, subitems: [item])
        let section = NSCollectionLayoutSection(group: group)
        section.interGroupSpacing = 0
        return UICollectionViewCompositionalLayout(section: section)
    }

    final class Coordinator: NSObject, UICollectionViewDelegate {
        var parent: ChatTranscriptView
        var dataSource: UICollectionViewDiffableDataSource<Int, String>?
        var rowsByID: [String: ChatTimelineRow] = [:]
        var currentIDs: [String] = []
        weak var collectionView: UICollectionView?
        weak var viewController: ChatTranscriptHostController?
        private var requestedOldestId: String?
        private var lastAppliedEffectiveInset: UIEdgeInsets?
        private var pendingInitialScrollPosition: SavedChatTranscriptPosition?
        private var hasAppliedInitialRows = false
        fileprivate var isHoldingInitialBottomPin = false
        var lastContentState: ContentState?
        var pendingViewportAnchor: ScrollAnchor?
        let messageFrameRegistry = ChatMessageFrameRegistry()
        private var lastLongPressFocus: (messageID: String, time: TimeInterval)?
        private(set) var highlightedMessageID: String?

        init(parent: ChatTranscriptView) {
            self.parent = parent
            self.pendingInitialScrollPosition =
                ChatTranscriptScrollPositionStore.shared.position(for: parent.roomID) ?? .bottom
        }

        func applyRows(_ rows: [ChatTimelineRow], animated: Bool, completion: (() -> Void)? = nil) {
            currentIDs = rows.map(\.id)
            rowsByID = Dictionary(uniqueKeysWithValues: rows.map { ($0.id, $0) })
            syncRequestedOldestId()

            var snapshot = NSDiffableDataSourceSnapshot<Int, String>()
            snapshot.appendSections([0])
            snapshot.appendItems(rows.map(\.id), toSection: 0)
            dataSource?.apply(snapshot, animatingDifferences: animated) {
                completion?()
            }
        }

        @discardableResult
        func reconfigureVisibleRows(
            with rows: [ChatTimelineRow],
            completion: (() -> Void)? = nil
        ) -> Bool {
            currentIDs = rows.map(\.id)
            rowsByID = Dictionary(uniqueKeysWithValues: rows.map { ($0.id, $0) })
            syncRequestedOldestId()

            guard let dataSource else { return false }
            let visibleIDs = visibleItemIDs()
            guard !visibleIDs.isEmpty else { return false }

            var snapshot = dataSource.snapshot()
            snapshot.reloadItems(visibleIDs)
            collectionView?.collectionViewLayout.invalidateLayout()
            dataSource.apply(snapshot, animatingDifferences: false) {
                completion?()
            }
            return true
        }

        func scrollToBottom(animated: Bool) {
            guard let collectionView else { return }
            applyEffectiveInsetsIfNeeded()
            collectionView.layoutIfNeeded()
            collectionView.setContentOffset(
                MessageCollectionLayout.bottomContentOffset(
                    contentHeight: collectionView.contentSize.height,
                    boundsHeight: collectionView.bounds.height,
                    topAdjustedInset: collectionView.adjustedContentInset.top,
                    bottomInset: collectionView.contentInset.bottom
                ),
                animated: animated
            )
        }

        @discardableResult
        func applyViewportInsetsIfNeeded() -> Bool {
            applyEffectiveInsetsIfNeeded()
        }

        func handleJumpButtonTap() {
            isHoldingInitialBottomPin = false
            DispatchQueue.main.async {
                self.parent.followsBottom = true
            }
            viewController?.setJumpButtonVisible(false, animated: true)
            scrollToBottom(animated: true)
        }

        func handleViewportGeometryChange() {
            let wasNearBottom = isNearBottom()
            let shouldPinToBottom = MessageCollectionLayout.shouldPinToBottom(
                isNearBottom: wasNearBottom,
                followsBottom: parent.followsBottom,
                isHoldingInitialBottomPin: isHoldingInitialBottomPin
            )
            _ = applyEffectiveInsetsIfNeeded()

            if let pendingInitialScrollPosition {
                guard hasAppliedInitialRows,
                      let viewController,
                      viewController.isViewportReadyForInitialBottomPin
                else { return }
                self.pendingInitialScrollPosition = nil
                applyInitialScrollPosition(pendingInitialScrollPosition)
                return
            }

            if let anchor = pendingViewportAnchor {
                pendingViewportAnchor = nil
                restore(anchor: anchor)
                return
            }

            guard shouldPinToBottom else { return }
            scrollToBottom(animated: false)
        }

        func handleContentSizeChange() {
            _ = applyEffectiveInsetsIfNeeded()

            if pendingInitialScrollPosition != nil {
                handleViewportGeometryChange()
                return
            }

            let shouldPinToBottom = MessageCollectionLayout.shouldPinToBottom(
                isNearBottom: isNearBottom(),
                followsBottom: parent.followsBottom,
                isHoldingInitialBottomPin: isHoldingInitialBottomPin
            )
            guard shouldPinToBottom else { return }
            scrollToBottom(animated: false)
        }

        func markInitialRowsApplied() {
            hasAppliedInitialRows = true
            handleViewportGeometryChange()
        }

        func persistCurrentScrollPosition() {
            guard collectionView != nil else { return }

            let position: SavedChatTranscriptPosition
            if isNearBottom() {
                position = .bottom
            } else if let anchor = captureTopAnchor() {
                position = .anchor(anchor)
            } else {
                return
            }

            ChatTranscriptScrollPositionStore.shared.set(position, for: parent.roomID)
        }

        func captureTopAnchor(edge: ScrollAnchorEdge = .top) -> ScrollAnchor? {
            guard let collectionView,
                  let dataSource,
                  let indexPath = collectionView.indexPathsForVisibleItems
                      .sorted(by: indexPathSort)
                      .first,
                  let itemID = dataSource.itemIdentifier(for: indexPath),
                  let attributes = collectionView.layoutAttributesForItem(at: indexPath)
            else { return nil }

            let itemPosition: CGFloat
            switch edge {
            case .top:
                itemPosition = attributes.frame.minY
            case .bottom:
                itemPosition = attributes.frame.maxY
            }
            return ScrollAnchor(
                itemID: itemID,
                distanceFromContentOffset: itemPosition - collectionView.contentOffset.y,
                edge: edge
            )
        }

        @discardableResult
        func restore(anchor: ScrollAnchor) -> Bool {
            guard let collectionView,
                  let dataSource,
                  let indexPath = dataSource.indexPath(for: anchor.itemID)
            else { return false }

            applyEffectiveInsetsIfNeeded()
            collectionView.layoutIfNeeded()
            collectionView.scrollToItem(at: indexPath, at: .top, animated: false)
            collectionView.layoutIfNeeded()

            guard let attributes = collectionView.layoutAttributesForItem(at: indexPath) else {
                return false
            }

            let minOffsetY = -collectionView.adjustedContentInset.top
            let maxOffsetY = max(
                minOffsetY,
                collectionView.contentSize.height - collectionView.bounds.height + collectionView.contentInset.bottom
            )
            let targetY = MessageCollectionLayout.contentOffsetY(
                preserving: anchor,
                itemFrame: attributes.frame,
                minOffsetY: minOffsetY,
                maxOffsetY: maxOffsetY
            )
            collectionView.setContentOffset(CGPoint(x: 0, y: targetY), animated: false)
            return true
        }

        func collectionView(
            _ collectionView: UICollectionView,
            willDisplay cell: UICollectionViewCell,
            forItemAt indexPath: IndexPath
        ) {
            guard indexPath.item <= 2 else { return }
            guard parent.canLoadOlder else { return }

            let oldestMessageId = parent.rows.first?.oldestMessageID
            guard let oldestMessageId, oldestMessageId != requestedOldestId else { return }
            requestedOldestId = oldestMessageId
            parent.onLoadOlderMessages?(oldestMessageId)
        }

        @objc func handleCollectionLongPress(_ recognizer: UILongPressGestureRecognizer) {
            guard recognizer.state == .began,
                  let collectionView
            else {
                return
            }

            let windowLocation = recognizer.location(in: collectionView.window)
            presentLongPressedMessage(at: windowLocation)
        }

        func collectionView(
            _ collectionView: UICollectionView,
            contextMenuConfigurationForItemAt indexPath: IndexPath,
            point: CGPoint
        ) -> UIContextMenuConfiguration? {
            let windowLocation = collectionView.convert(point, to: collectionView.window)
            presentLongPressedMessage(at: windowLocation)
            return nil
        }

        private func presentLongPressedMessage(at windowLocation: CGPoint) {
            guard let hit = messageFrameRegistry.hitMessage(at: windowLocation) else { return }
            let message = hit.message

            let now = CACurrentMediaTime()
            if let lastLongPressFocus,
               lastLongPressFocus.messageID == message.messageId,
               now - lastLongPressFocus.time < 0.45
            {
                return
            }
            lastLongPressFocus = (message.messageId, now)

            UIImpactFeedbackGenerator(style: .light).impactOccurred()
            parent.onLongPressMessage(message, hit.frame)
        }

        func jumpToMessage(_ messageID: String) {
            guard let collectionView,
                  let dataSource,
                  let rowItemID = rowID(containingMessageID: messageID),
                  let indexPath = dataSource.indexPath(for: rowItemID)
            else { return }

            UIImpactFeedbackGenerator(style: .light).impactOccurred()
            let previousHighlightedRowID = highlightedMessageID.flatMap {
                rowID(containingMessageID: $0)
            }
            highlightedMessageID = messageID
            var rowsToReconfigure: Set<String> = [rowItemID]
            if let previousHighlightedRowID {
                rowsToReconfigure.insert(previousHighlightedRowID)
            }
            reconfigureItemIDs(rowsToReconfigure)
            collectionView.scrollToItem(at: indexPath, at: .centeredVertically, animated: true)
            viewController?.setJumpButtonVisible(!isNearBottom(), animated: true)

            DispatchQueue.main.asyncAfter(deadline: .now() + 1.15) { [weak self] in
                guard let self, self.highlightedMessageID == messageID else { return }
                self.highlightedMessageID = nil
                self.reconfigureItemIDs([rowItemID])
            }
        }

        func scrollViewDidScroll(_ scrollView: UIScrollView) {
            if !isNearTop(scrollView) {
                requestedOldestId = nil
            }

            let nearBottom = isNearBottom()
            if isHoldingInitialBottomPin {
                viewController?.setJumpButtonVisible(false, animated: true)
                return
            }

            let isUserScrolling = [
                scrollView.isDragging,
                scrollView.isDecelerating,
                scrollView.isTracking,
            ].contains(true)
            let nextFollowsBottom = MessageCollectionLayout.nextFollowsBottom(
                current: parent.followsBottom,
                isNearBottom: nearBottom,
                isUserScrolling: isUserScrolling
            )
            viewController?.setJumpButtonVisible(
                MessageCollectionLayout.shouldShowJumpButton(
                    isNearBottom: nearBottom,
                    followsBottom: nextFollowsBottom
                ),
                animated: true
            )
            if nextFollowsBottom != parent.followsBottom {
                DispatchQueue.main.async {
                    self.parent.followsBottom = nextFollowsBottom
                }
            }
        }

        func scrollViewWillBeginDragging(_ scrollView: UIScrollView) {
            isHoldingInitialBottomPin = false
        }

        private func visibleItemIDs() -> [String] {
            guard let collectionView, let dataSource else { return [] }
            return collectionView.indexPathsForVisibleItems
                .sorted(by: indexPathSort)
                .compactMap { dataSource.itemIdentifier(for: $0) }
        }

        private func rowID(containingMessageID messageID: String) -> String? {
            let rows = currentIDs.compactMap { rowsByID[$0] }
            return ChatTimeline.rowID(containingMessageID: messageID, rows: rows)
        }

        private func reconfigureItemIDs(_ itemIDs: Set<String>) {
            guard let dataSource else { return }
            let existing = itemIDs.filter { currentIDs.contains($0) }
            guard !existing.isEmpty else { return }

            var snapshot = dataSource.snapshot()
            snapshot.reconfigureItems(Array(existing))
            dataSource.apply(snapshot, animatingDifferences: false)
        }

        private func applyInitialScrollPosition(_ position: SavedChatTranscriptPosition) {
            switch position {
            case .bottom:
                isHoldingInitialBottomPin = true
                DispatchQueue.main.async {
                    self.parent.followsBottom = true
                }
                viewController?.setJumpButtonVisible(false, animated: false)
                scrollToBottom(animated: false)

            case .anchor(let anchor):
                isHoldingInitialBottomPin = false
                let restored = restore(anchor: anchor)
                DispatchQueue.main.async {
                    self.parent.followsBottom = !restored
                }
                viewController?.setJumpButtonVisible(restored, animated: false)
                if !restored {
                    scrollToBottom(animated: false)
                }
            }
        }

        func isNearBottom() -> Bool {
            guard let collectionView else { return parent.followsBottom }
            return MessageCollectionLayout.isNearBottom(
                contentOffsetY: collectionView.contentOffset.y,
                boundsHeight: collectionView.bounds.height,
                contentHeight: collectionView.contentSize.height,
                topAdjustedInset: collectionView.adjustedContentInset.top,
                bottomInset: collectionView.contentInset.bottom
            )
        }

        @discardableResult
        private func applyEffectiveInsetsIfNeeded() -> Bool {
            guard let collectionView, let viewController else { return false }
            collectionView.layoutIfNeeded()

            let topChromeInset = max(
                0,
                collectionView.adjustedContentInset.top - collectionView.contentInset.top
            )
            let effectiveInset = MessageCollectionLayout.effectiveContentInset(
                boundsHeight: collectionView.bounds.height,
                contentHeight: collectionView.contentSize.height,
                topChromeInset: topChromeInset,
                bottomInset: viewController.bottomViewportInset
            )
            guard effectiveInset != lastAppliedEffectiveInset else { return false }
            lastAppliedEffectiveInset = effectiveInset
            collectionView.contentInset = effectiveInset
            collectionView.verticalScrollIndicatorInsets = .zero
            return true
        }

        private func indexPathSort(_ lhs: IndexPath, _ rhs: IndexPath) -> Bool {
            if lhs.section == rhs.section {
                return lhs.item < rhs.item
            }
            return lhs.section < rhs.section
        }

        private func syncRequestedOldestId() {
            guard let requestedOldestId else { return }
            let currentOldestId = parent.rows.first?.oldestMessageID
            if currentOldestId != requestedOldestId || !parent.canLoadOlder {
                self.requestedOldestId = nil
            }
        }

        private func isNearTop(_ scrollView: UIScrollView, tolerance: CGFloat = 24) -> Bool {
            scrollView.contentOffset.y <= -scrollView.adjustedContentInset.top + tolerance
        }
    }
}

final class ChatTranscriptHostController: UIViewController {
    fileprivate let collectionView: BoundsAwareCollectionView
    private let jumpButtonChromeView = UIVisualEffectView(effect: UIBlurEffect(style: .systemUltraThinMaterial))
    private let jumpButton = UIButton(type: .system)
    private var lastReportedBottomViewportInset: CGFloat = 0
    private var jumpButtonBottomConstraint: NSLayoutConstraint?
    private var isJumpButtonVisible = false

    var onViewportGeometryChange: (() -> Void)?
    var onWillDisappear: (() -> Void)?
    var onJumpToBottomTap: (() -> Void)?

    var bottomViewportInset: CGFloat {
        max(0, view.safeAreaInsets.bottom)
    }

    var isViewportReadyForInitialBottomPin: Bool {
        isViewLoaded && view.window != nil && collectionView.bounds.height > 0
    }

    init(layout: UICollectionViewLayout) {
        self.collectionView = BoundsAwareCollectionView(frame: .zero, collectionViewLayout: layout)
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .clear

        collectionView.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(collectionView)
        NSLayoutConstraint.activate([
            collectionView.topAnchor.constraint(equalTo: view.topAnchor),
            collectionView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            collectionView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            collectionView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])

        configureJumpButton()
        view.bringSubviewToFront(jumpButtonChromeView)
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        DispatchQueue.main.async { [weak self] in
            self?.onViewportGeometryChange?()
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        onWillDisappear?()
    }

    override func viewSafeAreaInsetsDidChange() {
        super.viewSafeAreaInsetsDidChange()
        onViewportGeometryChange?()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        updateJumpButtonBottomConstraint()
        let bottomViewportInset = self.bottomViewportInset
        guard abs(bottomViewportInset - lastReportedBottomViewportInset) > 0.5 else { return }
        lastReportedBottomViewportInset = bottomViewportInset
        onViewportGeometryChange?()
    }

    func setJumpButtonVisible(_ visible: Bool, animated: Bool) {
        guard visible != isJumpButtonVisible else { return }
        isJumpButtonVisible = visible

        let updates = {
            self.jumpButtonChromeView.alpha = visible ? 1 : 0
            self.jumpButtonChromeView.transform = visible
                ? .identity
                : CGAffineTransform(scaleX: 0.9, y: 0.9)
        }

        jumpButtonChromeView.isHidden = false
        jumpButtonChromeView.isUserInteractionEnabled = visible
        jumpButton.accessibilityElementsHidden = !visible

        if animated {
            UIView.animate(
                withDuration: 0.18,
                delay: 0,
                options: [.beginFromCurrentState, .curveEaseInOut]
            ) {
                updates()
            } completion: { _ in
                self.jumpButtonChromeView.isHidden = !visible
            }
        } else {
            updates()
            jumpButtonChromeView.isHidden = !visible
        }
    }

    private func configureJumpButton() {
        jumpButtonChromeView.translatesAutoresizingMaskIntoConstraints = false
        jumpButtonChromeView.layer.cornerRadius = 18
        jumpButtonChromeView.clipsToBounds = true
        jumpButtonChromeView.layer.borderWidth = 0.5
        jumpButtonChromeView.layer.borderColor = UIColor.quaternaryLabel.cgColor
        jumpButtonChromeView.alpha = 0
        jumpButtonChromeView.isHidden = true
        jumpButtonChromeView.isUserInteractionEnabled = false
        view.addSubview(jumpButtonChromeView)

        jumpButton.translatesAutoresizingMaskIntoConstraints = false
        jumpButton.tintColor = .label
        jumpButton.setImage(UIImage(systemName: "arrow.down"), for: .normal)
        jumpButton.setPreferredSymbolConfiguration(
            UIImage.SymbolConfiguration(pointSize: 13, weight: .semibold),
            forImageIn: .normal
        )
        jumpButton.accessibilityLabel = "Scroll to bottom"
        jumpButton.addTarget(self, action: #selector(handleJumpButtonTap), for: .touchUpInside)
        jumpButtonChromeView.contentView.addSubview(jumpButton)

        jumpButtonBottomConstraint = jumpButtonChromeView.bottomAnchor.constraint(
            equalTo: view.bottomAnchor
        )

        guard let jumpButtonBottomConstraint else {
            assertionFailure("jumpButtonBottomConstraint should exist before activation")
            return
        }

        NSLayoutConstraint.activate([
            jumpButtonChromeView.widthAnchor.constraint(equalToConstant: 36),
            jumpButtonChromeView.heightAnchor.constraint(equalToConstant: 36),
            jumpButtonChromeView.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -16),
            jumpButtonBottomConstraint,
            jumpButton.centerXAnchor.constraint(equalTo: jumpButtonChromeView.contentView.centerXAnchor),
            jumpButton.centerYAnchor.constraint(equalTo: jumpButtonChromeView.contentView.centerYAnchor),
        ])
        updateJumpButtonBottomConstraint()
    }

    private func updateJumpButtonBottomConstraint() {
        jumpButtonBottomConstraint?.constant = -MessageCollectionLayout.jumpButtonSpacing
    }

    @objc
    private func handleJumpButtonTap() {
        onJumpToBottomTap?()
    }
}

private final class BoundsAwareCollectionView: UICollectionView {
    var onBoundsSizeChange: ((CGSize) -> Void)?
    var onContentSizeChange: ((CGSize) -> Void)?
    private var lastReportedSize: CGSize = .zero
    private var lastReportedContentSize: CGSize = .zero

    override func layoutSubviews() {
        super.layoutSubviews()
        if contentSize != lastReportedContentSize {
            lastReportedContentSize = contentSize
            onContentSizeChange?(contentSize)
        }
        guard bounds.size != lastReportedSize else { return }
        lastReportedSize = bounds.size
        onBoundsSizeChange?(bounds.size)
    }
}

enum ScrollAnchorEdge {
    case top
    case bottom
}

struct ScrollAnchor {
    let itemID: String
    let distanceFromContentOffset: CGFloat
    var edge: ScrollAnchorEdge = .top
}

private enum SavedChatTranscriptPosition {
    case bottom
    case anchor(ScrollAnchor)
}

@MainActor
private final class ChatTranscriptScrollPositionStore {
    static let shared = ChatTranscriptScrollPositionStore()

    private var positions: [String: SavedChatTranscriptPosition] = [:]

    func position(for roomID: String) -> SavedChatTranscriptPosition? {
        positions[roomID]
    }

    func set(_ position: SavedChatTranscriptPosition, for roomID: String) {
        positions[roomID] = position
    }
}
