pub mod cursor;
pub mod link;

use crate::pagination::{
    cursor::{Cursor, CursorDirection, CursorScope, InvalidCursor},
    link::build_link_header,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageResult<T> {
    pub items: Vec<T>,
    pub total: usize,
    pub link_header: String,
    pub next_cursor: Option<String>,
    pub prev_cursor: Option<String>,
}

#[must_use]
pub fn resolve_limit(value: Option<&str>, default: u16, maximum: u16) -> Option<u16> {
    match value {
        None => Some(default),
        Some(value) if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            value
                .parse::<u16>()
                .ok()
                .filter(|value| (1..=maximum).contains(value))
        }
        Some(_) => None,
    }
}

pub fn paginate<T, F>(
    items: &[T],
    cursor: Option<&Cursor>,
    scope: &CursorScope<'_>,
    get_id: F,
    base_url: &str,
    query_pairs: &[(String, String)],
) -> Result<PageResult<T>, InvalidCursor>
where
    T: Clone,
    F: Fn(&T) -> &str,
{
    let limit = usize::from(scope.limit);
    let total = items.len();
    let start = match cursor {
        None => 0,
        Some(cursor) if !cursor.belongs_to(scope) => return Err(InvalidCursor),
        Some(cursor) => {
            let anchor = items
                .iter()
                .position(|item| get_id(item) == cursor.value)
                .ok_or(InvalidCursor)?;
            match cursor.direction {
                CursorDirection::Next => anchor + 1,
                CursorDirection::Prev => anchor.saturating_sub(limit),
            }
        }
    };
    let end = start.saturating_add(limit).min(total);
    let page_items = items[start..end].to_vec();

    let next_cursor = (end < total)
        .then(|| page_items.last())
        .flatten()
        .map(|item| Cursor::new(scope, CursorDirection::Next, get_id(item)).encode());
    let prev_cursor = (start > 0)
        .then(|| page_items.first())
        .flatten()
        .map(|item| Cursor::new(scope, CursorDirection::Prev, get_id(item)).encode());

    let mut owned_query = query_pairs.to_vec();
    owned_query.retain(|(key, _)| key != "limit" && key != "cursor");
    owned_query.push(("limit".to_owned(), scope.limit.to_string()));
    let borrowed_query = owned_query
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let link_header = build_link_header(
        base_url,
        &borrowed_query,
        next_cursor.as_deref(),
        prev_cursor.as_deref(),
    );

    Ok(PageResult {
        items: page_items,
        total,
        link_header,
        next_cursor,
        prev_cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::{paginate, resolve_limit};
    use crate::pagination::cursor::{Cursor, CursorDirection, CursorScope};

    fn scope(limit: u16) -> CursorScope<'static> {
        CursorScope {
            operation: "listItems",
            owner: None,
            repository: None,
            limit,
            category: None,
        }
    }

    #[test]
    fn three_pages_traverse_forward_and_backward_without_skips() {
        let items = (1..=7).map(|n| format!("item-{n}")).collect::<Vec<_>>();
        let first =
            paginate(&items, None, &scope(3), String::as_str, "/v1/items", &[]).expect("first");
        let second_cursor =
            super::cursor::decode_cursor(first.next_cursor.as_deref().unwrap()).expect("next");
        let second = paginate(
            &items,
            Some(&second_cursor),
            &scope(3),
            String::as_str,
            "/v1/items",
            &[],
        )
        .expect("second");
        let third_cursor =
            super::cursor::decode_cursor(second.next_cursor.as_deref().unwrap()).expect("next");
        let third = paginate(
            &items,
            Some(&third_cursor),
            &scope(3),
            String::as_str,
            "/v1/items",
            &[],
        )
        .expect("third");
        assert_eq!(first.items, items[0..3]);
        assert_eq!(second.items, items[3..6]);
        assert_eq!(third.items, items[6..7]);

        let back =
            super::cursor::decode_cursor(third.prev_cursor.as_deref().unwrap()).expect("prev");
        assert_eq!(back.direction, CursorDirection::Prev);
        assert_eq!(
            paginate(
                &items,
                Some(&back),
                &scope(3),
                String::as_str,
                "/v1/items",
                &[]
            )
            .expect("back")
            .items,
            second.items
        );
    }

    #[test]
    fn stale_and_changed_scope_cursors_fail() {
        let items = vec!["a".to_owned(), "b".to_owned()];
        let stale = Cursor::new(&scope(1), CursorDirection::Next, "missing");
        assert!(paginate(&items, Some(&stale), &scope(1), String::as_str, "/x", &[]).is_err());
        let changed = Cursor::new(&scope(2), CursorDirection::Next, "a");
        assert!(paginate(&items, Some(&changed), &scope(1), String::as_str, "/x", &[]).is_err());
    }

    #[test]
    fn limit_resolution_uses_exact_decimal_syntax_and_range() {
        assert_eq!(resolve_limit(None, 20, 100), Some(20));
        assert_eq!(resolve_limit(Some("1"), 20, 100), Some(1));
        assert_eq!(resolve_limit(Some("100"), 20, 100), Some(100));
        for value in ["", "+1", " 1", "1.0", "0", "101", "999999999999"] {
            assert_eq!(resolve_limit(Some(value), 20, 100), None);
        }
    }

    #[test]
    fn pagination_replaces_transport_parameters_and_preserves_filters() {
        let page = paginate(
            &["a".to_owned(), "b".to_owned()],
            None,
            &scope(1),
            String::as_str,
            "/v1/items",
            &[
                ("limit".to_owned(), "99".to_owned()),
                ("cursor".to_owned(), "stale".to_owned()),
                ("category".to_owned(), "tools".to_owned()),
            ],
        )
        .expect("first page");
        assert_eq!(page.link_header.matches("limit=").count(), 1);
        assert_eq!(page.link_header.matches("cursor=").count(), 1);
        assert!(page.link_header.contains("limit=1"));
        assert!(page.link_header.contains("category=tools"));
        assert!(!page.link_header.contains("limit=99"));
        assert!(!page.link_header.contains("cursor=stale"));
    }
}
