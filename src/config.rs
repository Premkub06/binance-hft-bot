use std::env;

/// Central configuration loaded once at startup from environment variables.
#[derive(Clone, Debug)]
pub struct Config {
    // ── Binance credentials ──
    pub api_key: String,
    pub api_secret: String,

    // ── Endpoints ──
    pub base_url: String,
    pub ws_url: String,

    // ── Strategy parameters ──
    pub margin_usd: f64,
    pub leverage: u32,
    pub volume_multiplier: f64,
    pub top_n_symbols: usize,

    // ── Risk parameters ──
    pub hard_stop_roe: f64,
    pub trailing_activation_roe: f64,
    pub trailing_stop_pct: f64,
    /// ATR multiplier for the hard stop (price-based fallback).
    /// Hard stop price = entry - (atr_hard_stop_mult × ATR).
    pub atr_hard_stop_mult: f64,
    /// ATR multiplier for the trailing stop distance.
    /// Trailing stop triggers when price pulls back > (atr_trail_mult × ATR) from peak.
    pub atr_trail_mult: f64,

    // ── Database ──
    pub db_path: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            api_key: env::var("BINANCE_API_KEY").expect("BINANCE_API_KEY is required"),
            api_secret: env::var("BINANCE_API_SECRET").expect("BINANCE_API_SECRET is required"),
            base_url: env::var("BINANCE_BASE_URL")
                .unwrap_or_else(|_| "https://fapi.binance.com".into()),
            ws_url: env::var("BINANCE_WS_URL")
                .unwrap_or_else(|_| "wss://fstream.binance.com".into()),
            margin_usd: 6.0,
            leverage: 10,
            volume_multiplier: 3.0,
            top_n_symbols: 100,
            hard_stop_roe: -10.0,
            trailing_activation_roe: 20.0,
            trailing_stop_pct: 5.0,
            // ATR-based stop multipliers (tunable via .env)
            // atr_hard_stop_mult=2.5 → stop at 2.5× ATR below entry
            // atr_trail_mult=2.0    → trailing stop 2× ATR below peak price
            atr_hard_stop_mult: env::var("ATR_HARD_STOP_MULT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2.5),
            atr_trail_mult: env::var("ATR_TRAIL_MULT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2.0),
            db_path: env::var("DB_PATH").unwrap_or_else(|_| "hft_bot.db".into()),
        }
    }
}
