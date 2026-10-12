import NetworkExtension

/// WarpSign's loopback VPN, so installing needs no other app: the same
/// trick as LocalDevVPN and SideStore's StosVPN (MIT), whose reflector this
/// follows.
///
/// The tunnel claims 10.7.0.0/24. WarpSign connects to `fakeIp`
/// (`LOOPBACK_ADDRESS` in the Rust core); each packet comes back with its
/// addresses rewritten so it arrives from `fakeIp` at `deviceIp`, the
/// tunnel's own address. To iOS that's a connection from another machine on
/// a network interface, which lockdownd and RemotePairing answer, as they
/// would a paired computer. Replies are rewritten the same way back.
///
/// Rewriting a source and destination that swap values leaves the IPv4,
/// TCP and UDP checksums correct, as they're sums over both. Nothing leaves
/// the device, and no other traffic uses the tunnel.
final class PacketTunnelProvider: NEPacketTunnelProvider {
    static let deviceIp = "10.7.0.0"
    static let fakeIp = "10.7.0.1"
    static let subnetMask = "255.255.255.0"

    private let deviceIp = PacketTunnelProvider.address(PacketTunnelProvider.deviceIp)
    private let fakeIp = PacketTunnelProvider.address(PacketTunnelProvider.fakeIp)

    override func startTunnel(
        options: [String: NSObject]?,
        completionHandler: @escaping (Error?) -> Void
    ) {
        let settings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: Self.deviceIp)
        let ipv4 = NEIPv4Settings(addresses: [Self.deviceIp], subnetMasks: [Self.subnetMask])
        ipv4.includedRoutes = [NEIPv4Route(destinationAddress: Self.deviceIp, subnetMask: Self.subnetMask)]
        ipv4.excludedRoutes = [.default()]
        settings.ipv4Settings = ipv4

        setTunnelNetworkSettings(settings) { error in
            if error == nil { self.reflect() }
            completionHandler(error)
        }
    }

    override func stopTunnel(
        with reason: NEProviderStopReason,
        completionHandler: @escaping () -> Void
    ) {
        completionHandler()
    }

    private func reflect() {
        packetFlow.readPackets { [weak self] packets, protocols in
            guard let self else { return }
            var reflected = packets
            for index in reflected.indices where Self.isIPv4(protocols[index], reflected[index]) {
                reflected[index].withUnsafeMutableBytes { bytes in
                    let source = Self.read(bytes, at: 12)
                    let destination = Self.read(bytes, at: 16)
                    if source == self.deviceIp { Self.write(self.fakeIp, to: bytes, at: 12) }
                    if destination == self.fakeIp { Self.write(self.deviceIp, to: bytes, at: 16) }
                }
            }
            self.packetFlow.writePackets(reflected, withProtocols: protocols)
            self.reflect()
        }
    }

    private static func isIPv4(_ family: NSNumber, _ packet: Data) -> Bool {
        family.int32Value == AF_INET && packet.count >= 20
    }

    private static func read(_ bytes: UnsafeMutableRawBufferPointer, at offset: Int) -> UInt32 {
        UInt32(bytes[offset]) << 24 | UInt32(bytes[offset + 1]) << 16
            | UInt32(bytes[offset + 2]) << 8 | UInt32(bytes[offset + 3])
    }

    private static func write(_ value: UInt32, to bytes: UnsafeMutableRawBufferPointer, at offset: Int) {
        bytes[offset] = UInt8(value >> 24)
        bytes[offset + 1] = UInt8(value >> 16 & 0xFF)
        bytes[offset + 2] = UInt8(value >> 8 & 0xFF)
        bytes[offset + 3] = UInt8(value & 0xFF)
    }

    private static func address(_ text: String) -> UInt32 {
        text.split(separator: ".").compactMap { UInt32($0) }.reduce(0) { $0 << 8 | $1 }
    }
}
