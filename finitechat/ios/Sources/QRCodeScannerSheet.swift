import AVFoundation
import SwiftUI
import UIKit

struct QRCodeScannerSheet: View {
    static var canUseCamera: Bool {
        !ProcessInfo.processInfo.isiOSAppOnMac && AVCaptureDevice.default(for: .video) != nil
    }

    let onScanned: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var authStatus = AVCaptureDevice.authorizationStatus(for: .video)
    @State private var scannerNonce = UUID()

    var body: some View {
        NavigationStack {
            VStack(spacing: 12) {
                content
                    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                    .overlay {
                        RoundedRectangle(cornerRadius: 12, style: .continuous)
                            .stroke(.secondary.opacity(0.25), lineWidth: 1)
                    }

                Spacer(minLength: 0)
            }
            .padding(16)
            .navigationTitle("Scan Code")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    GlassCircleCloseButton { dismiss() }
                }
            }
            .onAppear {
                ensureCameraPermission()
            }
            .onReceive(NotificationCenter.default.publisher(for: UIApplication.didBecomeActiveNotification)) { _ in
                refreshCameraAuthorization()
            }
        }
    }

    @ViewBuilder
    private var content: some View {
        switch authStatus {
        case .authorized:
            QRCodeScannerView { value in
                onScanned(value)
                dismiss()
            }
            .id(scannerNonce)
            .frame(maxWidth: .infinity)
            .aspectRatio(1, contentMode: .fit)

        case .notDetermined:
            ProgressView("Requesting camera permission...")
                .frame(maxWidth: .infinity, minHeight: 240)

        case .denied, .restricted:
            CameraPermissionUnavailableView()
            .frame(maxWidth: .infinity, minHeight: 240)

        @unknown default:
            ContentUnavailableView("Camera Unavailable", systemImage: "camera.fill")
                .frame(maxWidth: .infinity, minHeight: 240)
        }
    }

    private func ensureCameraPermission() {
        let status = AVCaptureDevice.authorizationStatus(for: .video)
        authStatus = status
        guard status == .notDetermined else { return }

        AVCaptureDevice.requestAccess(for: .video) { granted in
            DispatchQueue.main.async {
                authStatus = granted ? .authorized : .denied
                scannerNonce = UUID()
            }
        }
    }

    private func refreshCameraAuthorization() {
        authStatus = AVCaptureDevice.authorizationStatus(for: .video)
        scannerNonce = UUID()
    }
}

struct QRCodeScannerPanel: View {
    static let cardCornerRadius: CGFloat = 28

    let cornerRadius: CGFloat
    let expandsVertically: Bool
    let onScanned: (String) -> Void

    @State private var authStatus = AVCaptureDevice.authorizationStatus(for: .video)
    @State private var scannerNonce = UUID()

    init(
        cornerRadius: CGFloat = QRCodeScannerPanel.cardCornerRadius,
        expandsVertically: Bool = false,
        onScanned: @escaping (String) -> Void
    ) {
        self.cornerRadius = cornerRadius
        self.expandsVertically = expandsVertically
        self.onScanned = onScanned
    }

    var body: some View {
        content
            .clipShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .strokeBorder(Color(.separator).opacity(0.28), lineWidth: 1)
            }
            .onAppear {
                ensureCameraPermission()
            }
            .onReceive(NotificationCenter.default.publisher(for: UIApplication.didBecomeActiveNotification)) { _ in
                refreshCameraAuthorization()
            }
    }

    @ViewBuilder
    private var content: some View {
        if !QRCodeScannerSheet.canUseCamera {
            sizedPanel {
                unavailableCameraView
            }
        } else {
            switch authStatus {
            case .authorized:
                sizedPanel {
                    ZStack {
                        QRCodeScannerView(onCode: onScanned)
                            .id(scannerNonce)

                        ScannerReticle()
                            .stroke(.white.opacity(0.92), style: StrokeStyle(lineWidth: 4, lineCap: .round, lineJoin: .round))
                            .frame(width: 184, height: 184)
                            .shadow(color: .black.opacity(0.35), radius: 4, x: 0, y: 2)
                            .accessibilityHidden(true)
                    }
                }

            case .notDetermined:
                sizedPanel {
                    ProgressView("Requesting camera permission...")
                }

            case .denied, .restricted:
                sizedPanel {
                    cameraPermissionView
                }

            @unknown default:
                sizedPanel {
                    unavailableCameraView
                }
            }
        }
    }

    @ViewBuilder
    private func sizedPanel<Content: View>(@ViewBuilder content: () -> Content) -> some View {
        if expandsVertically {
            content()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            content()
                .frame(maxWidth: .infinity)
                .aspectRatio(0.78, contentMode: .fit)
        }
    }

    private var unavailableCameraView: some View {
        ContentUnavailableView(
            "Camera Unavailable",
            systemImage: "camera.fill",
            description: Text("Paste the profile code instead.")
        )
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(.secondarySystemGroupedBackground))
    }

    private var cameraPermissionView: some View {
        CameraPermissionUnavailableView()
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(.secondarySystemGroupedBackground))
    }

    private func ensureCameraPermission() {
        guard QRCodeScannerSheet.canUseCamera else { return }
        let status = AVCaptureDevice.authorizationStatus(for: .video)
        authStatus = status
        guard status == .notDetermined else { return }

        AVCaptureDevice.requestAccess(for: .video) { granted in
            DispatchQueue.main.async {
                authStatus = granted ? .authorized : .denied
                scannerNonce = UUID()
            }
        }
    }

    private func refreshCameraAuthorization() {
        guard QRCodeScannerSheet.canUseCamera else { return }
        authStatus = AVCaptureDevice.authorizationStatus(for: .video)
        scannerNonce = UUID()
    }
}

private struct CameraPermissionUnavailableView: View {
    @Environment(\.openURL) private var openURL

    var body: some View {
        ContentUnavailableView {
            Label("Camera Permission Needed", systemImage: "camera.fill")
        } description: {
            Text("Allow camera access in Settings to scan profile QR codes.")
        } actions: {
            Button("Open Settings") {
                openSettings()
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
        }
    }

    private func openSettings() {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        openURL(url)
    }
}

private struct ScannerReticle: Shape {
    func path(in rect: CGRect) -> Path {
        let cornerLength = min(rect.width, rect.height) * 0.22
        var path = Path()

        path.move(to: CGPoint(x: rect.minX, y: rect.minY + cornerLength))
        path.addLine(to: CGPoint(x: rect.minX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.minX + cornerLength, y: rect.minY))

        path.move(to: CGPoint(x: rect.maxX - cornerLength, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.minY + cornerLength))

        path.move(to: CGPoint(x: rect.maxX, y: rect.maxY - cornerLength))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY))
        path.addLine(to: CGPoint(x: rect.maxX - cornerLength, y: rect.maxY))

        path.move(to: CGPoint(x: rect.minX + cornerLength, y: rect.maxY))
        path.addLine(to: CGPoint(x: rect.minX, y: rect.maxY))
        path.addLine(to: CGPoint(x: rect.minX, y: rect.maxY - cornerLength))

        return path
    }
}

private struct QRCodeScannerView: UIViewControllerRepresentable {
    let onCode: (String) -> Void

    func makeUIViewController(context: Context) -> QRCodeScannerViewController {
        let viewController = QRCodeScannerViewController()
        viewController.onCode = onCode
        return viewController
    }

    func updateUIViewController(_ uiViewController: QRCodeScannerViewController, context: Context) {}
}

private final class QRCodeScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    var onCode: ((String) -> Void)?

    private let session = AVCaptureSession()
    private var previewLayer: AVCaptureVideoPreviewLayer?
    private var didEmit = false

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black

        guard let device = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: device),
              session.canAddInput(input)
        else {
            return
        }
        session.addInput(input)

        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else { return }
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: DispatchQueue.main)
        output.metadataObjectTypes = [.qr]

        let layer = AVCaptureVideoPreviewLayer(session: session)
        layer.videoGravity = .resizeAspectFill
        previewLayer = layer
        view.layer.addSublayer(layer)
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        previewLayer?.frame = view.bounds
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        didEmit = false
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.session.startRunning()
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.session.stopRunning()
        }
    }

    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        guard !didEmit else { return }
        guard let object = metadataObjects.first as? AVMetadataMachineReadableCodeObject,
              object.type == .qr,
              let value = object.stringValue,
              !value.isEmpty
        else {
            return
        }
        didEmit = true
        onCode?(value)
    }
}
