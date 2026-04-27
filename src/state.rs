use dashmap::{DashMap, DashSet};
use std::sync::Arc;

use crate::models::{Position, SymbolMeta, SymbolState};

// ═══════════════════════════════════════════════════════════════════
//  Type aliases for the shared concurrent maps
// ═══════════════════════════════════════════════════════════════════

/// Hot market data — sharded DashMap gives near-lock-free reads.
/// For 100 symbols spread across 64 default shards, contention ≈ 0.
pub type MarketState = Arc<DashMap<String, SymbolState>>;

/// Active positions being risk-monitored.
pub type PositionMap = Arc<DashMap<String, Position>>;

/// Symbol metadata (step sizes, precision) for quantity rounding.
pub type SymbolMetaMap = Arc<DashMap<String, SymbolMeta>>;

/// Symbols currently being processed (order sent but not yet confirmed).
/// Acts as an atomic in-flight guard — prevents duplicate API calls.
pub type PendingSet = Arc<DashSet<String>>;

// ═══════════════════════════════════════════════════════════════════
//  Constructors
// ═══════════════════════════════════════════════════════════════════

pub fn new_market_state() -> MarketState {
    Arc::new(DashMap::with_capacity(128))
}

pub fn new_position_map() -> PositionMap {
    Arc::new(DashMap::with_capacity(32))
}

pub fn new_symbol_meta_map() -> SymbolMetaMap {
    Arc::new(DashMap::with_capacity(128))
}

pub fn new_pending_set() -> PendingSet {
    Arc::new(DashSet::with_capacity(32))
}
