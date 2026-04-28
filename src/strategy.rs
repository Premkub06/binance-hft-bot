use crate::models::{Position, SymbolState, TradeSignal};

// ═══════════════════════════════════════════════════════════════════
//  Bi-directional Trend-Following Mean Reversion Strategy
// ═══════════════════════════════════════════════════════════════════
//
//  LONG entry:
//    price > EMA (uptrend) AND RSI < oversold threshold (mean reversion dip)
//    → The asset is trending up but temporarily oversold → buy the dip.
//
//  SHORT entry:
//    price < EMA (downtrend) AND RSI > overbought threshold (mean reversion pop)
//    → The asset is trending down but temporarily overbought → sell the pop.

/// Evaluate for a LONG or SHORT mean-reversion signal.
///
/// Returns `Some(TradeSignal)` with `side = "BUY"` or `"SELL"`,
/// or `None` if no conditions are met.
#[inline(always)]
pub fn evaluate_signal(
    symbol: &str,
    state: &SymbolState,
    rsi_oversold: f64,
    rsi_overbought: f64,
    has_open_position: bool,
) -> Option<TradeSignal> {
    if has_open_position {
        return None;
    }

    // Guard: need warmed-up indicators.
    if state.ema <= 0.0 || state.rsi_14 <= 0.0 {
        return None;
    }

    let price = state.current_price;
    if price <= 0.0 {
        return None;
    }

    // ── LONG: uptrend + oversold dip ──
    if price > state.ema && state.rsi_14 < rsi_oversold {
        return Some(TradeSignal {
            symbol: symbol.to_owned(),
            side: "BUY".to_owned(),
            price,
            volume_15m: state.current_15m_volume,
            avg_volume_7d: state.avg_volume_7d_15m,
            previous_day_high: state.previous_day_high,
            atr_14: state.atr_14,
            rsi_14: state.rsi_14,
        });
    }

    // ── SHORT: downtrend + overbought pop ──
    if price < state.ema && state.rsi_14 > rsi_overbought {
        return Some(TradeSignal {
            symbol: symbol.to_owned(),
            side: "SELL".to_owned(),
            price,
            volume_15m: state.current_15m_volume,
            avg_volume_7d: state.avg_volume_7d_15m,
            previous_day_high: state.previous_day_high,
            atr_14: state.atr_14,
            rsi_14: state.rsi_14,
        });
    }

    None
}

// ═══════════════════════════════════════════════════════════════════
//  Side-aware ROE calculation
// ═══════════════════════════════════════════════════════════════════

/// Calculate ROE (Return on Equity) percentage, accounting for position side.
///
/// LONG:  `((current - entry) / entry) × leverage × 100`
/// SHORT: `((entry - current) / entry) × leverage × 100`
#[inline(always)]
pub fn calculate_roe(position: &Position, current_price: f64) -> f64 {
    let price_delta = if position.side == "BUY" {
        current_price - position.entry_price
    } else {
        position.entry_price - current_price
    };
    (price_delta / position.entry_price) * position.leverage as f64 * 100.0
}

// ═══════════════════════════════════════════════════════════════════
//  ATR-based dynamic risk evaluator (side-aware)
// ═══════════════════════════════════════════════════════════════════

/// Determine if a position should be closed based on ATR-adjusted risk rules.
///
/// All stop logic is side-aware:
///   - LONG hard stop: price drops below entry - N×ATR
///   - SHORT hard stop: price rises above entry + N×ATR
///   - Trailing stop: price reverses N×ATR from peak ROE price
///
/// Returns `Some(reason)` if the position should be closed.
#[inline]
pub fn evaluate_risk(
    position: &Position,
    current_price: f64,
    hard_stop_roe: f64,
    trailing_activation_roe: f64,
    _trailing_stop_pct: f64,
    atr_hard_stop_mult: f64,
    atr_trail_mult: f64,
    break_even_target_roe: f64,
) -> Option<String> {
    let roe = calculate_roe(position, current_price);
    let is_long = position.side == "BUY";

    // ── Layer 0: Break-Even stop ──────────────────────────────────
    if position.break_even_active {
        if roe <= break_even_target_roe {
            return Some(format!(
                "BREAK_EVEN_STOP: ROE {:.2}% <= target {:.2}%",
                roe, break_even_target_roe
            ));
        }
    }

    // ── Layer 1: Hard stop ────────────────────────────────────────
    if position.atr_at_entry > 0.0 {
        let atr_distance = atr_hard_stop_mult * position.atr_at_entry;
        let stop_hit = if is_long {
            current_price <= position.entry_price - atr_distance
        } else {
            current_price >= position.entry_price + atr_distance
        };
        if stop_hit {
            return Some(format!(
                "HARD_STOP(ATR): {} price {:.6} | stop dist {:.1}×ATR={:.6} | ROE={:.2}%",
                position.side, current_price,
                atr_hard_stop_mult, position.atr_at_entry,
                roe
            ));
        }
    } else {
        if roe <= hard_stop_roe {
            return Some(format!("HARD_STOP(ROE): {:.2}% <= {:.2}%", roe, hard_stop_roe));
        }
    }

    // ── Layer 2: ATR trailing stop ────────────────────────────────
    if position.trailing_active {
        if position.atr_at_entry > 0.0 {
            // Reconstruct the peak price from max_roe.
            let peak_delta = position.entry_price
                * position.max_roe / (position.leverage as f64 * 100.0);
            let trail_distance = atr_trail_mult * position.atr_at_entry;

            let trail_hit = if is_long {
                let peak_price = position.entry_price + peak_delta;
                current_price <= peak_price - trail_distance
            } else {
                let peak_price = position.entry_price - peak_delta;
                current_price >= peak_price + trail_distance
            };

            if trail_hit {
                let drawdown_roe = position.max_roe - roe;
                return Some(format!(
                    "TRAILING_STOP(ATR): {} | ROE={:.2}% | peak_ROE={:.2}% | drawdown={:.2}%",
                    position.side, roe, position.max_roe, drawdown_roe
                ));
            }
        } else {
            let drawdown = position.max_roe - roe;
            if drawdown >= _trailing_stop_pct {
                return Some(format!(
                    "TRAILING_STOP(ROE): {:.2}% dropped {:.2}% from peak {:.2}%",
                    roe, drawdown, position.max_roe
                ));
            }
        }
    }

    let _ = trailing_activation_roe;
    None
}
