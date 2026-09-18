"""Contract tests against the packaged Hermes Python, using disposable state."""

import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


class SkillsInventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scratch = tempfile.TemporaryDirectory()
        cls.home = Path(cls.scratch.name)
        cls.environment = patch.dict(os.environ, {"HERMES_HOME": str(cls.home)})
        cls.environment.start()
        from hermes_cli import plugins, profiles, web_server
        from starlette.testclient import TestClient

        cls.plugins = plugins
        cls.server = web_server
        cls.patches = [
            patch.object(profiles, "_get_default_hermes_home", lambda: cls.home),
            patch.object(profiles, "_get_profiles_root", lambda: cls.home / "profiles"),
            patch.object(plugins, "get_bundled_plugins_dir", lambda: cls.home / "empty"),
        ]
        for mock in cls.patches:
            mock.start()
        cls.client = TestClient(web_server.app, raise_server_exceptions=False)
        cls.client.headers[web_server._SESSION_HEADER_NAME] = web_server._SESSION_TOKEN

    @classmethod
    def tearDownClass(cls):
        cls.client.close()
        for mock in reversed(cls.patches):
            mock.stop()
        cls.environment.stop()
        cls.scratch.cleanup()

    def prepare(self, name):
        from hermes_cli.agent_plugins import PLUGIN_SCHEMA_V1

        home = self.home / "profiles" / name
        home.mkdir(parents=True)
        (home / "config.yaml").write_text(
            "plugins:\n  enabled: [inventory.test]\nskills:\n  disabled: [disabled]\n"
        )
        for skill in ("local", "disabled"):
            self.write_skill(home / "skills" / skill, skill)
        plugin = home / "plugins" / "inventory"
        plugin.mkdir(parents=True)
        (plugin / "plugin.json").write_text(
            json.dumps({"$schema": PLUGIN_SCHEMA_V1, "name": "inventory.test"})
        )
        self.write_skill(plugin / "skills" / "plugin-skill", "plugin-skill")
        return home

    def write_skill(self, directory, name):
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "SKILL.md").write_text(
            f"---\nname: {name}\ndescription: Example {name}\n---\nPrivate body\n"
        )

    def inventory(self, profile):
        return self.client.get("/api/skills", params={"profile": profile, "inventory": "true"})

    def test_inventory_and_legacy_compatibility(self):
        home = self.prepare("compatibility")
        before = (home / "config.yaml").read_bytes()
        with patch.object(
            self.plugins, "discover_plugins", wraps=self.plugins.discover_plugins
        ) as discover:
            legacy = self.client.get("/api/skills?profile=compatibility")
            self.assertEqual(legacy.status_code, 200, legacy.text)
            self.assertIsInstance(legacy.json(), list)
            discover.assert_not_called()
            response = self.inventory("compatibility")
            self.assertEqual(response.status_code, 200, response.text)
            discover.assert_called_once_with()
        payload = response.json()
        self.assertEqual(payload["inventory_version"], 1)
        rows = {row["name"]: row for row in payload["skills"]}
        self.assertIn("local", rows)
        self.assertFalse(rows["disabled"]["enabled"])
        plugin = [row for name, row in rows.items() if name.endswith(":plugin-skill")]
        self.assertEqual(len(plugin), 1)
        self.assertEqual(plugin[0]["category"], "plugin")
        for row in rows.values():
            self.assertEqual(set(row), {"name", "description", "category", "enabled"})
        self.assertEqual((home / "config.yaml").read_bytes(), before)
        # An individual plugin skill can be disabled without disabling its plugin.
        import yaml

        config = yaml.safe_load(before)
        config["skills"]["disabled"].append(plugin[0]["name"])
        (home / "config.yaml").write_text(yaml.safe_dump(config))
        disabled_plugin = next(
            row
            for row in self.inventory("compatibility").json()["skills"]
            if row["name"] == plugin[0]["name"]
        )
        self.assertFalse(disabled_plugin["enabled"])

        again = self.client.get("/api/skills?profile=compatibility").json()
        self.assertEqual({row["name"] for row in again}, {row["name"] for row in legacy.json()})
        toggle = self.client.put(
            "/api/skills/toggle",
            json={"profile": "compatibility", "name": "local", "enabled": False},
        )
        self.assertEqual(toggle.status_code, 200, toggle.text)
        self.assertFalse(
            next(
                row
                for row in self.inventory("compatibility").json()["skills"]
                if row["name"] == "local"
            )["enabled"]
        )

    def test_profile_isolation_and_failure_restoration(self):
        import tools.skills_tool as skills_tool

        first = self.prepare("first")
        second = self.prepare("second")
        self.write_skill(first / "skills" / "first-only", "first-only")
        self.write_skill(second / "skills" / "second-only", "second-only")
        self.write_skill(
            first / "plugins" / "inventory" / "skills" / "first-plugin", "first-plugin"
        )
        original = skills_tool.SKILLS_DIR
        for profile, absent in (("first", "second-only"), ("second", "first-only")):
            response = self.inventory(profile)
            self.assertEqual(response.status_code, 200, response.text)
            names = {row["name"] for row in response.json()["skills"]}
            self.assertIn(profile + "-only", names)
            self.assertNotIn(absent, names)
            self.assertEqual(
                any(name.endswith(":first-plugin") for name in names),
                profile == "first",
            )
            self.assertEqual(skills_tool.SKILLS_DIR, original)
        with patch.object(
            self.plugins, "discover_plugins", side_effect=RuntimeError("unavailable")
        ):
            self.assertEqual(self.inventory("first").status_code, 500)
        self.assertEqual(skills_tool.SKILLS_DIR, original)
        self.assertEqual(self.inventory("second").status_code, 200)

    def test_refresh_after_native_cache_expiry(self):
        import tools.skills_tool as skills_tool

        home = self.prepare("refresh")
        self.inventory("refresh")
        (home / "skills" / "local" / "SKILL.md").unlink()
        self.write_skill(home / "skills" / "added", "added")
        (home / "skills" / "disabled" / "SKILL.md").write_text(
            "---\nname: disabled\ndescription: Edited description\n---\n"
        )
        with patch.object(skills_tool, "_SKILLS_CACHE_TTL_SECONDS", 0):
            response = self.inventory("refresh")
        self.assertEqual(response.status_code, 200, response.text)
        names = {row["name"] for row in response.json()["skills"]}
        self.assertIn("added", names)
        self.assertNotIn("local", names)
        self.assertEqual(
            next(
                row["description"] for row in response.json()["skills"] if row["name"] == "disabled"
            ),
            "Edited description",
        )

    def test_incompatible_plugin_skills_are_hidden(self):
        home = self.prepare("platform")
        skill = home / "plugins" / "inventory" / "skills" / "plugin-skill" / "SKILL.md"
        skill.write_text(
            "---\nname: plugin-skill\ndescription: Platform-gated skill\nplatforms: [unsupported-test-os]\n---\nBody\n"
        )
        response = self.inventory("platform")
        self.assertEqual(response.status_code, 200, response.text)
        self.assertFalse(
            any(row["name"].endswith(":plugin-skill") for row in response.json()["skills"])
        )

    def test_external_precedence_and_disabled_plugin(self):
        import yaml

        home = self.prepare("external")
        external = home / "external-skills"
        self.write_skill(external / "external-only", "external-only")
        self.write_skill(external / "local", "local")
        (external / "local" / "SKILL.md").write_text(
            "---\nname: local\ndescription: Must not override local\n---\n"
        )
        config = yaml.safe_load((home / "config.yaml").read_text())
        config["skills"]["external_dirs"] = [str(external)]
        config["plugins"]["disabled"] = ["inventory.test"]
        (home / "config.yaml").write_text(yaml.safe_dump(config))
        response = self.inventory("external")
        self.assertEqual(response.status_code, 200, response.text)
        rows = {row["name"]: row for row in response.json()["skills"]}
        self.assertIn("external-only", rows)
        self.assertEqual(rows["local"]["description"], "Example local")
        self.assertFalse(any(name.endswith(":plugin-skill") for name in rows))

    def test_refresh_does_not_reload_plugin_registration(self):
        home = self.prepare("registration")
        first = self.inventory("registration").json()
        self.write_skill(home / "plugins" / "inventory" / "skills" / "new-plugin", "new-plugin")
        self.assertEqual(self.inventory("registration").json(), first)

    def test_default_request_uses_current_home(self):
        (self.home / "config.yaml").write_text("{}\n")
        self.write_skill(self.home / "skills" / "current-home", "current-home")
        response = self.client.get("/api/skills?inventory=true")
        self.assertEqual(response.status_code, 200, response.text)
        self.assertEqual([row["name"] for row in response.json()["skills"]], ["current-home"])

    def test_authentication_required(self):
        from starlette.testclient import TestClient

        client = TestClient(self.server.app)
        self.addCleanup(client.close)
        response = client.get("/api/skills?inventory=true")
        self.assertIn(response.status_code, (401, 403), response.text)
        response = client.get(
            "/api/skills?inventory=true", headers={"Authorization": "Bearer invalid"}
        )
        self.assertIn(response.status_code, (401, 403), response.text)


if __name__ == "__main__":
    unittest.main()
