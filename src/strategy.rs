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

/// Determine if a position should be closed based on risk rules.
///
/// Returns `Some(reason)` if the position should be closed.
#[inline]
pub fn evaluate_risk(
    position: &Position,
    current_price: f64,
    hard_stop_roe: f64,
    _trailing_activation_roe: f64,
    trailing_stop_pct: f64,
) -> Option<String> {
    let roe = calculate_roe(position, current_price);

    // Hard stop: immediate liquidation protection.
    if roe <= hard_stop_roe {
        return Some(format!("HARD_STOP: ROE {:.2}% <= {:.2}%", roe, hard_stop_roe));
    }

    // Trailing stop logic.
    if position.trailing_active {
        let drawdown = position.max_roe - roe;
        if drawdown >= trailing_stop_pct {
            return Some(format!(
                "TRAILING_STOP: ROE {:.2}% dropped {:.2}% from peak {:.2}%",
                roe, drawdown, position.max_roe
            ));
        }
    }

    None
}
