language_code() {
    case "$1" in
        cht|zh_TW*|zh-TW*|zh_HK*|zh-HK*|zh_MO*|zh-MO*|zh_Hant*|zh-Hant*) echo cht ;;
        chs|zh|zh_*|zh-*) echo chs ;;
        *) echo en ;;
    esac
}

# The picker prints only the selected language code to stdout.
choose_language() {
    local choice
    printf '%s\n' '请选择语言 / 請選擇語言 / Choose a language' \
        '1) 简体中文' '2) 繁體中文' '3) English' >&2
    [ -z "${1:-}" ] || printf '0) %s\n' "$1" >&2
    while true; do
        printf '> ' >&2
        IFS= read -r choice || return 1
        case "$choice" in
            1) echo chs; return 0 ;;
            2) echo cht; return 0 ;;
            3) echo en; return 0 ;;
            0|"") [ -z "${1:-}" ] || return 1 ;;
        esac
        printf '%s\n' '请输入 1–3 / 請輸入 1–3 / Enter 1–3' >&2
    done
}

save_language() {
    local file=$1 language=$2 temp
    temp=$(mktemp "$file.XXXXXXXX") || return 1
    if printf '%s\n' "$language" >"$temp" && mv -fT "$temp" "$file"; then
        return 0
    fi
    rm -f "$temp"
    return 1
}

load_lang() {
    local saved=
    if ! IFS= read -r saved 2>/dev/null < "$CRASHDIR/configs/i18n.cfg" && [ -z "$saved" ]; then
        saved=${KOKOROBOX_LANG:-${LC_ALL:-${LC_MESSAGES:-${LANG:-en}}}}
    fi
    i18n=$(language_code "$saved")
    . "$CRASHDIR/lang/$i18n/$1.lang" || return 1
}
