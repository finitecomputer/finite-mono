---
name: trading-agent-finite
description: Analyze stocks, crypto, macro, and event-driven markets with the shared Finite runtime using yfinance, CCXT, FRED, Plotly charting, and Finite-managed Polymarket and Perplexity helpers.
tags: [finance, trading, charts, macro, crypto, stocks, polymarket]
---

# Trading Agent Finite

Use this when the user wants market analysis, charting, macro overlays, or trading-oriented research.

The Finite runtime already includes the core Python stack needed for this workflow:
- `plotly` + `kaleido` for chart rendering
- `pandas`
- `yfinance`
- `ccxt`
- `fredapi`
- `Pillow`

Do not export a manual `LD_LIBRARY_PATH` on current Finite runtimes. The platform now carries the needed native library path.

## Environment

Activate the Hermes venv before running Python snippets:

```bash
source ~/.hermes/venv/bin/activate
```

## Source Selection

Use the lightest reliable source for the question:

| Question | Best source |
|---|---|
| Quick stock / ETF / crypto OHLCV | `yfinance` |
| Exchange-specific crypto OHLCV or orderbook | `ccxt` |
| Macro series: rates, CPI, GDP, unemployment, yield curve | `fredapi` with `FRED_API_KEY` |
| Event probabilities / market sentiment / valuation odds | `polymarket-finite` |
| Private company valuation or latest funding rounds | `perplexity-research-finite` |

Notes:
- `FRED_API_KEY` is optional but preferred for macro work.
- Exchange API keys are per-user and should be requested only for authenticated trading actions.
- Do not rely on a shared Massive/Polygon key in the platform baseline.

## Workflow

1. Pick the right live source.
2. Pull the data with a short Python snippet in the Hermes venv.
3. If the user wants an image, build a Plotly chart and save it as `.jpg` under `/home/node/`.
4. If event odds or private-company valuation matter, augment with:
   - `polymarket-finite` for prediction-market probabilities
   - `perplexity-research-finite` for live funding and valuation research
5. When replying in Telegram, attach the chart with `MEDIA:/home/node/<name>.jpg`.

## Core Snippets

### yfinance OHLCV

```python
import yfinance as yf

def get_ohlcv(ticker, period="3mo", interval="1d"):
    df = yf.download(ticker, period=period, interval=interval, progress=False)
    df.columns = [c[0].lower() if isinstance(c, tuple) else c.lower() for c in df.columns]
    return df
```

### FRED macro data

```python
import os
from fredapi import Fred

fred = Fred(api_key=os.environ["FRED_API_KEY"])
series = fred.get_series("FEDFUNDS").tail(24)
```

Useful series:
- `FEDFUNDS`
- `CPIAUCSL`
- `CPILFESL`
- `UNRATE`
- `GDP`
- `PCE`
- `GS10`
- `GS2`
- `T10Y2Y`
- `VIXCLS`
- `M2SL`
- `DTWEXBGS`

### CCXT OHLCV / orderbook

```python
import ccxt
import pandas as pd

exchange = ccxt.binance({"enableRateLimit": True})
bars = exchange.fetch_ohlcv("BTC/USDT", timeframe="1d", limit=200)
df = pd.DataFrame(bars, columns=["timestamp", "open", "high", "low", "close", "volume"])
df["timestamp"] = pd.to_datetime(df["timestamp"], unit="ms")
df.set_index("timestamp", inplace=True)

book = exchange.fetch_order_book("BTC/USDT")
```

## Charting

Use a dark Plotly default and export charts as JPEG:

```python
import plotly.graph_objects as go
from plotly.subplots import make_subplots
from PIL import Image

CHART_DEFAULTS = dict(
    template="plotly_dark",
    width=1200,
    height=650,
    paper_bgcolor="#1a1a2e",
    plot_bgcolor="#16213e",
    font=dict(color="#eaeaea", size=13),
)

def save_chart(fig, path="/home/node/chart.jpg"):
    fig.update_layout(**CHART_DEFAULTS)
    png_path = path.replace(".jpg", ".png")
    fig.write_image(png_path)
    Image.open(png_path).convert("RGB").save(path, "JPEG", quality=95)
    return path
```

Candlestick + volume:

```python
fig = make_subplots(rows=2, cols=1, shared_xaxes=True, row_heights=[0.75, 0.25], vertical_spacing=0.02)
fig.add_trace(go.Candlestick(
    x=df.index,
    open=df["open"],
    high=df["high"],
    low=df["low"],
    close=df["close"],
    increasing_line_color="#26a69a",
    decreasing_line_color="#ef5350",
    name="Price",
), row=1, col=1)
fig.add_trace(go.Bar(
    x=df.index,
    y=df["volume"],
    marker_color=["#26a69a" if c >= o else "#ef5350" for c, o in zip(df["close"], df["open"])],
    name="Volume",
), row=2, col=1)
fig.update_layout(xaxis_rangeslider_visible=False)
```

## Finite Helper Integrations

### Prediction-market overlay

Use [polymarket-finite](../research/polymarket-finite/SKILL.md) instead of ad hoc Polymarket requests.

Examples:

```bash
python3 /profile-assets/hermes-local/managed-skills/research/polymarket-finite/scripts/polymarket.py search \
  --query "OpenAI IPO" \
  --limit 5

python3 /profile-assets/hermes-local/managed-skills/research/polymarket-finite/scripts/polymarket.py market \
  --slug openai-1t-ipo-before-2027
```

### Private-company valuation research

Use [perplexity-research-finite](../research/perplexity-research-finite/SKILL.md) for fast live valuation and funding context.

```bash
python3 /profile-assets/hermes-local/managed-skills/research/perplexity-research-finite/scripts/perplexity_research.py search \
  --query "OpenAI Anthropic xAI valuation funding round 2026" \
  --recency month \
  --max-results 5
```

## Delivery Rules

- Save Telegram-targeted charts as `.jpg`, not `.png`.
- Use descriptive filenames such as:
  - `/home/node/btc-daily-chart.jpg`
  - `/home/node/openai-ipo-odds.jpg`
  - `/home/node/macro-dashboard.jpg`
- Include source links after the chart when possible.
- Do not present stale model memory as live market truth.

## Pitfalls

- `yfinance` may return multi-index columns; flatten them immediately.
- `ccxt` should always be initialized with `enableRateLimit=True`.
- `FRED_API_KEY` may be absent on some boxes; if so, say macro data is unavailable until configured.
- Thin Polymarket markets are useful as sentiment, not as exact truth.
- Private-company valuation claims should come from live research, not memory.
