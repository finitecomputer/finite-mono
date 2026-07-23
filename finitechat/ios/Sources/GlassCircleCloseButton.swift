import SwiftUI

struct GlassCircleCloseButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: "xmark")
                .frame(width: 32, height: 32)
        }
        .buttonStyle(.glass)
        .accessibilityLabel("Close")
    }
}
