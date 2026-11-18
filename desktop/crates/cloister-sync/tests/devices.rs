//! Device statuses shown on the Devices screen: a word beside a square.

use chrono::{DateTime, Utc};
use cloister_sync::{DeviceStatus, SyncStatus};

fn t(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

#[test]
fn a_device_is_enforcing_only_while_it_reports_so_recently() {
    let now = t("2026-01-05T09:00:00Z");
    assert_eq!(DeviceStatus::classify(true, t("2026-01-05T08:59:30Z"), now), DeviceStatus::Enforcing);
    assert_eq!(DeviceStatus::classify(true, t("2026-01-05T08:55:00Z"), now), DeviceStatus::Offline);
    assert_eq!(DeviceStatus::classify(false, t("2026-01-05T08:59:59Z"), now), DeviceStatus::Offline);
    assert_eq!(DeviceStatus::Enforcing.label(), "Enforcing");
    assert_eq!(DeviceStatus::Offline.label(), "Offline");
}

#[test]
fn an_offline_device_says_it_is_still_enforcing() {
    assert_eq!(SyncStatus::Offline.label(), "Offline, still enforcing");
}
