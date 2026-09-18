import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "request_export", Path(__file__).parents[1] / "private_requests" / "export.py"
)
export = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(export)


class ExportContracts(unittest.TestCase):
    def rows(self):
        return [
            {
                "timestampNs": "1789650000000000000",
                "event": {
                    "reservationId": "synthetic-reservation",
                    "completionTokens": 19,
                    "promptTokens": 11,
                    "firstAnswerMs": None,
                },
            }
        ]

    def test_retry_preserves_exact_timestamp_and_line_and_acks_only_after_push(self):
        calls = []
        batches = []

        def send(batch):
            calls.append("push")
            batches.append(batch)

        def query(sql):
            calls.append("ack")
            self.assertIn("exported_at IS NULL", sql)

        export.export_batch(self.rows(), send, query)
        export.export_batch(self.rows(), send, query)
        self.assertEqual(calls, ["push", "ack", "push", "ack"])
        self.assertEqual(batches[0], batches[1])
        event = json.loads(batches[0][0]["values"][0][1])
        self.assertEqual(event["completionTokens"], 19)
        self.assertIsNone(event["firstAnswerMs"])

    def test_failed_push_does_not_ack_or_report_success(self):
        queries = []

        def failed_push(batch):
            raise OSError("synthetic failure")

        with self.assertRaises(OSError):
            export.export_batch(self.rows(), failed_push, queries.append)
        self.assertEqual(queries, [])
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "state.json"
            metrics = Path(directory) / "metrics.prom"
            state.write_text(
                json.dumps(
                    {
                        "collection_started": 50,
                        "last_export": 90,
                        "exported": 7,
                        "failures": 2,
                    }
                )
            )
            self.assertEqual(
                export.run(
                    state, metrics, lambda sql: self.rows(), failed_push, lambda: 100
                ),
                1,
            )
            self.assertEqual(
                json.loads(state.read_text()),
                {
                    "collection_started": 50,
                    "last_export": 90,
                    "exported": 7,
                    "failures": 3,
                },
            )
            self.assertIn("exporter_query_success 0", metrics.read_text())
            self.assertNotIn(
                "synthetic-reservation", state.read_text() + metrics.read_text()
            )

    def test_empty_success_updates_freshness_without_inventing_requests(self):
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "state.json"
            metrics = Path(directory) / "metrics.prom"

            def query(sql):
                return {"pending": 0, "oldest": 0} if "'pending'" in sql else []

            def unexpected_send(batch):
                self.fail("empty database must not invent Loki events")

            self.assertEqual(
                export.run(state, metrics, query, unexpected_send, lambda: 100), 0
            )
            saved = json.loads(state.read_text())
            self.assertEqual(saved["last_export"], 100)
            self.assertEqual(saved["exported"], 0)


if __name__ == "__main__":
    unittest.main()
