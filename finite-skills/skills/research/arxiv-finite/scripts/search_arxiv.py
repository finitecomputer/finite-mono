#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import sys
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from typing import Any

ARXIV_URL = "https://export.arxiv.org/api/query"
SEMANTIC_SCHOLAR_GRAPH_URL = "https://api.semanticscholar.org/graph/v1"
SEMANTIC_SCHOLAR_RECOMMEND_URL = "https://api.semanticscholar.org/recommendations/v1/papers/"
ATOM_NS = {"a": "http://www.w3.org/2005/Atom", "arxiv": "http://arxiv.org/schemas/atom"}
OPENSEARCH_NS = "{http://a9.com/-/spec/opensearch/1.1/}"


def get_json(url: str, *, method: str = "GET", body: dict[str, Any] | None = None) -> dict[str, Any]:
    request = urllib.request.Request(
        url,
        headers={"User-Agent": "finite-arxiv-finite/1.0", "Content-Type": "application/json"},
        method=method,
        data=(json.dumps(body).encode("utf-8") if body is not None else None),
    )
    with urllib.request.urlopen(request, timeout=20) as response:
        return json.loads(response.read().decode("utf-8"))


def get_xml(params: dict[str, Any]) -> ET.Element:
    url = ARXIV_URL + "?" + urllib.parse.urlencode(params)
    request = urllib.request.Request(url, headers={"User-Agent": "finite-arxiv-finite/1.0"})
    with urllib.request.urlopen(request, timeout=20) as response:
        return ET.fromstring(response.read())


def parse_entry(entry: ET.Element) -> dict[str, Any]:
    raw_id = (entry.findtext("a:id", default="", namespaces=ATOM_NS) or "").strip()
    full_id = raw_id.split("/abs/")[-1] if "/abs/" in raw_id else raw_id
    base_id = full_id.split("v")[0]
    summary = (entry.findtext("a:summary", default="", namespaces=ATOM_NS) or "").strip().replace("\n", " ")
    primary_category = entry.find("arxiv:primary_category", ATOM_NS)
    return {
        "id": base_id,
        "versioned_id": full_id,
        "title": (entry.findtext("a:title", default="", namespaces=ATOM_NS) or "").strip().replace("\n", " "),
        "published": (entry.findtext("a:published", default="", namespaces=ATOM_NS) or "")[:10],
        "updated": (entry.findtext("a:updated", default="", namespaces=ATOM_NS) or "")[:10],
        "authors": [author.findtext("a:name", default="", namespaces=ATOM_NS) for author in entry.findall("a:author", ATOM_NS)],
        "summary": summary,
        "categories": [category.get("term") for category in entry.findall("a:category", ATOM_NS)],
        "primary_category": primary_category.get("term") if primary_category is not None else None,
        "abs_url": f"https://arxiv.org/abs/{base_id}",
        "pdf_url": f"https://arxiv.org/pdf/{base_id}",
    }


def render_entries(entries: list[dict[str, Any]], *, total_results: int | None = None) -> str:
    lines: list[str] = []
    if total_results is not None:
        lines.append(f"Found {total_results} results (showing {len(entries)})")
        lines.append("")
    for index, entry in enumerate(entries, start=1):
        authors = ", ".join(entry.get("authors") or [])
        categories = ", ".join(entry.get("categories") or [])
        lines.append(f"{index}. [{entry.get('versioned_id', '?')}] {entry.get('title', '?')}")
        lines.append(f"   Authors: {authors}")
        lines.append(f"   Published: {entry.get('published', '?')} | Updated: {entry.get('updated', '?')}")
        lines.append(f"   Categories: {categories}")
        lines.append(f"   Abstract: {(entry.get('summary', '') or '')[:300]}{'...' if len(entry.get('summary', '') or '') > 300 else ''}")
        lines.append(f"   Links: {entry.get('abs_url', '')} | {entry.get('pdf_url', '')}")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def arxiv_query(*, query: str | None, author: str | None, category: str | None, ids: str | None, max_results: int, sort: str) -> tuple[int | None, list[dict[str, Any]]]:
    params: dict[str, Any] = {"max_results": str(max_results)}
    if ids:
        params["id_list"] = ids
    else:
        parts: list[str] = []
        if query:
            parts.append(f"all:{query}")
        if author:
            parts.append(f"au:{author}")
        if category:
            parts.append(f"cat:{category}")
        if not parts:
            raise SystemExit("Provide --query, --author, --category, or --id.")
        params["search_query"] = "+AND+".join(parts)
        params["sortBy"] = {"relevance": "relevance", "date": "submittedDate", "updated": "lastUpdatedDate"}[sort]
        params["sortOrder"] = "descending"
    root = get_xml(params)
    total_results = root.findtext(f"{OPENSEARCH_NS}totalResults")
    entries = [parse_entry(entry) for entry in root.findall("a:entry", ATOM_NS)]
    return (int(total_results) if total_results is not None else None), entries


def semantic_scholar_id(arxiv_id: str) -> str:
    return f"arXiv:{arxiv_id}"


def cmd_search(args: argparse.Namespace) -> int:
    total, entries = arxiv_query(
        query=args.query,
        author=args.author,
        category=args.category,
        ids=args.id_list,
        max_results=args.max_results,
        sort=args.sort,
    )
    if args.json:
        print(json.dumps({"total_results": total, "entries": entries}, indent=2))
    else:
        print(render_entries(entries, total_results=total), end="")
    return 0


def cmd_bibtex(args: argparse.Namespace) -> int:
    _, entries = arxiv_query(query=None, author=None, category=None, ids=args.id, max_results=1, sort="relevance")
    if not entries:
        raise SystemExit(f"No paper found for {args.id}.")
    entry = entries[0]
    authors = " and ".join(entry["authors"])
    year = (entry.get("published") or "0000")[:4]
    first_author_last = (entry["authors"][0].split()[-1] if entry.get("authors") else "paper")
    citation_key = f"{first_author_last}{year}_{entry['versioned_id'].replace('.', '').replace('/', '')}"
    lines = [
        f"@article{{{citation_key},",
        f"  title = {{{entry['title']}}},",
        f"  author = {{{authors}}},",
        f"  year = {{{year}}},",
        f"  eprint = {{{entry['versioned_id']}}},",
        "  archivePrefix = {arXiv},",
        f"  primaryClass = {{{entry.get('primary_category') or 'cs.LG'}}},",
        f"  url = {{{entry['abs_url']}}}",
        "}",
    ]
    print("\n".join(lines))
    return 0


def render_semantic_papers(items: list[dict[str, Any]]) -> str:
    lines: list[str] = []
    for index, item in enumerate(items, start=1):
        title = item.get("title") or "Untitled"
        year = item.get("year") or "?"
        citations = item.get("citationCount", "?")
        authors = ", ".join(author.get("name", "?") for author in (item.get("authors") or []))
        lines.append(f"{index}. {title} ({year})")
        lines.append(f"   Authors: {authors}")
        lines.append(f"   Citations: {citations}")
        external_ids = item.get("externalIds") or {}
        if external_ids.get("ArXiv"):
            lines.append(f"   arXiv: {external_ids['ArXiv']}")
        if item.get("url"):
            lines.append(f"   {item['url']}")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def cmd_semantic_paper(args: argparse.Namespace) -> int:
    fields = "title,authors,citationCount,referenceCount,influentialCitationCount,year,abstract,externalIds,url"
    payload = get_json(f"{SEMANTIC_SCHOLAR_GRAPH_URL}/paper/{semantic_scholar_id(args.id)}?fields={fields}")
    if args.json:
        print(json.dumps(payload, indent=2))
    else:
        print(render_semantic_papers([payload]), end="")
        abstract = payload.get("abstract")
        if abstract:
            print(abstract)
    return 0


def cmd_citations(args: argparse.Namespace) -> int:
    fields = "title,authors,year,citationCount,externalIds,url"
    payload = get_json(
        f"{SEMANTIC_SCHOLAR_GRAPH_URL}/paper/{semantic_scholar_id(args.id)}/citations?fields={fields}&limit={args.limit}"
    )
    data = [item.get("citingPaper") or item for item in (payload.get("data") or [])]
    if args.json:
        print(json.dumps(data, indent=2))
    else:
        print(render_semantic_papers(data), end="")
    return 0


def cmd_references(args: argparse.Namespace) -> int:
    fields = "title,authors,year,citationCount,externalIds,url"
    payload = get_json(
        f"{SEMANTIC_SCHOLAR_GRAPH_URL}/paper/{semantic_scholar_id(args.id)}/references?fields={fields}&limit={args.limit}"
    )
    data = [item.get("citedPaper") or item for item in (payload.get("data") or [])]
    if args.json:
        print(json.dumps(data, indent=2))
    else:
        print(render_semantic_papers(data), end="")
    return 0


def cmd_semantic_search(args: argparse.Namespace) -> int:
    fields = "title,authors,year,citationCount,externalIds,url"
    payload = get_json(
        f"{SEMANTIC_SCHOLAR_GRAPH_URL}/paper/search?query={urllib.parse.quote(args.query)}&limit={args.limit}&fields={fields}"
    )
    data = payload.get("data") or []
    if args.json:
        print(json.dumps(data, indent=2))
    else:
        print(render_semantic_papers(data), end="")
    return 0


def cmd_author_search(args: argparse.Namespace) -> int:
    fields = "name,hIndex,citationCount,paperCount,url"
    payload = get_json(
        f"{SEMANTIC_SCHOLAR_GRAPH_URL}/author/search?query={urllib.parse.quote(args.query)}&limit={args.limit}&fields={fields}"
    )
    data = payload.get("data") or []
    if args.json:
        print(json.dumps(data, indent=2))
    else:
        for index, author in enumerate(data, start=1):
            print(f"{index}. {author.get('name', '?')}")
            print(f"   h-index={author.get('hIndex', '?')} citations={author.get('citationCount', '?')} papers={author.get('paperCount', '?')}")
            if author.get("url"):
                print(f"   {author['url']}")
            print("")
    return 0


def cmd_recommend(args: argparse.Namespace) -> int:
    payload = get_json(
        SEMANTIC_SCHOLAR_RECOMMEND_URL,
        method="POST",
        body={"positivePaperIds": [semantic_scholar_id(args.id)], "negativePaperIds": []},
    )
    recommendations = payload.get("recommendedPapers") or payload.get("data") or []
    if args.json:
        print(json.dumps(recommendations, indent=2))
    else:
        print(render_semantic_papers(recommendations[: args.limit]), end="")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="arXiv + Semantic Scholar helper")
    subparsers = parser.add_subparsers(dest="command", required=True)

    search = subparsers.add_parser("search", help="Search arXiv")
    search.add_argument("--query")
    search.add_argument("--author")
    search.add_argument("--category")
    search.add_argument("--id-list")
    search.add_argument("--max-results", type=int, default=5)
    search.add_argument("--sort", choices=["relevance", "date", "updated"], default="relevance")
    search.add_argument("--json", action="store_true")
    search.set_defaults(func=cmd_search)

    get_paper = subparsers.add_parser("get", help="Get one or more arXiv papers by ID")
    get_paper.add_argument("--id-list", required=True)
    get_paper.add_argument("--json", action="store_true")
    get_paper.set_defaults(func=cmd_search, query=None, author=None, category=None, max_results=20, sort="relevance")

    bibtex = subparsers.add_parser("bibtex", help="Generate BibTeX for one arXiv paper")
    bibtex.add_argument("--id", required=True)
    bibtex.set_defaults(func=cmd_bibtex)

    semantic_paper = subparsers.add_parser("semantic-paper", help="Get Semantic Scholar details for an arXiv paper")
    semantic_paper.add_argument("--id", required=True)
    semantic_paper.add_argument("--json", action="store_true")
    semantic_paper.set_defaults(func=cmd_semantic_paper)

    citations = subparsers.add_parser("citations", help="List citations of a paper")
    citations.add_argument("--id", required=True)
    citations.add_argument("--limit", type=int, default=10)
    citations.add_argument("--json", action="store_true")
    citations.set_defaults(func=cmd_citations)

    references = subparsers.add_parser("references", help="List references from a paper")
    references.add_argument("--id", required=True)
    references.add_argument("--limit", type=int, default=10)
    references.add_argument("--json", action="store_true")
    references.set_defaults(func=cmd_references)

    semantic_search = subparsers.add_parser("semantic-search", help="Search Semantic Scholar")
    semantic_search.add_argument("--query", required=True)
    semantic_search.add_argument("--limit", type=int, default=5)
    semantic_search.add_argument("--json", action="store_true")
    semantic_search.set_defaults(func=cmd_semantic_search)

    author_search = subparsers.add_parser("author-search", help="Search Semantic Scholar authors")
    author_search.add_argument("--query", required=True)
    author_search.add_argument("--limit", type=int, default=5)
    author_search.add_argument("--json", action="store_true")
    author_search.set_defaults(func=cmd_author_search)

    recommend = subparsers.add_parser("recommend", help="Get related paper recommendations from Semantic Scholar")
    recommend.add_argument("--id", required=True)
    recommend.add_argument("--limit", type=int, default=5)
    recommend.add_argument("--json", action="store_true")
    recommend.set_defaults(func=cmd_recommend)

    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
