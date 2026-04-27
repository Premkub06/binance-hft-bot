mod backtest;
mod config;
mod db;
mod execution;
mod models;
mod risk;
mod state;
mod strategy;
mod websocket;

use anyhow::Result;
use clap::Parser;
use crossbeam_channel::Sender;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use models::{DbEvent, Position};
use state::PositionMap;

// ═══════════════════════════════════════════════════════════════════
//  CLI arguments
// ═══════════════════════════════════════════════════════════════════

/// Binance HFT Bot — Altcoin Breakout Strategy
#[derive(Parser, Debug)]
#[command(name = "binance-hft-bot", version, about)]
struct Cli {
    /// Run in backtest mode with a CSV file (OHLCV: timestamp,open,high,low,close,volume)
    #[arg(long, value_name = "FILE")]
    backtest: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════
//  Entry point
// ═══════════════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // ── Backtest mode: no network, no API keys needed ──
    if let Some(csv_path) = cli.backtest {
        return backtest::run_backtest(&csv_path);
    }

    // ── Live mode: full trading engine ──
    run_live().await
}

async fn run_live() -> Result<()> {
    // ── Initialize logging ──
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .with_thread_ids(true)
        .compact()
        .init();

    info!("═══════════════════════════════════════════════════════");
    info!("  Binance HFT Bot — Altcoin Breakout Strategy v0.1.0  ");
    info!("═══════════════════════════════════════════════════════");

    // ── Load configuration ──
    dotenvy::dotenv().ok();
    let config = config::Config::from_env();
    info!("Config loaded: {}x leverage, ${} margin, top {} symbols",
        config.leverage, config.margin_usd, config.top_n_symbols);

    // ── Initialize SQLite ──
    let conn = db::init_database(&config.db_path)?;
    let (db_tx, db_rx) = crossbeam_channel::unbounded::<models::DbEvent>();

    // Spawn DB writer on a dedicated OS thread (never blocks tokio).
    let db_thread = std::thread::Builder::new()
        .name("db-writer".into())
        .spawn(move || db::background_writer(conn, db_rx))?;

    // ── Initialize shared state ──
    let market_state = state::new_market_state();
    let positions = state::new_position_map();
    let pending = state::new_pending_set();
    let meta_map = state::new_symbol_meta_map();

    // ── Initialize Binance client ──
    let client = execution::BinanceClient::new(config.clone());

    // ── Fetch exchange info & symbol metadata ──
    client.fetch_symbol_meta(&meta_map).await?;

    // ── Get top N symbols ──
    let symbols = client.fetch_top_symbols(config.top_n_symbols).await?;
    if symbols.is_empty() {
        error!("No tradable symbols found, exiting.");
        return Ok(());
    }

    // ── Set leverage for all symbols ──
    info!("Setting {}x leverage for {} symbols...", config.leverage, symbols.len());
    for sym in &symbols {
        if let Err(e) = client.set_leverage(sym, config.leverage).await {
            error!("Failed to set leverage for {}: {}", sym, e);
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    // ══════════════════════════════════════════════════════════════
    //  STATE RECOVERY: Sync positions from Binance API on startup
    // ══════════════════════════════════════════════════════════════
    recover_positions(&client, &config, &positions, &db_tx).await;

    // ── Bootstrap historical data ──
    websocket::bootstrap_state(&client, &symbols, &market_state).await?;

    let _ = db_tx.send(models::DbEvent::SystemLog {
        level: "INFO".into(),
        message: format!("Bot started with {} symbols, {} recovered positions",
            symbols.len(), positions.len()),
    });

    // ── Spawn async tasks ──
    info!("Launching trading engine...");

    // Task 1: Kline WebSocket (strategy hot path)
    let kline_handle = tokio::spawn(websocket::run_kline_stream(
        config.clone(),
        symbols.clone(),
        market_state.clone(),
        positions.clone(),
        pending.clone(),
        meta_map.clone(),
        db_tx.clone(),
        client.clone(),
    ));

    // Task 2: Mark price WebSocket (real-time risk monitoring)
    let mark_handle = tokio::spawn(websocket::run_mark_price_stream(
        config.clone(),
        market_state.clone(),
        positions.clone(),
        meta_map.clone(),
        db_tx.clone(),
        client.clone(),
    ));

    // Task 3: Periodic risk sweep (safety net)
    let risk_handle = tokio::spawn(risk::run_risk_monitor(
        config.clone(),
        positions.clone(),
        meta_map.clone(),
        db_tx.clone(),
        client.clone(),
        market_state.clone(),
    ));

    // Task 4: Status reporter + live ROE flusher
    let status_state = market_state.clone();
    let status_positions = positions.clone();
    let status_db_tx = db_tx.clone();
    let status_config = config.clone();
    let status_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        let mut log_counter = 0u64;
        loop {
            interval.tick().await;
            log_counter += 1;

            // Flush live ROE/PnL to DB every 5 seconds for dashboard visibility.
            flush_live_roe(&status_positions, &status_state, &status_config, &status_db_tx);

            // Log status every 60 seconds (12 ticks × 5s).
            if log_counter % 12 == 0 {
                info!(
                    "📊 STATUS: {} symbols tracked | {} open positions",
                    status_state.len(),
                    status_positions.len()
                );
            }
        }
    });

    info!("✅ All systems online. Monitoring for breakout signals...");

    // ══════════════════════════════════════════════════════════════
    //  GRACEFUL SHUTDOWN
    // ══════════════════════════════════════════════════════════════
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Shutdown signal received, stopping...");
            let _ = db_tx.send(models::DbEvent::SystemLog {
                level: "INFO".into(),
                message: "Bot shutdown initiated".into(),
            });
        }
        _ = kline_handle => error!("Kline task exited unexpectedly"),
        _ = mark_handle => error!("Mark price task exited unexpectedly"),
        _ = risk_handle => error!("Risk monitor task exited unexpectedly"),
        _ = status_handle => error!("Status reporter exited unexpectedly"),
    }

    // Drop the sender to signal the background writer to drain and exit.
    info!("Flushing pending DB writes...");
    drop(db_tx);

    // Wait for the DB writer thread to finish (drains all pending events).
    if let Err(e) = db_thread.join() {
        error!("DB writer thread panicked: {:?}", e);
    }

    info!("Shutdown complete. All data flushed.");
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
//  State recovery: populate PositionMap from Binance API + sync DB
// ═══════════════════════════════════════════════════════════════════

async fn recover_positions(
    client: &execution::BinanceClient,
    config: &config::Config,
    positions: &PositionMap,
    db_tx: &Sender<DbEvent>,
) {
    info!("Recovering position state from Binance API...");

    match client.fetch_open_positions().await {
        Ok(binance_positions) => {
            // Collect symbols that are actually open on Binance.
            let mut live_symbols = std::collections::HashSet::new();

            for bp in &binance_positions {
                let qty: f64 = bp.position_amt.parse().unwrap_or(0.0);
                let entry: f64 = bp.entry_price.parse().unwrap_or(0.0);
                let lev: u32 = bp.leverage.parse().unwrap_or(config.leverage);

                if qty.abs() <= 0.0 || entry <= 0.0 {
                    continue;
                }

                live_symbols.insert(bp.symbol.clone());

                // Populate in-memory position map.
                let position = Position {
                    symbol: bp.symbol.clone(),
                    entry_price: entry,
                    quantity: qty.abs(),
                    leverage: lev,
                    margin_usd: (entry * qty.abs()) / lev as f64,
                    entry_time: chrono::Utc::now(),
                    max_roe: 0.0,
                    trailing_active: false,
                    order_id: 0, // Unknown from recovery.
                    atr_at_entry: 0.0, // ATR unknown at recovery; falls back to ROE hard stop.
                };

                info!(
                    "  ♻️  Recovered position: {} | qty={:.4} | entry={:.6}",
                    bp.symbol, qty.abs(), entry
                );
                positions.insert(bp.symbol.clone(), position);
            }

            // Mark DB trades as CLOSED if they're no longer open on Binance.
            // This handles positions that were manually closed or liquidated.
            let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "hft_bot.db".into());
            if let Ok(conn) = rusqlite::Connection::open(&db_path) {
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT DISTINCT symbol FROM trades WHERE status = 'OPEN'"
                ) {
                    if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
                        for row in rows.flatten() {
                            if !live_symbols.contains(&row) {
                                warn!("  🧹 Stale OPEN trade '{}' not on Binance, marking CLOSED", row);
                                let _ = db_tx.send(DbEvent::ForceClose {
                                    symbol: row,
                                    exit_reason: "RECOVERY_SYNC: not open on Binance".into(),
                                });
                            }
                        }
                    }
                }
            }

            info!("State recovery complete: {} live positions", positions.len());
        }
        Err(e) => {
            warn!("State recovery failed (continuing without): {}", e);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Live ROE flusher: writes current PnL/ROE to DB for dashboard
// ═══════════════════════════════════════════════════════════════════

fn flush_live_roe(
    positions: &PositionMap,
    market_state: &state::MarketState,
    config: &config::Config,
    db_tx: &Sender<DbEvent>,
) {
    for entry in positions.iter() {
        let pos = entry.value();
        let current_price = market_state
            .get(&pos.symbol)
            .map(|s| s.current_price)
            .unwrap_or(0.0);

        if current_price <= 0.0 || pos.entry_price <= 0.0 {
            continue;
        }

        let roe = ((current_price - pos.entry_price) / pos.entry_price)
            * config.leverage as f64
            * 100.0;
        let pnl = (current_price - pos.entry_price) * pos.quantity;

        let _ = db_tx.send(DbEvent::UpdateLiveRoe {
            symbol: pos.symbol.clone(),
            pnl_usd: pnl,
            roe_pct: roe,
        });
    }
}
