import Accelerate
import AVFoundation
import Speech
import SwiftUI

enum VoiceRecordingPhase: Equatable {
    case recording
    case paused
}

struct VoiceRecordingState: Equatable {
    var phase: VoiceRecordingPhase
    var durationSecs: TimeInterval
    var levels: [Float]
    var transcript: String = ""
}

func voiceRecordingCaption(_ recording: VoiceRecordingState?) -> String {
    recording?.transcript.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
}

enum VoiceRecordingAttachment {
    static let mimeType = "audio/mp4"

    static func filename(now: Date = Date()) -> String {
        "voice_\(Int(now.timeIntervalSince1970)).m4a"
    }

    static func outboundAttachment(data: Data, now: Date = Date()) throws -> OutboundAttachment {
        let filename = filename(now: now)
        guard data.count <= maxComposerAttachmentBytes else {
            throw ComposerAttachmentError.tooLarge(filename: filename)
        }
        return OutboundAttachment(
            filename: filename,
            mimeType: mimeType,
            kind: .voiceNote,
            bytes: data
        )
    }
}

enum VoiceRecordingError: LocalizedError, Equatable {
    case microphoneDenied
    case unavailableInput
    case cannotCreateFile
    case cannotStart
    case exportFailed
    case emptyRecording

    var errorDescription: String? {
        switch self {
        case .microphoneDenied:
            "Microphone access is required to record a voice message."
        case .unavailableInput:
            "No microphone input is available."
        case .cannotCreateFile:
            "Unable to create a voice recording file."
        case .cannotStart:
            "Unable to start voice recording."
        case .exportFailed:
            "Unable to finalize the voice recording."
        case .emptyRecording:
            "Voice recording was empty."
        }
    }
}

@MainActor
final class VoiceRecorder: ObservableObject {
    @Published private(set) var state: VoiceRecordingState?

    private var audioEngine: AVAudioEngine?
    private var audioFile: AVAudioFile?
    private var tempCAFURL: URL?
    private var timer: Timer?
    private var startedAt: Date?
    private var pausedAt: Date?
    private var accumulatedPausedDuration: TimeInterval = 0
    private var speechRecognizer: SFSpeechRecognizer?
    private nonisolated(unsafe) var speechRequest: SFSpeechAudioBufferRecognitionRequest?
    private var speechTask: SFSpeechRecognitionTask?
    private var lastTranscript = ""

    private nonisolated(unsafe) var latestPower: Float = 0

    func startRecording() async throws {
        guard state == nil else { return }
        guard await requestMicrophoneAccess() else {
            throw VoiceRecordingError.microphoneDenied
        }

        let session = AVAudioSession.sharedInstance()
        do {
            try session.setCategory(.playAndRecord, mode: .measurement, options: [
                .duckOthers,
                .defaultToSpeaker
            ])
            try session.setActive(true)
        } catch {
            throw VoiceRecordingError.cannotStart
        }

        let engine = AVAudioEngine()
        let inputNode = engine.inputNode
        let inputFormat = inputNode.outputFormat(forBus: 0)
        guard inputFormat.sampleRate > 0, inputFormat.channelCount > 0 else {
            try? session.setActive(false, options: .notifyOthersOnDeactivation)
            throw VoiceRecordingError.unavailableInput
        }

        let cafURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("finitechat_voice_\(UUID().uuidString).caf")
        let file: AVAudioFile
        do {
            file = try AVAudioFile(forWriting: cafURL, settings: inputFormat.settings)
        } catch {
            try? session.setActive(false, options: .notifyOthersOnDeactivation)
            throw VoiceRecordingError.cannotCreateFile
        }

        startSpeechRecognition()
        inputNode.installTap(onBus: 0, bufferSize: 1_024, format: inputFormat) {
            [weak self, file] buffer, _ in
            try? file.write(from: buffer)
            self?.speechRequest?.append(buffer)
            guard let channelData = buffer.floatChannelData?[0] else { return }
            let frames = buffer.frameLength
            guard frames > 0 else { return }
            var rms: Float = 0
            vDSP_measqv(channelData, 1, &rms, vDSP_Length(frames))
            self?.latestPower = sqrtf(rms)
        }

        do {
            try engine.start()
        } catch {
            inputNode.removeTap(onBus: 0)
            try? FileManager.default.removeItem(at: cafURL)
            try? session.setActive(false, options: .notifyOthersOnDeactivation)
            throw VoiceRecordingError.cannotStart
        }

        audioEngine = engine
        audioFile = file
        tempCAFURL = cafURL
        startedAt = Date()
        pausedAt = nil
        accumulatedPausedDuration = 0
        lastTranscript = ""
        latestPower = 0
        state = VoiceRecordingState(phase: .recording, durationSecs: 0, levels: [])
        startTimer()
    }

    func pauseRecording() {
        guard var next = state, next.phase == .recording else { return }
        audioEngine?.pause()
        next.durationSecs = currentDuration()
        next.phase = .paused
        pausedAt = Date()
        state = next
    }

    func resumeRecording() throws {
        guard var next = state, next.phase == .paused else { return }
        if let pausedAt {
            accumulatedPausedDuration += Date().timeIntervalSince(pausedAt)
        }
        self.pausedAt = nil
        do {
            try audioEngine?.start()
        } catch {
            throw VoiceRecordingError.cannotStart
        }
        next.phase = .recording
        state = next
    }

    func stopRecording() async throws -> URL {
        guard state != nil else { throw VoiceRecordingError.emptyRecording }
        let cafURL = tempCAFURL
        stopEngine()
        guard let cafURL else {
            resetState()
            throw VoiceRecordingError.emptyRecording
        }

        let outputURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("finitechat_voice_\(UUID().uuidString).m4a")
        let exported = await convertToM4A(from: cafURL, to: outputURL)
        try? FileManager.default.removeItem(at: cafURL)
        resetState()

        guard exported else {
            try? FileManager.default.removeItem(at: outputURL)
            throw VoiceRecordingError.exportFailed
        }
        let size = (try? outputURL.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
        guard size > 0 else {
            try? FileManager.default.removeItem(at: outputURL)
            throw VoiceRecordingError.emptyRecording
        }
        return outputURL
    }

    func cancelRecording() {
        stopEngine()
        if let tempCAFURL {
            try? FileManager.default.removeItem(at: tempCAFURL)
        }
        resetState()
    }

    private func startTimer() {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.timerTick()
            }
        }
    }

    private func timerTick() {
        guard var next = state else { return }
        next.durationSecs = currentDuration()
        guard next.phase == .recording else {
            state = next
            return
        }

        let rms = latestPower
        let db = 20 * log10f(max(rms, 0.000_001))
        let normalized = max(0, min(1, (db + 50) / 50))
        next.levels.append(normalized)
        if next.levels.count > 160 {
            next.levels.removeFirst(next.levels.count - 160)
        }
        state = next
    }

    private func currentDuration() -> TimeInterval {
        guard let startedAt else { return 0 }
        let end = pausedAt ?? Date()
        return max(0, end.timeIntervalSince(startedAt) - accumulatedPausedDuration)
    }

    private func stopEngine() {
        timer?.invalidate()
        timer = nil
        stopSpeechRecognition()
        audioEngine?.inputNode.removeTap(onBus: 0)
        audioEngine?.stop()
        audioEngine = nil
        audioFile = nil
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    private func resetState() {
        state = nil
        tempCAFURL = nil
        startedAt = nil
        pausedAt = nil
        accumulatedPausedDuration = 0
        lastTranscript = ""
        latestPower = 0
    }

    private func startSpeechRecognition() {
        let status = SFSpeechRecognizer.authorizationStatus()
        guard status == .authorized || status == .notDetermined else { return }
        guard let recognizer = SFSpeechRecognizer(), recognizer.isAvailable else { return }

        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        request.addsPunctuation = true

        speechRecognizer = recognizer
        speechRequest = request
        speechTask = recognizer.recognitionTask(with: request) { [weak self] result, _ in
            guard let transcript = result?.bestTranscription.formattedString else { return }
            Task { @MainActor [weak self] in
                self?.recordTranscript(transcript)
            }
        }
    }

    private func recordTranscript(_ transcript: String) {
        let trimmed = transcript.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed != lastTranscript else { return }
        lastTranscript = trimmed
        guard var next = state else { return }
        next.transcript = trimmed
        state = next
    }

    private func stopSpeechRecognition() {
        speechRequest?.endAudio()
        speechTask?.cancel()
        speechRequest = nil
        speechTask = nil
        speechRecognizer = nil
    }

    private func requestMicrophoneAccess() async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:
            return true
        case .notDetermined:
            return await withCheckedContinuation { continuation in
                AVCaptureDevice.requestAccess(for: .audio) { granted in
                    continuation.resume(returning: granted)
                }
            }
        case .denied, .restricted:
            return false
        @unknown default:
            return false
        }
    }

    private nonisolated func convertToM4A(from inputURL: URL, to outputURL: URL) async -> Bool {
        let asset = AVAsset(url: inputURL)
        guard let session = AVAssetExportSession(
            asset: asset,
            presetName: AVAssetExportPresetAppleM4A
        ) else {
            return false
        }
        session.outputURL = outputURL
        session.outputFileType = .m4a

        await session.export()
        return session.status == .completed
    }
}

struct VoiceRecordingComposerView: View {
    let recording: VoiceRecordingState
    let isSending: Bool
    let onSend: () -> Void
    let onCancel: () -> Void
    let onTogglePause: () -> Void

    var body: some View {
        VStack(spacing: 8) {
            HStack(spacing: 10) {
                HStack(spacing: 6) {
                    Circle()
                        .fill(Color.red)
                        .frame(width: 8, height: 8)
                        .opacity(recording.phase == .paused ? 0.35 : 1)

                    Text(formattedDuration(recording.durationSecs))
                        .font(.subheadline.monospacedDigit())
                        .foregroundStyle(.primary)
                }
                .frame(width: 70, alignment: .leading)

                LiveVoiceWaveformView(levels: recording.levels.map(CGFloat.init))
                    .frame(height: 28)
            }

            if !voiceRecordingCaption(recording).isEmpty {
                Text(voiceRecordingCaption(recording))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }

            HStack {
                Button(action: onCancel) {
                    Image(systemName: "trash")
                        .font(.body)
                        .frame(width: 36, height: 36)
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .accessibilityLabel("Cancel voice recording")

                Spacer()

                Button(action: onTogglePause) {
                    Image(systemName: recording.phase == .paused ? "record.circle" : "pause.circle.fill")
                        .font(.title2)
                        .frame(width: 36, height: 36)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(recording.phase == .paused ? "Resume voice recording" : "Pause voice recording")

                Spacer()

                Button(action: onSend) {
                    if isSending {
                        ProgressView()
                            .frame(width: 36, height: 36)
                    } else {
                        Image(systemName: "arrow.up.circle.fill")
                            .font(.title2)
                            .frame(width: 36, height: 36)
                    }
                }
                .buttonStyle(.plain)
                .disabled(isSending || recording.durationSecs < 0.2)
                .accessibilityLabel("Send voice recording")
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .accessibilityElement(children: .contain)
    }
}

private struct LiveVoiceWaveformView: View {
    let levels: [CGFloat]

    private let barWidth: CGFloat = 3
    private let barSpacing: CGFloat = 2

    var body: some View {
        GeometryReader { geometry in
            let maxBars = max(1, Int(geometry.size.width / (barWidth + barSpacing)))
            let visibleLevels = levels.suffix(maxBars)
            let height = geometry.size.height

            HStack(alignment: .center, spacing: barSpacing) {
                ForEach(Array(visibleLevels.enumerated()), id: \.offset) { _, level in
                    RoundedRectangle(cornerRadius: 1.5)
                        .fill(Color.accentColor.opacity(0.72))
                        .frame(width: barWidth, height: max(2, level * height))
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .trailing)
        }
    }
}

struct VoiceAttachmentRow: View {
    let attachment: ChatMediaAttachment
    let isMine: Bool
    let onDownload: () -> Void

    @StateObject private var player = VoiceAttachmentPlayer()

    var body: some View {
        if let path = localPath {
            playerRow(path: path)
                .onAppear {
                    player.load(url: URL(fileURLWithPath: path))
                }
        } else {
            downloadRow
        }
    }

    private func playerRow(path: String) -> some View {
        HStack(spacing: 8) {
            Button {
                player.toggle(url: URL(fileURLWithPath: path))
            } label: {
                Image(systemName: player.isPlaying ? "pause.fill" : "play.fill")
                    .font(.body.weight(.semibold))
                    .frame(width: 32, height: 32)
                    .background(
                        Circle().fill(isMine ? .white.opacity(0.18) : Color(uiColor: .systemGroupedBackground))
                    )
            }
            .buttonStyle(.plain)
            .accessibilityLabel(player.isPlaying ? "Pause voice message" : "Play voice message")

            StaticVoiceWaveformView(
                samples: player.waveformSamples,
                progress: player.progress,
                isMine: isMine
            )

            Text(formattedDuration(player.isPlaying ? player.currentTime : player.duration))
                .font(.caption.monospacedDigit())
                .foregroundStyle(isMine ? .white.opacity(0.76) : .secondary)
                .frame(width: 42, alignment: .trailing)
        }
        .foregroundStyle(isMine ? .white : .primary)
        .padding(.horizontal, 10)
        .padding(.vertical, 9)
    }

    private var downloadRow: some View {
        Button {
            if !isDownloading {
                onDownload()
            }
        } label: {
            HStack(spacing: 10) {
                Image(systemName: "waveform")
                    .font(.body.weight(.semibold))
                    .frame(width: 30, height: 30)
                    .background(
                        Circle().fill(isMine ? .white.opacity(0.16) : Color(uiColor: .systemGroupedBackground))
                    )

                VStack(alignment: .leading, spacing: 2) {
                    Text("Voice message")
                        .font(.subheadline.weight(.medium))
                    Text(detailText)
                        .font(.caption)
                        .foregroundStyle(isMine ? .white.opacity(0.72) : .secondary)
                }

                Spacer(minLength: 4)

                if isDownloading {
                    ProgressView()
                        .tint(isMine ? .white : .accentColor)
                } else {
                    Image(systemName: "arrow.down.circle")
                        .font(.body.weight(.semibold))
                        .foregroundStyle(isMine ? .white.opacity(0.82) : .accentColor)
                }
            }
            .foregroundStyle(isMine ? .white : .primary)
        }
        .buttonStyle(.plain)
        .disabled(isDownloading)
    }

    private var localPath: String? {
        attachmentLocalURL(attachment)?.path
    }

    private var isDownloading: Bool {
        attachment.downloadProgressPerMille != nil
    }

    private var detailText: String {
        if isDownloading {
            return "Downloading..."
        }
        return attachment.mimeType.isEmpty ? VoiceRecordingAttachment.mimeType : attachment.mimeType
    }
}

@MainActor
final class VoiceAttachmentPlayer: NSObject, ObservableObject {
    @Published private(set) var isPlaying = false
    @Published private(set) var currentTime: TimeInterval = 0
    @Published private(set) var duration: TimeInterval = 0
    @Published private(set) var progress: CGFloat = 0
    @Published private(set) var waveformSamples: [CGFloat] = []

    private var audioPlayer: AVAudioPlayer?
    private var timer: Timer?
    private var currentURL: URL?

    func load(url: URL) {
        guard waveformSamples.isEmpty || currentURL != url else { return }
        currentURL = url
        waveformSamples = []
        Task.detached(priority: .utility) {
            let samples = VoiceAttachmentPlayer.extractWaveform(from: url, sampleCount: 24)
            await MainActor.run {
                self.waveformSamples = samples
            }
        }
        if let player = try? AVAudioPlayer(contentsOf: url) {
            duration = player.duration
        }
    }

    func toggle(url: URL) {
        if isPlaying, currentURL == url {
            pause()
        } else {
            play(url: url)
        }
    }

    private func play(url: URL) {
        do {
            try AVAudioSession.sharedInstance().setCategory(.playback, mode: .default)
            try AVAudioSession.sharedInstance().setActive(true)
            let player = try AVAudioPlayer(contentsOf: url)
            player.delegate = self
            player.currentTime = currentTime
            player.play()
            audioPlayer = player
            currentURL = url
            duration = player.duration
            isPlaying = true
            startTimer()
        } catch {
            isPlaying = false
        }
    }

    private func pause() {
        currentTime = audioPlayer?.currentTime ?? currentTime
        audioPlayer?.pause()
        isPlaying = false
        stopTimer()
    }

    private func startTimer() {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 1.0 / 15.0, repeats: true) {
            [weak self] _ in
            Task { @MainActor [weak self] in
                self?.updateProgress()
            }
        }
    }

    private func stopTimer() {
        timer?.invalidate()
        timer = nil
    }

    private func updateProgress() {
        guard let audioPlayer else { return }
        currentTime = audioPlayer.currentTime
        duration = audioPlayer.duration
        progress = duration > 0 ? CGFloat(currentTime / duration) : 0
    }

    private func playbackFinished() {
        isPlaying = false
        currentTime = 0
        progress = 0
        stopTimer()
    }

    private nonisolated static func extractWaveform(from url: URL, sampleCount: Int) -> [CGFloat] {
        guard let audioFile = try? AVAudioFile(forReading: url) else { return [] }
        let format = audioFile.processingFormat
        let frameCount = AVAudioFrameCount(audioFile.length)
        guard frameCount > 0,
              let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frameCount)
        else {
            return []
        }
        do {
            try audioFile.read(into: buffer)
        } catch {
            return []
        }
        guard let channelData = buffer.floatChannelData?[0] else { return [] }
        let totalFrames = Int(buffer.frameLength)
        let samplesPerBin = max(1, totalFrames / sampleCount)
        var samples: [CGFloat] = []
        samples.reserveCapacity(sampleCount)

        for index in 0..<sampleCount {
            let start = index * samplesPerBin
            let end = min(start + samplesPerBin, totalFrames)
            guard start < totalFrames else { break }
            var rms: Float = 0
            vDSP_measqv(channelData.advanced(by: start), 1, &rms, vDSP_Length(end - start))
            let db = 20 * log10f(max(sqrtf(rms), 0.000_001))
            samples.append(CGFloat(max(0, min(1, (db + 50) / 50))))
        }
        return samples
    }
}

extension VoiceAttachmentPlayer: AVAudioPlayerDelegate {
    nonisolated func audioPlayerDidFinishPlaying(_ player: AVAudioPlayer, successfully flag: Bool) {
        Task { @MainActor [weak self] in
            self?.playbackFinished()
        }
    }
}

private struct StaticVoiceWaveformView: View {
    let samples: [CGFloat]
    let progress: CGFloat
    let isMine: Bool

    private let barWidth: CGFloat = 3
    private let barSpacing: CGFloat = 2
    private let maxBarHeight: CGFloat = 24

    var body: some View {
        HStack(alignment: .center, spacing: barSpacing) {
            ForEach(Array(displaySamples.enumerated()), id: \.offset) { index, level in
                let isPlayed = CGFloat(index) / CGFloat(max(displaySamples.count, 1)) <= progress
                RoundedRectangle(cornerRadius: 1.5)
                    .fill(barColor(isPlayed: isPlayed))
                    .frame(width: barWidth, height: max(2, level * maxBarHeight))
            }
        }
        .frame(maxWidth: .infinity, minHeight: maxBarHeight, maxHeight: maxBarHeight)
    }

    private var displaySamples: [CGFloat] {
        samples.isEmpty ? Array(repeating: 0.35, count: 24) : samples
    }

    private func barColor(isPlayed: Bool) -> Color {
        if isMine {
            return isPlayed ? .white : .white.opacity(0.38)
        }
        return isPlayed ? .accentColor : Color(uiColor: .tertiaryLabel)
    }
}

func formattedDuration(_ time: TimeInterval) -> String {
    let totalSeconds = max(0, Int(time.rounded(.down)))
    return String(format: "%d:%02d", totalSeconds / 60, totalSeconds % 60)
}
