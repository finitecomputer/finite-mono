import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("prebuilt", Path(__file__).parents[1] / "prepare_prebuilt_runtime_image.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
REF = "ghcr.io/finitecomputer/agent-runtime@sha256:" + "a" * 64


class PrebuiltTests(unittest.TestCase):
    def metadata(self):
        return {"Os": "linux", "Architecture": "amd64", "RepoDigests": [REF], "Id": "sha256:" + "b" * 64,
                "Config": {"Labels": {"computer.finite.runtime.hermes-agent-version": "0.21.0",
                                       "org.opencontainers.image.revision": "c" * 40}}}

    def test_preserves_real_provenance(self):
        report = module.verified_report(REF, self.metadata(), "0.21.0")
        self.assertEqual(report["mono_sha"], "c" * 40)
        self.assertFalse(report["built_locally"])

    def test_rejects_tags_and_foreign_registries(self):
        for ref in ["ghcr.io/finitecomputer/agent-runtime:latest", REF.replace("ghcr.io", "example.com"), REF.upper()]:
            with self.assertRaises(ValueError): module.validate_reference(ref)

    def test_rejects_wrong_digest_platform_and_version(self):
        for field, value in [("RepoDigests", []), ("Architecture", "arm64"), ("Os", "darwin")]:
            data = self.metadata(); data[field] = value
            with self.assertRaises(ValueError): module.verified_report(REF, data, "0.21.0")
        with self.assertRaises(ValueError): module.verified_report(REF, self.metadata(), "0.22.0")

    def test_missing_provenance_fails(self):
        data = self.metadata(); del data["Config"]["Labels"]["org.opencontainers.image.revision"]
        with self.assertRaises(ValueError): module.verified_report(REF, data, "0.21.0")


if __name__ == "__main__": unittest.main()
