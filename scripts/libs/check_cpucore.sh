# Copyright (C) Juewuy and KokoroBox contributors
linux_core_arch() {
    case $(uname -m) in
        x86_64) echo amd64-v1 ;;
        aarch64|arm64) echo arm64 ;;
        armv7*) echo armv7 ;;
        armv5*|armv6*) echo armv5 ;;
        mips*)
            if [ "$(python3 -c 'import sys; print(sys.byteorder)')" = little ]; then
                echo mipsle-softfloat
            else
                echo mips-softfloat
            fi ;;
        *) return 1 ;;
    esac
}
