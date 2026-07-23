//! Server-side rendering for Document Outputs.
//!
//! The source of truth is the active Version's authored Markdown files. This
//! module renders HTML on request, exposes raw Markdown companion URLs, and
//! keeps raw HTML inert by rendering it as text.

use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use finitesites_engine::{Engine, FoundFile};
use finitesites_store::SiteRecord;
use pulldown_cmark::{Event, Options, Parser, html};
use sha2::Digest as _;

use crate::content_type::content_type_for_path;
use crate::pages;

#[derive(Debug, Clone)]
struct MarkdownPage {
    path: String,
    route: String,
    companion: String,
    title: String,
    sha256: String,
}

pub fn serve_document(
    engine: &Engine,
    site: &SiteRecord,
    request_path: &str,
    headers: &HeaderMap,
    method: &Method,
) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return platform_html(StatusCode::METHOD_NOT_ALLOWED, pages::not_found());
    }

    let files = match engine.active_version_files(site) {
        Ok(files) => files,
        Err(error) => {
            eprintln!("finitesitesd document files error: {error}");
            return platform_html(StatusCode::INTERNAL_SERVER_ERROR, pages::not_found());
        }
    };
    let pages = document_pages(engine, site, &files);

    if request_path == "/llms-full.txt" {
        return llms_full_response(engine, site, &pages, headers, method);
    }

    if request_path.ends_with(".md") {
        if let Some(page) = page_for_companion(&pages, request_path) {
            return markdown_blob_response(engine, site, page, headers, method);
        }
        return platform_html(StatusCode::NOT_FOUND, crate::pages::not_found());
    }

    if let Some(page) = page_for_route(&pages, request_path) {
        return rendered_page_response(engine, site, &pages, page, headers, method);
    }

    if request_path == "/" {
        return generated_index_response(site, &pages, method);
    }

    if let Some(asset) = exact_asset(engine, site, request_path) {
        return blob_response(engine, site, &asset, headers, method, StatusCode::OK);
    }

    platform_html(StatusCode::NOT_FOUND, crate::pages::not_found())
}

fn document_pages(engine: &Engine, site: &SiteRecord, files: &[FoundFile]) -> Vec<MarkdownPage> {
    let entry = engine
        .project_output_for_site(site)
        .ok()
        .flatten()
        .and_then(|(_, output)| output.entry)
        .unwrap_or_else(|| "index.md".to_string());
    let entry_path = ensure_absolute_markdown_path(&entry);

    let mut pages = Vec::new();
    // Bounded by MAX_MANIFEST_FILES.
    for file in files {
        if !file.path.ends_with(".md") {
            continue;
        }
        let route = route_for_markdown_path(&file.path, &entry_path);
        let companion = companion_for_markdown_path(&file.path, &entry_path);
        let title = markdown_title(engine, file).unwrap_or_else(|| title_from_path(&file.path));
        pages.push(MarkdownPage {
            path: file.path.clone(),
            route,
            companion,
            title,
            sha256: file.sha256.clone(),
        });
    }
    pages.sort_by(|left, right| {
        left.route
            .cmp(&right.route)
            .then(left.path.cmp(&right.path))
    });
    pages
}

fn ensure_absolute_markdown_path(entry: &str) -> String {
    let trimmed = entry.trim_start_matches('/');
    if trimmed.is_empty() {
        "/index.md".to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn route_for_markdown_path(path: &str, entry_path: &str) -> String {
    if path == entry_path || path == "/index.md" || path == "/_index.md" {
        return "/".to_string();
    }
    if let Some(prefix) = path.strip_suffix("/index.md") {
        return format!("{prefix}/");
    }
    if let Some(prefix) = path.strip_suffix("/_index.md") {
        return format!("{prefix}/");
    }
    match path.strip_suffix(".md") {
        Some(route) if !route.is_empty() => route.to_string(),
        _ => "/".to_string(),
    }
}

fn companion_for_markdown_path(path: &str, entry_path: &str) -> String {
    if path == entry_path || path == "/index.md" || path == "/_index.md" {
        return "/index.md".to_string();
    }
    path.to_string()
}

fn page_for_route<'a>(pages: &'a [MarkdownPage], request_path: &str) -> Option<&'a MarkdownPage> {
    let normalized = if request_path.is_empty() {
        "/"
    } else {
        request_path
    };
    pages.iter().find(|page| page.route == normalized)
}

fn page_for_companion<'a>(
    pages: &'a [MarkdownPage],
    request_path: &str,
) -> Option<&'a MarkdownPage> {
    pages.iter().find(|page| page.companion == request_path)
}

fn markdown_title(engine: &Engine, file: &FoundFile) -> Option<String> {
    let bytes = engine.read_blob(&file.sha256).ok()?;
    let markdown = String::from_utf8(bytes).ok()?;
    let markdown = strip_frontmatter(&markdown);
    // Bounded by MAX_FILE_BYTES.
    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            let title = title.trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
}

fn title_from_path(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    let stem = trimmed
        .rsplit_once('/')
        .map(|(_, right)| right)
        .unwrap_or(trimmed)
        .trim_end_matches(".md")
        .trim_start_matches('_');
    if stem.is_empty() || stem == "index" {
        "Document".to_string()
    } else {
        stem.replace(['-', '_'], " ")
    }
}

fn llms_full_response(
    engine: &Engine,
    site: &SiteRecord,
    pages: &[MarkdownPage],
    headers: &HeaderMap,
    method: &Method,
) -> Response {
    let mut body = String::new();
    body.push_str("# Full Markdown Snapshot\n\n");
    body.push_str("Document: ");
    body.push_str(&site.name);
    body.push_str("\n\n");
    // Bounded by MAX_MANIFEST_FILES and MAX_FILE_BYTES per file.
    for page in pages {
        body.push_str("## ");
        body.push_str(&page.path);
        body.push_str("\n\n");
        match engine.read_blob(&page.sha256) {
            Ok(bytes) => body.push_str(&String::from_utf8_lossy(&bytes)),
            Err(error) => {
                eprintln!("finitesitesd document llms-full blob error: {error}");
                return platform_html(StatusCode::INTERNAL_SERVER_ERROR, crate::pages::not_found());
            }
        }
        body.push_str("\n\n");
    }
    text_response(body, "text/markdown; charset=utf-8", site, headers, method)
}

fn markdown_blob_response(
    engine: &Engine,
    site: &SiteRecord,
    page: &MarkdownPage,
    headers: &HeaderMap,
    method: &Method,
) -> Response {
    let Some(file) = engine.lookup_exact_file(site, &page.path).ok().flatten() else {
        return platform_html(StatusCode::NOT_FOUND, crate::pages::not_found());
    };
    blob_response(engine, site, &file, headers, method, StatusCode::OK)
}

fn rendered_page_response(
    engine: &Engine,
    site: &SiteRecord,
    pages: &[MarkdownPage],
    page: &MarkdownPage,
    headers: &HeaderMap,
    method: &Method,
) -> Response {
    let bytes = match engine.read_blob(&page.sha256) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("finitesitesd document blob error: {error}");
            return platform_html(StatusCode::INTERNAL_SERVER_ERROR, pages::not_found());
        }
    };
    let markdown = String::from_utf8_lossy(&bytes);
    let html_body = render_markdown(&markdown);
    let page_html = document_shell(site, pages, page, &html_body);
    // The representation includes navigation derived from every Markdown
    // page, so the page blob alone is not a valid cache validator.
    let digest = sha2::Sha256::digest(page_html.as_bytes());
    let etag = format!("\"doc-{}\"", finitesites_proto::hex::encode(&digest));
    let client_etag = headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok());
    if method == Method::GET && client_etag == Some(etag.as_str()) {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(ETAG, etag)
            .header(CACHE_CONTROL, cache_control(site))
            .body(Body::empty())
            .expect("document response builds");
    }
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(page_html)
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .header(ETAG, etag)
        .header(CACHE_CONTROL, cache_control(site))
        .body(body)
        .expect("document response builds")
}

fn generated_index_response(
    site: &SiteRecord,
    pages: &[MarkdownPage],
    method: &Method,
) -> Response {
    let mut body = String::new();
    body.push_str("<h1>");
    body.push_str(&escape_html(&site.name));
    body.push_str("</h1><ul>");
    for page in pages {
        body.push_str("<li><a href=\"");
        body.push_str(&escape_attr(&page.route));
        body.push_str("\">");
        body.push_str(&escape_html(&page.title));
        body.push_str("</a></li>");
    }
    body.push_str("</ul>");
    let page = MarkdownPage {
        path: "/index.md".to_string(),
        route: "/".to_string(),
        companion: "/index.md".to_string(),
        title: site.name.clone(),
        sha256: String::new(),
    };
    let html = document_shell(site, pages, &page, &body);
    let response_body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(html)
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .header(CACHE_CONTROL, cache_control(site))
        .body(response_body)
        .expect("document index response builds")
}

fn exact_asset(engine: &Engine, site: &SiteRecord, request_path: &str) -> Option<FoundFile> {
    engine.lookup_exact_file(site, request_path).ok().flatten()
}

fn blob_response(
    engine: &Engine,
    site: &SiteRecord,
    file: &FoundFile,
    headers: &HeaderMap,
    method: &Method,
    status: StatusCode,
) -> Response {
    let etag = format!("\"{}\"", file.sha256);
    let client_etag = headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok());
    if method == Method::GET && status == StatusCode::OK && client_etag == Some(etag.as_str()) {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(ETAG, etag)
            .header(CACHE_CONTROL, cache_control(site))
            .body(Body::empty())
            .expect("document asset response builds");
    }
    let bytes = match engine.read_blob(&file.sha256) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("finitesitesd document asset blob error: {error}");
            return platform_html(StatusCode::INTERNAL_SERVER_ERROR, pages::not_found());
        }
    };
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(bytes)
    };
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type_for_path(&file.path))
        .header(ETAG, etag)
        .header(CACHE_CONTROL, cache_control(site))
        .body(body)
        .expect("document asset response builds")
}

fn text_response(
    body: String,
    content_type: &'static str,
    site: &SiteRecord,
    headers: &HeaderMap,
    method: &Method,
) -> Response {
    let digest = sha2::Sha256::digest(body.as_bytes());
    let etag = format!("\"{}\"", finitesites_proto::hex::encode(&digest));
    let client_etag = headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok());
    if method == Method::GET && client_etag == Some(etag.as_str()) {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(ETAG, etag)
            .header(CACHE_CONTROL, cache_control(site))
            .body(Body::empty())
            .expect("document text response builds");
    }
    let response_body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(body)
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, content_type)
        .header(ETAG, etag)
        .header(CACHE_CONTROL, cache_control(site))
        .body(response_body)
        .expect("document text response builds")
}

fn render_markdown(markdown: &str) -> String {
    let stripped = strip_frontmatter(markdown);
    let with_wikilinks = rewrite_wikilinks(stripped);
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let parser = Parser::new_ext(&with_wikilinks, options).map(|event| match event {
        Event::Html(value) | Event::InlineHtml(value) => Event::Text(value),
        other => other,
    });
    let mut output = String::new();
    html::push_html(&mut output, parser);
    output
}

fn strip_frontmatter(markdown: &str) -> &str {
    let Some(rest) = markdown.strip_prefix("---\n") else {
        return markdown;
    };
    match rest.find("\n---\n") {
        Some(index) => &rest[index + "\n---\n".len()..],
        None => markdown,
    }
}

fn rewrite_wikilinks(markdown: &str) -> String {
    let mut output = String::with_capacity(markdown.len());
    let bytes = markdown.as_bytes();
    let mut index: usize = 0;
    // Bounded by markdown length, itself bounded by MAX_FILE_BYTES.
    while index < bytes.len() {
        if bytes.get(index) == Some(&b'[')
            && bytes.get(index + 1) == Some(&b'[')
            && let Some(end) = find_wikilink_end(bytes, index + 2)
        {
            let inside = &markdown[index + 2..end];
            let (target, label) = match inside.split_once('|') {
                Some((target, label)) => (target.trim(), label.trim()),
                None => (inside.trim(), inside.trim()),
            };
            if !target.is_empty() && !label.is_empty() {
                output.push('[');
                output.push_str(label);
                output.push_str("](");
                output.push_str(&wikilink_href(target));
                output.push(')');
                index = end + 2;
                continue;
            }
        }
        let character = markdown[index..]
            .chars()
            .next()
            .expect("index is inside markdown");
        output.push(character);
        index += character.len_utf8();
    }
    output
}

fn find_wikilink_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start;
    while index + 1 < bytes.len() {
        if bytes[index] == b']' && bytes[index + 1] == b']' {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn wikilink_href(target: &str) -> String {
    let mut trimmed = target.trim().trim_matches('/').trim_end_matches(".md");
    if trimmed.is_empty() {
        trimmed = "index";
    }
    let mut href = String::from("/");
    for (index, part) in trimmed.split('/').enumerate() {
        if index > 0 {
            href.push('/');
        }
        href.push_str(&part.replace(' ', "-"));
    }
    href
}

fn document_shell(
    site: &SiteRecord,
    pages: &[MarkdownPage],
    page: &MarkdownPage,
    content: &str,
) -> String {
    let mut nav = String::new();
    for nav_page in pages {
        if nav_page.route == page.route {
            nav.push_str("<a class=\"active\" aria-current=\"page\" href=\"");
        } else {
            nav.push_str("<a href=\"");
        }
        nav.push_str(&escape_attr(&nav_page.route));
        nav.push_str("\">");
        nav.push_str(&escape_html(&nav_page.title));
        nav.push_str("</a>");
    }
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title}</title>\
         <link rel=\"llms\" href=\"/llms.txt\">\
         <link rel=\"alternate\" type=\"text/markdown\" href=\"{companion}\">\
         <style>{style}</style></head><body>\
         <aside><div class=\"brand\">{name}</div><nav>{nav}</nav></aside>\
         <main><div class=\"agent-links\"><a href=\"/llms.txt\">llms.txt</a>\
         <a href=\"/llms-full.txt\">llms-full.txt</a>\
         <a href=\"{companion}\">Markdown</a></div><article>{content}</article></main>\
         </body></html>",
        title = escape_html(&page.title),
        companion = escape_attr(&page.companion),
        style = DOCUMENT_STYLE,
        name = escape_html(&site.name),
        nav = nav,
        content = content
    )
}

fn cache_control(site: &SiteRecord) -> &'static str {
    // Document routes and assets are mutable across publishes. Keep them
    // uncacheable until the edge is proven to preserve origin validators.
    if site.visibility == finitesites_store::Visibility::Public {
        "no-store"
    } else {
        "private, no-store"
    }
}

fn platform_html(status: StatusCode, body: String) -> Response {
    (status, [(CACHE_CONTROL, "no-store")], Html(body)).into_response()
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(value: &str) -> String {
    escape_html(value).replace('"', "&quot;")
}

const DOCUMENT_STYLE: &str = "
  :root {
    --background: oklch(0.965 0.005 80);
    --foreground: oklch(0.22 0.01 75);
    --card: oklch(1 0.003 80);
    --primary: oklch(0.52 0.14 255);
    --primary-foreground: oklch(0.99 0.005 80);
    --muted: oklch(0.955 0.006 80);
    --muted-foreground: oklch(0.5 0.012 75);
    --accent: oklch(0.94 0.01 75);
    --accent-foreground: oklch(0.28 0.012 75);
    --border: oklch(0.89 0.008 75);
    --code-bg: oklch(0.95 0.008 80);
    --shadow: 0 24px 70px rgb(20 20 20 / 0.10);
    --font-sans: \"Funnel Sans\", -apple-system, BlinkMacSystemFont, \"Segoe UI\", system-ui, sans-serif;
    --font-display: \"Funnel Display\", \"Funnel Sans\", -apple-system, BlinkMacSystemFont, \"Segoe UI\", system-ui, sans-serif;
    --font-mono: \"JetBrains Mono\", ui-monospace, \"SFMono-Regular\", Menlo, Monaco, Consolas, monospace;
    --radius: 0.5rem;
    color-scheme: light;
  }
  @media (prefers-color-scheme: dark) {
    :root {
      --background: oklch(0.155 0.008 75);
      --foreground: oklch(0.94 0.006 80);
      --card: oklch(0.19 0.008 75);
      --primary: oklch(0.68 0.14 255);
      --primary-foreground: oklch(0.15 0.01 255);
      --muted: oklch(0.24 0.008 75);
      --muted-foreground: oklch(0.68 0.012 80);
      --accent: oklch(0.26 0.01 75);
      --accent-foreground: oklch(0.94 0.006 80);
      --border: oklch(1 0 0 / 10%);
      --code-bg: oklch(0.22 0.008 75);
      --shadow: 0 24px 70px rgb(0 0 0 / 0.28);
      color-scheme: dark;
    }
  }
  * { box-sizing: border-box; }
  html {
    font-feature-settings: \"ss01\", \"cv11\", \"kern\";
    -webkit-text-size-adjust: 100%;
  }
  body {
    margin: 0;
    display: grid;
    grid-template-columns: 280px minmax(0, 1fr);
    min-height: 100vh;
    min-height: 100dvh;
    background: var(--background);
    color: var(--foreground);
    font: 0.9375rem/1.55 var(--font-sans);
    letter-spacing: 0;
    -webkit-font-smoothing: antialiased;
  }
  aside {
    border-right: 1px solid var(--border);
    background: var(--card);
    padding: 24px 18px;
  }
  .brand {
    margin: 0 4px 20px;
    color: var(--muted-foreground);
    font-size: 0.75rem;
    font-weight: 600;
    line-height: 1.35;
    overflow-wrap: anywhere;
  }
  nav {
    position: sticky;
    top: 18px;
    display: grid;
    gap: 4px;
  }
  nav a {
    min-width: 0;
    border-radius: var(--radius);
    color: var(--muted-foreground);
    padding: 7px 10px;
    font-size: 0.875rem;
    line-height: 1.35;
    overflow-wrap: anywhere;
    text-decoration: none;
    transition: background 120ms ease, color 120ms ease;
  }
  nav a:hover {
    background: var(--accent);
    color: var(--accent-foreground);
  }
  nav a.active {
    background: var(--accent);
    color: var(--foreground);
    font-weight: 600;
  }
  nav a:focus-visible,
  .agent-links a:focus-visible,
  article a:focus-visible {
    outline: 3px solid color-mix(in srgb, var(--primary), transparent 72%);
    outline-offset: 2px;
  }
  a {
    color: var(--primary);
    text-decoration: none;
  }
  article a:hover { text-decoration: underline; }
  main {
    width: min(100%, 880px);
    padding: 36px 32px 80px;
  }
  .agent-links {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-bottom: 28px;
    color: var(--muted-foreground);
    font-size: 0.75rem;
    line-height: 1.35;
  }
  .agent-links a {
    display: inline-flex;
    min-height: 32px;
    align-items: center;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--card);
    color: var(--muted-foreground);
    padding: 0 12px;
    font-weight: 500;
    text-decoration: none;
    transition: background 120ms ease, border-color 120ms ease, color 120ms ease;
  }
  .agent-links a:hover {
    border-color: color-mix(in srgb, var(--primary), var(--border) 55%);
    color: var(--foreground);
  }
  article {
    color: var(--foreground);
    overflow-wrap: break-word;
  }
  article :is(h1,h2,h3) {
    color: var(--foreground);
    font-family: var(--font-display);
    line-height: 1.16;
    letter-spacing: 0;
  }
  article h1 {
    margin: 0 0 22px;
    font-size: clamp(2rem, 1.7rem + 1vw, 2.75rem);
    font-weight: 600;
  }
  article h2 {
    margin: 42px 0 14px;
    font-size: 1.5rem;
    font-weight: 600;
  }
  article h3 {
    margin: 30px 0 10px;
    font-size: 1.125rem;
    font-weight: 600;
  }
  article p,
  article ul,
  article ol,
  article blockquote,
  article table,
  article pre {
    margin: 0 0 18px;
  }
  article p, article li { color: var(--foreground); }
  article li + li { margin-top: 6px; }
  article code {
    border: 1px solid var(--border);
    border-radius: 0.375rem;
    padding: 0.1em 0.3em;
    background: var(--code-bg);
    color: var(--foreground);
    font-family: var(--font-mono);
    font-size: 0.875em;
    font-variant-ligatures: none;
  }
  article pre {
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--code-bg);
    box-shadow: var(--shadow);
    padding: 16px;
  }
  article pre code { border: 0; padding: 0; }
  article table {
    display: block;
    width: 100%;
    overflow-x: auto;
    border-collapse: collapse;
  }
  article th, article td {
    border: 1px solid var(--border);
    padding: 8px 10px;
    text-align: left;
    vertical-align: top;
  }
  article th {
    background: var(--muted);
    font-weight: 600;
  }
  article blockquote {
    margin-left: 0;
    margin-right: 0;
    padding-left: 16px;
    border-left: 3px solid var(--border);
    color: var(--muted-foreground);
  }
  article img {
    max-width: 100%;
    height: auto;
    border-radius: var(--radius);
  }
  @media (max-width: 760px) {
    body { display: block; }
    aside {
      border-right: 0;
      border-bottom: 1px solid var(--border);
      padding: 18px;
    }
    .brand { margin-bottom: 12px; }
    nav {
      position: static;
      grid-auto-flow: column;
      grid-auto-columns: max-content;
      overflow-x: auto;
      padding-bottom: 2px;
    }
    nav a { white-space: nowrap; }
    main { padding: 26px 18px 60px; }
    article h1 { font-size: 2rem; }
  }
";

#[cfg(test)]
mod tests {
    use super::*;
    use finitesites_store::{SiteKind, SiteStatus};

    #[test]
    fn markdown_renderer_strips_frontmatter_and_escapes_html() {
        let html = render_markdown("---\ntitle: Hi\n---\n# Hi\n\n<div>raw</div>");
        assert!(html.contains("<h1>Hi</h1>"));
        assert!(html.contains("&lt;div&gt;raw&lt;/div&gt;"));
        assert!(!html.contains("title: Hi"));
    }

    #[test]
    fn wikilinks_become_markdown_links() {
        let rewritten = rewrite_wikilinks("Café says see [[Other Page|the page]] and [[Plain]].");
        assert_eq!(
            rewritten,
            "Café says see [the page](/Other-Page) and [Plain](/Plain)."
        );
    }

    #[test]
    fn routes_are_clean_and_root_entry_has_companion() {
        assert_eq!(route_for_markdown_path("/index.md", "/index.md"), "/");
        assert_eq!(route_for_markdown_path("/guide.md", "/index.md"), "/guide");
        assert_eq!(
            route_for_markdown_path("/section/index.md", "/index.md"),
            "/section/"
        );
        assert_eq!(
            companion_for_markdown_path("/index.md", "/index.md"),
            "/index.md"
        );
    }

    #[test]
    fn document_shell_uses_finite_tokens_and_marks_active_page() {
        let site = SiteRecord {
            id: "site_test".to_string(),
            name: "design-docs".to_string(),
            owner_pubkey: "owner".to_string(),
            status: SiteStatus::Published,
            visibility: finitesites_store::Visibility::Public,
            active_version_id: Some("version_test".to_string()),
            active_version_number: Some(1),
            active_version_spa: false,
            kind: SiteKind::Document,
            app_port: None,
            active_version_start: None,
        };
        let pages = vec![
            MarkdownPage {
                path: "/index.md".to_string(),
                route: "/".to_string(),
                companion: "/index.md".to_string(),
                title: "Home".to_string(),
                sha256: "sha-home".to_string(),
            },
            MarkdownPage {
                path: "/guide.md".to_string(),
                route: "/guide".to_string(),
                companion: "/guide.md".to_string(),
                title: "Guide".to_string(),
                sha256: "sha-guide".to_string(),
            },
        ];

        let html = document_shell(&site, &pages, &pages[1], "<h1>Guide</h1>");

        assert!(html.contains("class=\"active\" aria-current=\"page\" href=\"/guide\""));
        assert!(html.contains("--background: oklch(0.965 0.005 80)"));
        assert!(html.contains("--font-sans: \"Funnel Sans\""));
        assert!(html.contains("<a href=\"/llms-full.txt\">llms-full.txt</a>"));
    }
}
