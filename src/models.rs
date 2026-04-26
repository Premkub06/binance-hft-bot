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

// ═══════════════════════════════════════════════════════════════════
//  Internal position tracking (lives in PositionMap)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Position {
    pub symbol: String,
    pub entry_price: f64,
    pub quantity: f64,
    pub leverage: u32,
    pub margin_usd: f64,
    pub entry_time: chrono::DateTime<chrono::Utc>,
    pub max_roe: f64,
    pub trailing_active: bool,
    pub order_id: i64,
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
    /// Ring buffer of closed 15-min candle volumes for rolling average.
    pub volume_history: VecDeque<f64>,
}

impl SymbolState {
    pub fn new() -> Self {
        Self {
            previous_day_high: 0.0,
            current_day_high: 0.0,
            current_price: 0.0,
            current_15m_volume: 0.0,
            avg_volume_7d_15m: 0.0,
            volume_history: VecDeque::with_capacity(CANDLES_15M_7_DAYS + 16),
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
}

// ═══════════════════════════════════════════════════════════════════
//  Database event types (sent via crossbeam channel)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug)]
pub enum DbEvent {
    TradeOpened {
        symbol: String,
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
}

// ═══════════════════════════════════════════════════════════════════
//  Strategy signal
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct TradeSignal {
    pub symbol: String,
    pub price: f64,
    pub volume_15m: f64,
    pub avg_volume_7d: f64,
    pub previous_day_high: f64,
}
