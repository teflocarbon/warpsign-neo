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
    /// From "#RRGGBB" (sources' tint colours)
    init?(hex: String?) {
        guard let hex, let value = UInt32(hex.trimmingCharacters(in: CharacterSet(charactersIn: "#")), radix: 16) else {
            return nil
        }
        self.init(
            red: Double((value >> 16) & 0xFF) / 255,
            green: Double((value >> 8) & 0xFF) / 255,
            blue: Double(value & 0xFF) / 255
        )
    }
}
