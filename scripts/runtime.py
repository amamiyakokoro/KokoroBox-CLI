"""Build and validate Mihomo's local settings without rewriting Kokoro rules."""
from contextlib import contextmanager
import fcntl
import json
import os
import re
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile
from urllib.request import ProxyHandler, Request, build_opener

import yaml

from kokoro_i18n import KokoroError, tr

ROOT = Path(__file__).resolve().parent
RUN = Path("/run/kokorobox")
CONFIG = RUN / "config.yaml"


@contextmanager
def lock(path):
    with path.open("a") as stream:
        fcntl.flock(stream, fcntl.LOCK_EX)
        yield


def atomic_write(path, data):
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as f:
        tmp = Path(f.name)
    try:
        tmp.write_bytes(data)
        os.replace(tmp, path)
    finally:
        tmp.unlink(missing_ok=True)


def settings():
    return json.loads((ROOT / "configs/client.json").read_text())


def render(content, local):
    try:
        cfg = yaml.safe_load(content)
    except yaml.YAMLError:
        raise KokoroError(tr("Mihomo rejected the downloaded configuration.")) from None
    if not isinstance(cfg, dict):
        raise KokoroError(tr("Mihomo rejected the downloaded configuration."))
    # The server owns routing and DNS policy. Only the client may open listeners.
    for key in ("port", "socks-port", "redir-port", "tproxy-port", "listeners", "external-ui",
                "external-ui-url", "external-controller-tls", "external-controller-unix",
                "external-controller-pipe", "external-doh-server", "tunnels", "ss-config", "vmess-config"):
        cfg.pop(key, None)
    cfg.update({"mixed-port": local["port"], "allow-lan": False, "bind-address": "127.0.0.1",
                "external-controller": "127.0.0.1:" + str(local["controller"]),
                "secret": local["secret"], "mode": "rule", "tun": {"enable": False}})
    if local.get("authentication"):
        cfg["authentication"] = [local["authentication"]]
    if local["transparent"]:
        cfg["tun"] = {"enable": True, "device": "kbox0", "stack": "system", "auto-route": False,
                      "auto-redirect": False, "auto-detect-interface": True, "dns-hijack": ["any:53", "tcp://any:53"]}
        cfg.setdefault("ipv6", True)
        cfg["routing-mark"] = 0x4B43
        dns = cfg.setdefault("dns", {})
        if not isinstance(dns, dict):
            raise KokoroError(tr("Mihomo rejected the downloaded configuration."))
        dns["enable"] = True
        dns.setdefault("nameserver", ["1.1.1.1", "8.8.8.8"])
    dns = cfg.get("dns")
    if isinstance(dns, dict):
        dns.pop("listen", None)
    return yaml.safe_dump(cfg, allow_unicode=True, sort_keys=False).encode()


def validate(path):
    result = subprocess.run([str(ROOT / "mihomo"), "-t", "-d", str(ROOT), "-f", str(path)],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
    if result.returncode:
        raise KokoroError(tr("Mihomo rejected the downloaded configuration."))


def controller(path, data=None):
    local = settings()
    req = Request(f"http://127.0.0.1:{local['controller']}{path}",
                  headers={"Authorization": "Bearer " + local["secret"], "Content-Type": "application/json"},
                  method="GET" if data is None else "PUT",
                  data=None if data is None else json.dumps(data).encode())
    with build_opener(ProxyHandler({})).open(req, timeout=30) as res:
        return res.read()


def running():
    return subprocess.run(["/usr/local/bin/kokorobox", "--core-running"]).returncode == 0


def reload_config(data):
    controller("/configs?force=true", {"payload": data.decode()})
    if (RUN / "transparent").exists():
        subprocess.run(["/usr/local/bin/kokorobox", "--host-route", "refresh"], check=True)


def replace_link(path, target, stage):
    link = stage / "next-link"
    link.symlink_to(target)
    try:
        os.replace(link, path)
    finally:
        link.unlink(missing_ok=True)


def apply_profile(stage):
    """Validate, reload and publish an already downloaded generation under the service lock."""
    profiles = ROOT / "yamls"
    current = profiles / ".kokoro/current"
    profile = profiles / "config.yaml"
    source = Path(".kokoro/current/config.yaml")
    active = profile.is_symlink() and os.readlink(profile) == str(source)
    previous = os.readlink(current) if current.is_symlink() else None
    content = (stage / "config.yaml").read_bytes()
    if profile.is_file() and not profile.is_symlink():
        shutil.copyfile(profile, profiles / "config.yaml.bak")
    with lock(RUN / "service.lock"):
        data = render(content, settings())
        candidate = stage / "runtime.yaml"
        candidate.write_bytes(data)
        validate(candidate)
        candidate.unlink()
        live = running()
        old_data = CONFIG.read_bytes() if live else None
        reload_needed = live and data != old_data
        try:
            if reload_needed:
                reload_config(data)
                atomic_write(CONFIG, data)
            replace_link(current, stage.name, stage)
            if not active:
                replace_link(profile, source, stage)
        except BaseException as error:
            if current.is_symlink() and os.readlink(current) == stage.name:
                if previous:
                    replace_link(current, previous, stage)
                else:
                    current.unlink()
            if reload_needed:
                try:
                    reload_config(old_data)
                    atomic_write(CONFIG, old_data)
                except (OSError, subprocess.SubprocessError):
                    raise KokoroError(tr("Reload failed. Restart to recover the saved profile.")) from None
                if isinstance(error, (OSError, subprocess.SubprocessError)):
                    raise KokoroError(tr("Could not apply the new profile. Previous profile restored.")) from None
            raise
    if previous and re.fullmatch(r"profile-[\w-]+", previous):
        shutil.rmtree(current.parent / previous, ignore_errors=True)


def prepare():
    RUN.mkdir(mode=0o700, exist_ok=True)
    log = RUN / "core.log"
    if log.is_file() and log.stat().st_size > 2 * 1024 * 1024:
        os.replace(log, RUN / "core.log.1")
    raw = ROOT / "yamls/config.yaml"
    if not raw.is_file():
        raise KokoroError(tr("Run kokorobox login or kokorobox subscribe first."))
    local = settings()
    if local["transparent"] and subprocess.run(
            ["ip", "link", "show", "dev", "kbox0"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
        raise KokoroError(tr("The TUN interface kbox0 is already in use."))
    data = render(raw.read_bytes(), local)
    with tempfile.NamedTemporaryFile(dir=RUN) as f:
        f.write(data)
        f.flush()
        validate(f.name)
    atomic_write(CONFIG, data)
    marker = RUN / "transparent"
    if local["transparent"]:
        marker.touch(mode=0o600)
    else:
        marker.unlink(missing_ok=True)


def probe():
    import base64
    import http.client
    local = settings()
    headers = {}
    if local.get("authentication"):
        headers["Proxy-Authorization"] = "Basic " + base64.b64encode(local["authentication"].encode()).decode()
    conn = http.client.HTTPSConnection("127.0.0.1", local["port"], timeout=10)
    try:
        conn.set_tunnel("www.gstatic.com", port=443, headers=headers)
        conn.request("GET", "/generate_204")
        if conn.getresponse().status != 204:
            raise KokoroError(tr("Proxy connectivity test failed."))
        print(tr("Proxy connection OK."))
    finally:
        conn.close()


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit({"prepare": prepare, "probe": probe,
                  "apply-profile": lambda: apply_profile(Path(sys.argv[2])),
                  "running": lambda: 0 if running() else 1}[sys.argv[1]]())
    except (KokoroError, OSError, ValueError, subprocess.SubprocessError) as exc:
        sys.exit(str(exc))
