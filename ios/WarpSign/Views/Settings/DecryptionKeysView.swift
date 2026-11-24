import SwiftUI

/// The keys WarpSign decrypts App Store apps with (DecryptionKeys)
struct DecryptionKeysView: View {
    private var store: DecryptionKeys { .shared }
    @State private var adding = false
    @State private var renaming: DecryptionKeys.Key?
    @State private var newName = ""

    var body: some View {
        List {
            Section {
                ForEach(store.keys) { key in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(key.name)
                        Text("Added \(key.added.formatted(date: .abbreviated, time: .omitted))")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .swipeActions {
                        Button("Remove", role: .destructive) { store.remove(key) }
                        Button("Rename") {
                            newName = key.name
                            renaming = key
                        }
                    }
                }
                Button("Add a Key…", systemImage: "plus") { adding = true }
            } footer: {
                Text("A key decrypts every App Store app its Apple ID got, right here: no helper, nothing to install. Each Apple ID has its own. Get one with UnfairPlay on a Mac signed in to the App Store, or fairplay-decrypt on a jailbroken device. Keys stay in this device's Keychain.")
            }
        }
        .navigationTitle("Decryption Keys")
        .navigationBarTitleDisplayMode(.inline)
        .sheet(isPresented: $adding) { AddDecryptionKeyView() }
        .alert("Rename Key", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("Name", text: $newName)
            Button("Cancel", role: .cancel) {}
            Button("Rename") {
                let name = newName.trimmingCharacters(in: .whitespacesAndNewlines)
                if let renaming, !name.isEmpty { store.rename(renaming, to: name) }
            }
        }
    }
}

/// Add a key: pasted, or from a file
private struct AddDecryptionKeyView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var text = ""
    @State private var fromFile: Data?
    @State private var fileName: String?
    @State private var choosingFile = false
    @State private var error: String?

    private var key: Data? { fromFile ?? DecryptionKeys.parse(text) }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $name, prompt: Text("The Apple ID it's for"))
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                } footer: {
                    Text("Only for you to tell your keys apart.")
                }
                Section {
                    if let fileName {
                        LabeledContent("From", value: fileName)
                        Button("Paste Instead", role: .destructive) {
                            fromFile = nil
                            self.fileName = nil
                        }
                    } else {
                        SecureField("32 hex digits, or base64", text: $text)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .monospaced()
                        Button("Choose a Key File…", systemImage: "doc") { choosingFile = true }
                    }
                } header: {
                    Text("Key")
                } footer: {
                    if !text.isEmpty, fromFile == nil, key == nil {
                        Text("That isn't a key: it's 16 bytes, written as 32 hex digits or base64.")
                            .foregroundStyle(.red)
                    } else {
                        Text("A key file is the 16 bytes themselves, or the key written out.")
                    }
                }
                if let error {
                    Section { ErrorRow(message: error) }
                }
            }
            .navigationTitle("Add a Key")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") { add() }
                        .disabled(key == nil || name.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            // Any file, checked after: key files have no type of their own
            .fileImporter(isPresented: $choosingFile, allowedContentTypes: [.data]) { result in
                guard case .success(let url) = result else { return }
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
                guard size <= 256, let data = try? Data(contentsOf: url),
                      let key = DecryptionKeys.parse(file: data) else {
                    error = "\(url.lastPathComponent) isn't a key: a key file is 16 bytes, or the key written out."
                    return
                }
                error = nil
                fromFile = key
                fileName = url.lastPathComponent
            }
        }
    }

    private func add() {
        guard let key else { return }
        do {
            try DecryptionKeys.shared.add(name: name.trimmingCharacters(in: .whitespaces), key: key)
            dismiss()
        } catch {
            self.error = error.userMessage
        }
    }
}
