"use strict";

// The page carries no geometry: every rectangle, id and kind comes from
// /layout.json, and every colour comes from a light message. The only numbers
// this file owns are the wire protocol's (see mk3_server.rs): screens arrive
// as [u32 LE count][u8 kind][PNG], lights as [u32 LE count][u8 kind][JSON].
const KIND_SCREEN = 1;
const KIND_LIGHTS = 2;

const surface = document.getElementById("surface");
let layout = null;
let built = []; // { el, rect, id } in table order, rects straight from the table
let screens = []; // { canvas, ctx }, one canvas per screen control
const stripSegs = []; // { el, id } for the strip's 25 LEDs: "strip:01".."strip:25"
let lastScreenCount = null;
// A frame that arrives before the table exists is retained, never dropped.
// The socket opens after the build below, so these only fill if that order
// ever changes; either way the newest message per kind wins by arrival, and
// the count decides whether it still needs paint.
let pendingScreen = null;
let pendingLights = null;
const fingers = new Map(); // pointerId -> finger, one finger per pointer
let nextFinger = 0;
let socket = null;
let fails = 0; // consecutive closes without an open: the pairing hint's trigger.

function setStatus(text) {
  document.getElementById("status").textContent = text;
}
function send(msg) {
  if (socket !== null && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify(msg));
  }
}

// Size the surface and every control from the table: the scale derives from
// the viewport against the table's own width and height.
function fitSurface() {
  const scale = Math.min(window.innerWidth / layout.width, window.innerHeight / layout.height);
  surface.style.width = layout.width * scale + "px";
  surface.style.height = layout.height * scale + "px";
  for (const b of built) {
    b.el.style.left = b.rect.x * scale + "px";
    b.el.style.top = b.rect.y * scale + "px";
    b.el.style.width = b.rect.w * scale + "px";
    b.el.style.height = b.rect.h * scale + "px";
  }
}

function buildSurface(table) {
  layout = table;
  for (const c of table.controls) {
    const el = document.createElement("div");
    // The material comes from the kind and the cap: silicon buttons and dark
    // rgb caps (pads, display and group buttons) paint like the fork's
    // `.jam-btn` / `.btn-rgb`, and `.lit` mixes in the LED colour (see mk3.css).
    el.className = "control kind-" + c.kind + " cap-" + c.cap;
    if (c.kind === "screen") {
      const canvas = document.createElement("canvas");
      el.appendChild(canvas);
      screens.push({ canvas: canvas, ctx: canvas.getContext("2d") });
    } else if (c.kind === "strip") {
      // The strip is 25 LEDs of its own, not one control: the server sends them as
      // "strip:01".."strip:25" and each gets a span inside the bar.
      for (let i = 0; i < 25; i++) {
        const seg = document.createElement("span");
        seg.className = "mk3-seg";
        seg.style.left = i * (100 / 25) + (100 / 25) * 0.12 + "%";
        el.appendChild(seg);
        stripSegs.push({ el: seg, id: "strip:" + String(i + 1).padStart(2, "0") });
      }
    } else {
      // Pads, knobs, the jog and the display buttons carry no text in the
      // fork: only a non-empty silkscreen label becomes a text node.
      if (c.label) {
        el.textContent = c.label.toUpperCase();
      }
      el.addEventListener("pointerdown", onDown);
      el.addEventListener("pointermove", onMove);
      el.addEventListener("pointerup", onUp);
      el.addEventListener("pointercancel", onUp);
    }
    surface.appendChild(el);
    built.push({ el: el, rect: c.rect, id: c.id });
  }
  fitSurface();
  window.addEventListener("resize", fitSurface);
}

// Pointer coordinates are converted into the surface's own space — the same
// space as layout.json — so the server needs no pixel math.
function sendContact(type, finger, e) {
  const r = surface.getBoundingClientRect();
  const msg = {
    type: type,
    finger: finger,
    x: (e.clientX - r.left) * (layout.width / r.width),
    y: (e.clientY - r.top) * (layout.height / r.height),
  };
  if (e.pressure > 0) {
    msg.p = e.pressure;
  }
  send(msg);
}

function onDown(e) {
  e.preventDefault();
  // A finger that slides off its control still reports to it, and a second
  // finger is a second control: one finger per pointer id.
  e.currentTarget.setPointerCapture(e.pointerId);
  const finger = nextFinger++;
  fingers.set(e.pointerId, finger);
  sendContact("down", finger, e);
}

function onMove(e) {
  if (!fingers.has(e.pointerId)) {
    return;
  }
  e.preventDefault();
  sendContact("move", fingers.get(e.pointerId), e);
}

function onUp(e) {
  if (!fingers.has(e.pointerId)) {
    return;
  }
  send({ type: "up", finger: fingers.get(e.pointerId) });
  fingers.delete(e.pointerId);
}

// The PNG holds both screens side by side; each canvas takes the half that
// matches its table order. The count gate keeps repeats off the canvas.
function paintScreen(payload) {
  const url = URL.createObjectURL(new Blob([payload], { type: "image/png" }));
  const img = new Image();
  img.onload = () => {
    const half = img.naturalWidth / 2;
    screens.forEach((s, i) => {
      s.canvas.width = half;
      s.canvas.height = img.naturalHeight;
      s.ctx.drawImage(img, i * half, 0, half, img.naturalHeight, 0, 0, half, img.naturalHeight);
    });
    URL.revokeObjectURL(url);
  };
  img.src = url;
}

// Colours arrive per control id and land in `--led-color`, which the `.lit`
// rules mix into the cap like the fork does; an unlit control drops the class
// and keeps its dark sheet style so it stays tappable. The flash is resolved
// server-side: no animation here.
function paintLights(colours) {
  for (const b of built) {
    paintLight(b.el, colours[b.id]);
  }
  for (const s of stripSegs) {
    paintLight(s.el, colours[s.id]);
  }
}

function paintLight(el, rgb) {
  if (Array.isArray(rgb)) {
    el.style.setProperty("--led-color", "rgb(" + rgb[0] + "," + rgb[1] + "," + rgb[2] + ")");
    el.style.setProperty("--led-brightness", "1");
    el.classList.add("lit");
  } else {
    el.classList.remove("lit");
  }
}

function onWire(ev) {
  const view = new DataView(ev.data);
  const count = view.getUint32(0, true);
  const kind = view.getUint8(4);
  const payload = ev.data.slice(5);
  if (kind === KIND_SCREEN) {
    if (layout === null) {
      pendingScreen = { count: count, payload: payload };
      return;
    }
    if (count !== lastScreenCount) {
      lastScreenCount = count;
      paintScreen(payload);
    }
  } else if (kind === KIND_LIGHTS) {
    const text = new TextDecoder().decode(payload);
    if (layout === null) {
      pendingLights = text;
      return;
    }
    try {
      paintLights(JSON.parse(text));
    } catch {
    }
  }
}

function openSocket() {
  const proto = location.protocol === "https:" ? "wss://" : "ws://";
  // Pairing rides the socket's own URL: the `t` the status bar's URL carried.
  // Nothing else sends it back — the table fetch below stays bare.
  const token = new URLSearchParams(location.search).get("t");
  const wsPath = token === null ? "/ws" : "/ws?t=" + encodeURIComponent(token);
  socket = new WebSocket(proto + location.host + wsPath);
  socket.binaryType = "arraybuffer";
  socket.onmessage = onWire;
  socket.onopen = () => {
    fails = 0;
    setStatus("");
  };
  socket.onclose = () => {
    fails += 1;
    // A refused token and a restart look the same here (both just close the
    // socket), so after repeated failures the hint names the pairing token.
    if (fails >= 3) {
      setStatus("Can't reach the MK3: check the pairing token in the URL (?t=...).");
    }
    window.setTimeout(openSocket, 1000);
  };
}

surface.addEventListener(
  "wheel",
  (e) => {
    e.preventDefault();
    const steps = Math.sign(e.deltaY);
    if (steps !== 0) {
      send({ type: "wheel", steps: steps });
    }
  },
  { passive: false },
);

// Fetch the table, build the surface, open the socket, then paint: a frame
// that beats the table waits in pendingScreen/pendingLights above.
fetch("/layout.json")
  .then((r) => r.json())
  .then((table) => {
    buildSurface(table);
    openSocket();
    if (pendingScreen !== null) {
      lastScreenCount = pendingScreen.count;
      paintScreen(pendingScreen.payload);
      pendingScreen = null;
    }
    if (pendingLights !== null) {
      try {
        paintLights(JSON.parse(pendingLights));
      } catch {
      }
      pendingLights = null;
    }
  });
