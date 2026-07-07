use super::*;

/// Build a local plaintext search index from already-opened pages.
pub fn build_local_search_index(opened_pages: &[OpenedPage]) -> Vec<LocalSearchDocument> {
    opened_pages
        .iter()
        .map(|page| LocalSearchDocument {
            folder_id: page.folder_id.clone(),
            object_id: page.object_id.clone(),
            page_path: page.page_path.clone(),
            title: markdown_title(&page.markdown)
                .unwrap_or_else(|| title_from_path(&page.page_path)),
            body: page.markdown.clone(),
        })
        .collect()
}
