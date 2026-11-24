import SwiftUI
import UIKit

/// An app icon with the iOS corner shape, or a placeholder
struct AppIconView: View {
    let image: UIImage?
    var size: CGFloat = 52

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
            } else {
                ZStack {
                    LinearGradient(
                        colors: [.gray.opacity(0.35), .gray.opacity(0.2)],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                    Image(systemName: "app.dashed")
                        .font(.system(size: size * 0.45))
                        .foregroundStyle(.secondary)
                }
            }
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: size * 0.225, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: size * 0.225, style: .continuous)
                .strokeBorder(.quaternary, lineWidth: 0.5)
        }
    }
}

/// An app icon from a URL (sources)
struct RemoteIconView: View {
    let url: String?
    var size: CGFloat = 52

    var body: some View {
        AsyncImage(url: url.flatMap(URL.init(string:))) { phase in
            switch phase {
            case .success(let image):
                image.resizable().scaledToFill()
            default:
                ZStack {
                    Color.gray.opacity(0.2)
                    Image(systemName: "app.dashed")
                        .font(.system(size: size * 0.45))
                        .foregroundStyle(.secondary)
                }
            }
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: size * 0.225, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: size * 0.225, style: .continuous)
                .strokeBorder(.quaternary, lineWidth: 0.5)
        }
    }
}

struct Badge: View {
    let text: String
    let color: Color

    init(_ text: String, color: Color) {
        self.text = text
        self.color = color
    }

    var body: some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(color)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(color.opacity(0.15), in: .capsule)
    }
}

/// A message about something that went wrong
struct ErrorRow: View {
    let message: String

    var body: some View {
        Label {
            Text(message).textSelection(.enabled)
        } icon: {
            Image(systemName: "exclamationmark.triangle.fill")
        }
        .foregroundStyle(.red)
    }
}

extension Color {
    /// From "#RRGGBB" (sources' tint colours). Where it would be hard to
    /// read on the background, WarpSign's accent colour instead: sources
    /// pick one colour for both modes, and many pick black.
    init?(hex: String?) {
        guard let hex, let value = UInt32(hex.trimmingCharacters(in: CharacterSet(charactersIn: "#")), radix: 16) else {
            return nil
        }
        let red = Double((value >> 16) & 0xFF) / 255
        let green = Double((value >> 8) & 0xFF) / 255
        let blue = Double(value & 0xFF) / 255
        let color = UIColor(red: red, green: green, blue: blue, alpha: 1)
        // WCAG relative luminance, and its contrast with black or white
        func linear(_ channel: Double) -> Double {
            channel <= 0.03928 ? channel / 12.92 : pow((channel + 0.055) / 1.055, 2.4)
        }
        let luminance = 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
        let onBlack = (luminance + 0.05) / 0.05
        let onWhite = 1.05 / (luminance + 0.05)
        self.init(uiColor: UIColor { traits in
            let readable = traits.userInterfaceStyle == .dark ? onBlack >= 3 : onWhite >= 3
            return readable ? color : (UIColor(named: "AccentColor") ?? .systemBlue)
        })
    }
}
