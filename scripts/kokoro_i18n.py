"""Translate Kokoro CLI messages using the language selected by the shell."""
import json
import os
from pathlib import Path

language = os.environ.get("KOKOROBOX_UI_LANG", "en")
messages = {}
if language in ("chs", "cht"):
    catalog = Path(__file__).resolve().parent / "lang" / language / "kokoro.json"
    messages = json.loads(catalog.read_text(encoding="utf-8"))


def tr(message):
    return messages.get(message, message)


class KokoroError(Exception):
    """A user-facing, localized command error."""
