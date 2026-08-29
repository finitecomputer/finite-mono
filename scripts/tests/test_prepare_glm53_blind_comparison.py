from __future__ import annotations

import unittest

from scripts.prepare_glm53_blind_comparison import CASES, make_packet


class Glm53BlindComparisonTests(unittest.TestCase):
    def test_packet_is_complete_blinded_and_deterministic(self) -> None:
        def capture(lane: str) -> dict:
            return {
                "lane": lane,
                "results": [
                    {
                        "case": case["id"],
                        "message": {"content": f"answer-{index}-{lane == 'glm53'}"},
                    }
                    for index, case in enumerate(CASES)
                ],
            }

        packet, key = make_packet(capture("deepseek"), capture("glm53"), "seed")
        repeated_packet, repeated_key = make_packet(
            capture("deepseek"), capture("glm53"), "seed"
        )
        self.assertEqual(packet, repeated_packet)
        self.assertEqual(key, repeated_key)
        self.assertEqual(len(packet["cases"]), len(CASES))
        self.assertTrue(all("lane" not in entry for entry in packet["cases"]))
        self.assertTrue(all("model" not in entry for entry in packet["cases"]))
        for entry in packet["cases"]:
            self.assertIn("correctness", entry["review"]["response_a"])
            self.assertIn("tool_safety", entry["review"]["response_a"])
            self.assertIn("correctness", entry["review"]["response_b"])
            self.assertIn("tool_safety", entry["review"]["response_b"])
        self.assertEqual(
            {entry["a"] for entry in key["cases"]}
            | {entry["b"] for entry in key["cases"]},
            {"deepseek", "glm53"},
        )


if __name__ == "__main__":
    unittest.main()
