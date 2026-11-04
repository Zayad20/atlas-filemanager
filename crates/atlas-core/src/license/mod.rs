//! Premium entitlement: a Microsoft Store add-on or a Polar.sh license key.
//! Free builds stay limited until one verifies; see `atlas_model::edition`.
pub mod polar;
#[cfg(windows)]
pub mod store;

use crate::util::config::config_dir;
use atlas_model::edition;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// A key last validated within this window keeps working offline.
pub const OFFLINE_GRACE_SECS: u64 = 14 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Store,
    Polar,
}

/// Outcome of asking the license server about a saved key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Valid,
    Invalid,
    Unreachable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Saved {
    key: String,
    activation_id: String,
    validated_at: u64,
}

/// Whether a saved key still unlocks Premium. An explicit rejection always
/// wins; an unreachable server only grants the offline grace window.
pub const fn decide(check: Check, seconds_since_validation: u64) -> bool {
    match check {
        Check::Valid => true,
        Check::Invalid => false,
        Check::Unreachable => seconds_since_validation <= OFFLINE_GRACE_SECS,
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn path() -> PathBuf {
    config_dir().join("license.json")
}

async fn load() -> Option<Saved> {
    serde_json::from_slice(&tokio::fs::read(path()).await.ok()?).ok()
}

/// Write-then-rename so a crash never leaves a half-written license behind.
async fn save(saved: &Saved) {
    let Ok(bytes) = serde_json::to_vec(saved) else {
        return;
    };
    let target = path();
    let temp = target.with_extension("json.tmp");
    let _ = tokio::fs::create_dir_all(config_dir()).await;
    if tokio::fs::write(&temp, bytes).await.is_ok() {
        let _ = tokio::fs::rename(&temp, &target).await;
    }
}

async fn clear() {
    let _ = tokio::fs::remove_file(path()).await;
}

fn machine_label() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Atlas".into())
}

async fn polar_entitled() -> bool {
    let Some(saved) = load().await else {
        return false;
    };
    let age = now().saturating_sub(saved.validated_at);
    let check = match polar::validate(&saved.key, &saved.activation_id).await {
        Ok(true) => Check::Valid,
        Ok(false) | Err(polar::Error::Invalid) => Check::Invalid,
        Err(_) => Check::Unreachable,
    };
    match check {
        Check::Valid => save(&Saved { validated_at: now(), ..saved }).await,
        Check::Invalid => clear().await,
        Check::Unreachable => {}
    }
    decide(check, age)
}

async fn resolve() -> Option<Source> {
    #[cfg(windows)]
    {
        if tokio::task::spawn_blocking(store::owns_premium)
            .await
            .unwrap_or(false)
        {
            return Some(Source::Store);
        }
    }
    polar_entitled().await.then_some(Source::Polar)
}

/// Re-checks every entitlement and applies the result to the running edition.
pub async fn refresh() -> Option<Source> {
    let source = resolve().await;
    edition::set_entitled(source.is_some());
    source
}

/// Binds a Polar key to this device and unlocks Premium immediately.
pub async fn activate(key: &str) -> Result<(), polar::Error> {
    let key = key.trim();
    let activation_id = polar::activate(key, &machine_label()).await?;
    save(&Saved {
        key: key.to_owned(),
        activation_id,
        validated_at: now(),
    })
    .await;
    edition::set_entitled(true);
    Ok(())
}

/// Releases this device's activation slot so the key can move to another PC.
pub async fn deactivate() -> Result<(), polar::Error> {
    if let Some(saved) = load().await {
        match polar::deactivate(&saved.key, &saved.activation_id).await {
            Ok(()) | Err(polar::Error::Invalid) => clear().await,
            Err(other) => return Err(other),
        }
    }
    refresh().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_beats_grace_and_outage_gets_grace() {
        assert!(decide(Check::Valid, u64::MAX));
        assert!(!decide(Check::Invalid, 0));
        assert!(decide(Check::Unreachable, OFFLINE_GRACE_SECS));
        assert!(!decide(Check::Unreachable, OFFLINE_GRACE_SECS + 1));
    }

    #[test]
    fn saved_license_round_trips() {
        let saved = Saved {
            key: "ATLAS-XXXX".into(),
            activation_id: "a1".into(),
            validated_at: 42,
        };
        let json = serde_json::to_vec(&saved).unwrap();
        assert_eq!(serde_json::from_slice::<Saved>(&json).unwrap(), saved);
    }
}
