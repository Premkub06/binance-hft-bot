use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::path::Path;

use crate::models::CANDLES_15M_7_DAYS;

// ═══════════════════════════════════════════════════════════════════
//  Backtest configuration (mirrors live strategy parameters)
// ═══════════════════════════════════════════════════════════════════

const MARGIN_USD: f64 = 6.0;
const LEVERAGE: u32 = 10;
const VOLUME_MULTIPLIER: f64 = 3.0;
const HARD_STOP_ROE: f64 = -10.0;
const TRAILING_ACTIVATION_ROE: f64 = 20.0;
const TRAILING_STOP_PCT: f64 = 5.0;

/// Number of 15-minute candles per day (used to detect daily boundaries).
const CANDLES_PER_DAY: usize = 96;

// ═══════════════════════════════════════════════════════════════════
//  OHLCV candle from CSV
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug)]
#[allow(dead_code)]
struct Candle {
    timestamp: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

// ═══════════════════════════════════════════════════════════════════
//  Simulated position
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug)]
struct SimPosition {
    entry_price: f64,
    quantity: f64,
    max_roe: f64,
    trailing_active: bool,
}

impl SimPosition {
    fn roe_at(&self, price: f64) -> f64 {
        ((price - self.entry_price) / self.entry_price) * LEVERAGE as f64 * 100.0
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Trade result record
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug)]
struct TradeResult {
    entry_price: f64,
    exit_price: f64,
    pnl_usd: f64,
    roe_pct: f64,
    exit_reason: String,
}

// ═══════════════════════════════════════════════════════════════════
//  CSV loader
// ═══════════════════════════════════════════════════════════════════

fn load_candles(path: &Path) -> Result<Vec<Candle>> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_path(path)
        .with_context(|| format!("Failed to open CSV: {}", path.display()))?;

    let mut candles = Vec::new();

    for result in reader.records() {
        let record = result.context("Failed to read CSV row")?;

        // Expected format: timestamp, open, high, low, close, volume
        // Supports both integer timestamps and float timestamps.
        let timestamp: i64 = record
            .get(0)
            .context("Missing timestamp column")?
            .trim()
            .parse::<f64>()
            .map(|v| v as i64)
            .or_else(|_| record.get(0).unwrap().trim().parse::<i64>())
            .context("Invalid timestamp")?;

        let open: f64 = record.get(1).context("Missing open")?.trim().parse().context("Invalid open")?;
        let high: f64 = record.get(2).context("Missing high")?.trim().parse().context("Invalid high")?;
        let low: f64 = record.get(3).context("Missing low")?.trim().parse().context("Invalid low")?;
        let close: f64 = record.get(4).context("Missing close")?.trim().parse().context("Invalid close")?;
        let volume: f64 = record.get(5).context("Missing volume")?.trim().parse().context("Invalid volume")?;

        candles.push(Candle { timestamp, open, high, low, close, volume });
    }

    anyhow::ensure!(!candles.is_empty(), "CSV file contains no data rows");
    Ok(candles)
}

// ═══════════════════════════════════════════════════════════════════
//  Core backtest engine
// ═══════════════════════════════════════════════════════════════════

pub fn run_backtest(csv_path: &str) -> Result<()> {
    let path = Path::new(csv_path);
    let candles = load_candles(path)?;

    println!("═══════════════════════════════════════════════════════");
    println!("  BACKTEST ENGINE — Altcoin Breakout Strategy");
    println!("═══════════════════════════════════════════════════════");
    println!("  CSV file     : {}", csv_path);
    println!("  Total candles: {}", candles.len());
    println!("  Margin/trade : ${} × {}x leverage", MARGIN_USD, LEVERAGE);
    println!("  Hard stop    : {}% ROE", HARD_STOP_ROE);
    println!("  Trailing     : activate at +{}%, stop at {}% drawdown",
             TRAILING_ACTIVATION_ROE, TRAILING_STOP_PCT);
    println!("  Volume mult  : {}×", VOLUME_MULTIPLIER);
    println!("═══════════════════════════════════════════════════════\n");

    // ── State variables ──
    let mut volume_history: VecDeque<f64> = VecDeque::with_capacity(CANDLES_15M_7_DAYS + 16);
    let mut avg_volume_7d: f64 = 0.0;

    // Daily high tracking: use day boundaries from timestamps.
    // We assume timestamps are in milliseconds or seconds.
    let ts_divisor = detect_timestamp_unit(&candles);
    let mut current_day: i64 = -1;
    let mut current_day_high: f64 = 0.0;
    let mut previous_day_high: f64 = 0.0;

    let mut position: Option<SimPosition> = None;
    let mut trades: Vec<TradeResult> = Vec::new();

    // Equity tracking for drawdown.
    let mut equity: f64 = 0.0;
    let mut peak_equity: f64 = 0.0;
    let mut max_drawdown: f64 = 0.0;

    // ── Warm-up: need at least 1 full day + volume history ──
    let warmup_candles = CANDLES_PER_DAY + 1; // 1 day for previous_day_high

    for (i, candle) in candles.iter().enumerate() {
        let day = (candle.timestamp / ts_divisor) / 86400;

        // ── Daily boundary detection ──
        if day != current_day {
            if current_day >= 0 {
                // Rotate: current day's high becomes previous day's high.
                previous_day_high = current_day_high;
            }
            current_day = day;
            current_day_high = candle.high;
        } else if candle.high > current_day_high {
            current_day_high = candle.high;
        }

        // ── Update volume rolling average ──
        volume_history.push_back(candle.volume);
        if volume_history.len() > CANDLES_15M_7_DAYS {
            volume_history.pop_front();
        }
        if !volume_history.is_empty() {
            let sum: f64 = volume_history.iter().sum();
            avg_volume_7d = sum / volume_history.len() as f64;
        }

        // ── Risk management: check open position on every candle ──
        if let Some(ref mut pos) = position {
            // Check LOW of candle for hard stop (worst case intra-candle).
            let roe_at_low = pos.roe_at(candle.low);
            let roe_at_high = pos.roe_at(candle.high);
            let _roe_at_close = pos.roe_at(candle.close);

            // Update max ROE (use high of candle).
            if roe_at_high > pos.max_roe {
                pos.max_roe = roe_at_high;
            }

            // Activate trailing stop.
            if !pos.trailing_active && pos.max_roe >= TRAILING_ACTIVATION_ROE {
                pos.trailing_active = true;
            }

            let mut exit_price: Option<f64> = None;
            let mut exit_reason = String::new();

            // Hard stop check (use low — most conservative).
            if roe_at_low <= HARD_STOP_ROE {
                // Estimate stop price from ROE formula:
                // roe = ((price - entry) / entry) * leverage * 100
                // price = entry * (1 + roe / (leverage * 100))
                let stop_price = pos.entry_price * (1.0 + HARD_STOP_ROE / (LEVERAGE as f64 * 100.0));
                exit_price = Some(stop_price.max(candle.low));
                exit_reason = format!("HARD_STOP at ROE {:.2}%", HARD_STOP_ROE);
            }
            // Trailing stop check.
            else if pos.trailing_active {
                let drawdown = pos.max_roe - roe_at_low;
                if drawdown >= TRAILING_STOP_PCT {
                    // Estimate stop price from (max_roe - trailing_stop_pct).
                    let stop_roe = pos.max_roe - TRAILING_STOP_PCT;
                    let stop_price = pos.entry_price * (1.0 + stop_roe / (LEVERAGE as f64 * 100.0));
                    exit_price = Some(stop_price.max(candle.low));
                    exit_reason = format!(
                        "TRAILING_STOP: peak ROE {:.2}%, exited at ~{:.2}%",
                        pos.max_roe, pos.max_roe - TRAILING_STOP_PCT
                    );
                }
            }

            if let Some(ep) = exit_price {
                let pnl = (ep - pos.entry_price) * pos.quantity;
                let roe = pos.roe_at(ep);

                equity += pnl;
                if equity > peak_equity {
                    peak_equity = equity;
                }
                let dd = peak_equity - equity;
                if dd > max_drawdown {
                    max_drawdown = dd;
                }

                trades.push(TradeResult {
                    entry_price: pos.entry_price,
                    exit_price: ep,
                    pnl_usd: pnl,
                    roe_pct: roe,
                    exit_reason,
                });

                position = None;
            }
        }

        // ── Strategy evaluation (skip warm-up period) ──
        if i < warmup_candles {
            continue;
        }

        // Only evaluate if no open position.
        if position.is_some() {
            continue;
        }

        // Guard: need valid data.
        if previous_day_high <= 0.0 || avg_volume_7d <= 0.0 {
            continue;
        }

        // Breakout condition (same as live strategy).
        let price_breakout = candle.close > previous_day_high;
        let volume_surge = candle.volume > VOLUME_MULTIPLIER * avg_volume_7d;

        if price_breakout && volume_surge {
            let notional = MARGIN_USD * LEVERAGE as f64;
            let quantity = notional / candle.close;

            position = Some(SimPosition {
                entry_price: candle.close,
                quantity,
                max_roe: 0.0,
                trailing_active: false,
            });

            println!(
                "  📈 ENTRY #{:>3} | price={:.6} | vol={:.2} > {:.2}×avg | day_high={:.6}",
                trades.len() + 1,
                candle.close,
                candle.volume,
                VOLUME_MULTIPLIER,
                previous_day_high
            );
        }
    }

    // ── Force-close any remaining open position at last candle's close ──
    if let Some(pos) = position {
        let last = candles.last().unwrap();
        let pnl = (last.close - pos.entry_price) * pos.quantity;
        let roe = pos.roe_at(last.close);

        equity += pnl;
        if equity > peak_equity {
            peak_equity = equity;
        }
        let dd = peak_equity - equity;
        if dd > max_drawdown {
            max_drawdown = dd;
        }

        trades.push(TradeResult {
            entry_price: pos.entry_price,
            exit_price: last.close,
            pnl_usd: pnl,
            roe_pct: roe,
            exit_reason: "END_OF_DATA".to_string(),
        });
    }

    // ═══════════════════════════════════════════════════════════════
    //  Performance summary
    // ═══════════════════════════════════════════════════════════════

    let total_trades = trades.len();
    let winning_trades = trades.iter().filter(|t| t.pnl_usd > 0.0).count();
    let losing_trades = trades.iter().filter(|t| t.pnl_usd <= 0.0).count();
    let total_pnl: f64 = trades.iter().map(|t| t.pnl_usd).sum();
    let win_rate = if total_trades > 0 {
        (winning_trades as f64 / total_trades as f64) * 100.0
    } else {
        0.0
    };

    let avg_win = if winning_trades > 0 {
        trades.iter().filter(|t| t.pnl_usd > 0.0).map(|t| t.pnl_usd).sum::<f64>() / winning_trades as f64
    } else {
        0.0
    };
    let avg_loss = if losing_trades > 0 {
        trades.iter().filter(|t| t.pnl_usd <= 0.0).map(|t| t.pnl_usd).sum::<f64>() / losing_trades as f64
    } else {
        0.0
    };

    let profit_factor = if avg_loss.abs() > 0.0 {
        let gross_profit: f64 = trades.iter().filter(|t| t.pnl_usd > 0.0).map(|t| t.pnl_usd).sum();
        let gross_loss: f64 = trades.iter().filter(|t| t.pnl_usd <= 0.0).map(|t| t.pnl_usd.abs()).sum();
        if gross_loss > 0.0 { gross_profit / gross_loss } else { f64::INFINITY }
    } else {
        f64::INFINITY
    };

    let best_trade = trades.iter().map(|t| t.pnl_usd).fold(f64::NEG_INFINITY, f64::max);
    let worst_trade = trades.iter().map(|t| t.pnl_usd).fold(f64::INFINITY, f64::min);

    // ── Exit reason breakdown ──
    let hard_stops = trades.iter().filter(|t| t.exit_reason.starts_with("HARD_STOP")).count();
    let trailing_stops = trades.iter().filter(|t| t.exit_reason.starts_with("TRAILING_STOP")).count();
    let end_of_data = trades.iter().filter(|t| t.exit_reason == "END_OF_DATA").count();

    println!("\n═══════════════════════════════════════════════════════");
    println!("  📊 BACKTEST RESULTS");
    println!("═══════════════════════════════════════════════════════");
    println!();
    println!("  ┌─────────────────────┬────────────────────────┐");
    println!("  │ Metric              │ Value                  │");
    println!("  ├─────────────────────┼────────────────────────┤");
    println!("  │ Total Trades        │ {:>22} │", total_trades);
    println!("  │ Winning Trades      │ {:>22} │", winning_trades);
    println!("  │ Losing Trades       │ {:>22} │", losing_trades);
    println!("  │ Win Rate            │ {:>21.2}% │", win_rate);
    println!("  ├─────────────────────┼────────────────────────┤");
    println!("  │ Total PnL           │ {:>21.4}$ │", total_pnl);
    println!("  │ Avg Win             │ {:>21.4}$ │", avg_win);
    println!("  │ Avg Loss            │ {:>21.4}$ │", avg_loss);
    println!("  │ Best Trade          │ {:>21.4}$ │", if total_trades > 0 { best_trade } else { 0.0 });
    println!("  │ Worst Trade         │ {:>21.4}$ │", if total_trades > 0 { worst_trade } else { 0.0 });
    println!("  ├─────────────────────┼────────────────────────┤");
    println!("  │ Max Drawdown        │ {:>21.4}$ │", max_drawdown);
    println!("  │ Profit Factor       │ {:>22.2} │", if profit_factor.is_infinite() { 999.99 } else { profit_factor });
    println!("  ├─────────────────────┼────────────────────────┤");
    println!("  │ Hard Stops          │ {:>22} │", hard_stops);
    println!("  │ Trailing Stops      │ {:>22} │", trailing_stops);
    println!("  │ End-of-Data Closes  │ {:>22} │", end_of_data);
    println!("  └─────────────────────┴────────────────────────┘");
    println!();

    // ── Individual trade log ──
    if total_trades > 0 && total_trades <= 200 {
        println!("  ── Trade Log ──────────────────────────────────────");
        println!("  {:>4} {:>12} {:>12} {:>10} {:>8}  {}", "#", "Entry", "Exit", "PnL ($)", "ROE %", "Reason");
        for (i, t) in trades.iter().enumerate() {
            let pnl_indicator = if t.pnl_usd >= 0.0 { "✅" } else { "❌" };
            println!(
                "  {:>4} {:>12.6} {:>12.6} {:>9.4} {:>7.2}%  {} {}",
                i + 1, t.entry_price, t.exit_price, t.pnl_usd, t.roe_pct, pnl_indicator, t.exit_reason
            );
        }
        println!();
    } else if total_trades > 200 {
        println!("  (Trade log suppressed: {} trades — too many to display)\n", total_trades);
    }

    println!("═══════════════════════════════════════════════════════");
    println!("  Backtest complete.");
    println!("═══════════════════════════════════════════════════════");

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
//  Utility: detect if timestamps are seconds or milliseconds
// ═══════════════════════════════════════════════════════════════════

fn detect_timestamp_unit(candles: &[Candle]) -> i64 {
    if let Some(first) = candles.first() {
        // Millisecond timestamps are > 1e12, second timestamps are ~ 1e9.
        if first.timestamp > 1_000_000_000_000 {
            1000 // divide by 1000 to convert ms → s
        } else {
            1 // already in seconds
        }
    } else {
        1
    }
}
