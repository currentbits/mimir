---
name: Chart check
description: Use when the person asks how a stock, currency pair or coin looks on the chart: trend, levels, indicators. Not for advice on what to buy, and never for placing trades.
---
**When to use:** "How does NVDA look?", "levels for BTC?", "is EUR/USD trending?", "check gold's chart".

**You'll need:** Recent price data: daily and 4-hour candles (open, high, low, close, volume) from a public source you can read with web_fetch, such as a crypto exchange's public market-data endpoint or a quotes page; web_search to find one. memory_search for what they hold or watch.

**Steps:**
1. Pin down the instrument and its market (which exchange, which pair, which currency). Get at least 60 daily and 60 four-hour candles. Note the time of the last candle.
2. Levels: the pivot from yesterday's high, low and close, with two supports and two resistances; then the swing highs and lows the price turned at more than once. Strength: how often price respected the level.
3. Indicators, on the daily and then the 4-hour: 9 and 21 EMA (which is on top, did they just cross); RSI 14 (above 70 or below 30, and is it diverging from price); MACD (above or below zero, and the direction of the histogram); Bollinger Bands 20/2 (narrowing, or price outside); ATR 14 for normal daily range; volume against its 20-day average.
4. Regime: trending up, trending down, ranging, or volatile. In a range, RSI extremes mean more; in a trend, pullbacks to the EMAs do.
5. Score each indicator from −2 to +2 and add them up: a lean (bullish, bearish, neutral) and how strong. Before calling it, run the false-signal checks: do the timeframes agree, is volume behind the move, is a big event due (earnings, central bank, a weekend for crypto), is price at a level right now.
6. Risk, if they ask about a setup: a stop beyond the nearest level by about one ATR, and reward against risk to the next two levels.

**Check:** Every number comes from the candles you read (say where from and as of when); calculations you approximated are marked; the lean matches the scores.

**Return:** One line on the lean and how strong, then levels, indicators and regime in a short table, the risk numbers if asked, and what would change the picture. End with: a read of the chart, not advice.

**Stop and ask if:** You can't get real candles (don't estimate prices), the symbol is ambiguous, or they ask you to buy, sell or move money: you never trade or touch an account.
