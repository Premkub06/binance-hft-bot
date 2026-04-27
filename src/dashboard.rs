// ═══════════════════════════════════════════════════════════════════
//  Standalone TUI Dashboard for Binance HFT Bot
//  Reads from the shared hft_bot.db SQLite database.
//  Run alongside the bot: cargo run --bin dashboard
// ═══════════════════════════════════════════════════════════════════

use std::io::stdout;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
};
use rusqlite::Connection;

// ═══════════════════════════════════════════════════════════════════
//  Data structures
// ═══════════════════════════════════════════════════════════════════

#[derive(Default)]
struct Stats {
    total_trades: i64,
    open_positions: i64,
    closed_trades: i64,
    winning_trades: i64,
    total_pnl: f64,
    win_rate: f64,
}

struct TradeRow {
    symbol: String,
    status: String,
    entry_price: f64,
    pnl: Option<f64>,
    roe: Option<f64>,
    exit_reason: Option<String>,
    entry_time: String,
}

struct LogRow {
    level: String,
    message: String,
    created_at: String,
}

struct DashboardState {
    stats: Stats,
    trades: Vec<TradeRow>,
    logs: Vec<LogRow>,
}

impl Default for DashboardState {
    fn default() -> Self {
        Self {
            stats: Stats::default(),
            trades: Vec::new(),
            logs: Vec::new(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Database queries
// ═══════════════════════════════════════════════════════════════════

fn load_stats(conn: &Connection) -> Result<Stats> {
    let total_trades: i64 = conn
        .query_row("SELECT COUNT(*) FROM trades", [], |r| r.get(0))
        .unwrap_or(0);

    let open_positions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM trades WHERE status = 'OPEN'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let closed_trades: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM trades WHERE status = 'CLOSED'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let winning_trades: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM trades WHERE status = 'CLOSED' AND pnl_usd > 0",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let total_pnl: f64 = conn
        .query_row(
            "SELECT COALESCE(SUM(pnl_usd), 0.0) FROM trades WHERE status = 'CLOSED'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0.0);

    let win_rate = if closed_trades > 0 {
        (winning_trades as f64 / closed_trades as f64) * 100.0
    } else {
        0.0
    };

    Ok(Stats {
        total_trades,
        open_positions,
        closed_trades,
        winning_trades,
        total_pnl,
        win_rate,
    })
}

fn load_trades(conn: &Connection) -> Result<Vec<TradeRow>> {
    let mut stmt = conn.prepare(
        "SELECT symbol, status, entry_price, pnl_usd, roe_pct, exit_reason, entry_time
         FROM trades
         ORDER BY id DESC
         LIMIT 15",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(TradeRow {
            symbol: row.get(0)?,
            status: row.get(1)?,
            entry_price: row.get(2)?,
            pnl: row.get(3)?,
            roe: row.get(4)?,
            exit_reason: row.get(5)?,
            entry_time: row.get::<_, String>(6).unwrap_or_default(),
        })
    })?;

    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn load_logs(conn: &Connection) -> Result<Vec<LogRow>> {
    let mut stmt = conn.prepare(
        "SELECT level, message, created_at
         FROM system_logs
         ORDER BY id DESC
         LIMIT 10",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(LogRow {
            level: row.get(0)?,
            message: row.get(1)?,
            created_at: row.get(2)?,
        })
    })?;

    Ok(rows.filter_map(|r| r.ok()).collect())
}

fn refresh_data(conn: &Connection) -> DashboardState {
    DashboardState {
        stats: load_stats(conn).unwrap_or_default(),
        trades: load_trades(conn).unwrap_or_default(),
        logs: load_logs(conn).unwrap_or_default(),
    }
}

// ═══════════════════════════════════════════════════════════════════
//  UI rendering
// ═══════════════════════════════════════════════════════════════════

fn ui(frame: &mut Frame, state: &DashboardState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(5),  // Top: stats
            Constraint::Min(10),   // Middle: trades table
            Constraint::Length(14), // Bottom: logs
        ])
        .split(frame.area());

    render_stats(frame, chunks[0], &state.stats);
    render_trades(frame, chunks[1], &state.trades);
    render_logs(frame, chunks[2], &state.logs);
}

fn render_stats(frame: &mut Frame, area: Rect, stats: &Stats) {
    let pnl_color = if stats.total_pnl >= 0.0 {
        Color::Green
    } else {
        Color::Red
    };

    let pnl_sign = if stats.total_pnl >= 0.0 { "+" } else { "" };

    let text = vec![
        Line::from(vec![
            Span::styled("  Total PnL: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}${:.4}", pnl_sign, stats.total_pnl),
                Style::default().fg(pnl_color).add_modifier(Modifier::BOLD),
            ),
            Span::raw("    │    "),
            Span::styled("Win Rate: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{:.1}%", stats.win_rate),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  ({}/{})", stats.winning_trades, stats.closed_trades),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Total Trades: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}", stats.total_trades),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::raw("    │    "),
            Span::styled("Open Positions: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}", stats.open_positions),
                Style::default()
                    .fg(if stats.open_positions > 0 {
                        Color::Yellow
                    } else {
                        Color::White
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " 📊 Overall Stats ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_trades(frame: &mut Frame, area: Rect, trades: &[TradeRow]) {
    let header_cells = ["Symbol", "Status", "Entry", "PnL ($)", "ROE %", "Exit Reason", "Time"]
        .iter()
        .map(|h| {
            Cell::from(*h).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        });
    let header = Row::new(header_cells).height(1);

    let rows = trades.iter().map(|t| {
        let status_style = match t.status.as_str() {
            "OPEN" => Style::default().fg(Color::Yellow),
            "CLOSED" => Style::default().fg(Color::DarkGray),
            _ => Style::default(),
        };

        let pnl_str = t.pnl.map(|p| format!("{:.4}", p)).unwrap_or_else(|| "—".into());
        let pnl_style = match t.pnl {
            Some(p) if p > 0.0 => Style::default().fg(Color::Green),
            Some(p) if p < 0.0 => Style::default().fg(Color::Red),
            _ => Style::default().fg(Color::DarkGray),
        };

        let roe_str = t.roe.map(|r| format!("{:.2}%", r)).unwrap_or_else(|| "—".into());
        let roe_style = match t.roe {
            Some(r) if r > 0.0 => Style::default().fg(Color::Green),
            Some(r) if r < 0.0 => Style::default().fg(Color::Red),
            _ => Style::default().fg(Color::DarkGray),
        };

        let reason = t
            .exit_reason
            .as_deref()
            .unwrap_or("—")
            .chars()
            .take(25)
            .collect::<String>();

        let time_short = if t.entry_time.len() > 16 {
            &t.entry_time[5..16] // "MM-DD HH:MM"
        } else {
            &t.entry_time
        };

        Row::new(vec![
            Cell::from(t.symbol.clone()).style(Style::default().fg(Color::White)),
            Cell::from(t.status.clone()).style(status_style),
            Cell::from(format!("{:.4}", t.entry_price)),
            Cell::from(pnl_str).style(pnl_style),
            Cell::from(roe_str).style(roe_style),
            Cell::from(reason),
            Cell::from(time_short.to_string()).style(Style::default().fg(Color::DarkGray)),
        ])
    });

    let widths = [
        Constraint::Length(12),
        Constraint::Length(8),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Min(20),
        Constraint::Length(12),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta))
                .title(Span::styled(
                    " 📋 Recent Trades (last 15) ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .row_highlight_style(Style::default().add_modifier(Modifier::BOLD));

    frame.render_widget(table, area);
}

fn render_logs(frame: &mut Frame, area: Rect, logs: &[LogRow]) {
    let lines: Vec<Line> = logs
        .iter()
        .rev() // oldest first at top, newest at bottom
        .map(|log| {
            let level_style = match log.level.as_str() {
                "ERROR" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                "WARN" => Style::default().fg(Color::Yellow),
                "INFO" => Style::default().fg(Color::Green),
                _ => Style::default().fg(Color::Gray),
            };

            let time_short = if log.created_at.len() > 16 {
                &log.created_at[5..16]
            } else {
                &log.created_at
            };

            Line::from(vec![
                Span::styled(
                    format!(" {} ", time_short),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(format!("[{:>5}] ", log.level), level_style),
                Span::raw(&log.message),
            ])
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue))
        .title(Span::styled(
            " 🔍 System Logs (last 10) ",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ));

    let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

// ═══════════════════════════════════════════════════════════════════
//  Main loop
// ═══════════════════════════════════════════════════════════════════

fn main() -> Result<()> {
    // ── Open DB read-only (WAL allows concurrent reads) ──
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "hft_bot.db".into());
    let conn = Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("Cannot open database: {}", db_path))?;

    conn.pragma_update(None, "journal_mode", "WAL")?;

    // ── Setup terminal ──
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let tick_rate = Duration::from_secs(1);
    let mut last_tick = Instant::now();
    let mut state = refresh_data(&conn);

    // ── Event loop ──
    loop {
        terminal.draw(|f| ui(f, &state))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => break,
                        _ => {}
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            state = refresh_data(&conn);
            last_tick = Instant::now();
        }
    }

    // ── Restore terminal ──
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}
