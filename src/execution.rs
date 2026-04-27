use anyhow::{bail, Context, Result};
use hmac::{Hmac, Mac};
use reqwest::Client;
use sha2::Sha256;
use tracing::{info, warn};

use crate::config::Config;
use crate::models::{ExchangeInfo, OrderResponse, SymbolMeta};
use crate::state::SymbolMetaMap;

type HmacSha256 = Hmac<Sha256>;

// ═══════════════════════════════════════════════════════════════════
//  Binance REST client with connection pooling
// ═══════════════════════════════════════════════════════════════════

#[derive(Clone)]
pub struct BinanceClient {
    client: Client,
    config: Config,
}

impl BinanceClient {
    pub fn new(config: Config) -> Self {
        let client = Client::builder()
            .pool_max_idle_per_host(5)
            .tcp_nodelay(true)
            .build()
            .expect("Failed to build reqwest client");
        Self { client, config }
    }

    /// HMAC-SHA256 signature for Binance authenticated endpoints.
    fn sign(&self, query: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(self.config.api_secret.as_bytes())
            .expect("HMAC key error");
        mac.update(query.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    fn timestamp_ms() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    // ── Exchange info ──────────────────────────────────────────────

    /// Fetch exchange info and return the top N USDT-margined perpetual symbols.
    pub async fn fetch_top_symbols(&self, top_n: usize) -> Result<Vec<String>> {
        let url = format!("{}/fapi/v1/exchangeInfo", self.config.base_url);
        let info: ExchangeInfo = self.client.get(&url).send().await?.json().await?;

        let mut symbols: Vec<String> = info
            .symbols
            .iter()
            .filter(|s| {
                s.contract_type == "PERPETUAL"
                    && s.status == "TRADING"
                    && s.quote_asset == "USDT"
                    && s.symbol != "BTCUSDT"  // Exclude BTC (not altcoin)
                    && s.symbol != "ETHUSDT"  // Exclude ETH (not altcoin)
            })
            .map(|s| s.symbol.clone())
            .collect();

        // Sort by name for deterministic ordering; ideally sort by volume.
        symbols.truncate(top_n);
        info!("Fetched {} tradable USDT perpetual altcoins", symbols.len());
        Ok(symbols)
    }

    /// Extract step sizes from exchange info for quantity rounding.
    pub async fn fetch_symbol_meta(&self, meta_map: &SymbolMetaMap) -> Result<()> {
        let url = format!("{}/fapi/v1/exchangeInfo", self.config.base_url);
        let info: ExchangeInfo = self.client.get(&url).send().await?.json().await?;

        for sym in &info.symbols {
            let mut step_size = 1.0_f64;
            let mut tick_size = 0.01_f64;
            for filter in &sym.filters {
                if let Some(ft) = filter.get("filterType").and_then(|v| v.as_str()) {
                    if ft == "LOT_SIZE" {
                        if let Some(ss) = filter.get("stepSize").and_then(|v| v.as_str()) {
                            step_size = ss.parse().unwrap_or(1.0);
                        }
                    }
                    if ft == "PRICE_FILTER" {
                        if let Some(ts) = filter.get("tickSize").and_then(|v| v.as_str()) {
                            tick_size = ts.parse().unwrap_or(0.01);
                        }
                    }
                }
            }

            // Derive decimal precision from step_size.
            let precision = if step_size > 0.0 && step_size < 1.0 {
                (-step_size.log10()).ceil() as u32
            } else {
                0
            };

            meta_map.insert(
                sym.symbol.clone(),
                SymbolMeta {
                    symbol: sym.symbol.clone(),
                    step_size,
                    tick_size,
                    precision,
                },
            );
        }

        info!("Loaded symbol metadata for {} pairs", meta_map.len());
        Ok(())
    }

    // ── Historical kline data ──────────────────────────────────────

    /// Fetch historical kline data. Returns Vec of [open_time, open, high, low, close, volume, ...].
    pub async fn fetch_klines(
        &self,
        symbol: &str,
        interval: &str,
        limit: u16,
    ) -> Result<Vec<Vec<serde_json::Value>>> {
        let url = format!(
            "{}/fapi/v1/klines?symbol={}&interval={}&limit={}",
            self.config.base_url, symbol, interval, limit
        );
        let data: Vec<Vec<serde_json::Value>> = self.client.get(&url).send().await?.json().await?;
        Ok(data)
    }

    // ── Set leverage ───────────────────────────────────────────────

    pub async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<()> {
        let ts = Self::timestamp_ms();
        let query = format!(
            "symbol={}&leverage={}&timestamp={}&recvWindow=5000",
            symbol, leverage, ts
        );
        let sig = self.sign(&query);
        let url = format!(
            "{}/fapi/v1/leverage?{}&signature={}",
            self.config.base_url, query, sig
        );

        let resp = self
            .client
            .post(&url)
            .header("X-MBX-APIKEY", &self.config.api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            warn!("Set leverage for {} returned: {}", symbol, body);
        }
        Ok(())
    }

    // ── Place market order ─────────────────────────────────────────

    /// Place a MARKET order (BUY or SELL).
    pub async fn market_order(
        &self,
        symbol: &str,
        side: &str,
        quantity: f64,
        meta_map: &SymbolMetaMap,
    ) -> Result<OrderResponse> {
        let precision = meta_map
            .get(symbol)
            .map(|m| m.precision)
            .unwrap_or(3);

        let qty_str = format!("{:.prec$}", quantity, prec = precision as usize);

        let ts = Self::timestamp_ms();
        let query = format!(
            "symbol={}&side={}&type=MARKET&quantity={}&newOrderRespType=RESULT&timestamp={}&recvWindow=5000",
            symbol, side, qty_str, ts
        );
        let sig = self.sign(&query);
        let url = format!(
            "{}/fapi/v1/order?{}&signature={}",
            self.config.base_url, query, sig
        );

        let resp = self
            .client
            .post(&url)
            .header("X-MBX-APIKEY", &self.config.api_key)
            .send()
            .await
            .context("Order request failed")?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            bail!("Order rejected ({}): {}", status, body);
        }

        let order: OrderResponse =
            serde_json::from_str(&body).context("Failed to parse order response")?;

        info!(
            "ORDER FILLED: {} {} {} @ avg {}",
            order.side, order.executed_qty, symbol, order.avg_price
        );
        Ok(order)
    }

    // ── Fetch open positions (state recovery) ──────────────────────

    /// Fetch all currently open positions from Binance Futures API.
    /// Returns only positions with non-zero quantity (actually open).
    pub async fn fetch_open_positions(&self) -> Result<Vec<crate::models::BinancePositionRisk>> {
        let ts = Self::timestamp_ms();
        let query = format!("timestamp={}&recvWindow=5000", ts);
        let sig = self.sign(&query);
        let url = format!(
            "{}/fapi/v2/positionRisk?{}&signature={}",
            self.config.base_url, query, sig
        );

        let resp = self
            .client
            .get(&url)
            .header("X-MBX-APIKEY", &self.config.api_key)
            .send()
            .await
            .context("Failed to fetch position risk")?;

        let body = resp.text().await.unwrap_or_default();
        let all_positions: Vec<crate::models::BinancePositionRisk> =
            serde_json::from_str(&body).context("Failed to parse position risk")?;

        // Filter to only positions with non-zero positionAmt.
        let open: Vec<_> = all_positions
            .into_iter()
            .filter(|p| {
                let amt: f64 = p.position_amt.parse().unwrap_or(0.0);
                amt.abs() > 0.0
            })
            .collect();

        info!("Fetched {} open positions from Binance", open.len());
        Ok(open)
    }
}
