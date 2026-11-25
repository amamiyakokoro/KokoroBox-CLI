#!/bin/bash
# Run after a fresh install, before importing a profile.
set -euo pipefail
root=/etc/KokoroBox
work=$(mktemp -d)
file=$root/configs/i18n.cfg
if [[ -f "$file" ]]; then cp "$file" "$work/original"; fi
cleanup() {
    if [[ -f "$work/original" ]]; then cp "$work/original" "$file"; else rm -f "$file"; fi
    rm -rf "$work"
}
trap cleanup EXIT

for choice in 2 1 3; do
    case "$choice" in
        1) lang=chs; first='Kokoro 订阅'; second='启动/重启服务'; login='登录 Kokoro'; help='打开交互菜单' ;;
        2) lang=cht; first='Kokoro 訂閱'; second='啟動／重新啟動服務'; login='登入 Kokoro'; help='開啟互動選單' ;;
        3) lang=en; first='Kokoro subscriptions'; second='Start / Restart Service'; login='Sign in to Kokoro'; help='Interactive menu' ;;
    esac
    # Visit subscriptions before switching so its catalog must be reloaded in this process.
    printf '%s\n' 1 0 3 b "$choice" 0 1 w 0 0 | timeout 20 kokorobox -l >"$work/menu" 2>&1
    sed -E 's/\x1b\[[0-9;]*[mG]//g' "$work/menu" >"$work/plain"
    grep -Fq "1) $first" "$work/plain"
    grep -Fq "2) $second" "$work/plain"
    grep -Fq "l) $login" "$work/plain"
    if grep -Eq 'https?://|Telegram Channel|TG频道|TG頻道' "$work/plain"; then exit 1; fi
    [[ $(cat "$root/configs/i18n.cfg") == "$lang" ]]
    kokorobox --help | grep -Fq "$help"
    printf '%s\n' 3 0 6 0 | timeout 20 kokorobox -l >"$work/sections" 2>&1
    case "$lang" in
        chs) logs=本地日志 ;; cht) logs=本機日誌 ;; en) logs='Local logs' ;;
    esac
    grep -Fq "$logs" "$work/sections"
    if grep -Eiq 'Telegram|PushDeer|Pushover|VMess|Shadowsocks|\bPAC\b|low-flash|小闪存|小快閃' "$work/sections"; then exit 1; fi
    # Option 2 offers Kokoro setup when there is no profile to start.
    printf '%s\n' 2 0 | timeout 20 kokorobox -l >"$work/start" 2>&1
    grep -Fq "$login" "$work/start"
done
echo 'Menu ordering, live language switching, persistence and CLI help passed.'
