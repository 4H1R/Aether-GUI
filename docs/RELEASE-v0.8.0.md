# Aether-GUI 0.8.0

Updates the bundled tunnel engine from Aether 1.5.0 to **Aether 2.0.0**.

## Changes

- Add MASQUE-in-MASQUE (two MASQUE hops), HTTP/2 ClientHello fragmentation, an optional local HTTP CONNECT proxy, and an upstream proxy setting.
- Start Auto mode without interactive protocol or transport menus, and show connection progress as soon as the engine starts.
- Require a SOCKS5 handshake before reporting Connected; correctly probe IPv6 wildcard listeners.
- Cancel stale monitors and delayed retries when disconnecting, so they cannot restart or interfere with a later connection.
- Reject invalid listener/proxy settings before launch. Keep upstream credentials and Zero Trust credentials out of saved profiles and process arguments; redact terminal echoes.
- Fetch checksum-pinned core archives on Windows, macOS, and Linux with `npm run fetch:core`. Preserve the bundled `pt/` transport files and use the correct resource path in development and installed apps.
- Run frontend checks, downloader tests, Rust regressions, and a real child-process PTY test before producing installers.

## Downloads

- Windows x64: NSIS setup executable or MSI.
- macOS: separate Apple Silicon and Intel DMGs.
- Linux x64: AppImage, DEB, and RPM.

The GUI exposes WARP-based transports. Tor's support files are included with the core, but this release does not add Tor mode controls to the GUI. Zero Trust sign-in requires your own organization credentials.

## Verification

The default MASQUE HTTP/3 core was tested with a real HTTPS request through SOCKS5 and returned `warp=on`. HTTP/2 did not find a usable gateway on the test network; availability depends on the network and upstream endpoints. See the release's linked build for platform build/test results.
