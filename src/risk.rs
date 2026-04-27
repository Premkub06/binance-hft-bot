use crossbeam_channel::Sender;
use tracing::{error, info};

use crate::config::Config;
use crate::execution::BinanceClient;
use crate::models::DbEvent;
use crate::state::{PositionMap, SymbolMetaMap};
use crate::strategy;

// ═══════════════════════════════════════════════════════════════════
//  Periodic risk sweep (fallback safety net)
// ═══════════════════════════════════════════════════════════════════
//
//  Primary risk monitoring happens inline in the mark price WS stream.
//  This task is a safety net that runs every 5 seconds to catch any
//  positions that might have been missed (e.g., during WS reconnection).

pub async fn run_risk_monitor(
    config: Config,
    positions: PositionMap,
    meta_map: SymbolMetaMap,
    db_tx: Sender<DbEvent>,
    client: BinanceClient,
    market_state: crate::state::MarketState,
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

    loop {
        interval.tick().await;

        if positions.is_empty() {
            continue;
        }

        // Collect symbols that need closing (avoid holding DashMap ref across await).
        let mut to_close: Vec<(String, f64, f64, String)> = Vec::new();

        for entry in positions.iter() {
            let symbol = entry.key();
            let pos = entry.value();

            let current_price = market_state
                .get(symbol)
                .map(|s| s.current_price)
                .unwrap_or(0.0);

            if current_price <= 0.0 {
                continue;
            }

            if let Some(reason) = strategy::evaluate_risk(
                pos,
                current_price,
                config.hard_stop_roe,
                config.trailing_activation_roe,
                config.trailing_stop_pct,
                config.atr_hard_stop_mult,
                config.atr_trail_mult,
            ) {
                to_close.push((
                    symbol.clone(),
                    pos.quantity,
                    pos.entry_price,
                    reason,
                ));
            }
        }

        // Execute closures outside of iteration.
        for (symbol, quantity, entry_price, reason) in to_close {
            info!("🛡️ RISK SWEEP closing {}: {}", symbol, reason);

            match client
                .market_order(&symbol, "SELL", quantity, &meta_map)
                .await
            {
                Ok(order) => {
                    let exit_price: f64 = order.avg_price.parse().unwrap_or(0.0);
                    let pnl = (exit_price - entry_price) * quantity;
                    let roe = ((exit_price - entry_price) / entry_price)
                        * config.leverage as f64
                        * 100.0;

                    positions.remove(&symbol);

                    let _ = db_tx.send(DbEvent::TradeClosed {
                        symbol: symbol.clone(),
                        exit_price,
                        pnl_usd: pnl,
                        roe_pct: roe,
                        exit_reason: reason,
                    });

                    info!(
                        "🛡️ RISK CLOSED {} | PnL: ${:.4} | ROE: {:.2}%",
                        symbol, pnl, roe
                    );
                }
                Err(e) => {
                    error!("Risk sweep: failed to close {}: {}", symbol, e);
                }
            }
        }
    }
}
