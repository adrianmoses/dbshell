use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Filter {
    All,
    Eq {
        field: String,
        value: serde_json::Value,
    },
    Gt {
        field: String,
        value: serde_json::Value,
    },
    Lt {
        field: String,
        value: serde_json::Value,
    },
    Gte {
        field: String,
        value: serde_json::Value,
    },
    Lte {
        field: String,
        value: serde_json::Value,
    },
    Ne {
        field: String,
        value: serde_json::Value,
    },
    In {
        field: String,
        values: Vec<serde_json::Value>,
    },
    Like {
        field: String,
        pattern: String,
    },
    IsNull {
        field: String,
    },
    Between {
        field: String,
        low: serde_json::Value,
        high: serde_json::Value,
    },
    And(Vec<Filter>),
    Or(Vec<Filter>),
    Not(Box<Filter>),
}

/// Evaluate a `Filter` against a JSON object row. Usable by any driver
/// for client-side filtering when server-side pushdown isn't possible.
pub fn matches_filter(row: &serde_json::Value, filter: &Filter) -> bool {
    match filter {
        Filter::All => true,
        Filter::Eq { field, value } => row.get(field) == Some(value),
        Filter::Ne { field, value } => row.get(field) != Some(value),
        Filter::Gt { field, value } => cmp_field(row, field, value, Ordering::is_gt),
        Filter::Gte { field, value } => cmp_field(row, field, value, Ordering::is_ge),
        Filter::Lt { field, value } => cmp_field(row, field, value, Ordering::is_lt),
        Filter::Lte { field, value } => cmp_field(row, field, value, Ordering::is_le),
        Filter::In { field, values } => row.get(field).is_some_and(|v| values.contains(v)),
        Filter::Like { field, pattern } => row
            .get(field)
            .and_then(|v| v.as_str())
            .is_some_and(|s| like_match(s, pattern)),
        Filter::IsNull { field } => row.get(field).is_none_or(|v| v.is_null()),
        Filter::Between { field, low, high } => {
            cmp_field(row, field, low, Ordering::is_ge)
                && cmp_field(row, field, high, Ordering::is_le)
        }
        Filter::And(filters) => filters.iter().all(|f| matches_filter(row, f)),
        Filter::Or(filters) => filters.iter().any(|f| matches_filter(row, f)),
        Filter::Not(inner) => !matches_filter(row, inner),
    }
}

/// Compare two `serde_json::Value`s. Tries numeric first, then string.
pub fn cmp_json_values(a: &serde_json::Value, b: &serde_json::Value) -> Ordering {
    if let (Some(an), Some(bn)) = (a.as_f64(), b.as_f64()) {
        return an.partial_cmp(&bn).unwrap_or(Ordering::Equal);
    }
    if let (Some(a_s), Some(b_s)) = (a.as_str(), b.as_str()) {
        return a_s.cmp(b_s);
    }
    Ordering::Equal
}

fn cmp_field(
    row: &serde_json::Value,
    field: &str,
    target: &serde_json::Value,
    pred: impl FnOnce(Ordering) -> bool,
) -> bool {
    row.get(field)
        .map(|val| cmp_json_values(val, target))
        .is_some_and(pred)
}

/// SQL-style LIKE matching: `%` matches any sequence, `_` matches one char.
/// Iterative implementation to avoid stack overflow on long inputs.
pub fn like_match(haystack: &str, pattern: &str) -> bool {
    let s = haystack.as_bytes();
    let p = pattern.as_bytes();

    let mut si = 0;
    let mut pi = 0;
    // Bookmarks for backtracking on '%'
    let mut star_pi = usize::MAX;
    let mut star_si = 0;

    while si < s.len() {
        if pi < p.len() && p[pi] == b'%' {
            star_pi = pi;
            star_si = si;
            pi += 1;
        } else if pi < p.len() && (p[pi] == b'_' || s[si].eq_ignore_ascii_case(&p[pi])) {
            si += 1;
            pi += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_si += 1;
            si = star_si;
        } else {
            return false;
        }
    }

    // Consume trailing '%' in pattern
    while pi < p.len() && p[pi] == b'%' {
        pi += 1;
    }

    pi == p.len()
}
