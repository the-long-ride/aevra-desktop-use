//! Query all retained rows before choosing the visible page.
use super::activity::{ActivityRow, Tone};
use desktop_policy::activity::ActivityEntry;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Time,
    Tool,
    App,
    Result,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub search: String,
    pub verdict: Option<Tone>,
    pub app: Option<String>,
    pub sort: Sort,
    pub descending: bool,
    pub page: usize,
    pub page_size: usize,
}
impl Default for Query {
    fn default() -> Self {
        Self {
            search: String::new(),
            verdict: None,
            app: None,
            sort: Sort::Time,
            descending: true,
            page: 0,
            page_size: 25,
        }
    }
}
#[derive(Clone)]
pub struct Page {
    pub rows: Vec<ActivityRow>,
    pub total: usize,
    pub pages: usize,
    pub start: usize,
    pub end: usize,
}
impl Query {
    pub fn select_sort(&mut self, sort: Sort) {
        if self.sort == sort {
            self.descending = !self.descending;
        } else {
            self.sort = sort;
            self.descending = sort == Sort::Time;
        }
        self.page = 0;
    }
    pub fn run(&mut self, entries: &[ActivityEntry]) -> Page {
        let needle = self.search.trim().to_lowercase();
        let mut rows = super::activity::rows(entries);
        rows.retain(|row| {
            self.verdict.is_none_or(|tone| row.tone == tone)
                && self.app.as_ref().is_none_or(|app| &row.app == app)
                && (needle.is_empty()
                    || [&row.tool, &row.app, &row.outcome]
                        .iter()
                        .any(|value| value.to_lowercase().contains(&needle)))
        });
        rows.sort_by(|a, b| {
            let order = match self.sort {
                Sort::Time => a.timestamp.cmp(&b.timestamp),
                Sort::Tool => a.tool.to_lowercase().cmp(&b.tool.to_lowercase()),
                Sort::App => a.app.to_lowercase().cmp(&b.app.to_lowercase()),
                Sort::Result => a.outcome.to_lowercase().cmp(&b.outcome.to_lowercase()),
            };
            if self.descending {
                order.reverse()
            } else {
                order
            }
        });
        let total = rows.len();
        let size = self.page_size.max(1);
        let pages = total.div_ceil(size).max(1);
        self.page = self.page.min(pages - 1);
        let offset = self.page * size;
        let end = (offset + size).min(total);
        Page {
            rows: rows.into_iter().skip(offset).take(size).collect(),
            total,
            pages,
            start: if total == 0 { 0 } else { offset + 1 },
            end,
        }
    }
}

#[cfg(test)]
#[path = "activity_query_tests.rs"]
mod tests;
