//! Derived monitoring data; no schema changes, counters, or extra writers.
use finitesites_proto::limits::SITE_METRICS_HISTORY_DAYS;
use rusqlite::params;

use crate::{Store, StoreError};

const SECONDS_PER_DAY: u64 = 86_400;

#[derive(Debug, PartialEq, Eq)]
pub struct SiteCreationDay {
    pub start_unix: u64,
    pub count: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SiteMetrics {
    pub existing: u64,
    pub published: u64,
    pub collected_at: u64,
    pub created_by_day: Vec<SiteCreationDay>,
}

impl Store {
    /// One read transaction keeps totals and history on the same SQLite
    /// snapshot, including when Git's other connection commits during a scrape.
    /// Creation history includes soft-deleted sites and excludes bare projects.
    /// Restored/imported rows retain their stored dates; missing legacy rows
    /// cannot be reconstructed from this registry.
    pub fn site_metrics(&self, now: u64) -> Result<SiteMetrics, StoreError> {
        let today = now / SECONDS_PER_DAY;
        let first_day = today.saturating_sub(u64::from(SITE_METRICS_HISTORY_DAYS - 1));
        let end = today
            .checked_add(1)
            .and_then(|d| d.checked_mul(SECONDS_PER_DAY))
            .filter(|end| *end <= i64::MAX as u64)
            .ok_or(StoreError::Conflict("metrics time is out of range"))?;
        let tx = self.conn.unchecked_transaction()?;
        let (existing, published) = tx.query_row(
            "SELECT COUNT(CASE WHEN status != 'deleted' THEN 1 END),
                    COUNT(CASE WHEN status = 'published' THEN 1 END) FROM sites",
            [],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?)),
        )?;
        let mut days: Vec<_> = (first_day..=today)
            .map(|day| SiteCreationDay {
                start_unix: day * SECONDS_PER_DAY,
                count: 0,
            })
            .collect();
        {
            let mut statement = tx.prepare(
                "SELECT created_at / 86400, COUNT(*) FROM sites
                 WHERE created_at >= ?1 AND created_at < ?2
                 GROUP BY created_at / 86400 ORDER BY created_at / 86400",
            )?;
            let rows = statement.query_map(params![first_day * SECONDS_PER_DAY, end], |row| {
                Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?))
            })?;
            // WHERE bounds the grouped source to at most HISTORY_DAYS rows.
            for row in rows {
                let (day, count) = row?;
                let index = usize::try_from(day - first_day)
                    .map_err(|_| StoreError::CorruptState("metrics day index overflow"))?;
                days.get_mut(index)
                    .ok_or(StoreError::CorruptState("metrics day outside window"))?
                    .count = count;
            }
        }
        tx.commit()?;
        assert!(published <= existing);
        assert!(days.len() <= SITE_METRICS_HISTORY_DAYS as usize);
        Ok(SiteMetrics {
            existing,
            published,
            collected_at: now,
            created_by_day: days,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SiteStatus;

    fn create(store: &mut Store, name: &str, time: u64) {
        store
            .create_site_with_claim(name, &format!("claim-{name}"), name, &"a".repeat(64), time)
            .unwrap();
    }

    #[test]
    fn empty_registry_zero_fills_utc_days_and_rejects_invalid_time() {
        let store = Store::open_in_memory().unwrap();
        let metrics = store.site_metrics(200 * SECONDS_PER_DAY + 12).unwrap();
        assert_eq!((metrics.existing, metrics.published), (0, 0));
        assert_eq!(metrics.created_by_day.len(), 90);
        assert!(metrics.created_by_day.iter().all(|day| day.count == 0));
        assert_eq!(metrics.created_by_day[0].start_unix, 111 * SECONDS_PER_DAY);
        assert_eq!(metrics.created_by_day[89].start_unix, 200 * SECONDS_PER_DAY);
        assert!(store.site_metrics(u64::MAX).is_err());
    }

    #[test]
    fn history_survives_lifecycle_replay_reopen_and_read_only_collection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.db");
        let now = 200 * SECONDS_PER_DAY + 3600;
        let mut store = Store::open(&path).unwrap();
        create(&mut store, "old", 111 * SECONDS_PER_DAY - 1);
        create(&mut store, "first", 111 * SECONDS_PER_DAY);
        create(&mut store, "yesterday", 200 * SECONDS_PER_DAY - 1);
        create(&mut store, "today", 200 * SECONDS_PER_DAY);
        // Creation conflict/replay must not add a second allocation.
        assert!(
            store
                .create_site_with_claim("other", "other-claim", "today", &"a".repeat(64), now)
                .is_err()
        );
        store
            .set_site_status_by_name("first", SiteStatus::Published, "publish", now)
            .unwrap();
        store
            .set_site_status_by_name("today", SiteStatus::Published, "publish", now)
            .unwrap();
        store
            .set_site_status_by_name("today", SiteStatus::Published, "republish", now + 1)
            .unwrap();
        store
            .set_site_status_by_name("yesterday", SiteStatus::Deleted, "delete", now)
            .unwrap();
        store
            .set_site_status_by_name("old", SiteStatus::Disabled, "disable", now)
            .unwrap();
        let before = store.site_metrics(now).unwrap();
        assert_eq!((before.existing, before.published), (3, 2));
        assert_eq!(
            before.created_by_day.iter().map(|d| d.count).sum::<u64>(),
            3
        );
        assert_eq!(before.created_by_day[0].count, 1);
        assert_eq!(before.created_by_day[88].count, 1);
        assert_eq!(before.created_by_day[89].count, 1);
        let reader = store.open_reader().unwrap();
        drop(store);
        assert_eq!(reader.site_metrics(now).unwrap(), before);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.site_metrics(now).unwrap(), before);
        // A collection failure is an error, not an empty successful snapshot.
        reopened.conn.execute("DROP TABLE site_events", []).unwrap();
        reopened
            .conn
            .execute("ALTER TABLE sites RENAME TO sites_unavailable", [])
            .unwrap();
        assert!(reopened.site_metrics(now).is_err());
    }
}
