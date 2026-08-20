//! SQLite 哈希缓存：按 (path, size, mtime) 缓存部分哈希与全文件哈希

use rusqlite::{params, Connection};
use std::path::Path;

pub struct HashCache {
    conn: Connection,
    pub hits: u64,
}

/// 缓存中的一条哈希记录
pub struct CachedHash {
    pub partial: Option<u128>,
    pub full: Option<[u8; 32]>,
}

impl HashCache {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let p = Path::new(path);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS scan_cache (
                path TEXT PRIMARY KEY,
                size INTEGER NOT NULL,
                mtime INTEGER NOT NULL,
                partial_hash TEXT,
                full_hash TEXT,
                scanned_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS image_cache (
                path TEXT PRIMARY KEY,
                size INTEGER NOT NULL,
                mtime INTEGER NOT NULL,
                phash TEXT,
                scanned_at INTEGER NOT NULL
            );",
        )?;
        Ok(Self { conn, hits: 0 })
    }

    /// 查询图片感知哈希缓存；未命中或元数据变化返回 None
    pub fn get_phash(&mut self, path: &str, size: u64, mtime: u64) -> rusqlite::Result<Option<u64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT phash FROM image_cache WHERE path=?1 AND size=?2 AND mtime=?3")?;
        let mut rows = stmt.query(params![path, size as i64, mtime as i64])?;
        if let Some(row) = rows.next()? {
            let hex: Option<String> = row.get(0)?;
            let v = hex.and_then(|h| u64::from_str_radix(&h, 16).ok());
            Ok(v)
        } else {
            Ok(None)
        }
    }

    pub fn put_phash(&mut self, path: &str, size: u64, mtime: u64, phash: u64) -> rusqlite::Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.conn.execute(
            "INSERT OR REPLACE INTO image_cache (path, size, mtime, phash, scanned_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![path, size as i64, mtime as i64, format!("{phash:016x}"), now],
        )?;
        Ok(())
    }

    /// 命中返回 Some；未命中或元数据变化返回 None
    /// （hits 计数由调用方在确认所需字段存在时累加）
    pub fn get(&mut self, path: &str, size: u64, mtime: u64) -> rusqlite::Result<Option<CachedHash>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let mut stmt = self
            .conn
            .prepare("SELECT partial_hash, full_hash FROM scan_cache WHERE path=?1 AND size=?2 AND mtime=?3")?;
        let mut rows = stmt.query(params![path, size as i64, mtime as i64])?;
        if let Some(row) = rows.next()? {
            let partial_hex: Option<String> = row.get(0)?;
            let full_hex: Option<String> = row.get(1)?;
            let partial = partial_hex.and_then(|h| u128::from_str_radix(&h, 16).ok());
            let full = full_hex.and_then(|h| hex_to_32(&h));
            // 刷新访问时间
            let _ = self.conn.execute(
                "UPDATE scan_cache SET scanned_at=?1 WHERE path=?2",
                params![now, path],
            );
            Ok(Some(CachedHash { partial, full }))
        } else {
            Ok(None)
        }
    }

    pub fn put(
        &mut self,
        path: &str,
        size: u64,
        mtime: u64,
        partial: Option<u128>,
        full: Option<[u8; 32]>,
    ) -> rusqlite::Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let partial_hex = partial.map(|v| format!("{v:032x}"));
        let full_hex = full.map(|v| hex_from_32(&v));
        // 合并更新：新值为 NULL 的字段保留旧值（部分哈希与全哈希分阶段写入）
        self.conn.execute(
            "INSERT INTO scan_cache (path, size, mtime, partial_hash, full_hash, scanned_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(path) DO UPDATE SET
                size = excluded.size,
                mtime = excluded.mtime,
                partial_hash = COALESCE(excluded.partial_hash, scan_cache.partial_hash),
                full_hash = COALESCE(excluded.full_hash, scan_cache.full_hash),
                scanned_at = excluded.scanned_at",
            params![path, size as i64, mtime as i64, partial_hex, full_hex, now],
        )?;
        Ok(())
    }

    /// 清空缓存，返回删除的行数
    pub fn clear(&mut self) -> rusqlite::Result<u64> {
        let n1 = self.conn.execute("DELETE FROM scan_cache", [])?;
        let n2 = self.conn.execute("DELETE FROM image_cache", [])?;
        Ok((n1 + n2) as u64)
    }

    /// 统计缓存条目数（含图片哈希缓存）
    pub fn count(&self) -> rusqlite::Result<u64> {
        let n1: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM scan_cache", [], |r| r.get(0))?;
        let n2: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM image_cache", [], |r| r.get(0))?;
        Ok((n1 + n2) as u64)
    }
}

fn hex_to_32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        out[i] = ((hi << 4) | lo) as u8;
    }
    Some(out)
}

fn hex_from_32(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
