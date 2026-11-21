//! The server sink's contract, with no window: a browser client joins over a
//! WebSocket, receives framed screen and light messages, and its JSON input
//! arrives as MK3 events. Sockets are fine in tests; windows are not.

use crate::Mk3Out;
use crate::mk3_io::PadRoute;
use crate::mk3_server::Server;
use crate::mk3_sink::Mk3Sink;
use deckard_mk3::Event;
use deckard_mk3::leds::Leds;
use deckard_model::pad_touch::PadTouch;
use std::time::{Duration, Instant};
use tungstenite::Message;

const W: usize = 960;
const H: usize = 272;

type Ws = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

/// A frame the tests hand the server: fixed LEDs, `count` screens of one colour.
fn test_frame(count: u64, px: [u8; 4]) -> Mk3Out {
    frame_with_leds(count, px, Leds { buttons: [0x11; 63], pads: [0x22; 42] })
}

fn frame_with_leds(count: u64, px: [u8; 4], leds: Leds) -> Mk3Out {
    Mk3Out {
        route: PadRoute {
            play: true,
            group: 3,
            keys: None,
            chords: None,
            velocity: None,
            scaling: 0,
            fixed: 100,
            touches: [PadTouch::default(); 16],
        },
        leds,
        screens: Some((count, crate::mk3_io::ScreenFrame::Pixels(std::sync::Arc::new(vec![px; W * H])))),
    }
}

fn frame_without_screens(leds: Leds) -> Mk3Out {
    let mut frame = test_frame(0, [0, 0, 0, 255]);
    frame.leds = leds;
    frame.screens = None;
    frame
}

/// Deterministic incompressible pixels: a blocked pipe must fill, so the
/// slow-client test floods with PNGs near 1 MB, not solid colours.
fn noise(seed: u64) -> Vec<[u8; 4]> {
    let mut s = seed.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(1);
    (0..W * H)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let b = s.to_le_bytes();
            [b[0], b[1], b[2], 255]
        })
        .collect()
}

/// A test client: reads time out instead of hanging the suite.
fn connect(port: u16) -> Ws {
    let (mut ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/")).expect("the test client connects");
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
        s.set_read_timeout(Some(Duration::from_millis(250))).expect("a read timeout");
    }
    ws
}

/// Binary frames until one carries `want`, or `None` after `wait`.
fn read_kind(ws: &mut Ws, want: u8, wait: Duration) -> Option<(u32, Vec<u8>)> {
    let end = Instant::now() + wait;
    while Instant::now() < end {
        match ws.read() {
            Ok(Message::Binary(b)) if b.len() >= 5 && b[4] == want => {
                return Some((u32::from_le_bytes([b[0], b[1], b[2], b[3]]), b[5..].to_vec()));
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }
    None
}

/// Any binary frame, whatever its kind.
fn read_any(ws: &mut Ws, wait: Duration) -> Option<(u32, u8, Vec<u8>)> {
    let end = Instant::now() + wait;
    while Instant::now() < end {
        match ws.read() {
            Ok(Message::Binary(b)) if b.len() >= 5 => {
                return Some((u32::from_le_bytes([b[0], b[1], b[2], b[3]]), b[4], b[5..].to_vec()));
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }
    None
}

/// The server's queued input until `pred` holds, drained across polls.
fn drain_until(server: &mut Server, wait: Duration, pred: impl Fn(&[Event]) -> bool) -> Vec<Event> {
    let end = Instant::now() + wait;
    let mut acc = Vec::new();
    loop {
        acc.extend(server.events());
        if pred(&acc) || Instant::now() >= end {
            return acc;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let r = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    Some((r.info().width, r.info().height))
}

#[test]
fn a_client_receives_the_screen_it_joins_with() {
    let (mut server, port) = Server::start(Some(0)).expect("the server binds");
    server.show(&test_frame(7, [7, 8, 9, 255]));
    let mut ws = connect(port);
    let (count, payload) = read_kind(&mut ws, 1, Duration::from_secs(3)).expect("the screen arrives");
    assert_eq!(count, 7, "the frame carries Mk3Out's screen counter");
    assert!(!payload.is_empty(), "a screen frame carries PNG bytes");
    assert_eq!(png_size(&payload), Some((960, 272)), "the bytes decode as the 960 x 272 screens");
}

#[test]
fn a_client_s_input_arrives_as_mk3_events() {
    let (mut server, port) = Server::start(Some(0)).expect("the server binds");
    let mut ws = connect(port);
    let pad = crate::mk3_layout::control("pad1").expect("pad1 is in the table").rect.center();
    let down = serde_json::json!({"type":"down","finger":0,"x":pad.x,"y":pad.y,"p":0.5}).to_string();
    ws.send(Message::from(down)).expect("the down sends");
    let got = drain_until(&mut server, Duration::from_secs(3), |ev| ev.iter().any(|e| matches!(e, Event::PadOn(1, _))));
    let velocity = got.iter().find_map(|e| match e {
        Event::PadOn(1, v) => Some(*v),
        _ => None,
    });
    assert!((velocity.unwrap_or(-1.0) - 0.5).abs() < 1e-6, "pressure rides as velocity: {got:?}");
    let mv = serde_json::json!({"type":"move","finger":0,"x":pad.x,"y":pad.y + 60.0}).to_string();
    ws.send(Message::from(mv)).expect("the move sends");
    let got = drain_until(&mut server, Duration::from_secs(3), |ev| ev.iter().any(|e| matches!(e, Event::PadPressure(1, _))));
    assert!(got.iter().any(|e| matches!(e, Event::PadPressure(1, _))), "a drag reports pressure: {got:?}");
    let up = serde_json::json!({"type":"up","finger":0}).to_string();
    ws.send(Message::from(up)).expect("the up sends");
    let got = drain_until(&mut server, Duration::from_secs(3), |ev| ev.iter().any(|e| matches!(e, Event::PadOff(1))));
    assert!(got.contains(&Event::PadOff(1)), "lifting reports off: {got:?}");
    // Unknown types are ignored, never a disconnect: the socket still serves.
    ws.send(Message::from(r#"{"type":"frobnicate"}"#.to_string())).expect("unknown sends");
    ws.send(Message::from(r#"{"type":"wheel","steps":2}"#.to_string())).expect("wheel sends");
    let got = drain_until(&mut server, Duration::from_secs(3), |ev| ev.iter().any(|e| matches!(e, Event::Wheel(2))));
    assert!(got.contains(&Event::Wheel(2)), "unknown input is ignored, the wheel still turns: {got:?}");
}

/// A TCP connect that sends nothing must not stall the loop: another client's
/// input still round-trips well under the handshake timeout. (Before the
/// handshake moved to its own thread, `accept_all` blocked in `peek_request`
/// for the full 5 s on the silent socket first.)
#[test]
fn a_silent_connection_does_not_stall_other_clients() {
    let (mut server, port) = Server::start(Some(0)).expect("the server binds");
    let silent = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("a raw socket connects");
    silent.set_nonblocking(true).ok();
    std::thread::sleep(Duration::from_millis(200));
    // The upgrade itself must complete while the silent socket idles: the loop
    // never blocks on a handshake it hasn't finished.
    let start = Instant::now();
    let mut ws = connect(port);
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "the upgrade completes in {:?}: well under the 5 s handshake timeout",
        start.elapsed()
    );
    let pad = crate::mk3_layout::control("pad1").expect("pad1 is in the table").rect.center();
    let down = serde_json::json!({"type":"down","finger":0,"x":pad.x,"y":pad.y,"p":0.5}).to_string();
    ws.send(Message::from(down)).expect("the down sends");
    let start = Instant::now();
    let got = drain_until(&mut server, Duration::from_secs(4), |ev| ev.iter().any(|e| matches!(e, Event::PadOn(1, _))));
    let elapsed = start.elapsed();
    assert!(got.iter().any(|e| matches!(e, Event::PadOn(1, _))), "the client's input arrives while the silent socket idles: {got:?}");
    assert!(elapsed < Duration::from_secs(4), "the round trip takes {elapsed:?}: well under the 5 s handshake timeout");
    drop(silent);
}

/// A hostile wheel report clamps instead of wrapping: 2**40 steps is not a turn.
#[test]
fn absurd_wheel_steps_clamp_instead_of_wrapping() {
    let (mut server, port) = Server::start(Some(0)).expect("the server binds");
    let mut ws = connect(port);
    for (steps, want) in [(1i64 << 40, 32), (-(1i64 << 40), -32), (3, 3)] {
        ws.send(Message::from(serde_json::json!({"type":"wheel","steps":steps}).to_string())).expect("the wheel sends");
        let got = drain_until(&mut server, Duration::from_secs(3), |ev| ev.iter().any(|e| matches!(e, Event::Wheel(_))));
        assert!(got.contains(&Event::Wheel(want)), "{steps} steps clamp to {want}: {got:?}");
    }
}

#[test]
fn a_slow_client_is_a_dropped_frame_not_a_stalled_app() {
    let (mut server, port) = Server::start(Some(0)).expect("the server binds");
    let _slow = connect(port);
    // Stall the consumer with incompressible frames: each is ~900 KB on the
    // wire and slow to encode, so the server thread sits in the encoder while
    // nobody drains the channel. A blocking `show` would pile up behind it;
    // `try_send` drops instead. (48 x ~0.25 s of encode per frame is ~12 s of
    // stall; the burst below must still return at once.)
    let start = Instant::now();
    for n in 0..48u64 {
        let mut frame = test_frame(100 + n, [0, 0, 0, 255]);
        frame.screens = Some((100 + n, crate::mk3_io::ScreenFrame::Pixels(std::sync::Arc::new(noise(n)))));
        server.show(&frame);
    }
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_secs(5), "48 shows returned in {elapsed:?}: the channel drops, never blocks");
    // The app is unaffected: the server drains the backlog and serves again.
    // The fresh client first gets whatever the flood left retained, then a
    // live frame: keep showing until a live count arrives.
    let mut fresh = connect(port);
    let end = Instant::now() + Duration::from_secs(120);
    let mut want = 200u64;
    let mut seen = 0u64;
    while Instant::now() < end && seen < 200 {
        server.show(&test_frame(want, [1, 2, 3, 255]));
        if let Some((c, _)) = read_kind(&mut fresh, 1, Duration::from_millis(500)) {
            seen = seen.max(c as u64);
        }
        want += 1;
    }
    assert!(seen >= 200, "a fresh client gets a live frame after the flood");
}

#[test]
fn encoding_happens_off_the_frame_thread() {
    let (mut server, port) = Server::start(Some(0)).expect("the server binds");
    // A near-incompressible frame: if `show` encoded, it would sit in the
    // PNG encoder for seconds. It only copies and queues, so it returns fast.
    let mut frame = test_frame(11, [0, 0, 0, 255]);
    frame.screens = Some((11, crate::mk3_io::ScreenFrame::Pixels(std::sync::Arc::new(noise(11)))));
    let start = Instant::now();
    server.show(&frame);
    assert!(start.elapsed() < Duration::from_secs(2), "show copies and queues, it never encodes");
    let mut ws = connect(port);
    let (count, payload) = read_kind(&mut ws, 1, Duration::from_secs(30)).expect("the encoded frame arrives");
    assert_eq!(count, 11, "the slow frame still arrives whole");
    assert_eq!(png_size(&payload), Some((960, 272)), "the bytes decode as the 960 x 272 screens");
}
#[test]
fn two_clients_receive_the_same_frame() {
    let (mut server, port) = Server::start(Some(0)).expect("the server binds");
    let mut first = connect(port);
    let mut second = connect(port);
    server.show(&test_frame(9, [4, 5, 6, 255]));
    let (ca, pa) = read_kind(&mut first, 1, Duration::from_secs(3)).expect("the first client reads");
    let (cb, pb) = read_kind(&mut second, 1, Duration::from_secs(3)).expect("the second client reads");
    assert_eq!((ca, cb), (9, 9), "both carry the screen counter");
    assert_eq!(pa, pb, "both carry the same bytes");
}

#[test]
fn unchanged_frames_send_nothing_new() {
    let (mut server, port) = Server::start(Some(0)).expect("the server binds");
    let mut ws = connect(port);
    let leds = Leds { buttons: [0x11; 63], pads: [0x22; 42] };
    server.show(&frame_with_leds(7, [1, 2, 3, 255], leds.clone()));
    let (count, _) = read_kind(&mut ws, 1, Duration::from_secs(3)).expect("the screen arrives");
    assert_eq!(count, 7, "the frame carries Mk3Out's screen counter");
    read_kind(&mut ws, 2, Duration::from_secs(3)).expect("the first lights arrive");
    // The same LEDs with no screens is nothing new: the wire stays silent.
    server.show(&frame_without_screens(leds.clone()));
    assert!(read_any(&mut ws, Duration::from_millis(400)).is_none(), "an unchanged frame sends nothing");
    // A light change with no screens still goes out, under the screen counter.
    let mut lit = leds.clone();
    lit.buttons[0] = 0x80;
    server.show(&frame_without_screens(lit));
    let (count, payload) = read_kind(&mut ws, 2, Duration::from_secs(3)).expect("the light change arrives");
    assert_eq!(count, 7, "lights carry the screen counter");
    let json: serde_json::Value = serde_json::from_slice(&payload).expect("lights are JSON");
    assert!(json.is_object(), "lights are one object: {json}");
}

#[test]
fn closing_a_socket_mid_pad_releases_the_pad() {
    let (mut server, port) = Server::start(Some(0)).expect("the server binds");
    let mut ws = connect(port);
    let pad = crate::mk3_layout::control("pad3").expect("pad3 is in the table").rect.center();
    let down = serde_json::json!({"type":"down","finger":0,"x":pad.x,"y":pad.y,"p":0.7}).to_string();
    ws.send(Message::from(down)).expect("the down sends");
    let mut all = drain_until(&mut server, Duration::from_secs(3), |ev| ev.iter().any(|e| matches!(e, Event::PadOn(3, _))));
    assert!(all.iter().any(|e| matches!(e, Event::PadOn(3, _))), "the press arrives first: {all:?}");
    ws.close(None).ok();
    drop(ws);
    all.extend(drain_until(&mut server, Duration::from_secs(3), |ev| ev.iter().any(|e| matches!(e, Event::PadOff(3)))));
    let on = all.iter().position(|e| matches!(e, Event::PadOn(3, _)));
    let off = all.iter().position(|e| matches!(e, Event::PadOff(3)));
    assert!(matches!((on, off), (Some(a), Some(b)) if a < b), "PadOff(3) follows the PadOn in events(): {all:?}");
}

#[test]
fn two_clients_press_the_same_pad_once_each() {
    let (mut server, port) = Server::start(Some(0)).expect("the server binds");
    let mut first = connect(port);
    let mut second = connect(port);
    let pad = crate::mk3_layout::control("pad1").expect("pad1 is in the table").rect.center();
    let down = serde_json::json!({"type":"down","finger":0,"x":pad.x,"y":pad.y,"p":0.5}).to_string();
    first.send(Message::from(down.clone())).expect("the first down sends");
    second.send(Message::from(down)).expect("the second down sends");
    let got = drain_until(&mut server, Duration::from_secs(3), |ev| ev.iter().filter(|e| matches!(e, Event::PadOn(1, _))).count() >= 2);
    let ons = got.iter().filter(|e| matches!(e, Event::PadOn(1, _))).count();
    let offs = got.iter().filter(|e| matches!(e, Event::PadOff(1))).count();
    assert_eq!(ons, 2, "one press per client, merged un-deduped: {got:?}");
    assert_eq!(offs, 0, "nobody lifted, so nothing releases: {got:?}");
}

#[test]
fn closing_a_socket_mid_button_releases_the_button() {
    let (mut server, port) = Server::start(Some(0)).expect("the server binds");
    let ws = connect(port);
    let btn = crate::mk3_layout::control("BUTTON_GROUP_A").expect("BUTTON_GROUP_A is in the table").rect.center();
    let down = serde_json::json!({"type":"down","finger":0,"x":btn.x,"y":btn.y}).to_string();
    let mut ws = ws;
    ws.send(Message::from(down)).expect("the down sends");
    let mut all =
        drain_until(&mut server, Duration::from_secs(3), |ev| ev.iter().any(|e| matches!(e, Event::Button("BUTTON_GROUP_A", true))));
    assert!(all.contains(&Event::Button("BUTTON_GROUP_A", true)), "the press arrives first: {all:?}");
    // Abrupt drop, no close handshake: the error path must release too.
    drop(ws);
    all.extend(drain_until(&mut server, Duration::from_secs(3), |ev| {
        ev.iter().any(|e| matches!(e, Event::Button("BUTTON_GROUP_A", false)))
    }));
    assert!(all.contains(&Event::Button("BUTTON_GROUP_A", false)), "the held button releases into events(): {all:?}");
}

#[test]
fn closing_a_socket_mid_strip_touch_releases_the_finger() {
    let (mut server, port) = Server::start(Some(0)).expect("the server binds");
    let mut ws = connect(port);
    let strip = crate::mk3_layout::control("strip").expect("strip is in the table").rect.center();
    let down = serde_json::json!({"type":"down","finger":0,"x":strip.x,"y":strip.y}).to_string();
    ws.send(Message::from(down)).expect("the down sends");
    let mut all = drain_until(&mut server, Duration::from_secs(3), |ev| ev.iter().any(|e| matches!(e, Event::Touch(0, 0, _))));
    assert!(all.iter().any(|e| matches!(e, Event::Touch(0, 0, _))), "the touch arrives first: {all:?}");
    ws.close(None).ok();
    drop(ws);
    all.extend(drain_until(&mut server, Duration::from_secs(3), |ev| ev.iter().any(|e| matches!(e, Event::Touch(0, 2, _)))));
    assert!(all.iter().any(|e| matches!(e, Event::Touch(0, 2, _))), "the held strip finger releases into events(): {all:?}");
}

/// A plain HTTP GET over a raw socket: the same bytes curl sends. The server
/// answers and closes, so reads run to EOF instead of hanging the suite.
fn http_get(port: u16, path: &str) -> (String, Vec<u8>) {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("the page fetch connects");
    s.set_read_timeout(Some(Duration::from_secs(3))).expect("a read timeout");
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").expect("the GET sends");
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).expect("the response reads");
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("a header/body split");
    (String::from_utf8_lossy(&raw[..split]).into_owned(), raw[split + 4..].to_vec())
}

#[test]
fn the_page_and_its_assets_are_served() {
    let (_server, port) = Server::start(Some(0)).expect("the server binds");
    let (head, body) = http_get(port, "/");
    assert!(head.starts_with("HTTP/1.1 200"), "GET / is 200: {head}");
    assert!(head.contains("text/html"), "the page is HTML: {head}");
    let body = String::from_utf8_lossy(&body).into_owned();
    assert!(body.contains("mk3.js"), "the page pulls its script: {body}");
    assert!(!body.contains("screen_left"), "the page shell carries no geometry, it fetches the table: {body}");
    let (head, _) = http_get(port, "/mk3.css");
    assert!(head.starts_with("HTTP/1.1 200"), "GET /mk3.css is 200: {head}");
    assert!(head.contains("text/css"), "the stylesheet is CSS: {head}");
    let (head, body) = http_get(port, "/mk3.js");
    assert!(head.starts_with("HTTP/1.1 200"), "GET /mk3.js is 200: {head}");
    let body = String::from_utf8_lossy(&body).into_owned();
    assert!(body.contains("layout.json"), "the script builds from the table it fetches: {body}");
    let (head, body) = http_get(port, "/layout.json");
    assert!(head.starts_with("HTTP/1.1 200"), "GET /layout.json is 200: {head}");
    assert!(head.contains("application/json"), "the table is JSON: {head}");
    let table: serde_json::Value = serde_json::from_slice(&body).expect("the table parses");
    let pads = table["controls"].as_array().expect("controls is an array").iter().filter(|c| c["kind"] == "pad").count();
    assert_eq!(pads, 16, "the table carries 16 pads");
    let (head, _) = http_get(port, "/nope");
    assert!(head.starts_with("HTTP/1.1 404"), "anything else is 404: {head}");
}

/// A raw upgrade over a plain socket: the same bytes a browser sends. The server
/// answers and closes, so reads run to EOF instead of hanging the suite.
fn raw_upgrade(port: u16, target: &str) -> String {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("the upgrade connects");
    s.set_read_timeout(Some(Duration::from_secs(3))).expect("a read timeout");
    write!(
        s,
        "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )
    .expect("the handshake sends");
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).expect("the refusal reads to EOF");
    String::from_utf8_lossy(&raw).into_owned()
}

/// A test client with the pairing token in the upgrade, or without it.
fn connect_with(port: u16, query: &str) -> Ws {
    let (mut ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/{query}")).expect("the test client connects");
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_mut() {
        s.set_read_timeout(Some(Duration::from_millis(250))).expect("a read timeout");
    }
    ws
}

#[test]
fn a_client_without_the_token_is_refused() {
    let (mut server, port) = Server::start_paired(Some(0)).expect("the paired server binds");
    let token = server.token().expect("a paired server has a token").to_string();
    server.show(&test_frame(21, [1, 2, 3, 255]));
    // No token: 403, then EOF — a closed connection, not a hanging one.
    let refused = raw_upgrade(port, "/ws");
    assert!(refused.starts_with("HTTP/1.1 403"), "an upgrade without the token is refused: {refused}");
    // A wrong token fares no better.
    let refused = raw_upgrade(port, "/ws?t=wrong");
    assert!(refused.starts_with("HTTP/1.1 403"), "an upgrade with the wrong token is refused: {refused}");
    // The token in the upgrade: the frame arrives.
    let mut ws = connect_with(port, &format!("ws?t={token}"));
    let (count, _) = read_kind(&mut ws, 1, Duration::from_secs(3)).expect("the paired client reads");
    assert_eq!(count, 21, "the frame carries the screen counter");
}

#[test]
fn the_token_can_be_switched_off() {
    // Token off: the server binds loopback, and a bare upgrade is accepted.
    let (mut server, port) = Server::start(Some(0)).expect("the server binds");
    assert!(server.token().is_none(), "no token when pairing is off");
    assert!(server.local_addr().ip().is_loopback(), "switching the token off forces loopback");
    server.show(&test_frame(22, [4, 5, 6, 255]));
    let mut ws = connect(port);
    let (count, _) = read_kind(&mut ws, 1, Duration::from_secs(3)).expect("the unpaired client reads");
    assert_eq!(count, 22, "the frame carries the screen counter");
    // The invariant's backstop: a tokenless non-loopback bind is refused, never
    // "LAN and open".
    let err = match Server::start_on("0.0.0.0", Some(0), None) {
        Ok(_) => panic!("a tokenless LAN bind is refused"),
        Err(e) => e,
    };
    assert!(err.to_string().contains("token"), "the refusal names the token: {err}");
    // ...while a token earns the LAN bind the tablet needs.
    let (paired, _) = Server::start_paired(Some(0)).expect("the paired server binds");
    assert!(!paired.local_addr().ip().is_loopback(), "a paired server binds beyond loopback");
}

#[test]
fn a_script_error_does_not_break_the_panel_or_the_page() {
    // A surface whose scripts break on reload: the error is reported, but the
    // last working scripts stay loaded (deckard-surface's own contract).
    let tmp = tempfile::tempdir().expect("a scratch user dir");
    std::fs::create_dir_all(tmp.path().join("pages")).expect("pages/");
    let mut app = crate::App::default();
    app.new_project();
    app.surface = deckard_surface::Surface::with_user_dir(Some(tmp.path().to_path_buf()));
    assert_eq!(app.surface.error(), None, "the shipped scripts load");
    std::fs::write(tmp.path().join("global.lua"), "return { this is not lua").expect("the breaking edit");
    assert!(!app.surface.reload(), "the broken scripts do not load");
    let error = app.surface.error().expect("the error is reported");
    // The frame still builds from the last working scripts: lights and screens.
    let mut requests = Vec::new();
    let out = app.mk3_events(Vec::new(), &mut requests);
    assert!(out.screens.is_some(), "screens still deliver with a script error");
    // ...and the error reaches the status line, without panic.
    assert!(app.status.contains("controller script"), "the error reaches the status: {}", app.status);
    assert!(app.status.contains(&error), "the status names the error: {}", app.status);
    // The panel and the page still get the frame: through the sinks, the same
    // fan-out `mk3_sync` runs every frame.
    let mut sinks = crate::mk3_sink::Sinks::new();
    sinks.set_panel_open(true);
    let (server, port) = Server::start(Some(0)).expect("the server binds");
    sinks.add(server);
    sinks.show(&out);
    let mut ws = connect(port);
    let (count, _) = read_kind(&mut ws, 1, Duration::from_secs(3)).expect("the page still gets its frame");
    assert_eq!(count, out.screens.as_ref().map(|(n, _)| *n as u32).unwrap_or(0), "the frame carries the counter");
    let (_, lights) = read_kind(&mut ws, 2, Duration::from_secs(3)).expect("lights still deliver with a script error");
    let json: serde_json::Value = serde_json::from_slice(&lights).expect("lights are JSON");
    assert!(json.is_object(), "lights are one object: {json}");
    assert!(sinks.panel_open(), "the panel stays open");
}
