import Foundation
import OSLog
import QuartzCore

enum FinitePerformance {
    struct Interval {
        fileprivate let name: StaticString
        fileprivate let state: OSSignpostIntervalState
        fileprivate let startedAt: CFTimeInterval
        fileprivate let warningBudgetMilliseconds: Double?
    }

    private static let signposter = OSSignposter(
        subsystem: "computer.finite.finitechat",
        category: "Performance"
    )
    private static let logger = Logger(
        subsystem: "computer.finite.finitechat",
        category: "Performance"
    )
    private static let reportsBudgetOverruns = ProcessInfo.processInfo.arguments.contains(
        "--finitechat-performance-probes"
    )

    static func begin(
        _ name: StaticString,
        warningBudgetMilliseconds: Double? = nil
    ) -> Interval {
        Interval(
            name: name,
            state: signposter.beginInterval(name),
            startedAt: CACurrentMediaTime(),
            warningBudgetMilliseconds: warningBudgetMilliseconds
        )
    }

    static func end(_ interval: Interval) {
        signposter.endInterval(interval.name, interval.state)
        guard reportsBudgetOverruns,
              let budget = interval.warningBudgetMilliseconds
        else {
            return
        }
        let elapsedMilliseconds = (CACurrentMediaTime() - interval.startedAt) * 1_000
        guard elapsedMilliseconds > budget else { return }
        logger.warning(
            "\(String(describing: interval.name), privacy: .public) exceeded its \(budget, privacy: .public) ms budget: \(elapsedMilliseconds, privacy: .public) ms"
        )
    }

    static func recordComposerEdit() {
        guard reportsBudgetOverruns else { return }
        let enqueuedAt = CACurrentMediaTime()
        signposter.emitEvent("Composer edit")
        DispatchQueue.main.async {
            let mainQueueDelayMilliseconds = (CACurrentMediaTime() - enqueuedAt) * 1_000
            guard mainQueueDelayMilliseconds > 16.67 else { return }
            logger.warning(
                "Composer edit missed a 60 Hz frame; next-main-turn delay was \(mainQueueDelayMilliseconds, privacy: .public) ms"
            )
        }
    }
}
