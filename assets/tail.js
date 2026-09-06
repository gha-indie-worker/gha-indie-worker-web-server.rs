/* ============================================================================================
   Live log tail over the shared session protocol.
   ---------------------------------------------------------------------------------------------
   This is the only interactive code in the product that is not a form. It speaks the same frame
   vocabulary as the runners and the CLI (`lib_core::runtime::session`), encoded as JSON:

       -> {"type":"hello","version":1,"resume":null}
       <- {"type":"welcome","session":"…","resumed":false,"heartbeat_seconds":20}
       -> {"type":"subscribe","stream":"run:01H…:logs","from":0,"credit":64}
       <- {"type":"event","stream":"…","sequence":1,"payload":"+ cargo test"}
       -> {"type":"credit","stream":"…","additional":32}

   Backpressure is the point. The server may only send as many events as this page has granted,
   so a 200 MB build log cannot make the server buffer on our behalf, and "Pause" is not a UI
   affectation: it stops granting credit, and the server stops sending. Nothing is dropped
   silently — a gap arrives as an explicit `lagged` frame and is rendered as a visible row.

   No dependencies, no build step, one file, served from our own origin under `script-src 'self'`.
   ============================================================================================ */

(function () {
  "use strict";

  var config = window.__giw || {};
  var list = document.getElementById("logtail");
  var statusEl = document.getElementById("tail-status");
  var pauseButton = document.getElementById("tail-pause");
  if (!list || !config.stream || !config.socket) {
    return;
  }

  /* Grant in blocks, and top up when half the block has been spent. Small enough that a paused
     reader stops the flow quickly; large enough that a fast log is not one round trip per line. */
  var BLOCK = 64;
  var TOPUP_AT = 32;
  var MAX_ROWS = 5000;

  var socket = null;
  var delivered = Number(config.from || 0);
  var spent = 0;
  var paused = false;
  var closedByUs = false;
  var attempt = 0;
  var reconnectTimer = null;

  function setStatus(state, text) {
    if (!statusEl) return;
    statusEl.dataset.state = state;
    statusEl.textContent = text;
  }

  function send(frame) {
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify(frame));
      return true;
    }
    return false;
  }

  function trim() {
    while (list.childElementCount > MAX_ROWS) {
      list.removeChild(list.firstElementChild);
    }
  }

  function appendLine(sequence, text) {
    var row = document.createElement("li");
    var seq = document.createElement("span");
    seq.className = "seq";
    seq.setAttribute("aria-hidden", "true");
    seq.textContent = String(sequence);
    var line = document.createElement("span");
    line.className = "line";
    /* textContent, never innerHTML: a build log is attacker-influenced input by definition. */
    line.textContent = text;
    row.appendChild(seq);
    row.appendChild(line);
    list.appendChild(row);
    trim();
    stickToBottom();
  }

  function appendGap(text) {
    var row = document.createElement("li");
    var line = document.createElement("span");
    line.className = "line gap";
    line.setAttribute("role", "status");
    line.textContent = text;
    row.appendChild(line);
    list.appendChild(row);
    trim();
    stickToBottom();
  }

  /* Follow the tail only while the reader is already at the bottom; yanking the viewport away
     from a line somebody is reading is the single most annoying thing a log view can do. */
  function stickToBottom() {
    var slack = list.scrollHeight - list.scrollTop - list.clientHeight;
    if (slack < 80) {
      list.scrollTop = list.scrollHeight;
    }
  }

  function grantIfNeeded() {
    if (paused || spent < TOPUP_AT) return;
    if (send({ type: "credit", stream: config.stream, additional: spent })) {
      spent = 0;
    }
  }

  function onFrame(frame) {
    switch (frame.type) {
      case "welcome":
        attempt = 0;
        setStatus("live", "Live");
        send({
          type: "subscribe",
          stream: config.stream,
          from: delivered,
          credit: paused ? 0 : BLOCK
        });
        break;

      case "event": {
        if (frame.stream !== config.stream) return;
        var expected = delivered + 1;
        if (frame.sequence !== expected) {
          /* Out of order is a server bug, not something to paper over by rendering it anyway. */
          appendGap("stream desynchronised at " + frame.sequence + "; reconnecting");
          reconnect();
          return;
        }
        delivered = frame.sequence;
        spent += 1;
        appendLine(frame.sequence, frame.payload === undefined ? "" : frame.payload);
        grantIfNeeded();
        break;
      }

      case "lagged": {
        var skipped = frame.skipped_to - delivered - 1;
        delivered = frame.skipped_to;
        appendGap(skipped + " line" + (skipped === 1 ? "" : "s") + " dropped — the tail could not keep up");
        break;
      }

      case "ping":
        send({ type: "pong", nonce: frame.nonce });
        break;

      case "error":
        setStatus("disconnected", "Refused: " + (frame.code || "error"));
        break;

      case "close":
        closedByUs = true;
        setStatus("disconnected", frame.message || "Closed");
        if (socket) socket.close();
        break;

      default:
        break;
    }
  }

  function connect() {
    setStatus("connecting", "Connecting…");
    try {
      socket = new WebSocket(config.socket);
    } catch (error) {
      scheduleReconnect();
      return;
    }

    socket.addEventListener("open", function () {
      send({ type: "hello", version: 1, resume: null });
    });

    socket.addEventListener("message", function (event) {
      if (typeof event.data !== "string") return;
      var frame;
      try {
        frame = JSON.parse(event.data);
      } catch (error) {
        return;
      }
      if (frame && typeof frame.type === "string") onFrame(frame);
    });

    socket.addEventListener("close", function () {
      if (closedByUs) return;
      setStatus("disconnected", "Disconnected — retrying");
      scheduleReconnect();
    });

    socket.addEventListener("error", function () {
      /* `close` always follows; nothing useful to do here beyond not throwing. */
    });
  }

  /* Exponential backoff with jitter, capped. A thundering herd after a deploy is how one restart
     becomes an outage. */
  function scheduleReconnect() {
    if (reconnectTimer) return;
    attempt += 1;
    var base = Math.min(30000, 500 * Math.pow(2, Math.min(attempt, 6)));
    var delay = base / 2 + Math.random() * (base / 2);
    reconnectTimer = window.setTimeout(function () {
      reconnectTimer = null;
      connect();
    }, delay);
  }

  function reconnect() {
    if (socket) {
      closedByUs = true;
      socket.close();
      closedByUs = false;
    }
    scheduleReconnect();
  }

  if (pauseButton) {
    pauseButton.addEventListener("click", function () {
      paused = !paused;
      pauseButton.setAttribute("aria-pressed", paused ? "true" : "false");
      pauseButton.textContent = paused ? "Resume" : "Pause";
      if (paused) {
        setStatus("paused", "Paused — the server is not sending");
      } else {
        setStatus("live", "Live");
        /* Resuming grants a whole block plus whatever was spent while paused. */
        if (send({ type: "credit", stream: config.stream, additional: BLOCK + spent })) {
          spent = 0;
        }
      }
    });
  }

  window.addEventListener("beforeunload", function () {
    closedByUs = true;
    if (socket && socket.readyState === WebSocket.OPEN) {
      send({ type: "unsubscribe", stream: config.stream });
      socket.close();
    }
  });

  connect();
})();
