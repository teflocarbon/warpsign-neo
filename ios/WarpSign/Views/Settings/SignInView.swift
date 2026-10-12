import SwiftUI

/// Apple ID sign-in, then two-factor authentication
struct SignInView: View {
    @Environment(AccountStore.self) private var account
    @Environment(\.dismiss) private var dismiss

    @State private var appleId = ""
    @State private var password = ""
    @State private var remember = true
    @State private var challenge: TwoFactorInfo?
    @State private var code = ""
    /// Set once a code was sent by text message
    @State private var smsPhone: TrustedPhoneInfo?
    @State private var working = false
    @State private var error: String?
    /// Apple refused the sign-in: retrying can lock the Apple ID
    @State private var refused = false

    var body: some View {
        NavigationStack {
            Form {
                if let challenge {
                    codeSection(challenge)
                } else {
                    credentialsSection
                }
                if let error {
                    Section {
                        ErrorRow(message: error)
                        if refused {
                            Text("Don't try again straight away: repeated attempts can lock your Apple ID. Check you can sign in at developer.apple.com first.")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }
            .navigationTitle(challenge == nil ? "Sign In" : "Verification Code")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    if working {
                        ProgressView()
                    } else if challenge == nil {
                        Button("Sign In") { signIn() }
                            .disabled(appleId.isEmpty || password.isEmpty)
                    } else {
                        Button("Verify") { verify() }
                            .disabled(code.count < 6)
                    }
                }
            }
            .interactiveDismissDisabled(working)
        }
        .onAppear {
            appleId = account.appleId ?? ""
            remember = account.rememberPassword
        }
    }

    private var credentialsSection: some View {
        Section {
            TextField("Apple ID", text: $appleId)
                .textContentType(.username)
                .keyboardType(.emailAddress)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            SecureField("Password", text: $password)
                .textContentType(.password)
                .onSubmit { signIn() }
            Toggle("Remember Password", isOn: $remember)
        } footer: {
            if remember {
                Text("Your password goes only to Apple. It's kept in this device's Keychain, with the session, so WarpSign can sign in again by itself when Apple ends the session, without a verification code while Apple trusts this device.")
            } else {
                Text("Your password goes only to Apple, and isn't stored. WarpSign keeps the signed-in session in this device's Keychain; when Apple ends it, you'll sign in again.")
            }
        }
    }

    @ViewBuilder
    private func codeSection(_ challenge: TwoFactorInfo) -> some View {
        Section {
            TextField("Code", text: $code)
                .textContentType(.oneTimeCode)
                .keyboardType(.numberPad)
                .font(.title2.monospacedDigit())
                .onChange(of: code) { _, value in
                    let digits = String(value.filter(\.isNumber).prefix(Int(challenge.codeLength)))
                    if digits != value { code = digits }
                    if digits.count == Int(challenge.codeLength) && !working { verify() }
                }
        } footer: {
            if let phone = smsPhone ?? challenge.smsSentTo {
                Text("Enter the code sent to \(phone.number).")
            } else if challenge.trustedDevices {
                Text("Enter the code shown on your other Apple devices.")
            }
        }
        if !challenge.phoneNumbers.isEmpty {
            Section {
                ForEach(challenge.phoneNumbers, id: \.id) { phone in
                    Button("Text a Code to \(phone.number)") { sendSms(to: phone) }
                        .disabled(working)
                }
            } footer: {
                Text("Didn't get a code?")
            }
        }
    }

    private func signIn() {
        guard !working, !appleId.isEmpty, !password.isEmpty else { return }
        working = true
        error = nil
        refused = false
        Task {
            defer { working = false }
            do {
                if let challenge = try await account.signIn(appleId: appleId, password: password, remember: remember) {
                    self.challenge = challenge
                    smsPhone = challenge.smsSentTo
                } else {
                    dismiss()
                }
                password = ""
            } catch {
                self.error = error.userMessage
                refused = error.userMessage.contains("refused")
            }
        }
    }

    private func sendSms(to phone: TrustedPhoneInfo) {
        working = true
        error = nil
        Task {
            defer { working = false }
            do {
                try await account.requestSms(phone)
                smsPhone = phone
                code = ""
            } catch {
                self.error = error.userMessage
            }
        }
    }

    private func verify() {
        guard !working else { return }
        working = true
        error = nil
        Task {
            defer { working = false }
            do {
                try await account.verify(code: code, phone: smsPhone)
                dismiss()
            } catch {
                self.error = error.userMessage
                code = ""
            }
        }
    }
}
