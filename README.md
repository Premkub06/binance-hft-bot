# 🚀 Binance HFT Bot — Altcoin Breakout Strategy

ระบบเทรดอัตโนมัติ Ultra-Low Latency สำหรับ Binance Futures
สร้างด้วยภาษา Rust เพื่อประสิทธิภาพสูงสุด

---

## 📋 สารบัญ

1. [ข้อกำหนดเบื้องต้น](#ข้อกำหนดเบื้องต้น)
2. [การตั้งค่า Environment](#การตั้งค่า-environment)
3. [การคอมไพล์สำหรับ Production](#การคอมไพล์สำหรับ-production)
4. [การ Deploy บน VPS](#การ-deploy-บน-vps)
5. [สถาปัตยกรรมระบบ](#สถาปัตยกรรมระบบ)
6. [คำเตือนสำคัญ](#คำเตือนสำคัญ)

---

## ข้อกำหนดเบื้องต้น

### ซอฟต์แวร์ที่ต้องติดตั้ง

```bash
# ติดตั้ง Rust toolchain (rustup)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# ตรวจสอบเวอร์ชัน
rustc --version    # ต้องเป็น 1.75.0 ขึ้นไป
cargo --version

# ติดตั้ง dependencies สำหรับ Linux (Ubuntu/Debian)
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev

# สำหรับ CentOS/RHEL
# sudo yum groupinstall "Development Tools"
# sudo yum install openssl-devel
```

### ข้อกำหนดฮาร์ดแวร์ขั้นต่ำ (VPS)

| รายการ | ขั้นต่ำ | แนะนำ |
|--------|---------|-------|
| CPU | 2 vCPU | 4 vCPU |
| RAM | 2 GB | 4 GB |
| ดิสก์ | 10 GB SSD | 20 GB NVMe |
| ตำแหน่ง | โตเกียว (ap-northeast-1) | ใกล้ Binance server มากที่สุด |
| OS | Ubuntu 22.04 LTS | Ubuntu 24.04 LTS |

---

## การตั้งค่า Environment

### ขั้นตอนที่ 1: คัดลอกไฟล์ตัวอย่าง

```bash
cp .env.example .env
```

### ขั้นตอนที่ 2: แก้ไขไฟล์ `.env`

```bash
nano .env
```

ใส่ค่าต่อไปนี้:

```env
# ── API Key ของ Binance Futures ──────────────────────────────
# สร้างได้ที่: https://www.binance.com/en/my/settings/api-management
# ⚠️ เปิดสิทธิ์ Futures Trading เท่านั้น ห้ามเปิดสิทธิ์ Withdrawal!
BINANCE_API_KEY=ใส่_api_key_ของคุณ_ตรงนี้
BINANCE_API_SECRET=ใส่_api_secret_ของคุณ_ตรงนี้

# ── Endpoint ─────────────────────────────────────────────────
# สำหรับทดสอบ (Testnet) — แนะนำให้ใช้ก่อนเสมอ!
BINANCE_BASE_URL=https://testnet.binancefuture.com
BINANCE_WS_URL=wss://stream.binancefuture.com

# สำหรับเทรดจริง (Production) — ใช้เมื่อพร้อมแล้วเท่านั้น!
# BINANCE_BASE_URL=https://fapi.binance.com
# BINANCE_WS_URL=wss://fstream.binance.com

# ── ฐานข้อมูล ────────────────────────────────────────────────
DB_PATH=hft_bot.db

# ── ระดับ Log ────────────────────────────────────────────────
# debug = แสดงทุกอย่าง (สำหรับ debug)
# info  = แสดงเฉพาะข้อมูลสำคัญ (สำหรับ production)
# warn  = แสดงเฉพาะคำเตือน
RUST_LOG=binance_hft_bot=info
```

### ขั้นตอนที่ 3: ตรวจสอบ API Key

> ⚠️ **สำคัญมาก**: ตรวจสอบให้แน่ใจว่า:
> - API Key มีสิทธิ์ **Enable Futures** แล้ว
> - **ไม่ได้** เปิดสิทธิ์ Enable Withdrawals
> - ตั้ง IP Restriction ให้เฉพาะ IP ของ VPS เท่านั้น

---

## การคอมไพล์สำหรับ Production

### วิธีที่ 1: คอมไพล์บน VPS โดยตรง (แนะนำ)

```bash
# เข้าไปที่โฟลเดอร์โปรเจค
cd binance-hft-bot

# คอมไพล์แบบ Release พร้อมออปติไมซ์สำหรับ CPU ของเครื่อง
# flag นี้จะเปิดใช้ instruction set เฉพาะของ CPU (เช่น AVX2, SSE4)
RUSTFLAGS="-C target-cpu=native" cargo build --release

# ไฟล์ binary จะอยู่ที่:
# target/release/binance-hft-bot
```

### คำอธิบาย Release Optimizations

| การตั้งค่า | ค่า | คำอธิบาย |
|-----------|-----|----------|
| `opt-level` | `3` | ออปติไมซ์ความเร็วสูงสุด |
| `lto` | `"fat"` | Link-Time Optimization ข้ามทุก crate — ลดขนาด binary และเพิ่มความเร็ว |
| `codegen-units` | `1` | คอมไพล์เป็นหน่วยเดียว — ช่วยให้ LLVM ออปติไมซ์ได้ดีขึ้น |
| `panic` | `"abort"` | ยกเลิกทันทีเมื่อเกิด panic — ไม่ต้อง unwind stack |
| `strip` | `true` | ลบ debug symbols — ลดขนาดไฟล์ |
| `target-cpu=native` | RUSTFLAGS | ใช้ instruction set เฉพาะของ CPU ที่กำลังคอมไพล์ |

### วิธีที่ 2: Cross-Compile จากเครื่องอื่น

```bash
# ติดตั้ง target สำหรับ Linux x86_64
rustup target add x86_64-unknown-linux-gnu

# คอมไพล์ (ไม่ใช้ target-cpu=native เพราะ CPU อาจต่างกัน)
cargo build --release --target x86_64-unknown-linux-gnu

# คัดลอกไฟล์ไปยัง VPS
scp target/x86_64-unknown-linux-gnu/release/binance-hft-bot user@vps-ip:/opt/hft-bot/
```

---

## การ Deploy บน VPS

### ขั้นตอนที่ 1: เตรียมโฟลเดอร์

```bash
# สร้างโฟลเดอร์สำหรับ bot
sudo mkdir -p /opt/hft-bot
sudo chown $USER:$USER /opt/hft-bot

# คัดลอกไฟล์ที่จำเป็น
cp target/release/binance-hft-bot /opt/hft-bot/
cp .env /opt/hft-bot/
```

### ขั้นตอนที่ 2: สร้าง systemd service (ให้รันอัตโนมัติ)

```bash
sudo tee /etc/systemd/system/hft-bot.service << 'EOF'
[Unit]
Description=Binance HFT Altcoin Breakout Bot
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=hftbot
Group=hftbot
WorkingDirectory=/opt/hft-bot
ExecStart=/opt/hft-bot/binance-hft-bot
Restart=always
RestartSec=10

# ตั้งค่าความปลอดภัย
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/opt/hft-bot

# ตั้งค่า environment
EnvironmentFile=/opt/hft-bot/.env

# จำกัด resource
LimitNOFILE=65536
LimitNPROC=4096

[Install]
WantedBy=multi-user.target
EOF
```

### ขั้นตอนที่ 3: สร้าง user สำหรับรัน bot

```bash
sudo useradd -r -s /bin/false hftbot
sudo chown -R hftbot:hftbot /opt/hft-bot
```

### ขั้นตอนที่ 4: เริ่มและเปิดใช้งาน service

```bash
# โหลด service ใหม่
sudo systemctl daemon-reload

# เริ่ม bot
sudo systemctl start hft-bot

# ตั้งค่าให้เริ่มอัตโนมัติเมื่อ reboot
sudo systemctl enable hft-bot

# ดู status
sudo systemctl status hft-bot

# ดู log แบบ real-time
sudo journalctl -u hft-bot -f
```

### ขั้นตอนที่ 5: ปรับแต่ง Network (สำคัญสำหรับ Low Latency)

```bash
# ปรับ TCP tuning สำหรับ low latency
sudo tee -a /etc/sysctl.conf << 'EOF'

# === HFT Network Tuning ===
net.core.rmem_max = 16777216
net.core.wmem_max = 16777216
net.ipv4.tcp_rmem = 4096 87380 16777216
net.ipv4.tcp_wmem = 4096 65536 16777216
net.ipv4.tcp_nodelay = 1
net.ipv4.tcp_low_latency = 1
net.core.netdev_max_backlog = 5000
EOF

# ใช้การตั้งค่าใหม่
sudo sysctl -p
```

---

## สถาปัตยกรรมระบบ

```
┌─────────────────────────────────────────────────────────┐
│                    Main Thread (Tokio)                   │
├─────────────┬──────────────┬────────────┬───────────────┤
│  Task 1     │   Task 2     │  Task 3    │   Task 4      │
│  Kline WS   │  MarkPrice   │  Risk      │   Status      │
│  (Strategy) │  WS (Risk)   │  Sweep     │   Reporter    │
├─────────────┴──────────────┴────────────┴───────────────┤
│              DashMap (Lock-Free State)                   │
│  ┌─────────────────────┐ ┌────────────────────────┐     │
│  │ MarketState         │ │ PositionMap            │     │
│  │ • previous_day_high │ │ • entry_price          │     │
│  │ • current_price     │ │ • quantity             │     │
│  │ • volume_15m        │ │ • max_roe              │     │
│  │ • avg_volume_7d     │ │ • trailing_active      │     │
│  └─────────────────────┘ └────────────────────────┘     │
├─────────────────────────────────────────────────────────┤
│           crossbeam channel (fire & forget)              │
├─────────────────────────────────────────────────────────┤
│         OS Thread: SQLite Writer (WAL Mode)             │
│         • ไม่ block tokio runtime เด็ดขาด                │
└─────────────────────────────────────────────────────────┘
```

### กลยุทธ์ Breakout

| พารามิเตอร์ | ค่า | คำอธิบาย |
|-------------|-----|----------|
| เงื่อนไข 1 | ราคา > High ของวันก่อน | สัญญาณ breakout ระดับราคา |
| เงื่อนไข 2 | Volume 15 นาที > 3× ค่าเฉลี่ย 7 วัน | ยืนยันด้วยปริมาณซื้อขาย |
| Margin | $6 USD | ทุนต่อ 1 ออเดอร์ |
| Leverage | 10x | Notional = $60 ต่อเทรด |
| Hard Stop | ROE ≤ -10% | ขาดทุนสูงสุด ≈ $0.60 ต่อเทรด |
| Trailing Activation | ROE ≥ +20% | เริ่มติดตามกำไร |
| Trailing Stop | กำไรลดลง 5% จากจุดสูงสุด | ล็อกกำไร |

---

## คำเตือนสำคัญ

> ⚠️ **คำเตือน: ความเสี่ยงทางการเงิน**
>
> บอทนี้เทรดด้วยเงินจริง การใช้ Leverage 10x หมายความว่า:
> - ขาดทุนสูงสุดต่อเทรด: ≈ $0.60 (ที่ -10% ROE hard stop)
> - กำไรที่เป็นไปได้: ไม่จำกัด (trailing stop ปกป้องกำไร)
> - **เทสด้วย Testnet ก่อนเสมอ!**

> 🔒 **ความปลอดภัย**
>
> - อย่าเก็บ API Key ใน source code เด็ดขาด
> - ใช้ IP Restriction สำหรับ API Key
> - ไม่ต้องเปิดสิทธิ์ Withdrawal
> - ตั้ง file permission: `chmod 600 .env`

### คำสั่งที่มีประโยชน์

```bash
# ดู log ย้อนหลัง 100 บรรทัด
sudo journalctl -u hft-bot -n 100

# หยุด bot
sudo systemctl stop hft-bot

# restart bot
sudo systemctl restart hft-bot

# ดูข้อมูลในฐานข้อมูล
sqlite3 /opt/hft-bot/hft_bot.db "SELECT * FROM trades ORDER BY id DESC LIMIT 20;"

# ดูสรุป P&L
sqlite3 /opt/hft-bot/hft_bot.db "SELECT COUNT(*) as total, SUM(pnl_usd) as total_pnl, AVG(roe_pct) as avg_roe FROM trades WHERE status='CLOSED';"
```

---

**สร้างด้วย 🦀 Rust เพื่อความเร็วระดับ Nanosecond**
