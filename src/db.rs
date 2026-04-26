use anyhow::{Context, Result};
use crossbeam_channel::Receiver;
use rusqlite::Connection;
use tracing::{error, info};

use crate::models::DbEvent;

// ═══════════════════════════════════════════════════════════════════
//  Schema initialization
// ═══════════════════════════════════════════════════════════════════

/// Open SQLite in WAL mode and create tables.
pub fn init_database(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("Failed to open SQLite at {}", path))?;

    // WAL mode for concurrent reads + non-blocking writes.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "journal_size_limit", 67_108_864i64)?; // 64 MB
    conn.pragma_update(None, "cache_size", -16_000i64)?;            // 16 MB cache
    conn.pragma_update(None, "busy_timeout", 5000i64)?;

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS trades (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            symbol       TEXT    NOT NULL,
            side         TEXT    NOT NULL DEFAULT 'LONG',
            entry_price  REAL    NOT NULL,
            exit_price   REAL,
            quantity     REAL    NOT NULL,
            leverage     INTEGER NOT NULL,
            margin_usd   REAL    NOT NULL,
            pnl_usd      REAL,
            roe_pct      REAL,
            entry_time   TEXT    NOT NULL DEFAULT (datetime('now')),
            exit_time    TEXT,
            exit_reason  TEXT,
            order_id     INTEGER,
            status       TEXT    NOT NULL DEFAULT 'OPEN'
        );

        CREATE TABLE IF NOT EXISTS system_logs (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            level      TEXT NOT NULL,
            message    TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_trades_symbol ON trades(symbol);
        CREATE INDEX IF NOT EXISTS idx_trades_status ON trades(status);
        ",
    )?;

    info!("SQLite initialized at {} (WAL mode)", path);
    Ok(conn)
}

// ═══════════════════════════════════════════════════════════════════
//  Background writer — runs on a dedicated OS thread
// ═══════════════════════════════════════════════════════════════════

/// Blocking loop that drains the channel and writes to SQLite.
/// Runs on `std::thread::spawn` to never block the tokio runtime.
pub fn background_writer(conn: Connection, rx: Receiver<DbEvent>) {
    info!("DB background writer started");

    // Pre-prepare statements for speed.
    let insert_trade = "INSERT INTO trades (symbol, entry_price, quantity, leverage, margin_usd, order_id)
                        VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
    let close_trade  = "UPDATE trades SET exit_price = ?1, pnl_usd = ?2, roe_pct = ?3,
                        exit_reason = ?4, exit_time = datetime('now'), status = 'CLOSED'
                        WHERE symbol = ?5 AND status = 'OPEN'";
    let insert_log   = "INSERT INTO system_logs (level, message) VALUES (?1, ?2)";

    for event in rx.iter() {
        let result = match &event {
            DbEvent::TradeOpened {
                symbol,
                entry_price,
                quantity,
                leverage,
                margin_usd,
                order_id,
            } => conn.execute(
                insert_trade,
                rusqlite::params![symbol, entry_price, quantity, leverage, margin_usd, order_id],
            ),

            DbEvent::TradeClosed {
                symbol,
                exit_price,
                pnl_usd,
                roe_pct,
                exit_reason,
            } => conn.execute(
                close_trade,
                rusqlite::params![exit_price, pnl_usd, roe_pct, exit_reason, symbol],
            ),

            DbEvent::SystemLog { level, message } => {
                conn.execute(insert_log, rusqlite::params![level, message])
            }
        };

        if let Err(e) = result {
            error!("DB write failed for {:?}: {}", event, e);
        }
    }

    info!("DB background writer shutting down");
}
