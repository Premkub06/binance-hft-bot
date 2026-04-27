use anyhow::Result;
use crossbeam_channel::Sender;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::execution::BinanceClient;
use crate::models::*;
use crate::state::{MarketState, PendingSet, PositionMap, SymbolMetaMap};
use crate::strategy;

// ═══════════════════════════════════════════════════════════════════
//  Bootstrap: load historical data into memory
// ═══════════════════════════════════════════════════════════════════

pub async fn bootstrap_state(
    client: &BinanceClient,
    symbols: &[String],
    market_state: &MarketState,
) -> Result<()> {
    info!("Bootstrapping historical data for {} symbols...", symbols.len());

    for symbol in symbols {
        let mut state = SymbolState::new();

        // Fetch last 2 daily candles for previous day high.
        if let Ok(klines) = client.fetch_klines(symbol, "1d", 2).await {
            if klines.len() >= 2 {
                // klines[0] = previous day, klines[1] = current day
                if let Some(high_str) = klines[0].get(2).and_then(|v| v.as_str()) {
                    state.previous_day_high = high_str.parse().unwrap_or(0.0);
                }
                if let Some(high_str) = klines[1].get(2).and_then(|v| v.as_str()) {
                    state.current_day_high = high_str.parse().unwrap_or(0.0);
                }
                if let Some(close_str) = klines[1].get(4).and_then(|v| v.as_str()) {
                    state.current_price = close_str.parse().unwrap_or(0.0);
                }
            }
        }

        // Fetch 7 days of 15m candles (max 1500 per request, need 672).
        if let Ok(klines) = client.fetch_klines(symbol, "15m", 672).await {
            let mut prev_close = 0.0_f64;
            for k in &klines {
                let high: f64 = k.get(2).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let low:  f64 = k.get(3).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let close: f64 = k.get(4).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let vol:  f64 = k.get(5).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);

                state.volume_history.push_back(vol);
                if state.volume_history.len() > CANDLES_15M_7_DAYS {
                    state.volume_history.pop_front();
                }

                // Seed ATR-14 from historical candles.
                if prev_close > 0.0 && high > 0.0 {
                    state.push_true_range(high, low, prev_close);
                }
                if close > 0.0 { prev_close = close; }
            }
            state.recalc_avg_volume();

            // Current 15m candle volume (last element is still open).
            if let Some(last) = klines.last() {
                if let Some(vol_str) = last.get(5).and_then(|v| v.as_str()) {
                    state.current_15m_volume = vol_str.parse().unwrap_or(0.0);
                }
            }
        }

        market_state.insert(symbol.clone(), state);

        // Rate-limit REST calls to avoid 429s.
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    info!("Bootstrap complete. {} symbols loaded.", market_state.len());
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
//  WebSocket connection 1: Kline streams (1d + 15m)
// ═══════════════════════════════════════════════════════════════════

pub async fn run_kline_stream(
    config: Config,
    symbols: Vec<String>,
    market_state: MarketState,
    positions: PositionMap,
    pending: PendingSet,
    meta_map: SymbolMetaMap,
    db_tx: Sender<DbEvent>,
    client: BinanceClient,
) {
    loop {
        info!("Connecting to kline WebSocket stream...");
        match connect_kline_ws(&config, &symbols, &market_state, &positions, &pending, &meta_map, &db_tx, &client).await {
            Ok(_) => warn!("Kline WS stream ended normally, reconnecting..."),
            Err(e) => error!("Kline WS stream error: {}, reconnecting in 5s...", e),
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}

async fn connect_kline_ws(
    config: &Config,
    symbols: &[String],
    market_state: &MarketState,
    positions: &PositionMap,
    pending: &PendingSet,
    meta_map: &SymbolMetaMap,
    db_tx: &Sender<DbEvent>,
    client: &BinanceClient,
) -> Result<()> {
    // Build stream names: symbol@kline_15m and symbol@kline_1d
    let streams: Vec<String> = symbols
        .iter()
        .flat_map(|s| {
            let lower = s.to_lowercase();
            vec![
                format!("{}@kline_15m", lower),
                format!("{}@kline_1d", lower),
            ]
        })
        .collect();

    // Connect with combined stream URL.
    let url = format!("{}/stream", config.ws_url);
    let (ws_stream, _) = connect_async(&url).await?;
    let (mut write, mut read) = ws_stream.split();
    info!("Kline WS connected, subscribing to {} streams...", streams.len());

    // Subscribe via JSON message (avoids URL length limits).
    // Split into batches of 200 (Binance limit per subscribe message).
    for chunk in streams.chunks(200) {
        let sub_msg = serde_json::json!({
            "method": "SUBSCRIBE",
            "params": chunk,
            "id": 1
        });
        write.send(Message::Text(sub_msg.to_string())).await?;
    }

    // ── Message processing loop ──
    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Ping(d)) => {
                let _ = write.send(Message::Pong(d)).await;
                continue;
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => continue,
            Err(e) => {
                error!("WS read error: {}", e);
                break;
            }
        };

        // Parse combined stream wrapper.
        let combined: WsCombinedStream = match serde_json::from_str(&msg) {
            Ok(c) => c,
            Err(_) => continue, // subscription confirmations, etc.
        };

        // Parse kline event.
        let kline_event: WsKline = match serde_json::from_value(combined.data) {
            Ok(k) => k,
            Err(_) => continue,
        };

        let symbol = &kline_event.symbol;
        let kd = &kline_event.kline;
        let high: f64 = kd.high.parse().unwrap_or(0.0);
        let close: f64 = kd.close.parse().unwrap_or(0.0);
        let volume: f64 = kd.volume.parse().unwrap_or(0.0);

        // ── Update state + inline strategy evaluation (HOT PATH) ──
        let mut signal: Option<TradeSignal> = None;

        if let Some(mut entry) = market_state.get_mut(symbol) {
            let state = entry.value_mut();

            if kd.interval == "1d" {
                // Daily kline: track high.
                if high > state.current_day_high {
                    state.current_day_high = high;
                }
                state.current_price = close;

                // Daily candle closed → rotate highs.
                if kd.is_closed {
                    state.previous_day_high = state.current_day_high;
                    state.current_day_high = 0.0;
                    info!("[{}] New daily high set: {:.6}", symbol, state.previous_day_high);
                }
            } else if kd.interval == "15m" {
                // 15-minute kline: track volume + price.
                state.current_15m_volume = volume;
                state.current_price = close;

                // 15m candle closed → update rolling average and ATR.
                if kd.is_closed {
                    state.volume_history.push_back(volume);
                    if state.volume_history.len() > CANDLES_15M_7_DAYS {
                        state.volume_history.pop_front();
                    }
                    state.recalc_avg_volume();

                    // Update ATR-14 with the closed candle's true range.
                    let high: f64 = kd.high.parse().unwrap_or(0.0);
                    let low:  f64 = kd.low.parse().unwrap_or(0.0);
                    // prev_close ≈ current price before this candle closed.
                    if high > 0.0 && state.current_price > 0.0 {
                        state.push_true_range(high, low, state.current_price);
                    }
                }

                // ── STRATEGY EVALUATION (inline, zero extra lookups) ──
                let has_position = positions.contains_key(symbol)
                    || pending.contains(symbol.as_str());
                signal = strategy::evaluate_breakout(
                    symbol,
                    state,
                    config.volume_multiplier,
                    has_position,
                );
            }
        }

        // ── Execute trade signal (outside any lock) ──
        if let Some(sig) = signal {
            // ── ATOMIC CLAIM: mark symbol as in-flight BEFORE spawning ──
            // DashSet::insert returns false if the key already exists,
            // making this a single atomic operation with no TOCTOU window.
            if !pending.insert(sig.symbol.clone()) {
                // Another task already claimed this symbol.
                continue;
            }

            info!(
                "🚀 BREAKOUT SIGNAL: {} | price={:.6} > prev_high={:.6} | vol={:.2} > 3×avg={:.2} | ATR-14={:.6}",
                sig.symbol, sig.price, sig.previous_day_high, sig.volume_15m, sig.avg_volume_7d, sig.atr_14
            );
            let client = client.clone();
            let config = config.clone();
            let positions = positions.clone();
            let pending = pending.clone();
            let meta_map = meta_map.clone();
            let db_tx = db_tx.clone();

            tokio::spawn(async move {
                if let Err(e) = execute_signal(&client, &config, &positions, &pending, &meta_map, &db_tx, &sig).await {
                    error!("Trade execution failed for {}: {}", sig.symbol, e);
                    // ── RELEASE CLAIM on failure so future signals can retry ──
                    pending.remove(&sig.symbol);
                }
            });
        }
    }

    Ok(())
}

/// Execute a breakout signal: place market long, record position.
async fn execute_signal(
    client: &BinanceClient,
    config: &Config,
    positions: &PositionMap,
    pending: &PendingSet,
    meta_map: &SymbolMetaMap,
    db_tx: &Sender<DbEvent>,
    signal: &TradeSignal,
) -> Result<()> {
    // Secondary guard: if a position was inserted by another path
    // (e.g. state recovery) between the claim and here, abort cleanly.
    if positions.contains_key(&signal.symbol) {
        pending.remove(&signal.symbol);
        return Ok(());
    }

    // Calculate quantity: notional = margin × leverage.
    let notional = config.margin_usd * config.leverage as f64;
    let raw_qty = notional / signal.price;

    // Round to step size.
    let step = meta_map
        .get(&signal.symbol)
        .map(|m| m.step_size)
        .unwrap_or(0.001);
    let quantity = (raw_qty / step).floor() * step;

    if quantity <= 0.0 {
        warn!("Quantity for {} rounds to zero, skipping", signal.symbol);
        return Ok(());
    }

    // Place market BUY.
    let order = client
        .market_order(&signal.symbol, "BUY", quantity, meta_map)
        .await?;

    let parsed_price: f64 = order.avg_price.parse().unwrap_or(0.0);
    let entry_price = if parsed_price > 0.0 { parsed_price } else { signal.price };

    let parsed_qty: f64 = order.executed_qty.parse().unwrap_or(0.0);
    let exec_qty = if parsed_qty > 0.0 { parsed_qty } else { quantity };

    // Record position in memory.
    let position = Position {
        symbol: signal.symbol.clone(),
        entry_price,
        quantity: exec_qty,
        leverage: config.leverage,
        margin_usd: config.margin_usd,
        entry_time: chrono::Utc::now(),
        max_roe: 0.0,
        trailing_active: false,
        order_id: order.order_id,
        atr_at_entry: signal.atr_14,
        break_even_active: false,
    };

    info!(
        "✅ POSITION OPENED: {} | entry={:.6} | qty={:.4} | ATR={:.6} | hard_stop={:.6} | trail_stop_dist={:.6}",
        signal.symbol, entry_price, exec_qty, signal.atr_14,
        entry_price - config.atr_hard_stop_mult * signal.atr_14,
        config.atr_trail_mult * signal.atr_14
    );
    positions.insert(signal.symbol.clone(), position);

    // Release the pending claim now that position is in the map.
    // From this point on, positions.contains_key() will guard correctly.
    pending.remove(&signal.symbol);

    // Fire-and-forget DB write.
    let _ = db_tx.send(DbEvent::TradeOpened {
        symbol: signal.symbol.clone(),
        entry_price,
        quantity: exec_qty,
        leverage: config.leverage,
        margin_usd: config.margin_usd,
        order_id: order.order_id,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
//  WebSocket connection 2: Mark price stream (for risk monitoring)
// ═══════════════════════════════════════════════════════════════════

pub async fn run_mark_price_stream(
    config: Config,
    market_state: MarketState,
    positions: PositionMap,
    meta_map: SymbolMetaMap,
    db_tx: Sender<DbEvent>,
    client: BinanceClient,
) {
    loop {
        info!("Connecting to mark price WebSocket stream...");
        match connect_mark_price_ws(&config, &market_state, &positions, &meta_map, &db_tx, &client).await {
            Ok(_) => warn!("Mark price WS stream ended, reconnecting..."),
            Err(e) => error!("Mark price WS error: {}, reconnecting in 5s...", e),
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}

async fn connect_mark_price_ws(
    config: &Config,
    market_state: &MarketState,
    positions: &PositionMap,
    meta_map: &SymbolMetaMap,
    db_tx: &Sender<DbEvent>,
    client: &BinanceClient,
) -> Result<()> {
    let url = format!("{}/ws/!markPrice@arr@1s", config.ws_url);
    let (ws_stream, _) = connect_async(&url).await?;
    let (mut write, mut read) = ws_stream.split();
    info!("Mark price WS connected");

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Ping(d)) => {
                let _ = write.send(Message::Pong(d)).await;
                continue;
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => continue,
            Err(e) => {
                error!("Mark price WS read error: {}", e);
                break;
            }
        };

        // Parse mark price array.
        let prices: Vec<WsMarkPrice> = match serde_json::from_str(&msg) {
            Ok(p) => p,
            Err(_) => continue,
        };

        for mp in &prices {
            let price: f64 = match mp.mark_price.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            // Update current price in market state.
            if let Some(mut entry) = market_state.get_mut(&mp.symbol) {
                entry.value_mut().current_price = price;
            }

            // ── Risk evaluation for open positions ──
            if let Some(mut pos_entry) = positions.get_mut(&mp.symbol) {
                let pos = pos_entry.value_mut();
                let roe = strategy::calculate_roe(pos, price);

                // Update max ROE for trailing stop.
                if roe > pos.max_roe {
                    pos.max_roe = roe;
                }

                // Activate break-even stop at threshold.
                if !pos.break_even_active && roe >= config.break_even_trigger_roe {
                    pos.break_even_active = true;
                    info!(
                        "🛡️ Break-even stop ACTIVATED for {} | ROE: {:.2}%",
                        mp.symbol, roe
                    );
                }

                // Activate trailing stop at threshold.
                if !pos.trailing_active && roe >= config.trailing_activation_roe {
                    pos.trailing_active = true;
                    info!(
                        "📈 Trailing stop ACTIVATED for {} | ROE: {:.2}%",
                        mp.symbol, roe
                    );
                }

                // Check if position should be closed.
                let close_reason = strategy::evaluate_risk(
                    pos,
                    price,
                    config.hard_stop_roe,
                    config.trailing_activation_roe,
                    config.trailing_stop_pct,
                    config.atr_hard_stop_mult,
                    config.atr_trail_mult,
                    config.break_even_target_roe,
                );

                if let Some(reason) = close_reason {
                    let symbol = pos.symbol.clone();
                    let quantity = pos.quantity;
                    let entry_price = pos.entry_price;
                    let leverage = pos.leverage;
                    drop(pos_entry); // Release DashMap lock before async work.

                    info!("🛑 CLOSING {}: {}", symbol, reason);

                    // Close position via market SELL.
                    match client
                        .market_order(&symbol, "SELL", quantity, meta_map)
                        .await
                    {
                        Ok(order) => {
                            let exit_price: f64 =
                                order.avg_price.parse().unwrap_or(price);
                            let pnl = (exit_price - entry_price) * quantity;
                            let roe_final =
                                ((exit_price - entry_price) / entry_price)
                                    * leverage as f64
                                    * 100.0;

                            positions.remove(&symbol);

                            let _ = db_tx.send(DbEvent::TradeClosed {
                                symbol: symbol.clone(),
                                exit_price,
                                pnl_usd: pnl,
                                roe_pct: roe_final,
                                exit_reason: reason,
                            });

                            info!(
                                "💰 CLOSED {} | PnL: ${:.4} | ROE: {:.2}%",
                                symbol, pnl, roe_final
                            );
                        }
                        Err(e) => {
                            error!("Failed to close position {}: {}", symbol, e);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
