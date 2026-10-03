from __future__ import annotations

import unittest

from scripts.check_finite_private_v41_modelpacks import HOST, PACKS, check_pack


class ModelPackTests(unittest.TestCase):
    def job(self, name="deepseek"):
        return {"job_id": "test-job", "host": HOST, "status": "complete", **PACKS[name]}

    def test_exact_completed_host_pack_passes(self):
        self.assertTrue(check_pack(self.job(), "deepseek", "test-job")["passed"])

    def test_wrong_host_revision_or_any_mpk_component_fails(self):
        for field in ("host", "commit", "repo", "root_hash", "offset", "verity_uuid", "job_id"):
            with self.subTest(field=field):
                job = self.job()
                job[field] = "wrong"
                result = check_pack(job, "deepseek", "test-job")
                self.assertFalse(result["passed"])
                self.assertIn(field, result["mismatches"])

    def test_pending_or_failed_job_cannot_pass_with_expected_metadata(self):
        for state in ("pending", "running", "failed", None):
            with self.subTest(state=state):
                self.assertFalse(check_pack({**self.job(), "status": state},
                                            "deepseek", "test-job")["passed"])

    def test_schema_two_must_be_confirmed_not_merely_requested(self):
        job = self.job()
        job.pop("schema")
        job["schema_requested"] = 2
        self.assertFalse(check_pack(job, "deepseek", "test-job")["passed"])

    def test_original_schema_one_rollback_is_supported(self):
        job = self.job("glm")
        job.pop("schema")
        self.assertTrue(check_pack(job, "glm", "test-job")["passed"])

    def test_error_fails_even_if_status_claims_complete_and_logs_are_not_exposed(self):
        result = check_pack({**self.job(), "error": "provider failure", "logs": "private"},
                            "deepseek", "test-job")
        self.assertFalse(result["passed"])
        self.assertNotIn("private", str(result))


if __name__ == "__main__":
    unittest.main()
