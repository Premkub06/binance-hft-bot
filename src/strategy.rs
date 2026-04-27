use crate::models::{Position, SymbolState, TradeSignal};

/// Pure, zero-allocation evaluator for the altcoin breakout strategy.
///
/// Trigger conditions (both must be true):
///   1. `current_price > previous_day_high`
///   2. `current_15m_volume > volume_multiplier × avg_volume_7d_15m`
///
/// Returns `Some(TradeSignal)` if the trigger fires, `None` otherwise.
#[inline(always)]
pub fn evaluate_breakout(
    symbol: &str,
    state: &SymbolState,
    volume_multiplier: f64,
    has_open_position: bool,
) -> Option<TradeSignal> {
    // Skip if we already have a position on this symbol.
    if has_open_position {
        return None;
    }

    // Guard: need valid historical data to evaluate.
    if state.previous_day_high <= 0.0 || state.avg_volume_7d_15m <= 0.0 {
        return None;
    }

    let price_breakout = state.current_price > state.previous_day_high;
    let volume_surge = state.current_15m_volume > volume_multiplier * state.avg_volume_7d_15m;

    if price_breakout && volume_surge {
        Some(TradeSignal {
            symbol: symbol.to_owned(),
            price: state.current_price,
            volume_15m: state.current_15m_volume,
            avg_volume_7d: state.avg_volume_7d_15m,
            previous_day_high: state.previous_day_high,
            atr_14: state.atr_14,
        })
    } else {
        None
    }
}

/// Calculate ROE (Return on Equity) percentage for a LONG position.
#[inline(always)]
pub fn calculate_roe(position: &Position, current_price: f64) -> f64 {
    ((current_price - position.entry_price) / position.entry_price)
        * position.leverage as f64
        * 100.0
}

// ═══════════════════════════════════════════════════════════════════
//  ATR-based dynamic risk evaluator
// ═══════════════════════════════════════════════════════════════════

/// Determine if a position should be closed based on ATR-adjusted risk rules.
///
/// **Stop logic (dual-layer):**
///
/// Layer 1 — ATR Hard Stop (primary):
///   Fires if `current_price < entry_price - (atr_hard_stop_mult × ATR)`.
///   In ROE terms, this adapts to each coin's actual volatility.
///   Example: entry=$1.00, ATR=$0.03, mult=2.5 → stop at $0.925 → ROE = -7.5% × 10x = -75%
///   For a quieter coin: ATR=$0.01 → stop at $0.975 → tighter, less capital risk.
///
/// Layer 2 — ATR Trailing Stop (after activation ROE threshold):
///   Fires if `price < peak_price - (atr_trail_mult × ATR)`.
///   This gives the trade a fixed PRICE buffer (not % buffer) to breathe through
///   normal volatility, while still locking in gains after the move.
///
/// ROE-based hard stop (fallback):
///   Used when ATR is unavailable (atr_at_entry = 0) — falls back to the
///   static `hard_stop_roe` from config, ensuring the bot always has a stop.
///
/// Returns `Some(reason)` if the position should be closed.
#[inline]
pub fn evaluate_risk(
    position: &Position,
    current_price: f64,
    hard_stop_roe: f64,
    trailing_activation_roe: f64,
    _trailing_stop_pct: f64, // kept for signature compatibility
    atr_hard_stop_mult: f64,
    atr_trail_mult: f64,
) -> Option<String> {
    let roe = calculate_roe(position, current_price);

    // ── Layer 1: Hard stop ────────────────────────────────────────
    if position.atr_at_entry > 0.0 {
        // ATR-dynamic hard stop (price-based).
        let stop_price = position.entry_price - atr_hard_stop_mult * position.atr_at_entry;
        if current_price <= stop_price {
            return Some(format!(
                "HARD_STOP(ATR): price {:.6} <= stop {:.6} (entry={:.6} - {:.1}×ATR={:.6}) ROE={:.2}%",
                current_price, stop_price,
                position.entry_price, atr_hard_stop_mult, position.atr_at_entry,
                roe
            ));
        }
    } else {
        // Fallback to static ROE stop when no ATR data.
        if roe <= hard_stop_roe {
            return Some(format!("HARD_STOP(ROE): {:.2}% <= {:.2}%", roe, hard_stop_roe));
        }
    }

    // ── Layer 2: ATR trailing stop (only after activation) ────────
    if position.trailing_active {
        if position.atr_at_entry > 0.0 {
            // max_roe was set in price units by mark-price stream.
            // Reconstruct the peak price from max_roe.
            let peak_price = position.entry_price
                * (1.0 + position.max_roe / (position.leverage as f64 * 100.0));
            let trail_stop_price = peak_price - atr_trail_mult * position.atr_at_entry;

            if current_price <= trail_stop_price {
                let drawdown_roe = position.max_roe - roe;
                return Some(format!(
                    "TRAILING_STOP(ATR): price {:.6} <= trail {:.6} (peak={:.6} - {:.1}×ATR={:.6}) \
                     peak_ROE={:.2}% drawdown={:.2}%",
                    current_price, trail_stop_price, peak_price,
                    atr_trail_mult, position.atr_at_entry,
                    position.max_roe, drawdown_roe
                ));
            }
        } else {
            // Fallback: static ROE trailing stop.
            let drawdown = position.max_roe - roe;
            if drawdown >= _trailing_stop_pct {
                return Some(format!(
                    "TRAILING_STOP(ROE): {:.2}% dropped {:.2}% from peak {:.2}%",
                    roe, drawdown, position.max_roe
                ));
            }
        }
    }

    // ── Check trailing stop activation ────────────────────────────
    // (Returned as None — activation state is updated by the caller)
    let _ = trailing_activation_roe; // used by caller

    None
}
