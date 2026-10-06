# Builds and releases

Every push to `main` that changes code is built by GitHub Actions for Linux, macOS and Windows, and published on the [Releases page](https://github.com/srlegrand/dcc-xms/releases) as "Build N". Changes to documentation alone do not trigger a build.

| File | Platform |
|---|---|
| `xms-linux-x86_64.tar.gz` | Linux, 64-bit Intel or AMD |
| `xms-windows-x86_64.zip` | Windows, 64-bit |
| `xms-macos-arm64.tar.gz` | macOS, Apple silicon |
| `xms-macos-x86_64.tar.gz` | macOS, Intel |

Each archive holds the program (`xms`, or `xms.exe`), the README and this documentation.

- Linux: needs the ALSA, udev and X11 libraries, which desktop installs have.
- macOS: the program is not signed. On first run, right-click it and choose Open, or run `xattr -d com.apple.quarantine xms`.
- Windows: SmartScreen may warn about an unsigned program.

The workflow is `.github/workflows/release.yml`. It can also be started by hand from the Actions tab. The unit tests run on the Linux build.

## Building from source

    cargo run --release

On Ubuntu or Debian, first:

    sudo apt install build-essential pkg-config libasound2-dev libudev-dev libx11-dev libxkbcommon-x11-0
