//! SQLite owns statistics identities and immutable completed-combat snapshots.
//! The game host decides lifecycle boundaries; attribution never accesses this
//! crate. Payload schema 2 and attribution policy versions are independent of
//! database schema 1, stored in `user_version` under `statistics-v3`.
//!
//! Each completed combat first commits an intent, then its payload. An absent
//! payload therefore remains visible after restart without treating discarded
//! combats or gaps in global IDs as missing records. Failed intents retry at the
//! next explicit save; payloads do not. Process loss before any intent reaches
//! disk can lose that evidence. SQLite provides atomic commits, not proof that
//! an unrecorded game event occurred.
//!
//! One connection uses rollback journaling and FULL synchronization. No pool or
//! background queue reorders lifecycle operations. A 50 ms busy limit bounds
//! lock contention, not filesystem latency. Legacy import is one transaction;
//! the source files stay read-only and the completed marker prevents reimport.

#![deny(unsafe_code)]

mod record;

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::Duration;

use record::{Record, Run};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;
use serde_json::{Value, json};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const APPLICATION_ID: i32 = 0x5354_5052;
const DATABASE_VERSION: i32 = 1;
const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;
const RESPONSE_ENVELOPE_BYTES: usize = 4096;

pub struct Store {
    connection: Connection,
    pending: BTreeMap<u32, Option<Run>>,
}

#[derive(Clone, Copy)]
enum IdentityMatch {
    Missing,
    Unique(u32),
    Ambiguous,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    OpenRun {
        run: Box<Run>,
        continued: bool,
    },
    MaxCombatId,
    SaveCombat {
        record: Box<Record>,
    },
    SaveRun {
        run: Box<Run>,
    },
    LoadRun {
        run_id: String,
    },
    Select {
        profile: i32,
        seed: String,
        started_at: i64,
    },
    ImportStatus,
    Import {
        max_run_id: u32,
        max_combat_id: u32,
        runs: Vec<ImportRun>,
        ambiguous: Vec<Identity>,
        complete: bool,
        #[serde(default)]
        reasons: Vec<String>,
    },
}

#[derive(Deserialize)]
struct ImportRun {
    source_key: String,
    run: Run,
    combats: Vec<Record>,
    reasons: Vec<String>,
    finalized: bool,
}

#[derive(Deserialize)]
struct Identity {
    profile: i32,
    seed: String,
    started_at: i64,
}

impl Store {
    #[allow(clippy::too_many_lines)] // Version guards and one transactional bootstrap share a connection.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_millis(50))?;
        let version: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let application: i32 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let tables: u32 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        if !(version == 0 && application == 0 && tables == 0
            || version == DATABASE_VERSION && application == APPLICATION_ID)
        {
            return Err("unsupported statistics database version or application".into());
        }
        if version == DATABASE_VERSION {
            let check: String =
                connection.pragma_query_value(None, "quick_check", |row| row.get(0))?;
            if check != "ok" || connection.prepare("PRAGMA foreign_key_check")?.exists([])? {
                return Err("statistics database failed integrity checks".into());
            }
            let _: (u32, u32, i64) = connection.query_row(
                "SELECT last_run_id,last_combat_id,imported FROM metadata WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        }
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        if version == 0 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "CREATE TABLE metadata (
                    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                    last_run_id INTEGER NOT NULL CHECK(last_run_id BETWEEN 0 AND 4294967295),
                    last_combat_id INTEGER NOT NULL CHECK(last_combat_id BETWEEN 0 AND 4294967295),
                    imported INTEGER NOT NULL CHECK(imported IN (0, 1))
                );
                INSERT INTO metadata VALUES (1, 0, 0, 0);
                CREATE TABLE runs (
                    id INTEGER PRIMARY KEY CHECK(id BETWEEN 1 AND 4294967295),
                    profile INTEGER NOT NULL, seed TEXT NOT NULL, started_at INTEGER NOT NULL,
                    current_json TEXT NOT NULL, final_json TEXT, source_key TEXT UNIQUE
                );
                CREATE INDEX run_identity ON runs(profile, seed, started_at);
                CREATE TABLE combats (
                    sequence INTEGER PRIMARY KEY,
                    native_id INTEGER UNIQUE CHECK(native_id BETWEEN 1 AND 4294967295),
                    run_id INTEGER REFERENCES runs(id),
                    ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 1 AND 4294967295),
                    imported INTEGER NOT NULL CHECK(imported IN (0, 1)),
                    record_json TEXT
                );
                CREATE INDEX run_combats ON combats(run_id, sequence);
                CREATE TABLE gaps (
                    run_id INTEGER NOT NULL REFERENCES runs(id), reason TEXT NOT NULL,
                    PRIMARY KEY(run_id, reason)
                );
                CREATE TABLE ambiguous (
                    profile INTEGER NOT NULL, seed TEXT NOT NULL, started_at INTEGER NOT NULL,
                    PRIMARY KEY(profile, seed, started_at)
                );",
            )?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", DATABASE_VERSION)?;
            transaction.commit()?;
        }
        Ok(Self {
            connection,
            pending: BTreeMap::new(),
        })
    }

    pub fn execute(&mut self, request_json: &str) -> Result<Value> {
        if request_json.len() > MAX_DOCUMENT_BYTES {
            return Err("statistics request exceeds size limit".into());
        }
        match serde_json::from_str(request_json)? {
            Request::OpenRun { run, continued } => self.open_run(*run, continued),
            Request::MaxCombatId => {
                let persisted: u32 = self.connection.query_row(
                    "SELECT last_combat_id FROM metadata WHERE singleton = 1",
                    [],
                    |row| row.get(0),
                )?;
                Ok(json!(persisted.max(
                    self.pending.last_key_value().map_or(0, |(&id, _)| id)
                )))
            }
            Request::SaveCombat { record } => self.save_combat(*record),
            Request::SaveRun { run } => self.save_run(*run),
            Request::LoadRun { run_id } => {
                let id = Run::numeric_id(&run_id).ok_or("invalid run ID")?;
                self.load_run(id, false)
            }
            Request::Select {
                profile,
                seed,
                started_at,
            } => match self.match_identity(profile, &seed, started_at)? {
                IdentityMatch::Unique(id) => self.load_run(id, true),
                IdentityMatch::Missing | IdentityMatch::Ambiguous => Ok(Value::Null),
            },
            Request::ImportStatus => Ok(json!(self.imported()?)),
            Request::Import {
                max_run_id,
                max_combat_id,
                runs,
                ambiguous,
                complete,
                reasons,
            } => self.import(
                max_run_id,
                max_combat_id,
                runs,
                ambiguous,
                complete,
                &reasons,
            ),
        }
    }

    fn imported(&self) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT imported FROM metadata WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?)
    }

    fn match_identity(&self, profile: i32, seed: &str, started_at: i64) -> Result<IdentityMatch> {
        if profile < 0 || seed.is_empty() || started_at <= 0 {
            return Ok(IdentityMatch::Missing);
        }
        let blocked: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM ambiguous WHERE profile=?1 AND seed=?2 AND started_at=?3)",
            params![profile, seed, started_at],
            |row| row.get(0),
        )?;
        let mut query = self.connection.prepare(
            "SELECT id FROM runs WHERE profile=?1 AND seed=?2 AND started_at=?3
             AND (final_json IS NOT NULL OR EXISTS(SELECT 1 FROM combats WHERE run_id=runs.id))
             ORDER BY id LIMIT 2",
        )?;
        let mut ids = query
            .query_map(params![profile, seed, started_at], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<u32>>>()?;
        ids.extend(
            self.pending
                .values()
                .flatten()
                .filter(|run| {
                    run.profile == profile && run.seed == seed && run.started_at == started_at
                })
                .filter_map(|run| Run::numeric_id(&run.run_id)),
        );
        ids.sort_unstable();
        ids.dedup();
        if blocked || ids.len() > 1 {
            return Ok(IdentityMatch::Ambiguous);
        }
        Ok(ids
            .first()
            .copied()
            .map_or(IdentityMatch::Missing, IdentityMatch::Unique))
    }

    fn open_run(&mut self, mut run: Run, continued: bool) -> Result<Value> {
        run.validate(true, true)?;
        let prior = if continued {
            self.match_identity(run.profile, &run.seed, run.started_at)?
        } else {
            IdentityMatch::Missing
        };
        if matches!(prior, IdentityMatch::Ambiguous) {
            return Ok(Value::Null);
        }
        let transaction = self.connection.transaction()?;
        let id = match prior {
            IdentityMatch::Unique(id) => {
                let prior =
                    Self::read_run(&transaction, id, false)?.ok_or("run identity disappeared")?;
                run.preserved_run_ids = prior.preserved_run_ids;
                id
            }
            IdentityMatch::Missing => {
                run.preserved_run_ids.clear();
                Self::allocate_run(&transaction)?
            }
            IdentityMatch::Ambiguous => return Ok(Value::Null),
        };
        run.run_id = id.to_string();
        run.outcome = "active".into();
        run.ended_at = 0;
        run.validate(false, false)?;
        let json = serde_json::to_string(&run)?;
        if matches!(prior, IdentityMatch::Unique(_)) {
            transaction.execute(
                "UPDATE runs SET current_json=?1 WHERE id=?2",
                params![json, id],
            )?;
        } else {
            transaction.execute(
                "INSERT INTO runs(id,profile,seed,started_at,current_json) VALUES (?1,?2,?3,?4,?5)",
                params![id, run.profile, run.seed, run.started_at, json],
            )?;
        }
        transaction.commit()?;
        Ok(serde_json::to_value(run)?)
    }

    fn allocate_run(transaction: &Transaction<'_>) -> Result<u32> {
        let last: u32 = transaction.query_row(
            "SELECT last_run_id FROM metadata WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        let next = last.checked_add(1).ok_or("run IDs exhausted")?;
        transaction.execute(
            "UPDATE metadata SET last_run_id=?1 WHERE singleton=1",
            [next],
        )?;
        Ok(next)
    }

    fn read_run(connection: &Connection, id: u32, history: bool) -> Result<Option<Run>> {
        let row = connection.query_row(
            "SELECT profile,seed,started_at,current_json,final_json,source_key IS NOT NULL FROM runs WHERE id=?1",
            [id], |row| Ok((row.get::<_, i32>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, bool>(5)?)),
        ).optional()?;
        let Some((profile, seed, started_at, current, finalized, imported)) = row else {
            return Ok(None);
        };
        let selected = if history {
            finalized.as_ref().unwrap_or(&current)
        } else {
            &current
        };
        if selected.len() > MAX_DOCUMENT_BYTES {
            return Err("stored run exceeds size limit".into());
        }
        let run: Run = serde_json::from_str(selected)?;
        run.validate(false, imported)?;
        if run.run_id != id.to_string()
            || run.profile != profile
            || run.seed != seed
            || run.started_at != started_at
        {
            return Err("stored run identity contradicts its database owner".into());
        }
        Ok(Some(if history && finalized.is_none() {
            run.history_fallback()
        } else {
            run
        }))
    }

    fn save_combat(&mut self, record: Record) -> Result<Value> {
        record.validate(false)?;
        let run_id = record
            .run
            .as_ref()
            .map(|run| Run::numeric_id(&run.run_id).ok_or("invalid run ID"))
            .transpose()?;
        if let Some(run) = &record.run {
            let owner = Self::read_run(&self.connection, run_id.ok_or("invalid run ID")?, false)?
                .ok_or("combat run does not exist")?;
            if !owner.same_identity(run) {
                return Err("combat belongs to a different run".into());
            }
        }
        let id = record.combat.combat_id;
        let existing = self
            .connection
            .query_row(
                "SELECT run_id,record_json IS NOT NULL FROM combats WHERE native_id=?1",
                [id],
                |row| Ok((row.get::<_, Option<u32>>(0)?, row.get::<_, bool>(1)?)),
            )
            .optional()?;
        if existing.is_some_and(|(owner, written)| owner != run_id || written) {
            return Err("combat ID already has an intent or immutable record".into());
        }
        if let Some(prior) = self.pending.get(&id) {
            let same = match (prior, &record.run) {
                (Some(prior), Some(run)) => prior.run_id == run.run_id && prior.same_identity(run),
                (None, None) => true,
                _ => false,
            };
            if !same {
                return Err("combat ID already belongs to a different pending run".into());
            }
        }
        self.pending.insert(id, record.run.clone());
        self.retry_intents();
        if self.pending.contains_key(&id) {
            return Err("statistics intent write failed".into());
        }
        let encoded = serde_json::to_string(&record)?;
        let written = self.connection.execute(
            "UPDATE combats SET record_json=?1 WHERE native_id=?2 AND record_json IS NULL",
            params![encoded, id],
        )?;
        if written != 1 {
            return Err("combat record already exists".into());
        }
        Ok(json!(true))
    }

    fn retry_intents(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let result = (|| -> Result<()> {
            let transaction = self.connection.transaction()?;
            for (&id, run) in &self.pending {
                Self::persist_intent(&transaction, id, run.as_ref())?;
            }
            transaction.commit()?;
            Ok(())
        })();
        if result.is_ok() {
            self.pending.clear();
        }
    }

    fn persist_intent(
        transaction: &Transaction<'_>,
        combat_id: u32,
        run: Option<&Run>,
    ) -> Result<()> {
        let run_id = run
            .map(|run| Run::numeric_id(&run.run_id).ok_or("invalid run ID"))
            .transpose()?;
        let prior = transaction
            .query_row(
                "SELECT run_id FROM combats WHERE native_id=?1",
                [combat_id],
                |row| row.get::<_, Option<u32>>(0),
            )
            .optional()?;
        if let Some(prior) = prior {
            if prior != run_id {
                return Err("combat ID already belongs to a different run".into());
            }
        } else {
            transaction.execute(
                "INSERT INTO combats(native_id,run_id,ordinal,imported) VALUES (?1,?2,?1,0)",
                params![combat_id, run_id],
            )?;
        }
        if let Some(run) = run {
            let owner = Self::read_run(
                transaction,
                run_id.expect("a retained run has a parsed ID"),
                false,
            )?
            .ok_or("combat run does not exist")?;
            if !owner.same_identity(run) {
                return Err("combat belongs to a different run".into());
            }
            transaction.execute(
                "UPDATE runs SET current_json=?1 WHERE id=?2",
                params![serde_json::to_string(run)?, run_id],
            )?;
        }
        transaction.execute(
            "UPDATE metadata SET last_combat_id=MAX(last_combat_id,?1) WHERE singleton=1",
            [combat_id],
        )?;
        Ok(())
    }

    fn save_run(&mut self, run: Run) -> Result<Value> {
        run.validate(false, false)?;
        if !run.finalized() {
            return Err("run is not finalized".into());
        }
        let id = Run::numeric_id(&run.run_id).ok_or("invalid run ID")?;
        let owner = Self::read_run(&self.connection, id, false)?.ok_or("run does not exist")?;
        if !owner.same_identity(&run) {
            return Err("finalized run identity contradicts its owner".into());
        }
        self.retry_intents();
        if self
            .pending
            .values()
            .flatten()
            .any(|pending| pending.run_id == run.run_id)
        {
            return Err("run still has unrecorded combat intents".into());
        }
        let transaction = self.connection.transaction()?;
        let owner = Self::read_run(&transaction, id, false)?.ok_or("run does not exist")?;
        if !owner.same_identity(&run) {
            return Err("finalized run identity contradicts its owner".into());
        }
        let has_combats: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM combats WHERE run_id=?1)",
            [id],
            |row| row.get(0),
        )?;
        if !has_combats {
            return Ok(json!(false));
        }
        transaction.execute(
            "UPDATE runs SET final_json=COALESCE(final_json,?1) WHERE id=?2",
            params![serde_json::to_string(&run)?, id],
        )?;
        transaction.commit()?;
        Ok(json!(true))
    }

    #[allow(clippy::too_many_lines)] // Read budget and record-level corruption share one ordered pass.
    fn load_run(&self, id: u32, history: bool) -> Result<Value> {
        let Some(run) = Self::read_run(&self.connection, id, history)? else {
            return Ok(Value::Null);
        };
        let mut reasons = self
            .connection
            .prepare("SELECT reason FROM gaps WHERE run_id=?1 ORDER BY reason")?
            .query_map([id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        let mut response_bytes = serde_json::to_string(&run)?.len()
            + serde_json::to_string(&reasons)?.len()
            + RESPONSE_ENVELOPE_BYTES;
        if response_bytes > MAX_DOCUMENT_BYTES {
            return Err("statistics history exceeds response size limit".into());
        }
        let mut query = self.connection.prepare(
            "SELECT ordinal,imported,record_json FROM combats WHERE run_id=?1
             ORDER BY imported DESC, CASE WHEN imported=1 THEN sequence ELSE native_id END",
        )?;
        let mut rows = query.query([id])?;
        let mut combats = Vec::new();
        let mut last_ordinal = 0;
        while let Some(row) = rows.next()? {
            let ordinal: u32 = row.get(0)?;
            last_ordinal = last_ordinal.max(ordinal);
            let imported: bool = row.get(1)?;
            let encoded: Option<String> = row.get(2)?;
            let Some(encoded) = encoded else {
                reasons.push("statistics-record-missing".into());
                continue;
            };
            response_bytes += encoded.len() + 1;
            if response_bytes > MAX_DOCUMENT_BYTES {
                return Err("statistics history exceeds response size limit".into());
            }
            let parsed = (|| -> Result<Record> {
                if encoded.len() > MAX_DOCUMENT_BYTES {
                    return Err("stored combat exceeds size limit".into());
                }
                let record: Record = serde_json::from_str(&encoded)?;
                record.validate(imported)?;
                if record.ordinal != ordinal
                    || record.run_id != run.run_id
                    || record
                        .run
                        .as_ref()
                        .is_none_or(|owner| !owner.same_identity(&run))
                {
                    return Err("stored combat identity contradicts its database owner".into());
                }
                Ok(record)
            })();
            match parsed {
                Ok(record) => combats.push(record),
                Err(_) => reasons.push("statistics-read-failed".into()),
            }
        }
        for (&ordinal, pending) in &self.pending {
            if pending
                .as_ref()
                .is_some_and(|pending| pending.run_id == run.run_id)
            {
                last_ordinal = last_ordinal.max(ordinal);
                reasons.push("statistics-intent-write-failed".into());
            }
        }
        if history && run.finalized() && combats.is_empty() {
            reasons.push("statistics-record-missing".into());
        }
        reasons.sort();
        reasons.dedup();
        Ok(
            json!({"run": run, "combats": combats, "reasons": reasons, "last_ordinal": last_ordinal}),
        )
    }

    fn import(
        &mut self,
        max_run: u32,
        max_combat: u32,
        runs: Vec<ImportRun>,
        ambiguous: Vec<Identity>,
        complete: bool,
        reasons: &[String],
    ) -> Result<Value> {
        if !complete {
            return Err("legacy import is not complete".into());
        }
        if reasons.iter().any(String::is_empty)
            || ambiguous.iter().any(|identity| {
                identity.profile < 0 || identity.seed.is_empty() || identity.started_at <= 0
            })
        {
            return Err("invalid legacy import metadata".into());
        }
        let mut source_keys = HashSet::new();
        for entry in &runs {
            entry.run.validate(false, true)?;
            if entry.source_key.is_empty()
                || !source_keys.insert(&entry.source_key)
                || entry.finalized != entry.run.finalized()
                || entry.reasons.iter().any(String::is_empty)
                || Run::numeric_id(&entry.run.run_id).is_some_and(|id| id > max_run)
            {
                return Err("invalid legacy run metadata".into());
            }
            for record in &entry.combats {
                record.validate(true)?;
                let owner = record
                    .run
                    .as_ref()
                    .ok_or("imported combat has no run owner")?;
                if record.run_id != entry.run.run_id
                    || !owner.same_identity(&entry.run)
                    || record.combat.combat_id > max_combat
                {
                    return Err("imported combat contradicts its run or reserved IDs".into());
                }
            }
        }
        if self.imported()? {
            return Ok(json!(true));
        }
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "UPDATE metadata SET last_run_id=MAX(last_run_id,?1),last_combat_id=MAX(last_combat_id,?2) WHERE singleton=1",
            params![max_run, max_combat],
        )?;
        for entry in runs {
            Self::import_run(&transaction, entry, reasons)?;
        }
        for identity in ambiguous {
            transaction.execute(
                "INSERT OR IGNORE INTO ambiguous VALUES (?1,?2,?3)",
                params![identity.profile, identity.seed, identity.started_at],
            )?;
        }
        transaction.execute("UPDATE metadata SET imported=1 WHERE singleton=1", [])?;
        transaction.commit()?;
        Ok(json!(true))
    }

    fn import_run(
        transaction: &Transaction<'_>,
        mut entry: ImportRun,
        global_reasons: &[String],
    ) -> Result<()> {
        let numeric = Run::numeric_id(&entry.run.run_id);
        let available = if let Some(id) = numeric {
            !transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1)",
                [id],
                |row| row.get::<_, bool>(0),
            )?
        } else {
            false
        };
        let id = if available {
            numeric.expect("available IDs were parsed")
        } else {
            Self::allocate_run(transaction)?
        };
        transaction.execute(
            "UPDATE metadata SET last_run_id=MAX(last_run_id,?1) WHERE singleton=1",
            [id],
        )?;
        entry.run.reidentify(id);
        let encoded = serde_json::to_string(&entry.run)?;
        transaction.execute(
            "INSERT INTO runs(id,profile,seed,started_at,current_json,final_json,source_key) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![id, entry.run.profile, entry.run.seed, entry.run.started_at, encoded,
                entry.finalized.then_some(&encoded), entry.source_key],
        )?;
        for mut record in entry.combats {
            let owner = record
                .run
                .as_mut()
                .ok_or("imported combat has no run owner")?;
            owner.reidentify(id);
            record.run_id = id.to_string();
            transaction.execute(
                "INSERT INTO combats(run_id,ordinal,imported,record_json) VALUES (?1,?2,1,?3)",
                params![id, record.ordinal, serde_json::to_string(&record)?],
            )?;
        }
        for reason in entry.reasons.iter().chain(global_reasons) {
            transaction.execute(
                "INSERT OR IGNORE INTO gaps VALUES (?1,?2)",
                params![id, reason],
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
