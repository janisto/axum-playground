pub mod cursor;
pub mod link;

use crate::pagination::{cursor::Cursor, link::build_link_header};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageResult<T> {
    pub items: Vec<T>,
    pub total: usize,
    pub link_header: String,
    pub next_cursor: Option<String>,
    pub prev_cursor: Option<String>,
}

pub fn paginate<T, F>(
    items: &[T],
    cursor: &Cursor,
    limit: usize,
    cursor_kind: &str,
    get_id: F,
    base_url: &str,
    query_pairs: &[(String, String)],
) -> PageResult<T>
where
    T: Clone,
    F: Fn(&T) -> &str,
{
    let total = items.len();

    let start_index = if cursor.value.is_empty() {
        0
    } else {
        items
            .iter()
            .position(|item| get_id(item) == cursor.value)
            .map(|index| index + 1)
            .unwrap_or(0)
    };
    let end_index = start_index.saturating_add(limit).min(total);
    let page_items = items[start_index..end_index].to_vec();

    let next_cursor = if end_index < total && !page_items.is_empty() {
        page_items
            .last()
            .map(|item| Cursor::new(cursor_kind, get_id(item)).encode())
    } else {
        None
    };

    let prev_cursor = if start_index > 0 {
        if start_index <= limit {
            Some(Cursor::new(cursor_kind, "").encode())
        } else {
            let prev_last_index = start_index - 1;
            Some(Cursor::new(cursor_kind, get_id(&items[prev_last_index - limit])).encode())
        }
    } else {
        None
    };

    let mut owned_query = query_pairs.to_vec();
    if limit > 0 {
        owned_query.retain(|(key, _)| key != "limit");
        owned_query.push(("limit".to_owned(), limit.to_string()));
    }

    let borrowed_query = owned_query
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();

    PageResult {
        items: page_items,
        total,
        link_header: build_link_header(
            base_url,
            &borrowed_query,
            next_cursor.as_deref(),
            prev_cursor.as_deref(),
        ),
        next_cursor,
        prev_cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::paginate;
    use crate::pagination::cursor::Cursor;

    #[test]
    fn paginate_matches_prev_and_next_cursor_contract() {
        let items = (1..=30)
            .map(|index| format!("item-{index:03}"))
            .collect::<Vec<_>>();
        let cursor = Cursor::new("item", "item-010");

        let page = paginate(
            &items,
            &cursor,
            5,
            "item",
            |item| item.as_str(),
            "/v1/items",
            &[],
        );

        assert_eq!(page.items.first().map(String::as_str), Some("item-011"));
        assert!(page.link_header.contains("rel=\"next\""));
        assert!(page.link_header.contains("rel=\"prev\""));
    }
}
