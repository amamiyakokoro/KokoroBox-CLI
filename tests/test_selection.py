import sys
import unittest
from contextlib import redirect_stdout
from io import StringIO
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from kokoro_selection import Cancelled, select_settings


class FieldEditTest(unittest.TestCase):
    def setUp(self):
        self.saved = dict(plan="Premium", protocol="anytls", isp="ct", mode="direct",
                          rule_source="mirror", final_route="direct", rule_provider_auto_update=True,
                          profile_auto_update=True, profile_update_hours=24)
        self.options = {
            "formats": [{"value": "mihomo"}], "defaults": self.saved.copy(),
            "plans": [{"name": "Basic", "supported_isps": ["all"]},
                      {"name": "Premium", "supported_isps": ["ct"]}],
            "protocols": [{"value": p, "label": p, "supports_direct": p == "anytls"}
                          for p in ("vmess", "anytls")],
            "isps": [{"value": "ct", "label": "CT"}], "rule_sources": ["mirror"],
            "final_routes": ["direct"], "profile_update": {"min_hours": 1, "max_hours": 72},
        }

    def edit(self, *inputs):
        with patch("builtins.input", side_effect=inputs), redirect_stdout(StringIO()):
            return select_settings(self.options, self.saved)

    def test_change_plan_and_protocol_only_reconciles_dependent_fields(self):
        selected = self.edit("1", "1", "2", "1", "s")
        self.assertEqual(selected, {**self.saved, "plan": "Basic", "isp": None, "protocol": "vmess", "mode": "relay"})

    def test_stale_options_prefer_current_server_defaults(self):
        self.options["rule_sources"] = ["origin", "mirror"]
        self.saved.update(plan="retired", protocol="retired", rule_source="retired",
                          isp=None, rule_provider_auto_update=False, profile_auto_update=False)
        selected = self.edit("", "", "", "s")
        self.assertEqual(selected, {**self.saved, "plan": "Premium", "protocol": "anytls", "rule_source": "mirror"})

    def test_unavailable_server_default_uses_first_allowed_option(self):
        self.options["defaults"]["protocol"] = "removed"
        self.saved["protocol"] = "retired"
        selected = self.edit("", "s")
        self.assertEqual(selected, {**self.saved, "protocol": "vmess", "mode": "relay"})

    def test_cancel_discards_changes(self):
        original = self.saved.copy()
        with self.assertRaises(Cancelled):
            self.edit("1", "1", "q")
        self.assertEqual(self.saved, original)
