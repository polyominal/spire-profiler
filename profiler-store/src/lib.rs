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
//! Connections use rollback journaling and FULL synchronization. No pool or
//! background queue reorders lifecycle operations. A 50 ms busy limit bounds
//! lock contention, not filesystem latency. Legacy import stages one bounded
//! command at a time in a separate transaction; the completed marker and ID
//! reservations commit with its records. Source files stay read-only.
//!
//! The managed importer caps each source document at 64 MiB, and each native
//! command and response has the same cap. A source document whose wrapped
//! import command exceeds that cap aborts the import for a clean retry. History
//! pages share a read transaction and stay below the response cap; an oversized
//! stored row marks coverage incomplete while later rows remain readable.

#![deny(unsafe_code)]

mod record;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use record::{Record, Run};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;
use serde_json::{Value, json};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const APPLICATION_ID: i32 = 0x5354_5052;
const DATABASE_VERSION: i32 = 1;
const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_TRANSPORT_BYTES: usize = MAX_DOCUMENT_BYTES;
const RESPONSE_ENVELOPE_BYTES: usize = 4096;
const PAGE_TARGET_BYTES: usize = 4 * 1024 * 1024;
const PAGE_ROWS: usize = 128;

pub struct Store {
    connection: Connection,
    path: PathBuf,
    pending: BTreeMap<u32, Option<Run>>,
    import: Option<ImportSession>,
    read: Option<ReadSession>,
}

struct ImportSession {
    connection: Connection,
    max_run: u32,
    max_combat: u32,
    sources: HashMap<String, (u32, Run)>,
}

struct ReadSession {
    connection: Connection,
    run: Run,
    id: u32,
    history: bool,
    cursor: Option<(bool, i64, i64)>,
    last_ordinal: u32,
    valid_count: usize,
    reasons: BTreeSet<String>,
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
    ReadBeginLoad {
        run_id: String,
    },
    ReadBeginSelect {
        profile: i32,
        seed: String,
        started_at: i64,
    },
    ReadPage,
    ReadEnd,
    ImportStatus,
    ImportBegin {
        max_run_id: u32,
        max_combat_id: u32,
    },
    ImportRun {
        entry: Box<ImportRun>,
    },
    ImportRecord {
        source_key: String,
        record: Box<Record>,
    },
    ImportAmbiguous {
        identity: Identity,
    },
    ImportEnd {
        complete: bool,
        #[serde(default)]
        reasons: Vec<String>,
    },
}

#[derive(Deserialize)]
struct ImportRun {
    source_key: String,
    run: Run,
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
    #[allow(
        clippy::too_many_lines,
        reason = "Version guards and one transactional bootstrap share a connection."
    )]
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
            path: path.to_path_buf(),
            pending: BTreeMap::new(),
            import: None,
            read: None,
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Session guards and command dispatch form one boundary."
    )]
    pub fn execute(&mut self, request_json: &str) -> Result<Value> {
        if request_json.len() > MAX_TRANSPORT_BYTES {
            self.import = None;
            self.read = None;
            return Err("statistics request exceeds size limit".into());
        }
        let request: Request = match serde_json::from_str(request_json) {
            Ok(request) => request,
            Err(error) => {
                self.import = None;
                self.read = None;
                return Err(error.into());
            }
        };
        let import_command = matches!(
            request,
            Request::ImportRun { .. }
                | Request::ImportRecord { .. }
                | Request::ImportAmbiguous { .. }
                | Request::ImportEnd { .. }
        );
        let read_command = matches!(request, Request::ReadPage | Request::ReadEnd);
        if self.import.is_some() && !import_command {
            self.import = None;
            return Err("statistics import interrupted by another operation".into());
        }
        if self.read.is_some() && !read_command {
            self.read = None;
            return Err("statistics read interrupted by another operation".into());
        }
        let result = match request {
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
            Request::ReadBeginLoad { run_id } => {
                let id = Run::numeric_id(&run_id).ok_or("invalid run ID")?;
                let connection = self.session_connection("BEGIN")?;
                self.read_begin_on(connection, id, false)
            }
            Request::ReadBeginSelect {
                profile,
                seed,
                started_at,
            } => self.read_begin_select(profile, &seed, started_at),
            Request::ReadPage => self.read_page(),
            Request::ReadEnd => {
                self.read = None;
                Ok(json!(true))
            }
            Request::ImportStatus => Ok(json!(self.imported()?)),
            Request::ImportBegin {
                max_run_id,
                max_combat_id,
            } => self.import_begin(max_run_id, max_combat_id),
            Request::ImportRun { entry } => self.import_run(*entry),
            Request::ImportRecord { source_key, record } => {
                self.import_record(&source_key, *record)
            }
            Request::ImportAmbiguous { identity } => self.import_ambiguous(identity),
            Request::ImportEnd { complete, reasons } => self.import_end(complete, &reasons),
        };
        if result.is_err() {
            self.import = None;
            self.read = None;
        }
        result
    }

    fn imported(&self) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT imported FROM metadata WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?)
    }

    fn match_identity(&self, profile: i32, seed: &str, started_at: i64) -> Result<IdentityMatch> {
        self.match_identity_on(&self.connection, profile, seed, started_at)
    }

    fn match_identity_on(
        &self,
        connection: &Connection,
        profile: i32,
        seed: &str,
        started_at: i64,
    ) -> Result<IdentityMatch> {
        if profile < 0 || seed.is_empty() || started_at <= 0 {
            return Ok(IdentityMatch::Missing);
        }
        let blocked: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM ambiguous WHERE profile=?1 AND seed=?2 AND started_at=?3)",
            params![profile, seed, started_at],
            |row| row.get(0),
        )?;
        let mut query = connection.prepare(
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

    fn allocate_run(connection: &Connection) -> Result<u32> {
        let last: u32 = connection.query_row(
            "SELECT last_run_id FROM metadata WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        let next = last.checked_add(1).ok_or("run IDs exhausted")?;
        connection.execute(
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

    fn session_connection(&self, begin: &str) -> Result<Connection> {
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(Duration::from_millis(50))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.execute_batch(begin)?;
        Ok(connection)
    }

    fn read_begin_select(&mut self, profile: i32, seed: &str, started_at: i64) -> Result<Value> {
        let connection = self.session_connection("BEGIN")?;
        let id = match self.match_identity_on(&connection, profile, seed, started_at)? {
            IdentityMatch::Unique(id) => id,
            IdentityMatch::Missing | IdentityMatch::Ambiguous => return Ok(Value::Null),
        };
        self.read_begin_on(connection, id, true)
    }

    fn read_begin_on(&mut self, connection: Connection, id: u32, history: bool) -> Result<Value> {
        let Some(run) = Self::read_run(&connection, id, history)? else {
            return Ok(Value::Null);
        };
        let mut reasons = connection
            .prepare("SELECT reason FROM gaps WHERE run_id=?1 ORDER BY reason")?
            .query_map([id], |row| row.get(0))?
            .collect::<rusqlite::Result<BTreeSet<String>>>()?;
        let mut last_ordinal: u32 = connection.query_row(
            "SELECT COALESCE(MAX(ordinal),0) FROM combats WHERE run_id=?1",
            [id],
            |row| row.get(0),
        )?;
        for (&ordinal, pending) in &self.pending {
            if pending
                .as_ref()
                .is_some_and(|owner| owner.run_id == run.run_id)
            {
                last_ordinal = last_ordinal.max(ordinal);
                reasons.insert("statistics-intent-write-failed".into());
            }
        }
        let value = serde_json::to_value(&run)?;
        if serde_json::to_vec(&value)?.len() + RESPONSE_ENVELOPE_BYTES >= MAX_TRANSPORT_BYTES {
            return Err("stored run exceeds response size limit".into());
        }
        self.read = Some(ReadSession {
            connection,
            run,
            id,
            history,
            cursor: None,
            last_ordinal,
            valid_count: 0,
            reasons,
        });
        Ok(value)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "One ordered pass advances the cursor across valid and damaged rows."
    )]
    fn read_page(&mut self) -> Result<Value> {
        let session = self.read.as_mut().ok_or("no statistics read is active")?;
        let mut query = session.connection.prepare(
            "SELECT sequence,ordinal,imported,COALESCE(CASE WHEN imported=1 THEN sequence ELSE native_id END,0),record_json
             FROM combats WHERE run_id=?1 AND (?2 IS NULL OR imported < ?3 OR
               (imported=?3 AND (COALESCE(CASE WHEN imported=1 THEN sequence ELSE native_id END,0)>?4 OR
                 (COALESCE(CASE WHEN imported=1 THEN sequence ELSE native_id END,0)=?4 AND sequence>?5))))
             ORDER BY imported DESC,COALESCE(CASE WHEN imported=1 THEN sequence ELSE native_id END,0),sequence
             LIMIT ?6",
        )?;
        let (prior_imported, prior_key, prior_sequence) = session.cursor.unwrap_or((false, 0, 0));
        let mut rows = query.query(params![
            session.id,
            session.cursor.map(|_| 1),
            prior_imported,
            prior_key,
            prior_sequence,
            PAGE_ROWS as i64 + 1
        ])?;
        let mut combats = Vec::<Value>::new();
        let mut payload_bytes = 0;
        let mut scanned = 0;
        let mut done = true;
        while let Some(row) = rows.next()? {
            if scanned == PAGE_ROWS {
                done = false;
                break;
            }
            let sequence: i64 = row.get(0)?;
            let ordinal: u32 = row.get(1)?;
            let imported: bool = row.get(2)?;
            let key: i64 = row.get(3)?;
            let encoded: Option<String> = row.get(4)?;
            let parsed = encoded.as_deref().map(|encoded| -> Result<Record> {
                if encoded.len() > MAX_DOCUMENT_BYTES {
                    return Err("stored combat exceeds size limit".into());
                }
                let record: Record = serde_json::from_str(encoded)?;
                record.validate(imported)?;
                if record.ordinal != ordinal
                    || record.run_id != session.run.run_id
                    || record
                        .run
                        .as_ref()
                        .is_none_or(|owner| !owner.same_identity(&session.run))
                {
                    return Err("stored combat identity contradicts its database owner".into());
                }
                Ok(record)
            });
            let record = match parsed {
                None => {
                    session.reasons.insert("statistics-record-missing".into());
                    None
                }
                Some(Err(_)) => {
                    session.reasons.insert("statistics-read-failed".into());
                    None
                }
                Some(Ok(record)) => Some(record),
            };
            if let Some(record) = record {
                let value = serde_json::to_value(record)?;
                let bytes = serde_json::to_vec(&value)?.len() + 1;
                if !combats.is_empty() && payload_bytes + bytes > PAGE_TARGET_BYTES {
                    done = false;
                    break;
                }
                if payload_bytes + bytes + RESPONSE_ENVELOPE_BYTES >= MAX_TRANSPORT_BYTES {
                    session.reasons.insert("statistics-read-failed".into());
                } else {
                    payload_bytes += bytes;
                    combats.push(value);
                    session.valid_count += 1;
                }
            }
            session.cursor = Some((imported, key, sequence));
            scanned += 1;
        }
        drop(rows);
        drop(query);
        if done && session.history && session.run.finalized() && session.valid_count == 0 {
            session.reasons.insert("statistics-record-missing".into());
        }
        let reasons = if done {
            session.reasons.iter().collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let response = json!({"combats": combats, "reasons": reasons,
            "last_ordinal": session.last_ordinal, "done": done});
        if serde_json::to_vec(&response)?.len() + RESPONSE_ENVELOPE_BYTES >= MAX_TRANSPORT_BYTES {
            return Err("statistics page exceeds response size limit".into());
        }
        if done {
            self.read = None;
        }
        Ok(response)
    }

    fn import_begin(&mut self, max_run: u32, max_combat: u32) -> Result<Value> {
        if self.imported()? {
            return Err("legacy archive already imported".into());
        }
        let connection = self.session_connection("BEGIN IMMEDIATE")?;
        let imported: bool = connection.query_row(
            "SELECT imported FROM metadata WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        if imported {
            return Err("legacy archive already imported".into());
        }
        connection.execute(
            "UPDATE metadata SET last_run_id=MAX(last_run_id,?1),last_combat_id=MAX(last_combat_id,?2) WHERE singleton=1",
            params![max_run, max_combat],
        )?;
        self.import = Some(ImportSession {
            connection,
            max_run,
            max_combat,
            sources: HashMap::new(),
        });
        Ok(json!(true))
    }

    fn import_run(&mut self, mut entry: ImportRun) -> Result<Value> {
        let session = self
            .import
            .as_mut()
            .ok_or("no statistics import is active")?;
        entry.run.validate(false, true)?;
        if entry.source_key.is_empty()
            || session.sources.contains_key(&entry.source_key)
            || entry.finalized != entry.run.finalized()
            || entry.reasons.iter().any(String::is_empty)
            || Run::numeric_id(&entry.run.run_id).is_some_and(|id| id > session.max_run)
        {
            return Err("invalid legacy run metadata".into());
        }
        let original = entry.run.clone();
        let numeric = Run::numeric_id(&entry.run.run_id);
        let available = if let Some(id) = numeric {
            !session.connection.query_row(
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
            Self::allocate_run(&session.connection)?
        };
        session.connection.execute(
            "UPDATE metadata SET last_run_id=MAX(last_run_id,?1) WHERE singleton=1",
            [id],
        )?;
        entry.run.reidentify(id);
        let encoded = serde_json::to_string(&entry.run)?;
        session.connection.execute(
            "INSERT INTO runs(id,profile,seed,started_at,current_json,final_json,source_key) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![id, entry.run.profile, entry.run.seed, entry.run.started_at, encoded,
                entry.finalized.then_some(&encoded), entry.source_key],
        )?;
        for reason in entry.reasons {
            session.connection.execute(
                "INSERT OR IGNORE INTO gaps VALUES (?1,?2)",
                params![id, reason],
            )?;
        }
        session.sources.insert(entry.source_key, (id, original));
        Ok(json!(true))
    }

    fn import_record(&mut self, source_key: &str, mut record: Record) -> Result<Value> {
        let session = self
            .import
            .as_mut()
            .ok_or("no statistics import is active")?;
        let (id, original) = session
            .sources
            .get(source_key)
            .ok_or("unknown legacy run source")?;
        record.validate(true)?;
        let owner = record
            .run
            .as_mut()
            .ok_or("imported combat has no run owner")?;
        if record.run_id != original.run_id
            || !owner.same_identity(original)
            || record.combat.combat_id > session.max_combat
        {
            return Err("imported combat contradicts its run or reserved IDs".into());
        }
        owner.reidentify(*id);
        record.run_id = id.to_string();
        session.connection.execute(
            "INSERT INTO combats(run_id,ordinal,imported,record_json) VALUES (?1,?2,1,?3)",
            params![id, record.ordinal, serde_json::to_string(&record)?],
        )?;
        Ok(json!(true))
    }

    fn import_ambiguous(&mut self, identity: Identity) -> Result<Value> {
        let session = self
            .import
            .as_mut()
            .ok_or("no statistics import is active")?;
        if identity.profile < 0 || identity.seed.is_empty() || identity.started_at <= 0 {
            return Err("invalid legacy import metadata".into());
        }
        session.connection.execute(
            "INSERT OR IGNORE INTO ambiguous VALUES (?1,?2,?3)",
            params![identity.profile, identity.seed, identity.started_at],
        )?;
        Ok(json!(true))
    }

    fn import_end(&mut self, complete: bool, reasons: &[String]) -> Result<Value> {
        let session = self
            .import
            .as_mut()
            .ok_or("no statistics import is active")?;
        if !complete || reasons.iter().any(String::is_empty) {
            return Err("legacy import is not complete".into());
        }
        for reason in reasons {
            session.connection.execute(
                "INSERT OR IGNORE INTO gaps SELECT id,?1 FROM runs WHERE source_key IS NOT NULL",
                [reason],
            )?;
        }
        session
            .connection
            .execute("UPDATE metadata SET imported=1 WHERE singleton=1", [])?;
        session.connection.execute_batch("COMMIT")?;
        self.import = None;
        Ok(json!(true))
    }
}

#[cfg(test)]
mod tests;
