//! Progressive retention: dense recent history, sparse old history.

use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    /// Keep everything newer than this.
    pub keep_all_hours: i64,
    /// Then the newest per day, for this many days.
    pub daily_days: i64,
    /// Then the newest per ISO week, for this many weeks.
    pub weekly_weeks: i64,
    /// Never go below this many per device.
    pub min_keep: usize,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        RetentionPolicy { keep_all_hours: 24, daily_days: 30, weekly_weeks: 12, min_keep: 20 }
    }
}

#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub device_id: String,
    pub created_ms: i64,
    /// Lineage heads, unpushed, referenced by conflicts/operations, latest per
    /// branch: never pruned.
    pub protected: bool,
}

/// Ids that can be pruned under `policy` at `now_ms`.
pub fn select_prunable(items: &[Item], now_ms: i64, policy: &RetentionPolicy) -> Vec<String> {
    let mut by_dev: BTreeMap<&str, Vec<&Item>> = BTreeMap::new();
    for it in items {
        by_dev.entry(&it.device_id).or_default().push(it);
    }
    let mut prune = Vec::new();
    for (_, mut list) in by_dev {
        list.sort_by(|a, b| b.created_ms.cmp(&a.created_ms).then_with(|| b.id.cmp(&a.id)));
        let mut seen_day = HashSet::new();
        let mut seen_week = HashSet::new();
        for (rank, it) in list.iter().enumerate() {
            let age_h = (now_ms - it.created_ms) / 3_600_000;
            let t = DateTime::<Utc>::from_timestamp_millis(it.created_ms).unwrap_or_default();
            let day = (t.year(), t.ordinal());
            let wk = t.iso_week();
            let week = (wk.year(), wk.week());
            let keep = it.protected
                || rank < policy.min_keep
                || age_h < policy.keep_all_hours
                || (age_h < policy.daily_days * 24 && seen_day.insert(day))
                || (age_h < policy.weekly_weeks * 7 * 24 && seen_week.insert(week));
            // Mark buckets as used even when kept for another reason, so a
            // bucket keeps exactly its newest checkpoint.
            seen_day.insert(day);
            seen_week.insert(week);
            if !keep {
                prune.push(it.id.clone());
            }
        }
    }
    prune
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3_600_000;

    fn item(id: usize, age_h: i64, now: i64) -> Item {
        Item { id: format!("{id:04}"), device_id: "d".into(), created_ms: now - age_h * H, protected: false }
    }

    #[test]
    fn keeps_recent_daily_weekly() {
        let now = 1_750_000_000_000;
        let policy = RetentionPolicy { min_keep: 0, ..Default::default() };
        // One checkpoint every hour for 120 days.
        let items: Vec<Item> = (0..120 * 24).map(|h| item(h as usize, h, now)).collect();
        let pruned: HashSet<String> = select_prunable(&items, now, &policy).into_iter().collect();
        let kept: Vec<&Item> = items.iter().filter(|i| !pruned.contains(&i.id)).collect();
        // all of last 24h
        assert!(items.iter().filter(|i| (now - i.created_ms) / H < 24).all(|i| !pruned.contains(&i.id)));
        // roughly 24 + ~30 daily + ~8 weekly, and nothing older than 12 weeks
        assert!(kept.len() > 24 + 25 && kept.len() < 24 + 31 + 13, "kept {}", kept.len());
        assert!(kept.iter().all(|i| (now - i.created_ms) / H < 12 * 7 * 24));
    }

    #[test]
    fn protected_and_min_keep_survive() {
        let now = 1_750_000_000_000;
        let mut items: Vec<Item> = (0..10).map(|i| item(i, 5000 + i as i64, now)).collect();
        items[9].protected = true;
        let policy = RetentionPolicy { min_keep: 3, ..Default::default() };
        let pruned = select_prunable(&items, now, &policy);
        assert_eq!(pruned.len(), 6);
        assert!(!pruned.contains(&items[9].id));
        assert!(!pruned.contains(&items[0].id));
    }
}
