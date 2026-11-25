#!/bin/bash
# Copyright (C) Juewuy and KokoroBox contributors
set -euo pipefail
archive_source=${KOKOROBOX_MIHOMO_ARCHIVE:-}
if [[ -n $archive_source && $archive_source != /* ]]; then archive_source=$PWD/$archive_source; fi
cd -- "$(dirname -- "$0")"
. scripts/libs/i18n.sh
dest=/etc/KokoroBox
lang_file="$dest/configs/i18n.cfg"
i18n=$(language_code "${KOKOROBOX_LANG:-${LC_ALL:-${LC_MESSAGES:-${LANG:-en}}}}")
if [[ -s "$lang_file" ]]; then
    i18n=$(language_code "$(cat "$lang_file")")
fi
. "scripts/lang/$i18n/install.lang"
if [[ $(uname -s) != Linux || $(uname -o) != GNU/Linux || -e /etc/openwrt_release ]]; then
    echo "$INSTALL_LINUX" >&2
    exit 1
fi
(( EUID == 0 )) || { echo "$INSTALL_ROOT" >&2; exit 1; }
open_menu=false
initial_language=
if [[ ! -s "$lang_file" ]]; then
    if [[ -t 0 && -t 1 ]]; then
        initial_language=$(choose_language) || exit 1
        open_menu=true
    elif [[ -n ${KOKOROBOX_LANG:-} ]]; then
        initial_language=$(language_code "$KOKOROBOX_LANG")
    fi
    if [[ -n "$initial_language" ]]; then
        . "scripts/lang/$initial_language/install.lang"
    fi
fi
required=(python3 ip getent groupadd gzip sha256sum)
if [[ -z $archive_source ]]; then required+=(curl); fi
for tool in "${required[@]}"; do
    command -v "$tool" >/dev/null || { echo "$INSTALL_MISSING: $tool" >&2; exit 1; }
done
python3 -c "import yaml" 2>/dev/null || { echo "$INSTALL_MISSING: python3-yaml" >&2; exit 1; }
. scripts/libs/check_cpucore.sh
arch=$(linux_core_arch) || { echo "$INSTALL_ARCH: $(uname -m)" >&2; exit 1; }
# Validate the native entry point before stopping an installed service.
cli_build=$(mktemp -d)
trap 'rm -rf "$cli_build"' EXIT
cli=${KOKOROBOX_CLI_BIN:-}
if [[ -z $cli ]]; then
    command -v cargo >/dev/null || { echo "$INSTALL_MISSING: cargo (or KOKOROBOX_CLI_BIN)" >&2; exit 1; }
    cargo build --locked --release --target-dir "$cli_build"
    cli="$cli_build/release/kokorobox"
fi
[[ $("$cli" --version) == "kokorobox $(cat version)" ]] || exit 1
umask 077
install -d -m 700 "$dest" /run/kokorobox
install -d "$dest"/{configs,yamls,ruleset}
# The same Mihomo binary validates subscriptions and runs the proxy.
stage=$(mktemp "$dest/.mihomo.XXXXXXXX")
trap 'rm -rf "$cli_build"; rm -f "$stage" "${cli_stage:-}"' EXIT
. bin/version
asset="mihomo-linux-$arch-$mihomo_version.gz"
archive="$cli_build/$asset"
if [[ -n $archive_source ]]; then
    cp -- "$archive_source" "$archive"
else
    echo "$INSTALL_DOWNLOAD $asset"
    curl --disable --proto '=https' --proto-redir '=https' --fail --silent --show-error --location \
        --retry 2 --connect-timeout 15 --max-time 300 --output "$archive" \
        "https://github.com/MetaCubeX/mihomo/releases/download/$mihomo_version/$asset"
fi
# Verify the private copy, including when an archive is supplied offline.
checksum=$(awk -v asset="$asset" '$2 == asset { print }' bin/mihomo.sha256)
if [[ -z $checksum ]] || ! (cd "$cli_build" && sha256sum --check --status - <<<"$checksum"); then
    echo "$INSTALL_CHECKSUM $asset" >&2
    exit 1
fi
gzip -dc -- "$archive" >"$stage"
chmod 755 "$stage"
"$stage" -v >/dev/null
# The old firewall teardown knows the combined manual and inbound port rules.
resume_service=false
if [[ -f "$dest/start.sh" ]]; then
    if [[ -f "$dest/starts/fw_stop.sh" ]]; then
        if grep -Eq '^firewall_area=[23]$' "$dest/configs/KokoroBox.cfg"; then
            if [[ ! -c /dev/net/tun ]] || ! command -v nft >/dev/null || ! nft list tables >/dev/null; then
                echo "$INSTALL_MISSING: /dev/net/tun, nftables (CAP_NET_ADMIN)" >&2
                exit 1
            fi
        fi
        # Migrate schedules before the old stop path removes running jobs.
        python3 scripts/schedule.py migrate
        if pidof KokoroCore >/dev/null; then
            "$dest/start.sh" stop
            resume_service=true
        elif [[ -f /run/kokorobox/firewall-active ]]; then
            "$dest/starts/fw_stop.sh" || [[ ! -e /run/kokorobox/firewall-active ]]
        fi
    elif python3 "$dest/runtime.py" running; then
        "$dest/start.sh" stop
        resume_service=true
    fi
    if [[ -f /run/kokorobox/host-route && -x "$dest/starts/host-route.sh" ]]; then
        "$dest/starts/host-route.sh" stop
    fi
elif [[ -f "$dest/mihomo" && -x /usr/local/bin/kokorobox ]]; then
    if /usr/local/bin/kokorobox --core-running; then
        /usr/local/bin/kokorobox stop
        resume_service=true
    fi
    if [[ -f /run/kokorobox/host-route ]]; then
        /usr/local/bin/kokorobox --host-route stop
    fi
fi
if [[ -f "$dest/menus/bot_tg.sh" ]]; then
    # A bot can still be running after a standalone start or a core failure.
    if [[ -f /run/kokorobox/bot_tg.pid ]] && read -r bot_pid </run/kokorobox/bot_tg.pid &&
        [[ $bot_pid =~ ^[1-9][0-9]*$ && -r /proc/$bot_pid/cmdline ]] &&
        grep -Fzxq -- "$dest/menus/bot_tg.sh" "/proc/$bot_pid/cmdline"; then
        kill -TERM "$bot_pid" || [[ ! -e /proc/$bot_pid ]]
    fi
    rm -f /run/kokorobox/{bot_tg.pid,tgbot.log,jsons/vmess-in.json,jsons/ss-in.json}
    for cfg in "$dest/configs/"{KokoroBox,gateway}.cfg; do
        [[ -f $cfg ]] || continue
        sed -i -E '/^(TG_(TOKEN|CHATID|menupush)|chat_ID|bot_tg_service|device_name|task_push|push_(TG|Deer(_url)?|bark|Po(_key)?|PP|SynoChat|Gotify|Chat(URL|TOKEN|USERID))|vms_(host|port|service|uuid|ws_path)|sss_(cipher|port|pwd|service))=/d' "$cfg"
    done
fi
cp -R scripts/. "$dest/"
install -m 644 version LICENSE.txt "$dest/"
# Retire packaged modules without touching user profiles, custom lists, or JSON files.
rm -f "$dest/start.sh" "$dest"/starts/{host-route,run-core}.sh
rm -f "$dest"/{kokoro,kokoro_auth}.py "$dest"/__pycache__/{kokoro,kokoro_auth}.*.pyc
rm -f "$dest"/libs/{README.md,check_autostart.sh,check_cmd.sh,check_dir_avail.sh,check_target.sh,compare.sh} \
    "$dest"/libs/{core_tools.sh,gen_base64.sh,get_config.sh,get_ecsip.sh,logger.sh,meta_listeners.sh} \
    "$dest"/libs/{sb_endpoints.sh,sb_inbounds.sh,set_config.sh,set_cron.sh,set_proxy.sh,start_wait.sh} \
    "$dest"/libs/{urlencode.sh,web_get.sh,web_get_bin.sh,web_get_curlbar.sh,web_get_lite.sh,web_json.sh} \
    "$dest"/libs/{web_restore.sh,web_save.sh} \
    "$dest"/menus/{1_start.sh,2_settings.sh,4_setboot.sh,5_task.sh,6_core_config.sh,7_gateway.sh} \
    "$dest"/menus/{8_tools.sh,9_upgrade.sh,README.md,bot_tg.sh,bot_tg_bind.sh,bot_tg_service.sh} \
    "$dest"/menus/{check_port.sh,common.sh,dns.sh,fw_filter.sh,override.sh,providers.sh} \
    "$dest"/menus/{providers_clash.sh,providers_singbox.sh,running_status.sh,subconverter.sh,task_cmd.sh,tui_layout.sh} \
    "$dest"/menus/{tui_lite.sh,uninstall.sh,userguide.sh} \
    "$dest"/starts/{README.md,afstart.sh,bfstart.sh,check_cnip.sh,check_core.sh,check_geo.sh} \
    "$dest"/starts/{check_network.sh,clash_check.sh,clash_config_check.sh,clash_modify.sh,core_config.sh,core_exchange.sh} \
    "$dest"/starts/{fw_getlanip.sh,fw_iptables.sh,fw_ipv6_watch.sh,fw_nftables.sh,fw_start.sh,fw_stop.sh} \
    "$dest"/starts/{general_init.sh,singbox_check.sh,singbox_config_check.sh,singbox_modify.sh,start_error.sh,start_legacy.sh} \
    "$dest"/starts/start_legacy_wd.sh \
    "$dest"/task/task.sh \
    "$dest"/ui/pac
for locale in chs cht en; do
    for catalog in 1_start 2_settings 5_task 6_core_config 7_gateway 8_tools 9_upgrade bot_tg check_port common dns fw_filter override providers runtime setboot subconverter task_cmd userguide; do
        rm -f "$dest/lang/$locale/$catalog.lang"
    done
done
rm -f "$dest"/task/task_{chs,cht,en}.list
install -m 644 bin/geodata/Country.mmdb "$dest/Country.mmdb"
install -m 644 bin/geodata/geosite.dat "$dest/GeoSite.dat"
install -m 644 bin/geodata/mrs_geosite_cn.mrs "$dest/ruleset/cn.mrs"
# Keep existing core data when upgrading an installation that used external storage.
if [[ -f "$dest/configs/directories.env" ]]; then
    (
        BINDIR=$(sed -n 's/^BINDIR=//p' "$dest/configs/directories.env" | tail -n 1)
        if [[ $BINDIR = /* && -d $BINDIR && ! $BINDIR -ef $dest ]]; then
            for file in Country.mmdb GeoIP.dat GeoSite.dat geoip.db geosite.db cn_ip.txt cn_ipv6.txt cache.db; do
                [[ -f "$BINDIR/$file" && ! -L "$BINDIR/$file" ]] || continue
                install -m 600 "$BINDIR/$file" "$dest/$file"
            done
            if [[ -d "$BINDIR/ruleset" ]]; then
                for file in "$BINDIR"/ruleset/*.{mrs,srs}; do
                    [[ -f $file && ! -L $file ]] || continue
                    install -m 600 "$file" "$dest/ruleset/"
                done
            fi
            if [[ -f "$BINDIR/ui/pac" ]] && grep -Fq 'isInNet(host, "169.254.0.0", "255.255.0.0")' "$BINDIR/ui/pac" &&
                grep -Fq '; DIRECT; SOCKS5 ' "$BINDIR/ui/pac"; then
                rm -f "$BINDIR/ui/pac"
            fi
        fi
        if [[ -n $BINDIR ]]; then sed -i '/^BINDIR=/d' "$dest/configs/directories.env"; fi
    )
fi
mv -f "$stage" "$dest/mihomo"
install -d /usr/local/bin
cli_stage=$(mktemp /usr/local/bin/.kokorobox.XXXXXXXX)
install -m 755 "$cli" "$cli_stage"
mv -fT "$cli_stage" /usr/local/bin/kokorobox
bash "$dest/init.sh"
if [[ -n "$initial_language" && ! -s "$lang_file" ]]; then
    save_language "$lang_file" "$initial_language"
fi
if "$resume_service"; then /usr/local/bin/kokorobox start; fi
echo "$INSTALL_DONE"
if "$open_menu"; then
    exec /usr/local/bin/kokorobox
fi
