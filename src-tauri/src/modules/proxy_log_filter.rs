/// Shared by the row and count queries: request-ID search must paginate identically.
pub(crate) const PREDICATE: &str = "(?2=0 OR status<200 OR status>=400) AND
 (id LIKE ?1 OR url LIKE ?1 OR method LIKE ?1 OR model LIKE ?1 OR CAST(status AS TEXT) LIKE ?1 OR account_email LIKE ?1 OR client_ip LIKE ?1 OR session_id LIKE ?1)";

#[cfg(test)]
mod tests {
    use super::PREDICATE;
    use rusqlite::{params, Connection};
    #[test]
    fn request_id_search_and_error_filter_have_matching_counts_and_pages() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE request_logs(id TEXT,status INTEGER,url TEXT,method TEXT,model TEXT,account_email TEXT,client_ip TEXT,session_id TEXT); INSERT INTO request_logs(id,status) VALUES('bill_a',200),('bill_b',502),('other',403);").unwrap();
        for (filter, errors, expected) in [
            ("%bill_%", false, vec!["bill_a", "bill_b"]),
            ("%bill_%", true, vec!["bill_b"]),
            ("%absent%", false, vec![]),
            ("%%", true, vec!["bill_b", "other"]),
        ] {
            let count: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM request_logs WHERE {PREDICATE}"),
                    params![filter, errors],
                    |r| r.get(0),
                )
                .unwrap();
            let mut ids = Vec::<String>::new();
            for offset in 0..=expected.len() {
                let mut query=conn.prepare(&format!("SELECT id FROM request_logs WHERE {PREDICATE} ORDER BY id LIMIT ?3 OFFSET ?4")).unwrap();
                ids.extend(
                    query
                        .query_map(params![filter, errors, 1, offset], |r| {
                            r.get::<_, String>(0)
                        })
                        .unwrap()
                        .map(Result::unwrap),
                );
            }
            assert_eq!(ids, expected);
            assert_eq!(count as usize, ids.len());
        }
    }
}
