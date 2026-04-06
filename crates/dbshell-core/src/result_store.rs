use std::collections::{HashMap, VecDeque};

use uuid::Uuid;

use crate::result::ResultSet;

const MAX_RESULTS: usize = 100;

pub struct ResultStore {
    results: HashMap<String, ResultSet>,
    order: VecDeque<String>,
    last: Option<String>,
}

impl ResultStore {
    pub fn new() -> Self {
        ResultStore {
            results: HashMap::new(),
            order: VecDeque::new(),
            last: None,
        }
    }

    /// Store a result set and return its UUID. Evicts the oldest entry
    /// if the store exceeds MAX_RESULTS.
    pub fn store(&mut self, result: ResultSet) -> String {
        let id = Uuid::new_v4().to_string();

        if self.order.len() >= MAX_RESULTS {
            if let Some(old_id) = self.order.pop_front() {
                self.results.remove(&old_id);
            }
        }

        self.results.insert(id.clone(), result);
        self.order.push_back(id.clone());
        self.last = Some(id.clone());
        id
    }

    /// Get a result by ID. The special ID "last" resolves to the most recent.
    pub fn get(&self, id: &str) -> Option<&ResultSet> {
        if id == "last" {
            return self.last();
        }
        self.results.get(id)
    }

    pub fn last(&self) -> Option<&ResultSet> {
        self.last.as_ref().and_then(|id| self.results.get(id))
    }

    pub fn list_ids(&self) -> Vec<String> {
        self.order.iter().cloned().collect()
    }
}

impl Default for ResultStore {
    fn default() -> Self {
        Self::new()
    }
}
