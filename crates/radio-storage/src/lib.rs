use rusqlite::{params, Connection};
use thiserror::Error;
use std::path::PathBuf;
use radio_api::Station;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("db error: {0}")]
    Db(String),
}

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn new() -> Result<Self, StorageError> {
        let mut path = dirs::config_dir().unwrap_or(PathBuf::from("."));
        path.push("ham-radio-app");
        std::fs::create_dir_all(&path).map_err(|e| StorageError::Db(e.to_string()))?;
        path.push("favorites.db");

        let conn = Connection::open(&path).map_err(|e| StorageError::Db(e.to_string()))?;
        
        // Favorites table - full station data for ham rig memory
        conn.execute(
            "CREATE TABLE IF NOT EXISTS favorites (
                uuid TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                url TEXT NOT NULL,
                url_resolved TEXT,
                country TEXT,
                countrycode TEXT,
                tags TEXT,
                language TEXT,
                codec TEXT,
                bitrate INTEGER,
                favicon TEXT,
                homepage TEXT,
                votes INTEGER,
                added_at INTEGER
            )",
            [],
        ).map_err(|e| StorageError::Db(e.to_string()))?;

        // Migration: add missing columns if DB was created with old schema (Batch 1 had only uuid,name,url,added_at)
        // We try to add each column and ignore error if it already exists
        let migrations = vec![
            "ALTER TABLE favorites ADD COLUMN url_resolved TEXT",
            "ALTER TABLE favorites ADD COLUMN country TEXT",
            "ALTER TABLE favorites ADD COLUMN countrycode TEXT",
            "ALTER TABLE favorites ADD COLUMN tags TEXT",
            "ALTER TABLE favorites ADD COLUMN language TEXT",
            "ALTER TABLE favorites ADD COLUMN codec TEXT",
            "ALTER TABLE favorites ADD COLUMN bitrate INTEGER",
            "ALTER TABLE favorites ADD COLUMN favicon TEXT",
            "ALTER TABLE favorites ADD COLUMN homepage TEXT",
            "ALTER TABLE favorites ADD COLUMN votes INTEGER",
        ];
        for sql in migrations {
            let _ = conn.execute(sql, []);
        }

        // History table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                uuid TEXT NOT NULL,
                name TEXT NOT NULL,
                url TEXT NOT NULL,
                country TEXT,
                played_at INTEGER
            )",
            [],
        ).map_err(|e| StorageError::Db(e.to_string()))?;

        Ok(Self { conn })
    }

    // --- Favorites ---
    pub fn add_favorite(&self, station: &Station) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO favorites 
            (uuid, name, url, url_resolved, country, countrycode, tags, language, codec, bitrate, favicon, homepage, votes, added_at) 
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, strftime('%s','now'))",
            params![
                station.stationuuid,
                station.name,
                station.url,
                station.url_resolved,
                station.country,
                station.countrycode,
                station.tags,
                station.language,
                station.codec,
                station.bitrate as i64,
                station.favicon,
                station.homepage,
                station.votes as i64
            ],
        ).map_err(|e| StorageError::Db(e.to_string()))?;
        Ok(())
    }

    pub fn remove_favorite(&self, uuid: &str) -> Result<(), StorageError> {
        self.conn.execute(
            "DELETE FROM favorites WHERE uuid = ?1",
            params![uuid],
        ).map_err(|e| StorageError::Db(e.to_string()))?;
        Ok(())
    }

    pub fn is_favorite(&self, uuid: &str) -> Result<bool, StorageError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM favorites WHERE uuid = ?1",
            params![uuid],
            |row| row.get(0),
        ).map_err(|e| StorageError::Db(e.to_string()))?;
        Ok(count > 0)
    }

    pub fn list_favorites(&self) -> Result<Vec<Station>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT uuid, name, url, url_resolved, country, countrycode, tags, language, codec, bitrate, favicon, homepage, votes 
             FROM favorites ORDER BY added_at DESC"
        ).map_err(|e| StorageError::Db(e.to_string()))?;

        let stations = stmt.query_map([], |row| {
            Ok(Station {
                stationuuid: row.get(0)?,
                name: row.get(1)?,
                url: row.get(2)?,
                url_resolved: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                country: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                countrycode: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                tags: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                language: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
                codec: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
                bitrate: row.get::<_, Option<i64>>(9)?.unwrap_or(0) as u32,
                favicon: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
                homepage: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
                votes: row.get::<_, Option<i64>>(12)?.unwrap_or(0) as u32,
                ..Default::default()
            })
        }).map_err(|e| StorageError::Db(e.to_string()))?;

        let mut result = Vec::new();
        for s in stations {
            result.push(s.map_err(|e| StorageError::Db(e.to_string()))?);
        }
        Ok(result)
    }

    // --- History ---
    pub fn add_history(&self, station: &Station) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO history (uuid, name, url, country, played_at) VALUES (?1, ?2, ?3, ?4, strftime('%s','now'))",
            params![station.stationuuid, station.name, station.url, station.country],
        ).map_err(|e| StorageError::Db(e.to_string()))?;
        
        self.conn.execute(
            "DELETE FROM history WHERE id NOT IN (SELECT id FROM history ORDER BY played_at DESC LIMIT 100)",
            [],
        ).map_err(|e| StorageError::Db(e.to_string()))?;
        Ok(())
    }

    pub fn list_history(&self) -> Result<Vec<(Station, i64)>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT uuid, name, url, country, played_at FROM history ORDER BY played_at DESC LIMIT 50"
        ).map_err(|e| StorageError::Db(e.to_string()))?;

        let rows = stmt.query_map([], |row| {
            let station = Station {
                stationuuid: row.get(0)?,
                name: row.get(1)?,
                url: row.get(2)?,
                country: row.get(3)?,
                ..Default::default()
            };
            let played_at: i64 = row.get(4)?;
            Ok((station, played_at))
        }).map_err(|e| StorageError::Db(e.to_string()))?;

        let mut result = Vec::new();
        for r in rows {
            result.push(r.map_err(|e| StorageError::Db(e.to_string()))?);
        }
        Ok(result)
    }

    pub fn clear_history(&self) -> Result<(), StorageError> {
        self.conn.execute("DELETE FROM history", [])
            .map_err(|e| StorageError::Db(e.to_string()))?;
        Ok(())
    }
}
