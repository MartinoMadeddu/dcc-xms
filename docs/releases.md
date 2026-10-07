# Builds and releases

Every push to `main` that changes code is built by GitHub Actions for Linux, macOS and Windows, and published on the [Releases page](https://github.com/srlegrand/dcc-xms/releases) as "Build N". Changes to documentation alone do not trigger a build.

| File | Platform |
|---|---|
| `xms-linux-x86_64.tar.gz` | Linux, 64-bit Intel or AMD |
| `xms-windows-x86_64.zip` | Windows, 64-bit |
| `xms-macos-arm64.tar.gz` | macOS, Apple silicon |
| `xms-macos-x86_64.tar.gz` | macOS, Intel |

Each archive holds the program (`xms`, or `xms.exe`), the README, this documentation and the `examples` folder. Keep `examples` next to the program: the templates that load files look for it there.

- Linux: needs the ALSA, udev and X11 libraries, which desktop installs have.
- macOS: the program is not signed. On first run, right-click it and choose Open, or run `xattr -d com.apple.quarantine xms`.
- Windows: SmartScreen may warn about an unsigned program.

The workflow is `.github/workflows/release.yml`. It can also be started by hand from the Actions tab. The unit tests run on the Linux build.

## Building from source

    cargo run --release

On Ubuntu or Debian, first:

    sudo apt install build-essential pkg-config libasound2-dev libudev-dev libx11-dev libxkbcommon-x11-0

## Website

The page at https://srlegrand.github.io/dcc-xms/ is the `site/` folder of the repository, published by `.github/workflows/pages.yml` on every push to `main` that touches the site, the screenshots in `docs/` or the artwork in `assets/`.

Its download buttons point at `releases/latest/download/<file>`, so they always fetch the newest build without the page being touched. The page also names the newest build, from GitHub's public record of it.

The page is laid out as a 3D magazine of the 1990s: a cover, a contents page with the downloads on a cover disk, then a double page in its own colours for each area of the program, a reference card with the keys, the file formats and all 38 nodes, the build instructions as a type-in listing, and the known gaps. Its fonts are in `site/fonts/` and are served with it.

![The website](xms_site.png)

One-time setup on GitHub: Settings > Pages > Source: "GitHub Actions".

To look at the site before pushing:

    sh site/build.sh _site

then open `_site/index.html`.
