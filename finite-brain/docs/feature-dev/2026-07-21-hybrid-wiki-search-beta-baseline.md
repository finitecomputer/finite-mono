# Hybrid Wiki Search Internal-Beta Baseline

Date: 2026-07-21

This is the first repeatable retrieval baseline for Hybrid Wiki Search. It is
not a launch threshold and it is not a claim about production embedding-model
quality. Its purpose is to make later changes measurable against a stable set
of realistic wiki questions.

## Evaluation seam

- Fixture: `finite-brain/evaluations/hybrid-wiki-search-beta.json`
- Command boundary: the real `fbrain search --json` implementation used by the
  CLI tests
- Corpus: six Markdown Pages covering security, recovery, operations, beta
  consent, sharing, and search fallback
- Queries: six natural-language questions paired with expected Page-and-heading
  Sections; each Page also contains a non-relevant competing Section
- Semantic provider: deterministic local fake implementing the production
  adapter contract; this isolates retrieval plumbing from model drift
- Metric: Recall@5 for expected Page-and-heading Sections, plus mean wall-clock
  command latency
- Comparison: automatic hybrid retrieval versus the same query and corpus with
  `--lexical-only`

## First recorded run

The focused test run on 2026-07-21 produced:

| Mode | Recall@5 | Mean command latency |
| --- | ---: | ---: |
| Hybrid | 1.000 (6/6) | 10,657 microseconds |
| Lexical-only | 0.667 (4/6) | 3,406 microseconds |

The latency numbers are a local development-machine snapshot, not a service
SLO. The test always records fresh measurements and verifies the deterministic
hybrid retrieval contract, but deliberately does not fail on a fixed latency or
lexical-quality threshold.

Run the baseline with:

```sh
scripts/with-dev-env cargo test -p finite-brain-cli \
  hybrid_search_beta_fixture_records_quality_and_latency_baseline -- --nocapture
```

## Interpretation

The first baseline shows the intended product shape: BM25 remains fast and
useful for exact vocabulary, while semantic retrieval recovers paraphrased
questions that do not share enough terms with the expected Page. Rank fusion
does not replace the lexical evidence; it combines the independent lists and
reports which signals contributed.

This fixture should grow from internal-beta misses. Production evaluation must
use the selected embedding model, representative customer-scale wikis, and the
actual provider/network placement before any quality or latency target is set.
