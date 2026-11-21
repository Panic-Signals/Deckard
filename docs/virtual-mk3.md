# The virtual MK3: the on-screen panel and the browser client

DECKARD can show an MK3 without the hardware: the same pads, buttons, knobs,
strip, both screens and lights, run by the same controller layer as the
hardware. Two presentations share one layout table and one script run:

- the **on-screen panel**, docked under the main window or in its own window;
- the **browser client**, a page served over the local network for a tablet.

Both are sinks in the same seam (`apps/viewer/src/mk3_sink.rs`): the hardware,
the panel and every browser client get the same frame, and their input merges
un-deduped — two devices pressing the same pad is two presses.

## The on-screen panel

Controller ▸ On-screen MK3 ▸ Docked shows the panel in a column at the right of
the main window, next to the script controller's column; Separate Window puts it in
its own window; Hidden closes it. `--mk3-panel`
opens it at start-up. The window remembers its size and position across
restarts; closing it hides the panel, so a restart honours that.

Controller ▸ On-screen MK3 ▸ Portrait turns the surface 90° clockwise, so a
vertical touch monitor becomes a bigger MK3 (1280 × 800 becomes 800 × 1280).
Velocity comes from the touch; pressure if the screen reports it.

## Serving the page

`deckard project.mxprj --serve 8420` starts the server. The page, its script
and stylesheet, and the geometry table are embedded in the binary — no files
to install. Open the URL from the status bar on the tablet (see Pairing).

Plain HTTP on the same listener serves the page: `GET /` the page,
`GET /mk3.css` its stylesheet, `GET /mk3.js` its script,
`GET /layout.json` the geometry table, `GET /ws` the socket upgrade.
Anything else is 404, including `GET /ws` without upgrade headers. A phone held
upright gets a letterboxed surface that says to turn it sideways.
## Pairing

Every start mints a fresh token, shown in the status bar
(`serving the browser client on 8420 ?t=…`) and in
Preferences ▸ Hardware ▸ Controller, beside the port — copy the URL with it.
Open `http://<mac>.local:8420/?t=<token>`: the page forwards the token on the
socket's own URL (`/ws?t=<token>`) and nowhere else — the table fetch stays
bare, the served HTML never contains it, and it never reaches the logs.

An upgrade without the token (or with the wrong one) gets a 403 and a closed
connection, not a hanging socket. After three straight failed connects the page
says the pairing token in the URL may be wrong. The token check is a plain
equality check against a 128-bit secret, enough for a LAN.

Preferences ▸ Hardware ▸ Controller ▸ Require a pairing token switches
pairing off (takes effect on restart). Pairing off forces loopback: the
server listens on 127.0.0.1 only. A non-loopback listen never exists without
the token — asking for one without it is an error, not a silent LAN.

## The protocol

The page carries no geometry: every rectangle, id and kind comes from
`/layout.json` (`{width, height, controls: [{id, kind, rect, label, …}]`),
every colour from a light message.

**Output** (app → client), one binary WebSocket message per change:

```text
[u32 LE count][u8 kind][payload]
```

- `kind = 1`: `payload` is the PNG of the 960 × 272 screens (both screens
  side by side; each canvas takes the half matching its table order).
- `kind = 2`: `payload` is JSON lights: one object mapping each lit
  control's id to its `[r, g, b]`, off controls absent.

`count` is the screens' change counter, carried by both kinds, so a client
tells a repeated frame from a new one and paints only what moved. A client
joining between redraws gets the latest frame of each kind at once, never a
blank page. A slow client is a dropped frame, never a stalled app.

**Input** (client → app), one JSON message per contact, decoded through that
client's own panel — two clients never share gesture state:

```json
{"type": "down", "finger": 0, "x": 123.4, "y": 56.7, "p": 0.5}
{"type": "move", "finger": 0, "x": 123.4, "y": 60.0}
{"type": "up", "finger": 0}
{"type": "wheel", "steps": 2}
```

Coordinates are in the table's surface space; `p` (pressure, 0–1) is optional
and rides as the pad's velocity. Unknown types are ignored, never a
disconnect. A closed socket releases whatever it still held — a tab closed
mid-pad leaves no stuck note and no stuck light.

## Deliberately not there

- **Layout editor**: the page mimics the MK3; custom pages (XY pads, fader
  banks) are future work.
- **Phone portrait**: the page targets a tablet in landscape. Portrait is the
  on-screen panel's own window, not the page.
- **App-wide multi-touch**: multi-touch lives on the MK3 surface (the panel
  and the page); the rest of the UI is single-pointer.
- **Real hardware**: the visuals have not yet been checked against a real
  MK3 on the same project.

The Jam's twin of all this — its panel, its page and the one surface behind both — is
[docs/virtual-jam.md](virtual-jam.md).
