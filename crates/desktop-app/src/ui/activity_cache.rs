use super::activity_query::{Page, Query};
use desktop_policy::activity::ActivityEntry;

#[derive(Default)]
pub struct Cache {
    key: Option<(Query, u64)>,
    page: Option<Page>,
}
impl Cache {
    pub fn resolve(&mut self, query: &mut Query, entries: &[ActivityEntry], revision: u64) -> Page {
        if self
            .key
            .as_ref()
            .is_none_or(|(previous, version)| previous != query || *version != revision)
        {
            self.page = Some(query.run(entries));
            self.key = Some((query.clone(), revision));
        }
        self.page.as_ref().expect("resolved activity page").clone()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_pages_follow_query_and_real_history_revisions() {
        let mut cache = Cache::default();
        let mut query = Query {
            page: 1,
            ..Query::default()
        };
        let entries = vec![ActivityEntry::new("desktop_windows", "allow"); 60];
        let first = cache.resolve(&mut query, &entries, 1);
        assert_eq!((first.start, first.end), (26, 50));
        let again = cache.resolve(&mut query, &entries, 1);
        assert_eq!((again.start, again.end), (26, 50));
        query.search = "missing".into();
        assert_eq!(cache.resolve(&mut query, &entries, 1).total, 0);
        query.search.clear();
        let new_entries = vec![ActivityEntry::new("desktop_windows", "allow"); 70];
        assert_eq!(cache.resolve(&mut query, &new_entries, 2).total, 70);
    }
}
