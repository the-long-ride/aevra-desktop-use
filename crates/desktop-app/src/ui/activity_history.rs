//! Read bounded, rotated logs away from the UI thread.
use desktop_policy::activity::{ActivityEntry, ActivityLog};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

pub struct History {
    pub entries: Vec<ActivityEntry>,
    pub loading: bool,
    pub initialized: bool,
    pub revision: u64,
    sender: Sender<Vec<ActivityEntry>>,
    receiver: Receiver<Vec<ActivityEntry>>,
}
impl Default for History {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            entries: Vec::new(),
            loading: false,
            initialized: false,
            revision: 0,
            sender,
            receiver,
        }
    }
}
impl History {
    pub fn poll(&mut self) {
        if let Ok(entries) = self.receiver.try_recv() {
            if self.entries != entries {
                self.entries = entries;
                self.revision = self.revision.wrapping_add(1);
            }
            self.initialized = true;
            self.loading = false;
        }
    }
    pub fn refresh(&mut self, ctx: &egui::Context, dir: PathBuf) {
        if self.loading {
            return;
        }
        self.loading = true;
        let sender = self.sender.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(ActivityLog::new(dir).retained());
            ctx.request_repaint();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unchanged_refresh_keeps_the_published_revision_stable() {
        let mut history = History::default();
        let entries = vec![ActivityEntry::new("desktop_windows", "allow")];
        history.sender.send(entries.clone()).unwrap();
        history.poll();
        assert!(history.initialized);
        assert_eq!(history.revision, 1);
        history.sender.send(entries.clone()).unwrap();
        history.loading = true;
        history.poll();
        assert_eq!(history.revision, 1);
        assert_eq!(history.entries, entries);
        assert!(!history.loading);
        history.sender.send(Vec::new()).unwrap();
        history.poll();
        assert_eq!(history.revision, 2);
        assert!(history.entries.is_empty());
    }
    #[test]
    fn first_empty_snapshot_finishes_initial_loading_once() {
        let mut history = History::default();
        history.sender.send(Vec::new()).unwrap();
        history.poll();
        assert!(history.initialized);
        assert_eq!(history.revision, 0);
    }
}
