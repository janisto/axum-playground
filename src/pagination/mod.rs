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

#[must_use]
pub fn resolve_limit(limit: Option<i64>, default: usize, maximum: usize) -> Option<usize> {
    match limit {
        None => Some(default),
        Some(limit) => usize::try_from(limit)
            .ok()
            .filter(|limit| (1..=maximum).contains(limit)),
    }
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
    owned_query.retain(|(key, _)| key != "limit");
    owned_query.push(("limit".to_owned(), limit.to_string()));

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
    use super::{paginate, resolve_limit};
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

        assert_eq!(
            page.items.iter().map(String::as_str).collect::<Vec<_>>(),
            ["item-011", "item-012", "item-013", "item-014", "item-015"]
        );
        assert_eq!(
            page.next_cursor,
            Some(Cursor::new("item", "item-015").encode())
        );
        assert_eq!(
            page.prev_cursor,
            Some(Cursor::new("item", "item-005").encode())
        );
        assert!(page.link_header.contains("rel=\"next\""));
        assert!(page.link_header.contains("rel=\"prev\""));
    }

    #[test]
    fn paginate_handles_first_and_terminal_page_boundaries() {
        let items = (1..=6)
            .map(|index| format!("item-{index:03}"))
            .collect::<Vec<_>>();

        let first = paginate(
            &items,
            &Cursor::new("", ""),
            3,
            "item",
            |item| item.as_str(),
            "/v1/items",
            &[],
        );
        assert_eq!(first.items, items[..3]);
        assert_eq!(first.prev_cursor, None);
        assert_eq!(
            first.next_cursor,
            Some(Cursor::new("item", "item-003").encode())
        );

        let terminal = paginate(
            &items,
            &Cursor::new("item", "item-003"),
            3,
            "item",
            |item| item.as_str(),
            "/v1/items",
            &[],
        );
        assert_eq!(terminal.items, items[3..]);
        assert_eq!(terminal.next_cursor, None);
        assert_eq!(terminal.prev_cursor, Some(Cursor::new("item", "").encode()));
    }

    #[test]
    fn limit_resolution_accepts_only_the_documented_range() {
        assert_eq!(resolve_limit(None, 20, 100), Some(20));
        assert_eq!(resolve_limit(Some(1), 20, 100), Some(1));
        assert_eq!(resolve_limit(Some(100), 20, 100), Some(100));
        assert_eq!(resolve_limit(Some(0), 20, 100), None);
        assert_eq!(resolve_limit(Some(-1), 20, 100), None);
        assert_eq!(resolve_limit(Some(101), 20, 100), None);
    }
}
