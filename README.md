<div align="center">

<img src="https://raw.githubusercontent.com/amamiyakokoro/KokoroBox-Desktop/9ab5782f688500f52d5324d28dc596ff0cd77998/images/kokorobox-readme.png" width="128" height="128" alt="KokoroBox">

# KokoroBox-CLI

A GNU/Linux command-line client for Kokoro subscriptions and [Mihomo](https://github.com/MetaCubeX/mihomo).

[Download](https://github.com/amamiyakokoro/KokoroBox-CLI/releases) · [License](LICENSE.txt)

</div>

## Features

- Device-code login over SSH
- Kokoro account subscriptions and external Mihomo links, with manual or scheduled sync
- Local HTTP/SOCKS and optional TUN transparent proxy

## Install

Prebuilt archives support Linux AMD64 and ARM64 on Debian 12+ and Ubuntu 24.04+. Each includes the CLI, installer, and rule data.

```sh
sudo apt-get install bash python3 python3-yaml curl ca-certificates iproute2 util-linux passwd

(
    set -e
    case $(uname -m) in
        x86_64) arch=amd64 ;;
        aarch64|arm64) arch=arm64 ;;
        *) echo "Build from source for this architecture." >&2; exit 1 ;;
    esac
    version=v0.3.1
    archive="KokoroBox-CLI-$version-linux-$arch.tar.gz"
    release="https://github.com/amamiyakokoro/KokoroBox-CLI/releases/download/$version"
    curl -fLO "$release/$archive"
    curl -fLO "$release/SHA256SUMS"
    grep -F "  $archive" SHA256SUMS | sha256sum --check
    tar -xzf "$archive"
    cd "${archive%.tar.gz}"
    sudo env KOKOROBOX_CLI_BIN="$PWD/kokorobox" bash install.sh
)
```

The installer downloads and verifies Mihomo. For offline installation, add `KOKOROBOX_MIHOMO_ARCHIVE=/absolute/path/to/mihomo.gz` to the `sudo env` command and use the official archive for your architecture in the [checksum list](bin/mihomo.sha256). To upgrade, run the installer from the newer release.

## Get started

Run `sudo kokorobox login`, then open the URL shown in the terminal on another device and approve the code.

```sh
sudo kokorobox select   # Choose subscription settings
sudo kokorobox start    # Start Mihomo
sudo kokorobox status   # Check service and subscription status
```

The HTTP/SOCKS proxy listens on `127.0.0.1:7890`. Run `sudo kokorobox` for the interactive menu, or pass a subcommand.

| Subcommand | Purpose |
|---|---|
| `whoami`, `logout` | Check the signed-in account or sign out |
| `subscribe`, `sync` | Import a Kokoro link or refresh the active subscription |
| `auto-sync on` | Enable hourly sync through cron |
| `transparent on` | Enable local transparent proxying with nftables and TUN |
| `stop`, `restart`, `logs`, `test` | Manage and diagnose the service |

## Build from source

Install the runtime dependencies above and Rust/Cargo 1.85 or newer.

```sh
sudo apt-get install git build-essential pkg-config libcurl4-openssl-dev libssl-dev
git clone --depth 1 --branch v0.3.1 https://github.com/amamiyakokoro/KokoroBox-CLI.git
cd KokoroBox-CLI
cargo build --release --locked
sudo env KOKOROBOX_CLI_BIN="$PWD/target/release/kokorobox" bash install.sh
```

## License

Derived from [ShellCrash](https://github.com/juewuy/ShellCrash) under [GPLv3](LICENSE.txt). Mihomo and the bundled rule data retain their respective licenses.
