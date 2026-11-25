#!/bin/bash
set -eu
cd /etc/KokoroBox
umask 077
getent group kokorobox >/dev/null || groupadd --system kokorobox
python3 - <<'PY'
import json
from pathlib import Path
import secrets
from runtime import atomic_write
config = Path("configs/client.json")
if not config.exists():
    path = Path("configs/KokoroBox.cfg")
    old = dict(line.split("=", 1) for line in path.read_text().splitlines() if "=" in line) if path.exists() else {}
    def port(key, default):
        value = old.get(key, "")
        return int(value) if value.isdigit() and 0 < int(value) < 65536 else default
    settings = {"port": port("mix_port", 7890), "controller": port("db_port", 9999),
                "secret": old.get("secret") or secrets.token_hex(24),
                "transparent": old.get("firewall_area") in ("2", "3"),
                "authentication": old.get("authentication", "")}
    atomic_write(config, json.dumps(settings).encode())
PY
chmod +x menu.sh
if [[ -d /run/systemd/system ]]; then
    install -m 644 starts/kokorobox.service /etc/systemd/system/kokorobox.service
    systemctl daemon-reload
elif command -v openrc-run >/dev/null; then
    install -m 755 starts/kokorobox.openrc /etc/init.d/kokorobox
fi
