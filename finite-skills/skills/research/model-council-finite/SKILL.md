---
name: model-council-finite
description: Use when the user explicitly asks for a council, panel, jury, debate, or multiple frontier models to weigh an important decision, plan, research question, code direction, or strategy.
---

# Model Council

Use this skill only when the user explicitly asks for a model council, a debate
between frontier models, or multiple independent model opinions. This skill is
intentionally expensive compared with a normal answer.

## Workflow

1. Restate the question in one sentence.
2. Choose the closest mode:
   - `decision` for choosing between options.
   - `strategy` for product, business, roadmap, or architecture direction.
   - `research` for an evidence-seeking question.
   - `code-review` for implementation risk and software design critique.
3. If the question depends on current facts, benchmark numbers, vendor docs,
   prices, model availability, legal/compliance details, or citations, gather
   source notes first with the appropriate research skill. Put those notes in a
   file and pass them with `--input-file`.
4. Run the council script:

```bash
python3 ~/.finite/managed-skills/current/research/model-council-finite/scripts/model_council.py \
  --mode decision \
  --question "Should we do X or Y?"
```

5. Read the full output before answering the user.
6. Give the user the synthesis, the strongest disagreements, and your own final
   recommendation.

## Notes

- The script uses `OPENROUTER_API_KEY` from the runtime environment.
- Override the default panel with `MODEL_COUNCIL_MODELS`, using comma-separated
  OpenRouter model IDs.
- For sensitive tasks, do not send secrets, private keys, passwords, tokens, or
  unnecessary private data into the council prompt.
- For research claims that require citations, use a grounded research skill
  first, then feed the source notes into the council.
- Treat council output as deliberation, not proof. If the panel gives concrete
  numbers without supplied sources, label them as estimates before presenting
  them to the user.
