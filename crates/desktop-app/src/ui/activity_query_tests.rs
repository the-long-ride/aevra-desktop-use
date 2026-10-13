use super::*;
use desktop_policy::activity::ActivityEntry;
fn entry(day: u32, tool: &str, app: &str, verdict: &str) -> ActivityEntry {
    let mut e = ActivityEntry::new(tool, verdict);
    e.ts = format!("2026-10-{day:02}T08:00:00.000Z");
    e.app = Some(app.into());
    e
}
#[test]
fn filters_and_search_apply_before_pagination_across_more_than_200_rows() {
    let mut entries = vec![entry(1, "desktop_windows", "Editor", "allow"); 240];
    entries[0] = entry(1, "desktop_invoke", "Notepad", "deny");
    entries[0].code = Some("DESKTOP_APP_NOT_ALLOWED".into());
    let mut q = Query {
        search: "  not_allowed ".into(),
        verdict: Some(Tone::Denied),
        app: Some("Notepad".into()),
        ..Query::default()
    };
    let page = q.run(&entries);
    assert_eq!(page.total, 1);
    assert_eq!(page.rows[0].app, "Notepad");
    q.search = "EDITOR".into();
    q.verdict = None;
    q.app = None;
    assert_eq!(q.run(&entries).total, 239);
}
#[test]
fn timestamp_sort_uses_the_date_and_clamps_out_of_range_pages() {
    let entries = vec![entry(13, "z", "Z", "allow"), entry(12, "a", "A", "error")];
    let mut q = Query {
        page: 99,
        ..Query::default()
    };
    let page = q.run(&entries);
    assert_eq!(q.page, 0);
    assert_eq!(page.rows[0].tool, "z");
    q.select_sort(Sort::Time);
    assert_eq!(q.run(&entries).rows[0].tool, "a");
}
#[test]
fn every_column_sorts_and_changing_sort_resets_page() {
    let entries = vec![entry(12, "z", "A", "error"), entry(13, "a", "Z", "allow")];
    let mut q = Query {
        page: 4,
        ..Query::default()
    };
    q.select_sort(Sort::Tool);
    assert_eq!(q.page, 0);
    assert_eq!(q.run(&entries).rows[0].tool, "a");
    q.select_sort(Sort::Tool);
    assert_eq!(q.run(&entries).rows[0].tool, "z");
    q.select_sort(Sort::App);
    assert_eq!(q.run(&entries).rows[0].app, "A");
    q.select_sort(Sort::Result);
    assert_eq!(q.run(&entries).rows[0].outcome, "error");
}
#[test]
fn page_ranges_cover_matching_rows_once_and_empty_results_are_safe() {
    let entries = vec![entry(12, "a", "A", "allow"); 61];
    let mut q = Query::default();
    let first = q.run(&entries);
    assert_eq!((first.start, first.end, first.pages), (1, 25, 3));
    q.page = 2;
    let last = q.run(&entries);
    assert_eq!((last.start, last.end, last.rows.len()), (51, 61, 11));
    q.search = "missing".into();
    let empty = q.run(&entries);
    assert_eq!(
        (empty.start, empty.end, empty.total, empty.pages),
        (0, 0, 0, 1)
    );
    assert_eq!(q.page, 0);
}
#[test]
fn identical_timestamps_keep_newest_log_entry_first_and_query_is_private() {
    let entries = vec![
        entry(12, "older", "A", "allow"),
        entry(12, "newer", "A", "allow"),
    ];
    let mut q = Query::default();
    assert_eq!(q.run(&entries).rows[0].tool, "newer");
    q.search = "secret-typed-value".into();
    assert_eq!(q.run(&entries).total, 0);
}
