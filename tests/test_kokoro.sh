#!/bin/bash
# Run in a disposable GNU/Linux installation after install.sh.
set -euo pipefail
trap 'echo "Kokoro integration failed at line $LINENO" >&2' ERR
umask 077
cd "$(dirname "$0")"
if [[ -e /etc/KokoroBox/yamls/config.yaml ]]; then
    echo 'Subscription integration test requires a fresh installation.' >&2
    exit 1
fi
work=$(mktemp -d)
server=
cleanup() {
    kokorobox stop >/dev/null 2>&1 || true
    [[ -z $server ]] || kill "$server" 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=amamiyakoko.ro \
    -addext 'subjectAltName=DNS:amamiyakoko.ro' -keyout "$work/key" -out "$work/cert" 2>/dev/null
python3 kokoro_server.py "$work" &
server=$!
for attempt in {1..50}; do [[ -s "$work/port" ]] && break; sleep 0.1; done
export SSL_CERT_FILE="$work/cert" https_proxy="http://127.0.0.1:$(cat "$work/port")" no_proxy=127.0.0.1
cli=/usr/local/bin/kokorobox
url='https://amamiyakoko.ro/api/config?uuid=synthetic-fixture&client=meta'
root=/etc/KokoroBox
ctl() {
    python3 - "$1" <<'PYCTRL'
import sys
sys.path.insert(0, "/etc/KokoroBox")
import runtime
sys.stdout.buffer.write(runtime.controller(sys.argv[1]))
PYCTRL
}
# Settings left by the upstream importer must never be used by unattended jobs.
printf '%s\n' 'Url=https://example.invalid/legacy' 'Https=https://example.invalid/config' >>"$root/configs/KokoroBox.cfg"
if "$cli" sync >>"$work/log" 2>&1; then exit 1; fi
[[ ! -e "$root/yamls/config.yaml" ]]
if printf '%s\n' 'https://example.invalid/api/config?uuid=legacy' | "$cli" subscribe >>"$work/log" 2>&1; then exit 1; fi
printf '%s\n' "$url" | "$cli" subscribe >>"$work/log"
profile=$root/yamls/config.yaml
grep -q 'MATCH,DIRECT' "$profile"
[[ $(stat -Lc %a "$profile") == 600 ]]
for mode in invalid redirect denied; do
    echo "$mode" >"$work/mode"
    if "$cli" sync >>"$work/log" 2>&1; then exit 1; fi
    grep -q 'MATCH,DIRECT' "$profile"
done
echo updated >"$work/mode"
"$cli" start
old_pid=$(cat /run/kokorobox/core.pid)
if [[ -d /run/systemd/system ]]; then
    [[ $(systemctl show -p MainPID --value kokorobox.service) == "${old_pid%% *}" ]]
fi
"$cli" start
[[ $(cat /run/kokorobox/core.pid) == "$old_pid" ]]
"$cli" sync >>"$work/log"
[[ $(cat /run/kokorobox/core.pid) == "$old_pid" ]]
ctl /rules | grep -qi one.test
ctl /providers/rules | python3 -c 'import json,sys; assert json.load(sys.stdin)["providers"]["kokoro-test"]["ruleCount"] == 1'
python3 - <<'PYRULES'
import yaml
cfg = yaml.safe_load(open("/run/kokorobox/config.yaml"))
assert cfg["rules"] == ["DOMAIN,user.test,REJECT", "RULE-SET,kokoro-test,DIRECT", "DOMAIN,one.test,DIRECT", "MATCH,DIRECT"]
PYRULES
echo invalid >"$work/mode"
if "$cli" sync >>"$work/log" 2>&1; then exit 1; fi
ctl /rules | grep -qi one.test
echo updated >"$work/mode"
"$cli" status | grep -qx running

grep -q 'DOMAIN,one.test,DIRECT' "$profile"
"$cli" sync | grep -q 'current'
if grep -q synthetic-fixture "$work/log"; then exit 1; fi
# An unavailable device endpoint or expired challenge must leave no credentials.
for scenario in device-unavailable device-expiring; do
    touch "$work/$scenario"
    if "$cli" login >"$work/login-error" 2>&1; then exit 1; fi
    case $scenario in
        device-unavailable) grep -Fq 'does not offer device login' "$work/login-error" ;;
        device-expiring) grep -Fq 'Device code expired' "$work/login-error" ;;
    esac
    [[ ! -e "$root/auth/session.json" ]]
    rm "$work/$scenario"
done
"$cli" login >>"$work/log"
grep -q ABCD-EFGH "$work/log"
session=$root/auth/session.json
[[ $(stat -c %a "$session") == 600 ]]
"$cli" whoami | tee -a "$work/log" | grep -Fqx 'Fixture User (osu! 123)'
touch "$work/require-refresh"
"$cli" whoami >>"$work/log" &
whoami_pid=$!
"$cli" sync >>"$work/log"
wait "$whoami_pid"
# Login and the command used by cron must preserve the imported source.
"$cli" status >"$work/status"
grep -Fq 'Subscription source  External link' "$work/status"
grep -Fq 'Rule sets with rules / total  1/1' "$work/status"
if grep -Eq 'synthetic-fixture|https?://' "$work/status" || [[ -e "$work/resolved.json" ]]; then exit 1; fi
[[ -f "$root/yamls/.kokoro/current/url" ]]
# Select Premium/AnyTLS, its allowed ISP and direct mode. The profile fixture
# deliberately returns identical YAML so changing only preferences is exercised.
printf '%s\n' 2 2 2 2 2 2 2 1 24 1 | "$cli" select >>"$work/log"
cat >"$work/check_selection.py" <<'PYTHON'
import json, sys
from pathlib import Path
selected = json.loads(Path(sys.argv[1]).read_text())
assert selected == {"format": "mihomo", "plan": "Premium", "protocol": "anytls", "isp": "ct",
                    "mode": "direct", "rule_source": "mirror", "final_route": "direct",
                    "rule_provider_auto_update": False, "profile_auto_update": True, "profile_update_hours": 24}
PYTHON
python3 "$work/check_selection.py" "$work/resolved.json"
"$cli" status | grep -Fq 'Plan Premium'
[[ $(cat /run/kokorobox/core.pid) == "$old_pid" ]]
# Editing a single field neither walks the wizard again nor resets other fields.
printf '%s\n' 5 1 s | "$cli" select >>"$work/log"
python3 -c 'import json,sys; s=json.load(open(sys.argv[1])); assert (s["rule_source"],s["plan"],s["isp"],s["mode"]) == ("origin","Premium","ct","direct")' "$work/resolved.json"
printf '%s\n' 5 2 s | "$cli" select >>"$work/log"
python3 "$work/check_selection.py" "$work/resolved.json"
"$cli" status | grep -Fq 'Subscription source  Kokoro account'
selected=$(readlink "$root/yamls/.kokoro/current")
printf '%s\n' 1 1 q | "$cli" select >>"$work/log"
[[ $(readlink "$root/yamls/.kokoro/current") == "$selected" ]]
touch "$work/reject-selection"
if printf '%s\n' 2 1 s | "$cli" select >>"$work/log" 2>&1; then exit 1; fi
[[ $(readlink "$root/yamls/.kokoro/current") == "$selected" ]]
rm "$work/reject-selection"
touch "$work/other-account"
if "$cli" sync >>"$work/log" 2>&1; then exit 1; fi
rm "$work/other-account"
"$cli" sync >>"$work/log"
python3 "$work/check_selection.py" "$work/resolved.json"
# Import while signed in, then emulate an older generation without source metadata.
printf '%s\n' "$url" | "$cli" subscribe >>"$work/log"
rm "$root/yamls/.kokoro/current/active.json"
"$cli" sync >>"$work/log"
[[ -f "$root/yamls/.kokoro/current/url" ]]
if grep -q account.test "$profile"; then exit 1; fi
printf 's\n' | "$cli" select >>"$work/log"
touch "$work/bad-origin"
if "$cli" sync >>"$work/log" 2>&1; then exit 1; fi
grep -q 'DOMAIN,account.test,DIRECT' "$profile"
"$cli" logout >>"$work/log"
[[ -f "$work/revoked" && ! -e "$session" ]]
for command in whoami sync; do
    if "$cli" "$command" >>"$work/log" 2>&1; then exit 1; fi
done
grep -q 'DOMAIN,account.test,DIRECT' "$profile"
if grep -Eq 'fixture-(device-secret|access-|refresh-|private-uuid)' "$work/log"; then exit 1; fi
# Old listener flags must have no effect on a newly generated runtime profile.
printf '%s\n' 'vms_service=ON' 'sss_service=ON' >>"$root/configs/KokoroBox.cfg"
unset https_proxy SSL_CERT_FILE
"$cli" restart
[[ $(cat /run/kokorobox/core.pid) != "$old_pid" ]]
ctl /version >/dev/null
"$cli" status | grep -q running
[[ ! -e "$root/ui/pac" ]]
if grep -Eq 'name: "(vmess-in|ss-in)"' /run/kokorobox/config.yaml; then exit 1; fi
for node in fixture-vmess fixture-ss; do
    ctl "/proxies/$node" >"$work/node.json"
    python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["name"] == sys.argv[2]' "$work/node.json" "$node"
done
ctl /rules | grep -qi one.test
curl --max-time 8 --noproxy '' -fsS --proxy http://127.0.0.1:7890 "http://127.0.0.1:$(cat "$work/port")/probe" | grep -qx ok
"$cli" stop
if "$cli" status; then exit 1; else [[ $? == 3 ]]; fi
echo 'Kokoro subscription and proxy integration passed'
