// Shared helpers for octotrack web UI

// ---------------------------------------------------------------------------
// Single-player: only one <audio> element plays at a time
// ---------------------------------------------------------------------------

let _currentAudio = null;

function registerAudio(el) {
  el.addEventListener('play', () => {
    if (_currentAudio && _currentAudio !== el) {
      _currentAudio.pause();
      _currentAudio.currentTime = 0;
    }
    _currentAudio = el;
  });
}

// ---------------------------------------------------------------------------
// Auth helpers
// ---------------------------------------------------------------------------

function getToken() {
  return localStorage.getItem('octotrack_token') || '';
}

function setToken(token) {
  localStorage.setItem('octotrack_token', token);
}

function clearToken() {
  localStorage.removeItem('octotrack_token');
}

function redirectToLogin() {
  clearToken();
  window.location.href = '/';
}

// ---------------------------------------------------------------------------
// API fetch wrapper
// ---------------------------------------------------------------------------

async function apiFetch(method, path, body) {
  const token = getToken();
  const opts = {
    method,
    headers: {
      'Authorization': 'Bearer ' + token,
    },
  };
  if (body !== undefined) {
    opts.headers['Content-Type'] = 'application/json';
    opts.body = JSON.stringify(body);
  }
  const resp = await fetch(path, opts);
  if (resp.status === 401) {
    redirectToLogin();
    return null;
  }
  return resp;
}

// ---------------------------------------------------------------------------
// SSE setup
// ---------------------------------------------------------------------------

let _sseSource = null;
const _sseHandlers = {};

function onSseEvent(type, handler) {
  _sseHandlers[type] = handler;
}

function connectSse() {
  if (_sseSource) return;
  // Use EventSource with Authorization header workaround via URL token param
  // (EventSource doesn't support custom headers; use cookie auth which actix sets)
  _sseSource = new EventSource('/api/events');
  _sseSource.onmessage = (e) => {
    try {
      const event = JSON.parse(e.data);
      const handler = _sseHandlers[event.type];
      if (handler) handler(event);
    } catch (_) {}
  };
  _sseSource.onerror = () => {
    _sseSource.close();
    _sseSource = null;
    // Reconnect after 3s
    setTimeout(connectSse, 3000);
  };
}

// ---------------------------------------------------------------------------
// Level meter canvas renderer
// ---------------------------------------------------------------------------

/**
 * Render level meters onto a canvas element.
 * @param {HTMLCanvasElement} canvas
 * @param {number[]} levels - values 0.0 to 1.0 per channel
 */
function renderLevels(canvas, levels) {
  if (!canvas || !levels || levels.length === 0) return;
  const ctx = canvas.getContext('2d');
  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  canvas.width = w;
  canvas.height = h;

  ctx.clearRect(0, 0, w, h);

  const n = levels.length;
  const gap = 3;
  const barW = Math.max(1, (w - gap * (n - 1)) / n);

  for (let i = 0; i < n; i++) {
    const level = Math.min(1.0, Math.max(0, levels[i]));
    const barH = Math.round(level * h);
    const x = i * (barW + gap);
    const y = h - barH;

    // Color: green < 0.7, yellow < 0.9, red >= 0.9
    let color;
    if (level >= 0.9) {
      color = '#f44336';
    } else if (level >= 0.7) {
      color = '#ff9800';
    } else {
      color = '#4caf50';
    }

    ctx.fillStyle = '#1e1e1e';
    ctx.fillRect(x, 0, barW, h);
    ctx.fillStyle = color;
    ctx.fillRect(x, y, barW, barH);
  }
}

// ---------------------------------------------------------------------------
// Waveform renderer
// ---------------------------------------------------------------------------

/**
 * Render per-channel waveform peaks onto a canvas.
 * @param {HTMLCanvasElement} canvas
 * @param {number[][]} channels - array of channels, each an array of 0.0-1.0 peak values
 */
function renderWaveform(canvas, channels) {
  if (!canvas || !channels || channels.length === 0) return;

  // Palette: distinct colours for up to 8 channels, then repeating.
  const COLORS = ['#4caf50','#2196f3','#ff9800','#e91e63','#9c27b0','#00bcd4','#ffeb3b','#f44336'];

  const nCh = channels.length;
  const CH_H = 32; // px per channel row
  const GAP = 2;   // px between rows
  const totalH = nCh * CH_H + (nCh - 1) * GAP;

  // Resize the canvas to fit all channels.
  const w = canvas.clientWidth || 200;
  canvas.width = w;
  canvas.height = totalH;
  canvas.style.height = totalH + 'px';

  const ctx = canvas.getContext('2d');
  ctx.fillStyle = '#1e1e1e';
  ctx.fillRect(0, 0, w, totalH);

  for (let c = 0; c < nCh; c++) {
    const peaks = channels[c];
    if (!peaks || peaks.length === 0) continue;
    const yTop = c * (CH_H + GAP);
    const mid = yTop + CH_H / 2;

    ctx.fillStyle = COLORS[c % COLORS.length];
    const n = peaks.length;
    for (let i = 0; i < n; i++) {
      const x = (i / n) * w;
      const pw = Math.max(1, w / n);
      const amp = peaks[i] * (CH_H / 2);
      ctx.fillRect(x, mid - amp, pw, amp * 2 || 1);
    }
  }
}

// ---------------------------------------------------------------------------
// Navigation: mark active link
// ---------------------------------------------------------------------------

function markActiveNav() {
  const path = window.location.pathname.replace(/^\//, '') || 'dashboard';
  document.querySelectorAll('nav a').forEach(a => {
    const href = a.getAttribute('href').replace(/^\//, '');
    if (href === path) {
      a.classList.add('active');
    }
  });
}

document.addEventListener('DOMContentLoaded', markActiveNav);
