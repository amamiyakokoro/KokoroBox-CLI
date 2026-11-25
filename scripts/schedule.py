"""Manage only KokoroBox-owned subscription entries in the installing user's crontab."""
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

from kokoro_i18n import KokoroError, tr
from runtime import RUN, lock

ROOT = Path("/etc/KokoroBox")

TAG = "# kokorobox-sync"
SYNC = "/usr/local/bin/kokorobox sync"
TASK = re.escape(str(ROOT / "task/task.sh"))
JOB = re.compile(r"^(?P<when>(?:\S+\s+){5}|@\w+\s+)" + TASK + r"\s+(?P<id>\d+)(?:\s|$)")
WATCH = str(ROOT / "starts/start_legacy_wd.sh")


def change(action):
    if not shutil.which("crontab"):
        if action in ("off", "migrate", "show"):
            if action == "show":
                print(tr("Off"))
            return
        raise KokoroError(tr("Install cron to enable automatic sync."))
    with lock(RUN / "schedule.lock"):
        current = subprocess.run(["crontab", "-l"], env={**os.environ, "LC_ALL": "C"},
                                 text=True, capture_output=True)
        if current.returncode and not (current.returncode == 1 and current.stderr.strip() == "no crontab for root"):
            raise KokoroError(tr("Unable to read crontab."))
        old = current.stdout.splitlines()
        if action == "show":
            print("\n".join(s.split(SYNC)[0].strip() for s in old if s.endswith(TAG)) or tr("Off"))
            return
        lines = []
        if action == "migrate":
            # Stored running jobs may not be installed while the old core is stopped.
            stored = []
            for name in ("cron", "running"):
                path = ROOT / "configs/task" / name
                if path.is_file():
                    stored.extend(path.read_text().splitlines())
            for s in old:
                job = JOB.match(s)
                if job and job["id"] in ("104", "105"):
                    s = job["when"] + SYNC + " " + TAG
                elif job or re.match(r"^\* \* \* \* \* /bin/sh " + re.escape(WATCH) + r" (kokorobox|bot_tg)( |$)", s):
                    continue
                if not s.endswith(TAG) or s not in lines:
                    lines.append(s)
            for s in stored:
                job = JOB.match(s)
                if job and job["id"] in ("104", "105"):
                    entry = job["when"] + SYNC + " " + TAG
                    if entry not in lines:
                        lines.append(entry)
        else:
            lines = [s for s in old if not s.endswith(TAG)]
            if action == "on":
                lines.append("0 * * * * " + SYNC + " " + TAG)
        if lines != old:
            subprocess.run(["crontab", "-"], input="".join(s + "\n" for s in lines), text=True, check=True)


if __name__ == "__main__":
    try:
        change(sys.argv[1])
    except (KokoroError, OSError, subprocess.CalledProcessError) as exc:
        sys.exit(str(exc))
