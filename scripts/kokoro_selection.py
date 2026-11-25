"""Interactive selection from Kokoro's account-specific subscription options."""
import json
from pathlib import Path
import sys

from kokoro_i18n import KokoroError, tr


class Cancelled(Exception):
    pass


def answer(prompt):
    try:
        value = input(prompt).strip()
    except EOFError:
        raise Cancelled from None
    if value.lower() == "q":
        raise Cancelled
    return value


def choose(title, items, preferred):
    if not items:
        raise KokoroError(tr("No choices available for {title}.").format(title=title))
    print(title)
    values = [value for value, _ in items]
    default = values.index(preferred) if preferred in values else 0
    for index, (_, label) in enumerate(items):
        print(f"  {index + 1}. {label}" + (" *" if index == default else ""))
    if len(items) == 1:
        return values[0]
    while True:
        value = answer(f"[{default + 1}] ")
        if not value:
            return values[default]
        if value.isascii() and value.isdecimal() and len(value) <= 6 and 1 <= int(value) <= len(items):
            return values[int(value) - 1]


FIELDS = {
    "plan": "Plan", "protocol": "Protocol", "isp": "ISP", "mode": "Connection mode",
    "rule_source": "Rule source", "final_route": "Final route",
    "rule_provider_auto_update": "Rule provider updates",
    "profile_auto_update": "Advertise a profile update interval",
    "profile_update_hours": "Update interval (hours)",
}


def label(value):
    if not isinstance(value, str) or not value or not value.isprintable():
        raise ValueError
    return value


def select_settings(options, saved):
    if not any(row["value"] == "mihomo" for row in options["formats"]):
        raise KokoroError(tr("This account has no Mihomo format."))
    prefs = {**options["defaults"], **saved}
    plans = {label(row["name"]): row for row in options["plans"]}
    protocols = {label(row["value"]): row for row in options["protocols"]}
    toggle = [(True, tr("On")), (False, tr("Off"))]
    low, high = (options["profile_update"][k] for k in ("min_hours", "max_hours"))

    def choices(key):
        if key == "plan":
            return [(k, k) for k in plans]
        if key == "protocol":
            return [(k, label(row["label"])) for k, row in protocols.items()]
        if key == "isp":
            allowed = plans[prefs["plan"]]["supported_isps"]
            return [(None, tr("Default"))] + [
                (label(row["value"]), label(row["label"])) for row in options["isps"]
                if row["value"] and row["value"] in allowed and allowed != ["all"]]
        if key == "mode":
            modes = [("relay", tr("Relay"))]
            if protocols[prefs["protocol"]]["supports_direct"]:
                modes.append(("direct", tr("Direct")))
            return modes
        if key in ("rule_source", "final_route"):
            return [(label(v), v) for v in options[key + "s"]]
        return toggle

    def edit(key, *, repair_only=False):
        value = prefs.get(key)
        title = tr(FIELDS[key])
        if key == "profile_update_hours":
            default = max(low, min(high, value))
            if prefs["profile_auto_update"] and not (repair_only and default == value):
                while True:
                    raw = answer(f"{title} {low}–{high} [{default}] ")
                    if not raw:
                        break
                    if raw.isascii() and raw.isdecimal() and len(raw) <= 6 and low <= int(raw) <= high:
                        default = int(raw)
                        break
            prefs[key] = default
            return
        items = choices(key)
        if value in dict(items):
            if repair_only and key in prefs:
                return
        else:
            value = options["defaults"].get(key)
        prefs[key] = choose(title, items, value)

    print(tr("Choose a number. Enter accepts *; q cancels."))
    for key in FIELDS:
        edit(key, repair_only=bool(saved))
    if saved:
        while True:
            keys = [k for k in FIELDS if k != "profile_update_hours" or prefs["profile_auto_update"]]
            for index, key in enumerate(keys, 1):
                value = prefs[key] if key == "profile_update_hours" else dict(choices(key))[prefs[key]]
                print(f"  {index}) {tr(FIELDS[key])}  {value}")
            action = answer(tr("Field number to edit, s to save, q to cancel: "))
            if action.lower() == "s":
                break
            if not (action.isascii() and action.isdecimal() and len(action) <= 2 and 1 <= int(action) <= len(keys)):
                continue
            key = keys[int(action) - 1]
            edit(key)
            if key == "plan":
                edit("isp", repair_only=True)
            elif key == "protocol":
                edit("mode", repair_only=True)
    elif not choose(tr("Download and save this selection?"),
                    [(True, tr("Save")), (False, tr("Cancel"))], True):
        raise Cancelled
    return {key: prefs[key] for key in FIELDS}


if __name__ == "__main__":
    try:
        data = json.loads(Path(sys.argv[1]).read_text())
        selected = select_settings(data["options"], data["saved"])
        Path(sys.argv[2]).write_text(json.dumps(selected))
    except Cancelled:
        pass
    except KokoroError as exc:
        sys.exit(str(exc))
    except (KeyError, TypeError, ValueError):
        sys.exit(tr("Invalid Kokoro subscription options."))
