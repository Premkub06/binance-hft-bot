use serde::Deserialize;
use std::collections::VecDeque;

// ═══════════════════════════════════════════════════════════════════
//  Binance REST API types
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct ExchangeInfo {
    pub symbols: Vec<SymbolInfo>,
}

#[derive(Debug, Deserialize)]
pub struct SymbolInfo {
    pub symbol: String,
    #[serde(rename = "contractType")]
    pub contract_type: String,
    pub status: String,
    #[serde(rename = "quoteAsset")]
    pub quote_asset: String,
    pub filters: Vec<serde_json::Value>,
}

/// Metadata extracted from exchange info for quantity rounding.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SymbolMeta {
    pub symbol: String,
    pub step_size: f64,
    pub tick_size: f64,
    pub precision: u32,
}

// ═══════════════════════════════════════════════════════════════════
//  WebSocket message types
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct WsCombinedStream {
    pub stream: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct WsKline {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "k")]
    pub kline: KlineData,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct KlineData {
    #[serde(rename = "t")]
    pub open_time: i64,
    #[serde(rename = "T")]
    pub close_time: i64,
    #[serde(rename = "i")]
    pub interval: String,
    #[serde(rename = "o")]
    pub open: String,
    #[serde(rename = "c")]
    pub close: String,
    #[serde(rename = "h")]
    pub high: String,
    #[serde(rename = "l")]
    pub low: String,
    #[serde(rename = "v")]
    pub volume: String,
    #[serde(rename = "q")]
    pub quote_volume: String,
    #[serde(rename = "x")]
    pub is_closed: bool,
}

/// Mark price array element from `!markPrice@arr@1s`.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct WsMarkPrice {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "p")]
    pub mark_price: String,
    #[serde(rename = "i")]
    pub index_price: String,
    #[serde(rename = "E")]
    pub event_time: i64,
}

// ═══════════════════════════════════════════════════════════════════
//  Order types
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OrderResponse {
    #[serde(rename = "orderId")]
    pub order_id: i64,
    pub symbol: String,
    pub status: String,
    #[serde(rename = "avgPrice")]
    pub avg_price: String,
    #[serde(rename = "executedQty")]
    pub executed_qty: String,
    pub side: String,
}

/// Position risk from GET /fapi/v2/positionRisk (for state recovery).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct BinancePositionRisk {
    pub symbol: String,
    #[serde(rename = "positionAmt")]
    pub position_amt: String,
    #[serde(rename = "entryPrice")]
    pub entry_price: String,
    #[serde(rename = "unRealizedProfit")]
    pub unrealized_profit: String,
    pub leverage: String,
    #[serde(rename = "markPrice")]
    pub mark_price: String,
}

// ═══════════════════════════════════════════════════════════════════
//  Internal position tracking (lives in PositionMap)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Position {
    pub symbol: String,
    /// "BUY" (long) or "SELL" (short).
    pub side: String,
    pub entry_price: f64,
    pub quantity: f64,
    pub leverage: u32,
    pub margin_usd: f64,
    pub entry_time: chrono::DateTime<chrono::Utc>,
    pub max_roe: f64,
    pub trailing_active: bool,
    pub order_id: i64,
    /// ATR-14 (price units) captured at the moment of entry.
    /// Used to set a volatility-adjusted trailing stop distance.
    pub atr_at_entry: f64,
    /// Whether the break-even dynamic hard stop is active.
    pub break_even_active: bool,
}

// ═══════════════════════════════════════════════════════════════════
//  In-memory market state per symbol
// ═══════════════════════════════════════════════════════════════════

/// Number of 15-minute candles in 7 days (96 per day × 7).
pub const CANDLES_15M_7_DAYS: usize = 672;

#[derive(Debug)]
pub struct SymbolState {
    pub previous_day_high: f64,
    pub current_day_high: f64,
    pub current_price: f64,
    pub current_15m_volume: f64,
    pub avg_volume_7d_15m: f64,
    /// EMA value (price units). Zero means still in warmup — signals are suppressed.
    pub ema: f64,
    /// RSI-14 value (0–100). Zero means still in warmup.
    pub rsi_14: f64,
    /// Rolling 14-period ATR computed from 15m candles (price units).
    pub atr_14: f64,
    /// Ring buffer of closed 15-min candle volumes for rolling average.
    pub volume_history: VecDeque<f64>,
    /// Ring buffer of last 15 true ranges for ATR calculation.
    pub tr_history: VecDeque<f64>,
    /// Warmup accumulator: holds the first `ema_period` closes for SMA seeding.
    /// Cleared once the EMA is seeded to free memory.
    ema_warmup: Vec<f64>,
    /// RSI internals: Wilder-smoothed average gain/loss.
    rsi_avg_gain: f64,
    rsi_avg_loss: f64,
    rsi_prev_close: f64,
    rsi_count: usize,
    rsi_warmup_gains: Vec<f64>,
    rsi_warmup_losses: Vec<f64>,
}

impl SymbolState {
    pub fn new() -> Self {
        Self {
            previous_day_high: 0.0,
            current_day_high: 0.0,
            current_price: 0.0,
            current_15m_volume: 0.0,
            avg_volume_7d_15m: 0.0,
            ema: 0.0,
            rsi_14: 0.0,
            atr_14: 0.0,
            volume_history: VecDeque::with_capacity(CANDLES_15M_7_DAYS + 16),
            tr_history: VecDeque::with_capacity(16),
            ema_warmup: Vec::new(),
            rsi_avg_gain: 0.0,
            rsi_avg_loss: 0.0,
            rsi_prev_close: 0.0,
            rsi_count: 0,
            rsi_warmup_gains: Vec::with_capacity(14),
            rsi_warmup_losses: Vec::with_capacity(14),
        }
    }

    /// Recalculate the rolling 7-day average from the volume ring buffer.
    #[inline]
    pub fn recalc_avg_volume(&mut self) {
        if self.volume_history.is_empty() {
            self.avg_volume_7d_15m = 0.0;
            return;
        }
        let sum: f64 = self.volume_history.iter().sum();
        self.avg_volume_7d_15m = sum / self.volume_history.len() as f64;
    }

    /// Push a new closed candle's True Range and recalculate ATR-14.
    /// TR = max(high-low, |high-prev_close|, |low-prev_close|)
    #[inline]
    pub fn push_true_range(&mut self, high: f64, low: f64, prev_close: f64) {
        let tr = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());

        self.tr_history.push_back(tr);
        if self.tr_history.len() > 14 {
            self.tr_history.pop_front();
        }

        if !self.tr_history.is_empty() {
            let sum: f64 = self.tr_history.iter().sum();
            self.atr_14 = sum / self.tr_history.len() as f64;
        }
    }

    /// Feed one closed candle's close price into the EMA calculation.
    ///
    /// **Seeding strategy (industry-standard):**
    ///   Phase 1 — warmup: accumulate the first `period` closes into `ema_warmup`.
    ///   Phase 2 — seed:   when `ema_warmup` reaches `period` entries, compute
    ///                     their SMA and use it as the initial EMA value.
    ///   Phase 3 — live:   every subsequent close applies the standard EMA formula:
    ///                     `ema = close × k + prev_ema × (1-k)`  where k = 2/(N+1).
    ///
    /// The warmup Vec is cleared after seeding to reclaim memory.
    #[inline]
    pub fn push_close_for_ema(&mut self, close: f64, period: usize) {
        if close <= 0.0 || period == 0 {
            return;
        }

        if self.ema > 0.0 {
            // Phase 3: EMA is seeded — apply standard multiplier.
            let k = 2.0 / (period as f64 + 1.0);
            self.ema = close * k + self.ema * (1.0 - k);
        } else {
            // Phase 1 & 2: still warming up.
            self.ema_warmup.push(close);

            if self.ema_warmup.len() >= period {
                // Phase 2: seed EMA with the SMA of the first `period` closes.
                let sum: f64 = self.ema_warmup.iter().sum();
                self.ema = sum / self.ema_warmup.len() as f64;

                // Free the warmup buffer — no longer needed.
                self.ema_warmup.clear();
                self.ema_warmup.shrink_to_fit();
            }
            // Phase 1: still accumulating, ema remains 0.0 until seeded.
        }
    }

    /// Feed one closed candle’s close price to update RSI-14 (Wilder smoothing).
    ///
    /// Phase 1 (warmup): accumulate the first 14 gain/loss deltas.
    /// Phase 2 (seed):   SMA of those 14 deltas as initial avg_gain / avg_loss.
    /// Phase 3 (live):   Wilder formula: `avg = (prev_avg * 13 + current) / 14`.
    #[inline]
    pub fn push_close_for_rsi(&mut self, close: f64) {
        if close <= 0.0 {
            return;
        }
        if self.rsi_prev_close <= 0.0 {
            // First close — just store it, no delta yet.
            self.rsi_prev_close = close;
            return;
        }

        let delta = close - self.rsi_prev_close;
        self.rsi_prev_close = close;
        let gain = if delta > 0.0 { delta } else { 0.0 };
        let loss = if delta < 0.0 { -delta } else { 0.0 };

        self.rsi_count += 1;

        if self.rsi_14 > 0.0 {
            // Phase 3: Wilder smoothing.
            self.rsi_avg_gain = (self.rsi_avg_gain * 13.0 + gain) / 14.0;
            self.rsi_avg_loss = (self.rsi_avg_loss * 13.0 + loss) / 14.0;
        } else {
            // Phase 1 & 2: warmup.
            self.rsi_warmup_gains.push(gain);
            self.rsi_warmup_losses.push(loss);

            if self.rsi_warmup_gains.len() >= 14 {
                let sum_gain: f64 = self.rsi_warmup_gains.iter().sum();
                let sum_loss: f64 = self.rsi_warmup_losses.iter().sum();
                self.rsi_avg_gain = sum_gain / 14.0;
                self.rsi_avg_loss = sum_loss / 14.0;

                self.rsi_warmup_gains.clear();
                self.rsi_warmup_gains.shrink_to_fit();
                self.rsi_warmup_losses.clear();
                self.rsi_warmup_losses.shrink_to_fit();
            } else {
                return; // Not enough data yet.
            }
        }

        // Compute RSI.
        if self.rsi_avg_loss == 0.0 {
            self.rsi_14 = 100.0;
        } else {
            let rs = self.rsi_avg_gain / self.rsi_avg_loss;
            self.rsi_14 = 100.0 - (100.0 / (1.0 + rs));
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Database event types (sent via crossbeam channel)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug)]
pub enum DbEvent {
    TradeOpened {
        symbol: String,
        side: String,
        entry_price: f64,
        quantity: f64,
        leverage: u32,
        margin_usd: f64,
        order_id: i64,
    },
    TradeClosed {
        symbol: String,
        exit_price: f64,
        pnl_usd: f64,
        roe_pct: f64,
        exit_reason: String,
    },
    SystemLog {
        level: String,
        message: String,
    },
    /// Periodic flush of live ROE/PnL for OPEN positions.
    UpdateLiveRoe {
        symbol: String,
        pnl_usd: f64,
        roe_pct: f64,
    },
    /// Mark stale OPEN trades as CLOSED (state recovery).
    ForceClose {
        symbol: String,
        exit_reason: String,
    },
}

// ═══════════════════════════════════════════════════════════════════
//  Strategy signal
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct TradeSignal {
    pub symbol: String,
    /// "BUY" for long, "SELL" for short.
    pub side: String,
    pub price: f64,
    pub volume_15m: f64,
    pub avg_volume_7d: f64,
    pub previous_day_high: f64,
    /// ATR-14 at the moment the signal fires (price units).
    pub atr_14: f64,
    /// RSI-14 at signal time.
    pub rsi_14: f64,
}
