import requests
import csv

# ตั้งค่าเหรียญและ Timeframe
symbol = "ETHUSDT"  # เปลี่ยนเป็นเหรียญซิ่งๆ ที่อยากเทสต์
interval = "15m"    # แท่ง 15 นาที
limit = 1500      # จำนวนแท่งย้อนหลัง (Binance ให้สูงสุด 1500 ต่อ 1 Request)

print(f"กำลังดึงข้อมูล {symbol}...")
url = f"https://fapi.binance.com/fapi/v1/klines?symbol={symbol}&interval={interval}&limit={limit}"
response = requests.get(url)
data = response.json()

# บันทึกลงไฟล์ CSV ตามฟอร์แมตที่บอท Rust ต้องการ
filename = "data.csv"
with open(filename, mode='w', newline='') as file:
    writer = csv.writer(file)
    writer.writerow(['timestamp', 'open', 'high', 'low', 'close', 'volume'])
    
    for row in data:
        # ดึงเฉพาะคอลัมน์ที่จำเป็น (0=เวลา, 1=Open, 2=High, 3=Low, 4=Close, 5=Volume)
        writer.writerow([row[0], row[1], row[2], row[3], row[4], row[5]])

print(f"✅ บันทึกข้อมูล {len(data)} แท่งลงไฟล์ {filename} เรียบร้อยแล้ว!")