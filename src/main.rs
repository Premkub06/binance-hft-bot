mod config;
mod db;
mod execution;
mod models;
mod risk;
mod state;
mod strategy;
mod websocket;

use anyhow::Result;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
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
    std::thread::Builder::new()
        .name("db-writer".into())
        .spawn(move || db::background_writer(conn, db_rx))?;

    // ── Initialize shared state ──
    let market_state = state::new_market_state();
    let positions = state::new_position_map();
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

    // ── Bootstrap historical data ──
    websocket::bootstrap_state(&client, &symbols, &market_state).await?;

    let _ = db_tx.send(models::DbEvent::SystemLog {
        level: "INFO".into(),
        message: format!("Bot started with {} symbols", symbols.len()),
    });

    // ── Spawn async tasks ──
    info!("Launching trading engine...");

    // Task 1: Kline WebSocket (strategy hot path)
    let kline_handle = tokio::spawn(websocket::run_kline_stream(
        config.clone(),
        symbols.clone(),
        market_state.clone(),
        positions.clone(),
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

    // Task 4: Status reporter
    let status_state = market_state.clone();
    let status_positions = positions.clone();
    let status_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            info!(
                "📊 STATUS: {} symbols tracked | {} open positions",
                status_state.len(),
                status_positions.len()
            );
        }
    });

    info!("✅ All systems online. Monitoring for breakout signals...");

    // ── Graceful shutdown ──
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

    info!("Shutdown complete.");
    Ok(())
}
