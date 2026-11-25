#!/bin/bash
# Installs on a fresh GNU/Linux test host and leaves the installation for subsequent tests.
set -euo pipefail
cd "$(dirname "$0")/.."
file=/etc/KokoroBox/configs/i18n.cfg
[[ ! -e /etc/KokoroBox ]] || { echo "This test requires a fresh installation." >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
prompt='Choose a language'
# Supply a different host preference to ensure the install waits for the user's choice.
printf '%s\n' x 3 0 | KOKOROBOX_LANG=cht timeout 45 script -qec 'bash install.sh' /dev/null >"$work/install"
grep -Fq "$prompt" "$work/install"
grep -Fq 'Enter 1–3' "$work/install"
grep -Fq 'Kokoro subscriptions' "$work/install"
[[ $(cat "$file") == en ]]
printf '0\n' | timeout 15 script -qec kokorobox /dev/null >"$work/menu"
grep -Fq 'Kokoro subscriptions' "$work/menu"
if grep -Fq "$prompt" "$work/menu"; then exit 1; fi
KOKOROBOX_LANG=cht timeout 45 script -qec 'bash install.sh' /dev/null </dev/null >"$work/upgrade"
if grep -Fq "$prompt" "$work/upgrade"; then exit 1; fi
[[ $(cat "$file") == en ]]
echo 'First install, main menu handoff and saved language on upgrade passed.'

# Recompress the genuine core. It still executes and reports the expected version,
# but the package bytes no longer match the pinned upstream release digest.
printf '%s\n' 'rules:' '  - MATCH,DIRECT' >/etc/KokoroBox/yamls/config.yaml
kokorobox start
pid=$(cat /run/kokorobox/core.pid)
sha256sum /etc/KokoroBox/mihomo /usr/local/bin/kokorobox "$file" /etc/KokoroBox/yamls/config.yaml >"$work/before"
gzip -n -1 -c /etc/KokoroBox/mihomo >"$work/repacked.gz"
if KOKOROBOX_MIHOMO_ARCHIVE="$work/repacked.gz" bash install.sh </dev/null >"$work/rejected" 2>&1; then
    echo 'Installer accepted a repacked Mihomo archive' >&2
    exit 1
fi
. scripts/lang/en/install.lang
grep -Fq "$INSTALL_CHECKSUM" "$work/rejected"
sha256sum --check --status "$work/before"
[[ $(cat /run/kokorobox/core.pid) == "$pid" ]]
kokorobox status | grep -qx running
kokorobox stop
rm /etc/KokoroBox/yamls/config.yaml
echo 'A failed archive checksum left the installed files and running core intact.'
