# Builds and releases

Every push to `main` that changes code is built by GitHub Actions for Linux, macOS and Windows, and published on the [Releases page](https://github.com/MartinoMadeddu/xms-imago/releases) as "Build N". Changes to documentation alone do not trigger a build.

| File | Platform |
|---|---|
| `xms-linux-x86_64.tar.gz` | Linux, 64-bit Intel or AMD |
| `xms-windows-x86_64.zip` | Windows, 64-bit |
| `xms-macos-arm64.tar.gz` | macOS, Apple silicon |
| `xms-macos-x86_64.tar.gz` | macOS, Intel |

Each archive holds the program (`xms`, or `xms.exe`; Cargo builds it as `xms-imago`), the README, this documentation and the `examples` folder. Keep `examples` next to the program: the templates that load files look for it there.

- Linux: needs the ALSA, udev and X11 libraries, which desktop installs have.
- macOS: the program is not signed. On first run, right-click it and choose Open, or run `xattr -d com.apple.quarantine xms`.
- Windows: SmartScreen may warn about an unsigned program.

The workflow is `.github/workflows/release.yml`. It can also be started by hand from the Actions tab. The unit tests run on the Linux build.

## Building from source

    cargo run --release

On Ubuntu or Debian, first:

    sudo apt install build-essential pkg-config libasound2-dev libudev-dev libx11-dev libxkbcommon-x11-0

## Website

The page at https://martinomadeddu.github.io/xms-imago/ is the `site/` folder of the repository, published by `.github/workflows/pages.yml` on every push to `main` that touches the site, the screenshots in `docs/` or the artwork in `assets/`.

The downloads are in four places: a bar that stays at the top of the window, whose button fetches the build for the visitor's machine, a band of four large buttons under the cover, the cover disk, and a second band at the end. Its download buttons point at `releases/latest/download/<file>`, so they always fetch the newest build without the page being touched. The page also names the newest build, from GitHub's public record of it.

The page is laid out as a 3D magazine of the 1990s: a cover, a contents page with the downloads on a cover disk, then a double page in its own colours for each area of the program, a reference card with the keys, the file formats and all 38 nodes, the build instructions as a type-in listing, and the known gaps. Its fonts are in `site/fonts/` and are served with it.

The adverts down the margins are parodies of the period, for products that do not exist: seventeen GIFs in `site/ads/`, six of them animated. They are not all of one kind: 16-colour dithered ones, a four-colour one, an amber text screen, a one-ink newspaper halftone, and 48 to 128 colour ones, one of them a proper raytrace. Three carry a rendered "photograph" given the grain, the cast and the crooked scan of a bad print. Three carry real photographs of Martino and Simon. They are pictures only and cannot be clicked. The margins exist in windows 1640 pixels wide or more, and the adverts grow with them, up to 380 pixels wide; in narrower windows twelve of them sit on a page of their own before the credits.

![The website](xms_site.png)

One-time setup on GitHub: Settings > Pages > Source: "GitHub Actions". With "Deploy from a branch" selected instead, GitHub also publishes the README as a plain page at the same address, and whichever of the two finished last is what visitors get.

To look at the site before pushing:

    sh site/build.sh _site

then open `_site/index.html`.
