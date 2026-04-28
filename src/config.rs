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
    /// Maximum number of concurrent open positions allowed.
    pub max_open_positions: usize,
    /// EMA trend filter period (e.g., 50 or 200).
    pub ema_period: usize,
    /// RSI-14 threshold for oversold (LONG entry). Default 30.0.
    pub rsi_oversold: f64,
    /// RSI-14 threshold for overbought (SHORT entry). Default 70.0.
    pub rsi_overbought: f64,

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

    // ── Break-even / risk-free mechanism ──
    /// ROE (%) at which the hard stop is raised to break-even.
    /// Default 15.0 → when ROE crosses +15%, the stop moves up.
    pub break_even_trigger_roe: f64,
    /// ROE (%) the hard stop is raised TO once triggered.
    /// Default 1.0 → guarantees a small profit even if stopped out.
    pub break_even_target_roe: f64,

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
            max_open_positions: env::var("MAX_OPEN_POSITIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),
            ema_period: env::var("EMA_PERIOD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50),
            rsi_oversold: env::var("RSI_OVERSOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30.0),
            rsi_overbought: env::var("RSI_OVERBOUGHT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(70.0),
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
            break_even_trigger_roe: env::var("BREAK_EVEN_TRIGGER_ROE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(15.0),
            break_even_target_roe: env::var("BREAK_EVEN_TARGET_ROE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1.0),
            db_path: env::var("DB_PATH").unwrap_or_else(|_| "hft_bot.db".into()),
        }
    }
}
