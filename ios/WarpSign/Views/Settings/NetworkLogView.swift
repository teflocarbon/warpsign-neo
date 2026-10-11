import SwiftUI
import UIKit

/// Recent requests to Apple, newest first
struct NetworkLogView: View {
    @State private var entries: [NetworkLog.Entry] = []

    var body: some View {
        List {
            Section {
                ForEach(entries.reversed()) { entry in
                    VStack(alignment: .leading, spacing: 3) {
                        HStack {
                            Text(entry.method).font(.caption.monospaced().weight(.semibold))
                            Spacer()
                            status(entry.status)
                        }
                        Text(entry.url)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                            .truncationMode(.middle)
                        if let detail = entry.detail {
                            Text(detail)
                                .font(.caption2.monospaced())
                                .foregroundStyle(.red)
                                .lineLimit(6)
                                .textSelection(.enabled)
                        }
                        Text(entry.date, format: .dateTime.hour().minute().second())
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
                    }
                }
            } footer: {
                Text("Kept in memory until WarpSign quits. Cookies and tokens are never recorded.")
            }
        }
        .overlay {
            if entries.isEmpty {
                ContentUnavailableView("No Requests Yet", systemImage: "network")
            }
        }
        .navigationTitle("Network Log")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Menu("Actions", systemImage: "ellipsis.circle") {
                    Button("Copy Log", systemImage: "doc.on.doc") {
                        UIPasteboard.general.string = NetworkLog.shared.transcript
                    }
                    Button("Clear", systemImage: "trash", role: .destructive) {
                        NetworkLog.shared.clear()
                        entries = []
                    }
                }
            }
        }
        .refreshable { entries = NetworkLog.shared.snapshot }
        .onAppear { entries = NetworkLog.shared.snapshot }
    }

    @ViewBuilder
    private func status(_ status: Int?) -> some View {
        if let status {
            Badge("\(status)", color: (200..<400).contains(status) ? .green : .red)
        } else {
            Badge("failed", color: .red)
        }
    }
}
