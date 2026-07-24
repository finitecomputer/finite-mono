# BM25, Dense Retrieval, and RAG for a Large LLM Wiki

Status: source-grounded research note, 2026-07-18

Scope: recent primary research and first-party evaluations relevant to search
over a very large Markdown wiki. Results from general retrieval benchmarks do
not automatically transfer to FiniteBrain; a corpus-specific evaluation remains
necessary.

## Short answer

**BM25 is not an alternative to RAG.** RAG is the overall pattern of retrieving
material and giving it to a model; BM25 is one inexpensive way to perform the
retrieval. The useful comparison is therefore:

- lexical RAG: retrieve with BM25;
- dense RAG: retrieve with embeddings;
- hybrid RAG: retrieve with both, then combine or rerank the results; or
- no retrieval: let an agent navigate files or load a sufficiently small corpus
  directly into its context window.

Recent evidence does not establish a universal winner. It shows a stable
tradeoff:

- BM25 is strong for exact names, error strings, code symbols, rare terms, and
  shared vocabulary. It is cheap, local, explainable, and easy to rebuild.
- Dense retrieval helps when a query and the relevant page express the same idea
  with different words.
- Hybrid retrieval usually improves average robustness because the two signals
  fail differently, but it adds an embedding pipeline and still does not solve
  every document-context or multi-hop problem.
- Reranking can improve which of the retrieved candidates reach the model, but
  it adds latency and helps only if the first-stage retrievers found the right
  material.

For FiniteBrain, the practical next layer remains **section-level BM25 over the
authorized Markdown files**, with the agent opening and reasoning over the
original files. Dense retrieval should be a later, evidence-driven addition,
not the default architecture.

## What recent primary evidence says

### BM25 and dense retrieval are complementary

Anthropic's 2024 Contextual Retrieval evaluation covered codebases, fiction,
ArXiv papers, and scientific papers. With its best tested embedding
configuration and top-20 retrieval, contextual embeddings reduced its retrieval
failure rate from 5.7% to 3.7%; adding contextual BM25 reduced it further to
2.9%. Anthropic also found reranking beneficial. This is good evidence that
lexical and dense signals can stack, but it is a first-party experiment with
Anthropic's own corpus construction, contextualization process, models, and
failure metric—not a universal expected improvement for another wiki.
([Anthropic, *Contextual Retrieval in AI Systems*, 2024](https://www.anthropic.com/engineering/contextual-retrieval))

The EMNLP 2024 study *Searching for Best Practices in Retrieval-Augmented
Generation* likewise found dense retrieval and BM25 complementary: dense search
handled semantic relationships while BM25 recovered rare terminology and
out-of-vocabulary strings. In its end-to-end experiments, ordinary hybrid
retrieval improved the average score over dense-only retrieval from 0.273 to
0.318 with essentially the same measured latency (1.44 versus 1.45 seconds).
Hybrid plus HyDE reached 0.353, but latency rose to 11.71 seconds. Its numbers
come from a particular multi-dataset pipeline and should be read as a tradeoff,
not a product forecast.
([Wang et al., EMNLP 2024](https://aclanthology.org/2024.emnlp-main.981/))

A 2025 EMNLP paper tested a mixture of sparse, dense, and other retrievers on
diverse queries. Its 0.8B-parameter mixture outperformed every individual
retriever by 10.8% on average and even larger 7B retrievers by 3.9% on average.
The important architectural point is not the exact model: different query types
benefited from different retrieval signals, so a fixed retriever did not
generalize best.
([Kalra et al., *MoR*, EMNLP 2025](https://aclanthology.org/2025.emnlp-main.601/))

### Plain BM25 is a serious baseline, not the final word

The ICLR 2025 BRIGHT benchmark deliberately tests 1,384
reasoning-intensive queries across 12 domains, including coding, mathematics,
and technical web material. BM25 was competitive with dense models below one
billion parameters but trailed stronger large dense retrievers. In an
end-to-end StackExchange experiment using Claude 3.5 Sonnet, the reported
average answer score was 77.7 with no retrieved context, 78.3 with BM25, 79.6
with the best tested dense retriever, and 81.8 with oracle documents. This is a
useful example of dense retrieval helping on semantic, reasoning-heavy queries,
but also of modest end-to-end gains: the generator and evaluator can mask
retrieval differences. A pretrained MiniLM reranker actually hurt BM25 on this
out-of-domain benchmark, while Gemini and GPT-4 rerankers helped, illustrating
that reranking is not automatically beneficial without domain fit.
([Su et al., *BRIGHT*, ICLR 2025](https://arxiv.org/abs/2407.12883))

The 2026 SemEval MTRAGEval shared-task report is a useful corrective to the idea
that dense search automatically wins. Two of its top three retrieval systems
were sparse, and the top sparse run achieved nDCG@5 of 0.578 versus 0.548 for
the best dense run and 0.545 for the best hybrid run. However, the organizers'
plain BM25 baseline scored only 0.354. That distinction matters: **sparse
retrieval can be state of the art, while untuned BM25 can still leave substantial
recall on the table.** Shared-task systems also varied in query rewriting,
training, reranking, and other details, so the labels alone do not isolate the
effect of each retrieval family.
([MTRAGEval task report, SemEval 2026](https://aclanthology.org/2026.semeval-1.447/))

### Document structure matters as much as the search algorithm

The ACL 2024 DAPR benchmark evaluated retrieval where a short passage depends
on its parent document. Plain BM25 averaged 28.4 nDCG@10 across five datasets;
the tested neural retrievers averaged 44.9–46.4. Fusing BM25 document retrieval
with neural passage retrieval improved the best average to 48.4. Yet these
hybrid methods still scored below 3.5 on a deliberately hard subset requiring
document context. Simply prepending titles to passages performed far better on
that subset, although it hurt the Genomics dataset.

For a Markdown wiki, this is highly relevant: preserving the file path, page
title, section hierarchy, and nearby text may matter more than switching from
one fashionable retriever to another. No retriever can reliably infer context
that indexing discarded.
([Yang et al., *DAPR*, ACL 2024](https://aclanthology.org/2024.acl-long.236/))

### Retrieval is a cost/attention tradeoff, not always an accuracy win

An EMNLP 2024 comparison found that, when given sufficient resources, loading
long context directly into the tested models outperformed RAG on average; RAG
was substantially cheaper. Its proposed router retained performance close to
long-context processing while using retrieval when appropriate. This supports
the LLM-wiki intuition: if an agent has already narrowed the problem to a
coherent folder or a few files, reading that material directly can be better
than repeatedly reducing it to isolated top-k chunks.
([Li et al., EMNLP Industry 2024](https://aclanthology.org/2024.emnlp-industry.66/))

## Practical recommendation for FiniteBrain

### Add one lightweight read path

Build a disposable local index over only the Folders already decrypted for the
acting principal:

1. Split Markdown at meaningful headings, not arbitrary token counts.
2. Index the path, page title, heading ancestry, body text, and link targets.
3. Give path, title, and heading matches more weight than ordinary prose.
4. Return ranked sections with short excerpts and exact file locations.
5. Let the agent open the original section, its neighboring sections, and linked
   pages using its normal file tools.

SQLite FTS5 is sufficient for a first implementation and already provides BM25
ranking. The index is derived state: it should contain no knowledge that cannot
be reconstructed from the authorized files, and it should be destroyed or
invalidated on lock, revocation, identity change, or Folder-key rotation.

This is technically a lightweight form of retrieval augmentation when its
results are placed in an agent's context, but it does not require a new
authoritative database, vector service, or automatic answer endpoint.

### Evaluate before adding embeddings

Create a small test set from real wiki searches and separate at least these
query classes:

- exact identifiers, filenames, error messages, and code symbols;
- natural-language questions using the wiki's own vocabulary;
- paraphrases and synonyms absent from the relevant page;
- questions whose answer spans multiple linked pages;
- questions where the correct behavior is “not found”; and
- attempts to retrieve across unauthorized Folders.

Measure section recall at 5 and 20, whether the agent opened the right complete
file, citation accuracy, answer quality, latency, and unauthorized-result count.
The last value must remain zero.

Only add a Folder-local dense index if BM25 repeatedly misses important
paraphrase queries. If that happens, keep BM25, add embeddings in parallel, and
fuse their ranked lists. Only add a reranker if the relevant section is usually
present in the candidate set but appears too low to be used.

## What not to infer from the literature

- “Hybrid wins on average” does not mean it wins for FiniteBrain's queries.
- Retrieval scores do not guarantee better generated answers or faithful
  citations.
- Bigger top-k values can improve recall while distracting the model and
  increasing cost.
- Embeddings are derived plaintext with confidentiality and provider-egress
  implications; they are not harmless metadata.
- Chunking and loss of page context can dominate the choice between BM25 and
  dense retrieval.
- A search layer cannot replace good filenames, headings, `_index.md` pages,
  links, or an agent's deliberate exploration of the wiki.

## Bottom line

For a very large LLM wiki, BM25 is likely the best **next** step, not necessarily
the best **last** step. It captures most exact and domain-specific lookup needs
at very low complexity. Preserve the complete folder-and-file world for agent
reasoning, measure real failures, and add dense retrieval only to cover the
semantic misses that actually occur.

The governing design remains:

> One authoritative write path: folders and Markdown. Multiple optional,
> rebuildable read paths—added only when measured search failures justify them.
