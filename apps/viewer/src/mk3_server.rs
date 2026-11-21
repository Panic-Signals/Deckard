//! The browser client's server: one more sink in the seam, enabled only when
//! `--serve` is set. A thread owns the listener and every socket: non-blocking
//! accepts, polled reads, closed or errored sockets removed. Frames travel out
//! as binary WebSocket messages, pointer and wheel contact comes back as JSON
//! and decodes through that client's own [`Panel`](crate::onscreen_mk3::Panel):
//! one panel per client, so two clients never share gesture state, and a
//! closed socket releases what it held into the server's event queue.
//!
//! Wire output (app → client), binary:
//! `[u32 LE count][u8 kind=1][PNG bytes]` for a screen frame,
//! `[u32 LE count][u8 kind=2][JSON lights]` for a light change. The count is
//! always `Mk3Out`'s screen counter, so a client tells a repeated frame from
//! a new one. Wire input (client → app), JSON, one message per client event:
//! `{"type":"down"|"up"|"move","finger":N,"x":F,"y":F,"p":F?}` and
//! `{"type":"wheel","steps":N}`. Unknown types are ignored, never a disconnect.
//!
//! `show` runs on the frame thread: it copies the screens only when the change
//! counter moved, and the PNG encoding happens on the server thread. The
//! outbox is bounded (`sync_channel(4)`): when it is full the frame is
//! dropped — a slow client is a dropped frame, never a stalled app.
//!
//! The same listener also serves the page itself over plain HTTP, told apart
//! from a client by peeking for the upgrade headers (the peek consumes
//! nothing, so the handshake replays nothing): `GET /` the page,
//! `GET /mk3.css` and `GET /mk3.js` its assets (embedded with `include_str!`,
//! the way the surface ships its Lua), `GET /layout.json` the geometry table,
//! `GET /ws` the upgrade. Anything else is 404. With pairing on, the upgrade
//! carries `?t=<token>` and anything without it gets a 403 and a closed
//! connection, never a hanging one.

use crate::Mk3Out;
use crate::mk3_sink::Mk3Sink;
use crate::onscreen_mk3::Panel;
use deckard_mk3::Event;
use deckard_mk3::leds::Leds;
use std::collections::HashMap;
use std::io;
use std::io::{Read as _, Write as _};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::{Duration, Instant};

/// The screens' size, as [`Mk3Out`] documents: one 960 × 272 RGBA image.
const SCREEN_W: u32 = 960;
/// The screens' height, as [`Mk3Out`] documents.
const SCREEN_H: u32 = 272;
/// A screen frame's kind byte on the wire.
const KIND_SCREEN: u8 = 1;
/// A light change's kind byte on the wire.
const KIND_LIGHTS: u8 = 2;
/// Frames the server thread may lag behind by before `show` drops instead of
/// waiting: a slow client is a dropped frame, not a stalled app.
const OUTBOX_CAP: usize = 4;
/// How long a connect may take to finish its handshake before it is dropped.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Handshakes in flight past which a new connect is dropped instead of spawning:
/// each holds a short-lived thread, so a scan of the LAN port must not grow them
/// without bound.
const MAX_PENDING_HANDSHAKES: usize = 32;
/// How long a socket may sit idle before the thread pings it, so a vanished
/// client is noticed instead of held onto.
const PING_IDLE: Duration = Duration::from_secs(20);
/// The poll cadence: accepts, the outbox and every socket, then a breath so
/// the thread never spins hot. Input latency stays under one frame either way.
const POLL_PAUSE: Duration = Duration::from_millis(5);
/// The page the tablet touches, embedded the way the surface ships its Lua:
/// no filesystem reads at runtime, so the binary serves the whole client.
const INDEX_HTML: &str = include_str!("../assets/mk3/index.html");
/// The page's stylesheet: dark chrome only, no rectangles (see `mk3.css`).
const MK3_CSS: &str = include_str!("../assets/mk3/mk3.css");
/// The page's script: every control comes from `/layout.json` (see `mk3.js`).
const MK3_JS: &str = include_str!("../assets/mk3/mk3.js");

/// One frame's worth of work for the server thread: the LEDs when they
/// changed, the screens when the counter moved, and the screen counter the
/// light message carries even when the screens did not move.
struct ToServer {
    leds: Option<Leds>,
    screens: Option<(u64, Vec<[u8; 4]>)>,
    count: u64,
}

/// The browser client's server: a sink that is enabled only when `serve` is
/// set. `show` never blocks; the thread encodes and broadcasts. Pairing rides
/// on the upgrade's `?t=` query (see [`Server::start_paired`]); the bind rule
/// is structural — a non-loopback listen needs the token, so switching the
/// token off forces loopback.
pub struct Server {
    tx: SyncSender<ToServer>,
    inbox: Arc<Mutex<Vec<Event>>>,
    last_leds: Option<Leds>,
    last_screen: Option<u64>,
    /// The pairing token upgrades must carry, if pairing is on.
    token: Option<String>,
    /// What the listener bound: loopback without a token, all interfaces with one.
    addr: SocketAddr,
}

impl Server {
    /// Bind loopback, no pairing token: the offline and test path. Upgrades need
    /// no `?t=`, and the server never listens beyond this machine.
    pub fn start(port: Option<u16>) -> io::Result<(Server, u16)> {
        Self::start_on("127.0.0.1", port, None)
    }

    /// Bind all interfaces with a fresh pairing token: the tablet-on-the-LAN
    /// path. Upgrades carry `?t=<token>`; anything else gets a 403 and a close.
    pub fn start_paired(port: Option<u16>) -> io::Result<(Server, u16)> {
        Self::start_on("0.0.0.0", port, Some(generate_token()))
    }

    /// Bind `addr`, requiring `token` for every non-loopback listen: a tokenless
    /// LAN bind is an error, never "LAN and open". There is no other shape to
    /// ask for, so switching the token off forces loopback structurally.
    pub fn start_on(addr: &str, port: Option<u16>, token: Option<String>) -> io::Result<(Server, u16)> {
        let ip: IpAddr =
            addr.parse().map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("a bad bind address {addr}: {e}")))?;
        if token.is_none() && !ip.is_loopback() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a pairing token is required to serve beyond loopback"));
        }
        let listener = TcpListener::bind((addr, port.unwrap_or(0)))?;
        listener.set_nonblocking(true)?;
        let local = listener.local_addr()?;
        let bound = local.port();
        let (tx, rx) = sync_channel::<ToServer>(OUTBOX_CAP);
        let inbox: Arc<Mutex<Vec<Event>>> = Arc::default();
        let serving = Arc::clone(&inbox);
        let thread_token = token.clone();
        std::thread::Builder::new().name("mk3-server".into()).spawn(move || run(listener, rx, serving, thread_token))?;
        Ok((Server { tx, inbox, last_leds: None, last_screen: None, token, addr: local }, bound))
    }

    /// The pairing token upgrades must carry, if pairing is on.
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// What the listener bound.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
}

/// One server's pairing secret: 128 bits from two independently keyed hashes
/// over the process, a counter and the clock. No new dependency for it: the
/// hasher's random per-process keys are what make consecutive starts differ.
/// A plain equality check against a secret this long is enough for a LAN token.
fn generate_token() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let salt = (
        std::process::id(),
        CTR.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|d| (d.as_secs(), d.subsec_nanos())),
    );
    let mut first = DefaultHasher::new();
    salt.hash(&mut first);
    let mut second = DefaultHasher::new();
    ("mk3-pairing", &salt).hash(&mut second);
    format!("{:016x}{:016x}", first.finish(), second.finish())
}

impl Mk3Sink for Server {
    /// Queue what changed for the server thread — the screens only when the
    /// counter moved — and return at once in every case, including a full
    /// channel: `try_send` drops instead of waiting.
    fn show(&mut self, out: &Mk3Out) {
        let screen_moved = out.screens.as_ref().is_some_and(|(n, _)| Some(*n) != self.last_screen);
        let leds_moved = Some(&out.leds) != self.last_leds.as_ref();
        if !screen_moved && !leds_moved {
            return;
        }
        if let Some((n, _)) = &out.screens {
            self.last_screen = Some(*n);
        }
        if leds_moved {
            self.last_leds = Some(out.leds.clone());
        }
        let msg = ToServer {
            leds: leds_moved.then(|| out.leds.clone()),
            screens: screen_moved.then(|| out.screens.as_ref().map(|(n, f)| (*n, f.clone().into_pixels()))).flatten(),
            count: self.last_screen.unwrap_or(0),
        };
        let _ = self.tx.try_send(msg);
    }

    /// Input the server thread decoded since the last frame.
    fn events(&mut self) -> Vec<Event> {
        std::mem::take(&mut *self.inbox.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Whether any control's colour LED is flashing now, as
    /// [`Host`](crate::onscreen_mk3::Host) reports it: the app repaints on the
    /// [`crate::mk3_lights`] cadence while one is, so the page never animates
    /// but never freezes on a flash either.
    fn wants_frames(&self) -> bool {
        let Some(leds) = &self.last_leds else { return false };
        crate::mk3_layout::controls().iter().any(|c| {
            let Some(name) = crate::mk3_layout::led(c.id) else { return false };
            let Some(byte) = leds.get(&name) else { return false };
            deckard_mk3::leds::is_colour(&name) && matches!(byte >> 2, 1..=17) && byte & 1 != 0
        })
    }
}

/// One browser client: its stable id (the key to its panel), the socket, and
/// when it last spoke, so idle ones get a ping. Each client keeps its own
/// panel entry, already tracked separately from the token check above.
struct Client {
    id: usize,
    ws: tungstenite::WebSocket<TcpStream>,
    active: Instant,
}

/// The server thread: accept, upgrade, register the retained frame, then loop
/// over accepts, the outbox and every socket. Nothing here touches the app
/// state: frames arrive through `rx`, input leaves through `inbox`. The token
/// is the pairing secret every upgrade must carry, if pairing is on.
fn run(listener: TcpListener, rx: Receiver<ToServer>, inbox: Arc<Mutex<Vec<Event>>>, token: Option<String>) {
    let mut panels: HashMap<usize, Panel> = HashMap::new();
    let mut next_id: usize = 0;
    let mut clients: Vec<Client> = Vec::new();
    // The latest of each kind, framed and ready: a client joining between
    // redraws gets the current picture at once, never a blank page.
    let mut screen: Option<(u64, Vec<u8>)> = None;
    let mut lights: Option<(u64, Vec<u8>)> = None;
    // Handshakes in flight: each new connect is classified on its own thread, so a
    // silent socket never holds the loop; an established socket comes back over
    // `ready` and joins non-blockingly with the retained frame.
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Option<tungstenite::WebSocket<TcpStream>>>();
    let mut pending: usize = 0;
    loop {
        accept_all(&listener, &ready_tx, &mut pending, &token);
        while let Ok(done) = ready_rx.try_recv() {
            pending = pending.saturating_sub(1);
            if let Some(ws) = done {
                admit(ws, &mut clients, &mut panels, &mut next_id, &screen, &lights);
            }
        }
        // Drain the outbox: encode off the frame thread, retain, broadcast.
        while let Ok(msg) = rx.try_recv() {
            if let Some((n, px)) = msg.screens
                && let Some(png) = encode_png(&px)
            {
                let framed = frame(n, KIND_SCREEN, &png);
                broadcast(&mut clients, &mut panels, &inbox, &framed);
                screen = Some((n, framed));
            }
            if let Some(leds) = msg.leds {
                let json = lights_json(&leds, Instant::now());
                let framed = frame(msg.count, KIND_LIGHTS, &json);
                broadcast(&mut clients, &mut panels, &inbox, &framed);
                lights = Some((msg.count, framed));
            }
        }
        poll_clients(&mut clients, &mut panels, &inbox);
        std::thread::sleep(POLL_PAUSE);
    }
}

/// Take every pending connect without blocking the loop: each new socket is
/// classified and handshook on its own short-lived thread (a silent connect or a
/// trickling head holds only its thread, up to `HANDSHAKE_TIMEOUT`), and an
/// established socket comes back over `ready` for the loop to admit. Past
/// `MAX_PENDING_HANDSHAKES` in flight, a new connect is dropped instead of
/// spawning: the loop never waits, whatever the LAN sends it.
fn accept_all(
    listener: &TcpListener,
    ready: &std::sync::mpsc::Sender<Option<tungstenite::WebSocket<TcpStream>>>,
    pending: &mut usize,
    token: &Option<String>,
) {
    loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
            Err(_) => return,
        };
        if *pending >= MAX_PENDING_HANDSHAKES {
            continue;
        }
        *pending += 1;
        let (tx, tok) = (ready.clone(), token.clone());
        if std::thread::Builder::new()
            .name("mk3-handshake".into())
            .spawn(move || {
                let _ = tx.send(handshake(stream, &tok));
            })
            .is_err()
        {
            *pending = pending.saturating_sub(1);
        }
    }
}

/// One connect's handshake, on its own thread: peek, classify (a page fetch vs.
/// an upgrade), serve or refuse HTTP directly, or complete the WebSocket upgrade.
/// `None` is a served page, a refused token, a timeout or a failed upgrade — in
/// every case the loop never waited for it.
fn handshake(stream: TcpStream, token: &Option<String>) -> Option<tungstenite::WebSocket<TcpStream>> {
    // Peek before touching: a browser page fetch is plain HTTP, a client
    // is a WebSocket upgrade, and the peek consumes nothing, so the
    // upgrade path hands the stream to tungstenite exactly as before.
    let head = peek_request(&stream)?;
    let wants_ws = is_upgrade(&head) && matches!(request_path(&head), Some("/") | Some("/ws"));
    if !wants_ws {
        serve_http(stream);
        return None;
    }
    // Pairing: the token rides the upgrade's `?t=` query. A refused client
    // gets a closed connection, not a hanging one.
    if !token_ok(token, request_target(&head)) {
        refuse_http(stream);
        return None;
    }
    // The handshake runs blocking (with a timeout); the polls stay non-blocking.
    stream.set_nonblocking(false).ok();
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).ok();
    tungstenite::accept(stream).ok()
}

/// Admit an established socket, non-blockingly, with the retained frame first so
/// a client joining between redraws paints at once. A slow join is a dropped
/// retained frame, never a stall.
fn admit(
    ws: tungstenite::WebSocket<TcpStream>,
    clients: &mut Vec<Client>,
    panels: &mut HashMap<usize, Panel>,
    next_id: &mut usize,
    screen: &Option<(u64, Vec<u8>)>,
    lights: &Option<(u64, Vec<u8>)>,
) {
    let mut client = Client { id: *next_id, ws, active: Instant::now() };
    client.ws.get_mut().set_read_timeout(None).ok();
    if client.ws.get_mut().set_nonblocking(true).is_err() {
        return;
    }
    let mut alive = true;
    if let Some((_, framed)) = screen {
        alive &= push(&mut client.ws, framed);
    }
    if alive && let Some((_, framed)) = lights {
        alive &= push(&mut client.ws, framed);
    }
    if alive {
        let id = client.id;
        clients.push(client);
        panels.insert(id, Panel::default());
        *next_id += 1;
    }
}

/// Peek at a fresh connect without consuming a byte: `Some` carries the full
/// HTTP head the client already sent, `None` when nothing arrived before the
/// handshake timeout or the socket is already gone. The stream stays
/// untouched, so the upgrade path replays nothing.
fn peek_request(stream: &TcpStream) -> Option<Vec<u8>> {
    let start = Instant::now();
    let mut buf = vec![0u8; 8192];
    loop {
        match stream.peek(&mut buf) {
            Ok(n) => {
                if n > 0 && (buf[..n].windows(4).any(|w| w == b"\r\n\r\n") || n == buf.len()) {
                    return Some(buf[..n].to_vec());
                }
                if n == 0 || start.elapsed() > HANDSHAKE_TIMEOUT {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if start.elapsed() > HANDSHAKE_TIMEOUT {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
}

/// Whether the peeked head carries a WebSocket upgrade: the page's plain
/// fetches never do, so this tells `GET /` the page apart from `GET /` the
/// socket. Case-insensitive; tungstenite validates for real on the upgrade path.
fn is_upgrade(head: &[u8]) -> bool {
    let lower = head.to_ascii_lowercase();
    lower.windows(7).any(|w| w == b"upgrade") && lower.windows(9).any(|w| w == b"websocket")
}

/// The request target of the peeked head's request line (`/mk3.js?x=1` reads
/// as `/mk3.js`): `None` when the head is not a `GET` request line at all,
/// which answers the same as an unknown path — 404.
fn request_path(head: &[u8]) -> Option<&str> {
    let line = head.split(|b| *b == b'\n').next()?;
    let line = std::str::from_utf8(line).ok()?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let target = parts.next()?;
    Some(target.split('?').next().unwrap_or(target))
}

/// The request target with its query kept (`/ws?t=abc` whole): the upgrade's
/// pairing token rides there. `None` when the head is not a `GET` line at all.
fn request_target(head: &[u8]) -> Option<&str> {
    let line = head.split(|b| *b == b'\n').next()?;
    let line = std::str::from_utf8(line).ok()?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    parts.next()
}

/// Whether the upgrade may proceed: no token configured accepts everything
/// (pairing is off); otherwise the `t` query must equal it exactly. A plain
/// equality check is enough for a LAN token of this length.
fn token_ok(expected: &Option<String>, target: Option<&str>) -> bool {
    let Some(want) = expected else { return true };
    let Some(target) = target else { return false };
    query_token(target).is_some_and(|got| got == *want)
}

/// The `t` query of a request target (`/ws?t=abc` reads `abc`): `None` when
/// absent or empty.
fn query_token(target: &str) -> Option<&str> {
    target.split_once('?')?.1.split('&').find_map(|p| p.strip_prefix("t=").filter(|v| !v.is_empty()))
}

/// Refuse an upgrade without the pairing token: 403, then close. Never a hang.
/// The token itself never appears in the answer.
fn refuse_http(stream: TcpStream) {
    let mut stream = stream;
    stream.set_nonblocking(false).ok();
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).ok();
    // The peek consumed nothing, so drain the head before answering: unread
    // bytes on a closing socket risk a RST.
    let mut seen = Vec::new();
    let mut chunk = [0u8; 1024];
    while !seen.windows(4).any(|w| w == b"\r\n\r\n") && seen.len() <= 16384 {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => seen.extend_from_slice(&chunk[..n]),
            Err(_) => return,
        }
    }
    let body = b"pairing token required";
    let head = format!(
        "HTTP/1.1 403 Forbidden\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}
/// Answer one plain-HTTP fetch and close: the page, its two assets, the
/// geometry table, and 404 for everything else — including `GET /ws` without
/// upgrade headers, which is a fetch, not a socket.
fn serve_http(stream: TcpStream) {
    let mut stream = stream;
    stream.set_nonblocking(false).ok();
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).ok();
    // The peek consumed nothing, so drain the head this side already saw,
    // then answer and close: unread bytes on a closing socket risk a RST.
    let mut seen = Vec::new();
    let mut chunk = [0u8; 1024];
    while !seen.windows(4).any(|w| w == b"\r\n\r\n") && seen.len() <= 16384 {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => seen.extend_from_slice(&chunk[..n]),
            Err(_) => return,
        }
    }
    let (status, mime, body): (&str, &str, Vec<u8>) = match request_path(&seen) {
        Some("/") => ("200 OK", "text/html; charset=utf-8", INDEX_HTML.as_bytes().to_vec()),
        Some("/mk3.css") => ("200 OK", "text/css; charset=utf-8", MK3_CSS.as_bytes().to_vec()),
        Some("/mk3.js") => ("200 OK", "application/javascript; charset=utf-8", MK3_JS.as_bytes().to_vec()),
        Some("/layout.json") => ("200 OK", "application/json; charset=utf-8", crate::mk3_layout::as_json().to_string().into_bytes()),
        _ => ("404 Not Found", "text/plain; charset=utf-8", b"not found".to_vec()),
    };
    let head = format!("HTTP/1.1 {status}\r\ncontent-type: {mime}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}

/// Read every socket dry: JSON input decodes through that client's own panel
/// into the inbox, a closed or errored socket releases what it held and is
/// removed. Unknown or malformed input is ignored, never a disconnect.
fn poll_clients(clients: &mut Vec<Client>, panels: &mut HashMap<usize, Panel>, inbox: &Arc<Mutex<Vec<Event>>>) {
    let mut dead = Vec::new();
    for (i, client) in clients.iter_mut().enumerate() {
        let mut gone = false;
        loop {
            match client.ws.read() {
                Ok(tungstenite::Message::Text(text)) => {
                    client.active = Instant::now();
                    let panel = panels.entry(client.id).or_default();
                    decode_input(&text, panel);
                    let ev = panel.take();
                    if !ev.is_empty() {
                        inbox.lock().unwrap_or_else(|e| e.into_inner()).extend(ev);
                    }
                }
                Ok(tungstenite::Message::Binary(_)) => {
                    client.active = Instant::now();
                }
                // A live-but-idle client answers pings with pongs: either proves it is
                // still there, so neither restarts the idle clock.
                Ok(tungstenite::Message::Pong(_)) | Ok(tungstenite::Message::Ping(_)) => {
                    client.active = Instant::now();
                }
                Ok(tungstenite::Message::Close(_)) => {
                    client.ws.close(None).ok();
                    gone = true;
                    break;
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(tungstenite::Error::ConnectionClosed) | Err(tungstenite::Error::AlreadyClosed) => {
                    gone = true;
                    break;
                }
                Err(_) => {
                    gone = true;
                    break;
                }
            }
        }
        if !gone && client.active.elapsed() > PING_IDLE {
            // A vanished client is noticed on the next poll; a full socket
            // buffer is a dropped ping, never a stall.
            match client.ws.send(tungstenite::Message::Ping(vec![].into())) {
                Ok(()) => client.active = Instant::now(),
                Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(_) => gone = true,
            }
        }
        if gone {
            dead.push(i);
        }
    }
    for i in dead.into_iter().rev() {
        remove_client(clients, panels, inbox, i);
    }
}

/// Drop a dead client: release everything it still held into the server's
/// queue — a disconnect must never leave a note or a light stuck — then forget
/// the socket and its panel. The queue is the server's, so the releases survive
/// the client's own removal, ordered after that client's earlier input (which
/// was already queued): arrivals stay in arrival order.
fn remove_client(clients: &mut Vec<Client>, panels: &mut HashMap<usize, Panel>, inbox: &Arc<Mutex<Vec<Event>>>, index: usize) {
    let client = clients.swap_remove(index);
    if let Some(mut panel) = panels.remove(&client.id) {
        panel.release_all();
        let ev = panel.take();
        if !ev.is_empty() {
            inbox.lock().unwrap_or_else(|e| e.into_inner()).extend(ev);
        }
    }
}

/// One client event: `down`/`up`/`move` at table coordinates with optional
/// pressure, or a wheel tick. Anything else is ignored.
fn decode_input(text: &str, panel: &mut Panel) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return };
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match kind {
        "down" | "move" => {
            let (Some(finger), Some(x), Some(y)) =
                (v.get("finger").and_then(|n| n.as_u64()), v.get("x").and_then(|n| n.as_f64()), v.get("y").and_then(|n| n.as_f64()))
            else {
                return;
            };
            let p = v.get("p").and_then(|n| n.as_f64()).map(|f| f as f32);
            let at = eframe::egui::pos2(x as f32, y as f32);
            if kind == "down" {
                panel.pointer_down(finger as usize, at, p);
            } else {
                panel.pointer_move(finger as usize, at);
            }
        }
        "up" => {
            if let Some(finger) = v.get("finger").and_then(|n| n.as_u64()) {
                panel.pointer_up(finger as usize);
            }
        }
        "wheel" => {
            if let Some(steps) = v.get("steps").and_then(|n| n.as_i64()) {
                // A hostile `steps` (2**40 from the network) would wrap the cast into
                // a spurious turn: clamp to a sane range first.
                panel.wheel(steps.clamp(-32, 32) as i32);
            }
        }
        _ => {}
    }
}

/// The lights the page paints, from [`crate::mk3_lights::color`] — never the
/// raw bytes, so the page agrees with every other painter: one object mapping
/// each lit control's id to its `[r, g, b]`, off controls absent.
fn lights_json(leds: &Leds, now: Instant) -> Vec<u8> {
    let mut map = serde_json::Map::new();
    for c in crate::mk3_layout::controls() {
        if let Some([r, g, b]) = crate::mk3_lights::color(leds, c.id, now) {
            map.insert(c.id.to_string(), serde_json::json!([r, g, b]));
        }
    }
    // The touch strip is 26 LEDs of its own rather than one control, so the page gets
    // them under `strip:01`..`strip:26`: same function, same colours as the panel.
    for i in 0..crate::mk3_lights::STRIP_SEGMENTS {
        if let Some([r, g, b]) = crate::mk3_lights::strip_segment(leds, i, now) {
            map.insert(format!("strip:{:02}", i + 1), serde_json::json!([r, g, b]));
        }
    }
    serde_json::Value::Object(map).to_string().into_bytes()
}

/// The screens as PNG bytes: the 960 × 272 RGBA image [`Mk3Out`] carries.
/// `None` when the pixels are not what they claim to be — a corrupt frame
/// never reaches the wire.
fn encode_png(px: &[[u8; 4]]) -> Option<Vec<u8>> {
    if px.len() != SCREEN_W as usize * SCREEN_H as usize {
        return None;
    }
    let mut buf = Vec::new();
    let mut enc = png::Encoder::new(&mut buf, SCREEN_W, SCREEN_H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().ok()?;
    let flat: Vec<u8> = px.iter().flatten().copied().collect();
    w.write_image_data(&flat).ok()?;
    drop(w);
    Some(buf)
}

/// One wire message: the screen counter (a repeated frame reads the same),
/// the kind, then the payload.
fn frame(count: u64, kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut framed = Vec::with_capacity(5 + payload.len());
    framed.extend_from_slice(&(count as u32).to_le_bytes());
    framed.push(kind);
    framed.extend_from_slice(payload);
    framed
}

/// Send one message to every client; a closed or errored socket is removed,
/// never treated as an app error.
fn broadcast(clients: &mut Vec<Client>, panels: &mut HashMap<usize, Panel>, inbox: &Arc<Mutex<Vec<Event>>>, bytes: &[u8]) {
    let mut dead = Vec::new();
    for (i, client) in clients.iter_mut().enumerate() {
        if !push(&mut client.ws, bytes) {
            dead.push(i);
        }
    }
    for i in dead.into_iter().rev() {
        remove_client(clients, panels, inbox, i);
    }
}

/// Send one message, flushing at once. A slow client is a dropped frame
/// (`false` only when the socket itself is gone): the thread never blocks on
/// a client that stopped reading.
fn push(ws: &mut tungstenite::WebSocket<TcpStream>, bytes: &[u8]) -> bool {
    match ws.send(tungstenite::Message::Binary(bytes.to_vec().into())) {
        Ok(()) => true,
        Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => true,
        Err(_) => false,
    }
}
