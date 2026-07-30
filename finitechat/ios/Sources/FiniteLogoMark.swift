import SwiftUI

struct FiniteLogoMark: Shape {
    private struct Bar {
        let x: CGFloat
        let y: CGFloat
        let width: CGFloat
        let height: CGFloat
        let radius: CGFloat
    }

    private static let bars: [Bar] = [
        Bar(x: 45.3336, y: 69.3335, width: 10.6667, height: 2.66668, radius: 1.33334),
        Bar(x: 15.9998, y: 69.3335, width: 10.6667, height: 2.66668, radius: 1.33334),
        Bar(x: 5.33289, y: 63.999, width: 21.3334, height: 2.66668, radius: 1.33334),
        Bar(x: 45.3336, y: 63.999, width: 21.3334, height: 2.66668, radius: 1.33334),
        Bar(x: 47.1108, y: 58.6675, width: 23.1112, height: 2.66668, radius: 1.33334),
        Bar(x: 1.77722, y: 58.6675, width: 22.2223, height: 2.66668, radius: 1.33334),
        Bar(x: 48.8887, y: 53.3335, width: 23.1112, height: 2.66668, radius: 1.33334),
        Bar(x: 0, y: 53.3335, width: 18.6667, height: 2.66668, radius: 1.33334),
        Bar(x: 49.7778, y: 47.9995, width: 22.2223, height: 2.66668, radius: 1.33334),
        Bar(x: 0, y: 47.9995, width: 13.3334, height: 2.66668, radius: 1.33334),
        Bar(x: 50.6665, y: 42.6655, width: 21.3334, height: 2.66668, radius: 1.33334),
        Bar(x: 0, y: 42.6655, width: 9.77782, height: 2.66668, radius: 1.33334),
        Bar(x: 18.6669, y: 42.6655, width: 6.22225, height: 2.66668, radius: 1.33334),
        Bar(x: 52.4449, y: 37.334, width: 19.5556, height: 2.66668, radius: 1.33334),
        Bar(x: 0, y: 37.334, width: 24.889, height: 2.66668, radius: 1.33334),
        Bar(x: 29.3331, y: 31.9995, width: 3.55557, height: 2.66668, radius: 1.33334),
        Bar(x: 38.2223, y: 31.9995, width: 2.66668, height: 2.66668, radius: 1.33334),
        Bar(x: 45.3336, y: 31.9995, width: 3.55557, height: 2.66668, radius: 1.33334),
        Bar(x: 54.2222, y: 31.9995, width: 17.7779, height: 2.66668, radius: 1.33334),
        Bar(x: 0, y: 31.9995, width: 24.0001, height: 2.66668, radius: 1.33334),
        Bar(x: 56.0006, y: 26.668, width: 16.0001, height: 2.66668, radius: 1.33334),
        Bar(x: 46.2222, y: 26.668, width: 5.33336, height: 2.66668, radius: 1.33334),
        Bar(x: 37.3337, y: 26.668, width: 4.44446, height: 2.66668, radius: 1.33334),
        Bar(x: 28.4452, y: 26.668, width: 3.55557, height: 2.66668, radius: 1.33334),
        Bar(x: 0, y: 26.668, width: 22.2223, height: 2.66668, radius: 1.33334),
        Bar(x: 0, y: 21.334, width: 21.3334, height: 2.66668, radius: 1.33334),
        Bar(x: 47.1115, y: 21.334, width: 24.889, height: 2.66668, radius: 1.33334),
        Bar(x: 26.6667, y: 21.334, width: 5.33336, height: 2.66668, radius: 1.33334),
        Bar(x: 37.3337, y: 21.334, width: 5.33336, height: 2.66668, radius: 1.33334),
        Bar(x: 0, y: 16.0005, width: 21.3334, height: 2.66668, radius: 1.33334),
        Bar(x: 25.3334, y: 16.0005, width: 7.11114, height: 2.66668, radius: 1.33334),
        Bar(x: 37.3337, y: 16.0005, width: 6.22225, height: 2.66668, radius: 1.33334),
        Bar(x: 48, y: 16.0005, width: 24.0001, height: 2.66668, radius: 1.33334),
        Bar(x: 37.3337, y: 10.6655, width: 32.889, height: 2.66668, radius: 1.33334),
        Bar(x: 1.77783, y: 10.6655, width: 30.889, height: 2.66668, radius: 1.33334),
        Bar(x: 5.33289, y: 5.33154, width: 61.3336, height: 2.66668, radius: 1.33334),
        Bar(x: 15.9998, y: 0, width: 40.0002, height: 2.66668, radius: 1.33334),
    ]

    func path(in rect: CGRect) -> Path {
        let side = min(rect.width, rect.height)
        let scale = side / 72
        let origin = CGPoint(
            x: rect.midX - side / 2,
            y: rect.midY - side / 2
        )
        var path = Path()

        for bar in Self.bars {
            path.addRoundedRect(
                in: CGRect(
                    x: origin.x + bar.x * scale,
                    y: origin.y + bar.y * scale,
                    width: bar.width * scale,
                    height: bar.height * scale
                ),
                cornerSize: CGSize(
                    width: bar.radius * scale,
                    height: bar.radius * scale
                )
            )
        }

        return path
    }
}
