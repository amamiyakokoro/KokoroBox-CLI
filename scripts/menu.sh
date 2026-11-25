#!/bin/bash
set -u
umask 077
cd -- "$(dirname "$(readlink -f "$0")")"
export CRASHDIR=$PWD
. libs/i18n.sh
lang_file=$PWD/configs/i18n.cfg
if [[ $EUID == 0 && -t 0 && -t 1 && ! -s $lang_file && ( $# == 0 || $1 == -l ) ]]; then
    picked=$(choose_language) && save_language "$lang_file" "$picked" || exit 1
fi
catalog() { load_lang menu; export KOKOROBOX_UI_LANG=$i18n; }
catalog
(( EUID == 0 )) || { echo "$MENU_NEED_ROOT" >&2; exit 1; }
case ${1:-} in
    -u|uninstall)
        read -rp "$MENU_CONFIRM_UNINSTALL" reply
        [[ $reply == y ]] || exit 0
        /usr/local/bin/kokorobox stop && python3 schedule.py off || exit 1
        if [[ -d /run/systemd/system ]]; then systemctl disable kokorobox.service; fi
        if command -v rc-update >/dev/null; then rc-update del kokorobox default; fi
        rm -f /etc/systemd/system/kokorobox.service /etc/init.d/kokorobox /usr/local/bin/kokorobox
        rm -rf /etc/KokoroBox /run/kokorobox
        if [[ -d /run/systemd/system ]]; then systemctl daemon-reload; fi
        exit ;;
    ""|-l) ;;
    *) exec /usr/local/bin/kokorobox "$@" ;;
esac
subscriptions() {
    while true; do
        printf '\n%s\n' "$MENU_MAIN_1" "l) $LOGIN" "w) $WHO" "o) $LOGOUT" "p) $SELECT" "k) $IMPORT" "s) $SYNC" "0) $BACK"
        read -rp '> ' choice || return
        case $choice in
            l) /usr/local/bin/kokorobox login ;; w) /usr/local/bin/kokorobox whoami ;; o) /usr/local/bin/kokorobox logout ;;
            k) /usr/local/bin/kokorobox subscribe ;; p) /usr/local/bin/kokorobox select ;; s) /usr/local/bin/kokorobox sync ;; 0|"") return ;;
        esac
    done
}
settings_menu() {
    while true; do
        printf '\n%s\n' "$SETTINGS" "1) $HOST_MODE ($ON)" "2) $HOST_MODE ($OFF)" "3) $AUTO ($ON)" "4) $AUTO ($OFF)" "b) $LANGUAGE" "0) $BACK"
        read -rp '> ' choice || return
        case $choice in
            1) /usr/local/bin/kokorobox transparent on ;; 2) /usr/local/bin/kokorobox transparent off ;;
            3) /usr/local/bin/kokorobox auto-sync on ;; 4) /usr/local/bin/kokorobox auto-sync off ;;
            b) picked=$(choose_language "$BACK") && save_language "$lang_file" "$picked" && catalog ;;
            0|"") return ;;
        esac
    done
}
while true; do
    printf '\nKokoroBox-CLI\n%s\n' "1) $MENU_MAIN_1"
    printf '%s\n' "2) $MENU_MAIN_2" "3) $SETTINGS" "4) $STOP" "5) $STATUS" "6) $LOGS" "7) $TEST" "0) $EXIT"
    read -rp '> ' choice || exit 0
    case $choice in
        1) subscriptions ;; 2) if [[ -s yamls/config.yaml ]]; then /usr/local/bin/kokorobox restart; else subscriptions; fi ;;
        3) settings_menu ;; 4) /usr/local/bin/kokorobox stop ;; 5) /usr/local/bin/kokorobox status ;; 6) /usr/local/bin/kokorobox logs ;; 7) /usr/local/bin/kokorobox test ;; 0|"") exit 0 ;;
    esac
done
