#!/bin/bash
# Run inside an isolated Linux network namespace with CAP_NET_ADMIN and /dev/net/tun.
set -euo pipefail
trap 'echo "Host route test failed at line $LINENO: $BASH_COMMAND" >&2' ERR
# Exercise the direct supervisor even when the runner itself uses systemd.
if [[ -d /run/systemd/system ]]; then
    [[ $(readlink /proc/self/ns/mnt) != "$(readlink /proc/1/ns/mnt)" ]]
    mount -t tmpfs tmpfs /run/systemd
fi
cd /etc/KokoroBox
work=$(mktemp -d)
server=
cleanup() {
    kokorobox stop || true
    [[ -z $server ]] || kill "$server" 2>/dev/null || true
    rm -f yamls/config.yaml
    [[ ! -e "$work/original" && ! -L "$work/original" ]] || mv "$work/original" yamls/config.yaml
    cp "$work/client.json" configs/client.json
    rm -rf "$work"
}
cp configs/client.json "$work/client.json"
trap cleanup EXIT
if [[ -e yamls/config.yaml || -L yamls/config.yaml ]]; then mv yamls/config.yaml "$work/original"; fi
ip link set lo up
ip link add kbtest type dummy
ip addr add 192.0.2.2/24 dev kbtest
ip link set kbtest up
ip route add default dev kbtest metric 999
ip -6 addr add 2001:db8:77::2/64 dev kbtest nodad
ip -6 route add default dev kbtest metric 999
cat >yamls/config.yaml <<'YAML'
hosts:
  probe.kokoro.test: 198.51.100.31
dns:
  enhanced-mode: redir-host
proxies:
  - {name: local-test, type: http, server: 127.0.0.1, port: 18443}
rules:
  - IP-CIDR,198.51.100.17/32,local-test,no-resolve
  - IP-CIDR6,2001:db8:88::17/128,local-test,no-resolve
  - MATCH,DIRECT
YAML
python3 - <<'PY' &
from http.server import BaseHTTPRequestHandler, HTTPServer
class Proxy(BaseHTTPRequestHandler):
    def do_CONNECT(self):
        assert self.path in ("198.51.100.17:80", "[2001:db8:88::17]:80")
        self.send_response(200)
        self.end_headers()
        assert self.rfile.readline().startswith(b"GET /probe ")
        while self.rfile.readline().strip():
            pass
        self.wfile.write(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\ncaptured")
HTTPServer(("127.0.0.1", 18443), Proxy).serve_forever()
PY
server=$!
# A stale PID stamp for another live process must not cause stop to signal it.
printf '%s %s\n' "$server" "$(awk '{ print $22 }' "/proc/$server/stat")" >/run/kokorobox/core.pid
kokorobox stop
kill -0 "$server"

kokorobox transparent on
status=$(kokorobox status) || [[ $? == 3 ]]
grep -Fq 'On next start' <<<"$status"

# A failed startup must leave the foreign rule intact and remove any partial capture.
ip -6 rule add pref 18992 to 2001:db8:feed::/64 lookup main
if kokorobox start; then exit 1; fi
if kokorobox status; then exit 1; else [[ $? == 3 ]]; fi
ip -6 rule show | grep -Eq '^18992:.*2001:db8:feed::/64.*lookup main'
if ip -4 rule show | grep -q '^18992:' || nft list table inet kokorobox_host 2>/dev/null; then exit 1; fi
[[ ! -e /run/kokorobox/host-route ]]
ip -6 rule del pref 18992 to 2001:db8:feed::/64 lookup main

# Changing mode on a live explicit proxy must restore that service if capture cannot start.
kokorobox transparent off
# The same service lock serializes direct starts from separate CLI processes.
start_and_record() {
    kokorobox start
    cat /run/kokorobox/core.pid >"$1"
}
start_and_record "$work/first-pid" &
first=$!
start_and_record "$work/second-pid" &
second=$!
wait "$first"
wait "$second"
cmp "$work/first-pid" "$work/second-pid"
nft add table inet kokorobox_host
nft add chain inet kokorobox_host foreign_owner
if kokorobox transparent on; then exit 1; fi
kokorobox status | grep -Fq 'Local transparent proxy  Off'
nft list chain inet kokorobox_host foreign_owner >/dev/null
nft delete table inet kokorobox_host

kokorobox transparent on
kokorobox status | grep -Fq 'Local transparent proxy  Active'
ip -4 rule del pref 18992 iif lo fwmark 0x4b42 table 18992
kokorobox status | grep -Fq 'Local transparent proxy  Incomplete'
ip -4 rule add pref 18992 iif lo fwmark 0x4b42 table 18992
# The kernel TUN link appears before Mihomo's userspace adapter accepts the first connection.
[[ $(curl --noproxy '*' -fsS --retry 3 --retry-all-errors --retry-delay 1 --max-time 8 http://198.51.100.17/probe) == captured ]]
[[ $(curl --noproxy '*' -gfsS --max-time 8 'http://[2001:db8:88::17]/probe') == captured ]]
python3 - <<'PYDNS'
import socket
import struct
import runtime

# A UDP DNS query to a destination with no DNS server can only succeed via TUN.
question = b"\x05probe\x06kokoro\x04test\0" + struct.pack("!HH", 1, 1)
query = struct.pack("!6H", 17, 256, 1, 0, 0, 0) + question
with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
    s.settimeout(4)
    s.sendto(query, ("198.51.100.53", 53))
    reply = s.recv(1024)
assert reply[:2] == query[:2] and socket.inet_aton("198.51.100.31") in reply
runtime.reload_config(runtime.CONFIG.read_bytes())
PYDNS
[[ $(curl --noproxy '*' -fsS --max-time 8 http://198.51.100.17/probe) == captured ]]
# A reload may recreate the TUN device. It must not replace a route taken over externally.
ip -4 route replace default dev kbtest table 18992
if kokorobox --host-route refresh; then exit 1; fi
ip -4 route show table 18992 | grep -Eq '^default dev kbtest([[:space:]]|$)'
ip -4 route replace default dev kbox0 table 18992

# An unexpected core exit must release its routes so a new instance can start.
read -r pid _ </run/kokorobox/core.pid
IFS= read -r -d '' executable <"/proc/$pid/cmdline"
[[ $executable == /etc/KokoroBox/mihomo ]]
kill -KILL "$pid"
for attempt in {1..100}; do
    [[ -e /run/kokorobox/host-route ]] || break
    sleep 0.1
done
[[ ! -e /run/kokorobox/host-route ]]
if nft list table inet kokorobox_host 2>/dev/null; then exit 1; fi
kokorobox start
kokorobox status | grep -Fq 'Local transparent proxy  Active'
# Stop must remove our rules without claiming an identically named foreign nft table.
nft delete table inet kokorobox_host
nft add table inet kokorobox_host
nft add chain inet kokorobox_host replacement_owner
kokorobox stop
nft list chain inet kokorobox_host replacement_owner >/dev/null
nft delete table inet kokorobox_host
kokorobox stop
if nft list table inet kokorobox_host 2>/dev/null || ip rule | grep -q 'lookup 18992'; then exit 1; fi
if curl --noproxy '*' -fsS --max-time 1 http://198.51.100.17/probe 2>/dev/null; then exit 1; fi
echo 'Local unconfigured TCP connection matched a Mihomo rule via TUN; stopping removed capture.'
