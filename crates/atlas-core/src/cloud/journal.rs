//! Durable cloud intent/state. Active attempts recovered after a crash are
//! quarantined until their backend outcome is reconciled, never blindly replayed.
use super::transfer::{Transfer, TransferState};
use std::{cell::RefCell, collections::HashMap, path::Path};

pub struct TransferJournal(rusqlite::Connection, RefCell<HashMap<u64, String>>);

impl TransferJournal {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = rusqlite::Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS cloud_transfer (id INTEGER PRIMARY KEY, payload TEXT NOT NULL);")?;
        Ok(Self(connection, RefCell::new(HashMap::new())))
    }

    pub fn load(&self) -> anyhow::Result<Vec<Transfer>> {
        let mut query = self
            .0
            .prepare("SELECT payload FROM cloud_transfer ORDER BY id")?;
        let rows = query.query_map([], |row| row.get::<_, String>(0))?;
        let mut transfers = Vec::new();
        for row in rows {
            let mut transfer: Transfer = serde_json::from_str(&row?)?;
            transfer.state = match transfer.state {
                TransferState::Running | TransferState::Pausing | TransferState::Cancelling =>
                    TransferState::Interrupted("Atlas closed before the backend outcome was recorded. Review the destination before retrying.".into()),
                TransferState::Queued => TransferState::Paused,
                other => other,
            };
            transfers.push(transfer);
        }
        Ok(transfers)
    }

    pub fn save(&self, transfers: &[Transfer]) -> anyhow::Result<()> {
        let mut cached = self.1.borrow_mut();
        let changed = transfers
            .iter()
            .map(|transfer| Ok((transfer.id, serde_json::to_string(transfer)?)))
            .collect::<anyhow::Result<Vec<_>>>()?
            .into_iter()
            .filter(|(id, payload)| cached.get(id) != Some(payload))
            .collect::<Vec<_>>();
        if changed.is_empty() {
            return Ok(());
        }
        let transaction = self.0.unchecked_transaction()?;
        for (id, payload) in &changed {
            transaction.execute(
                "INSERT INTO cloud_transfer(id,payload) VALUES(?1,?2)
                ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",
                rusqlite::params![id, payload],
            )?;
        }
        transaction.execute("DELETE FROM cloud_transfer WHERE (json_extract(payload,'$.state') IN ('Done','Cancelled') OR json_type(payload,'$.state.Failed')='text')
            AND id NOT IN (SELECT id FROM cloud_transfer WHERE json_extract(payload,'$.state') IN ('Done','Cancelled') OR json_type(payload,'$.state.Failed')='text' ORDER BY id DESC LIMIT 256)", [])?;
        transaction.commit()?;
        cached.extend(changed);
        let retained: std::collections::HashSet<_> =
            transfers.iter().map(|transfer| transfer.id).collect();
        cached.retain(|id, _| retained.contains(id));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_preserves_intents_and_never_replays_an_unconfirmed_attempt() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.sqlite");
        let transfer = |id, state| Transfer {
            id,
            state,
            kind: super::super::transfer::TransferKind::Upload,
            label: "backup".into(),
            remote_id: format!("drive-{id}"),
            remote_path: "backup".into(),
            local_path: "source".into(),
            bytes_done: 17,
            bytes_total: 100,
            retries: 2,
            open_on_done: false,
            delete_source: false,
            source: None,
        };
        let journal = TransferJournal::open(&path).unwrap();
        journal
            .save(&[
                transfer(1, TransferState::Running),
                transfer(2, TransferState::Queued),
                transfer(3, TransferState::Done),
            ])
            .unwrap();
        drop(journal);
        let loaded = TransferJournal::open(&path).unwrap().load().unwrap();
        assert!(matches!(loaded[0].state, TransferState::Interrupted(_)));
        assert_eq!(loaded[1].state, TransferState::Paused);
        assert_eq!(loaded[2].state, TransferState::Done);
        assert_eq!(loaded[0].bytes_done, 17);
        assert_eq!(loaded[0].remote_id, "drive-1");
    }
}
