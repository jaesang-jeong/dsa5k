// DSA5000 Studio Frontend Application (Rust Native Edition)
// High-Speed Canvas Visualization & Real-Time Controls

// Distinct vibrant colors for 16 channels
const CHANNEL_COLORS = [
  '#00e5ff', '#ff3d00', '#00e676', '#ffea00',
  '#d500f9', '#2979ff', '#ff9100', '#00b0ff',
  '#76ff03', '#f50057', '#1de9b6', '#ff6d00',
  '#651fff', '#c6ff00', '#ff1744', '#00e5ff'
];

// App State
const state = {
  ws: null,
  isConnected: false,
  isScanning: false,
  isPaused: false,
  timeRangeSec: 5,
  autoScale: true,
  yMin: 0,
  yMax: 10,
  activeChannels: new Array(16).fill(true),
  dataBuffer: [], // Array of { time, pressures: Float32Array(16) }
  channelStats: Array.from({ length: 16 }, () => ({
    current: 0,
    sum: 0,
    count: 0,
    min: Infinity,
    max: -Infinity,
    sumSq: 0
  }))
};

// DOM Elements
const el = {
  connBadge: document.getElementById('connBadge'),
  inputHost: document.getElementById('inputHost'),
  inputCmdPort: document.getElementById('inputCmdPort'),
  inputDataPort: document.getElementById('inputDataPort'),
  chkSimulator: document.getElementById('chkSimulator'),
  btnConnect: document.getElementById('btnConnect'),
  btnDisconnect: document.getElementById('btnDisconnect'),
  statFps: document.getElementById('statFps'),
  statFrames: document.getElementById('statFrames'),
  statDropped: document.getElementById('statDropped'),
  infoModel: document.getElementById('infoModel'),
  infoSN: document.getElementById('infoSN'),
  infoRate: document.getElementById('infoRate'),
  infoUnits: document.getElementById('infoUnits'),
  btnStartScan: document.getElementById('btnStartScan'),
  btnStopScan: document.getElementById('btnStopScan'),
  btnCalz: document.getElementById('btnCalz'),
  valveStatusText: document.getElementById('valveStatusText'),
  valveBtns: document.querySelectorAll('.btn-valve'),
  logStatusText: document.getElementById('logStatusText'),
  logFrameCount: document.getElementById('logFrameCount'),
  logFileName: document.getElementById('logFileName'),
  btnStartLog: document.getElementById('btnStartLog'),
  btnStopLog: document.getElementById('btnStopLog'),
  selTimeRange: document.getElementById('selTimeRange'),
  chkAutoScale: document.getElementById('chkAutoScale'),
  btnPauseChart: document.getElementById('btnPauseChart'),
  btnResetChart: document.getElementById('btnResetChart'),
  channelChips: document.getElementById('channelChips'),
  waveformCanvas: document.getElementById('waveformCanvas'),
  barMetersContainer: document.getElementById('barMetersContainer'),
  statsTableBody: document.getElementById('statsTableBody'),
  // Modals
  modalTerminal: document.getElementById('modalTerminal'),
  btnOpenTerminal: document.getElementById('btnOpenTerminal'),
  btnCloseTerminal: document.getElementById('btnCloseTerminal'),
  terminalOutput: document.getElementById('terminalOutput'),
  terminalInput: document.getElementById('terminalInput'),
  btnSendTerminal: document.getElementById('btnSendTerminal'),
  modalSettings: document.getElementById('modalSettings'),
  btnOpenSettings: document.getElementById('btnOpenSettings'),
  btnCloseSettings: document.getElementById('btnCloseSettings'),
  btnApplySettings: document.getElementById('btnApplySettings'),
  btnCancelSettings: document.getElementById('btnCancelSettings'),
  cfgRate: document.getElementById('cfgRate'),
  cfgUnits: document.getElementById('cfgUnits'),
  modalCalz: document.getElementById('modalCalz'),
  calzStatusText: document.getElementById('calzStatusText'),
  calzProgressBar: document.getElementById('calzProgressBar')
};

// Canvas 2D Context
const ctx = el.waveformCanvas.getContext('2d');

// Initialize UI
function init() {
  buildChannelChips();
  buildBarMeters();
  buildStatsTable();
  setupEventListeners();
  resizeCanvas();
  window.addEventListener('resize', resizeCanvas);

  connectWebSocket();
  requestAnimationFrame(renderChart);
}

// Build Channel Toggle Chips
function buildChannelChips() {
  el.channelChips.innerHTML = '';
  for (let ch = 0; ch < 16; ch++) {
    const chip = document.createElement('div');
    chip.className = 'ch-chip';
    chip.style.backgroundColor = CHANNEL_COLORS[ch];
    chip.style.color = '#000';
    chip.textContent = `Ch ${ch + 1}`;
    chip.dataset.ch = ch;

    chip.addEventListener('click', () => {
      state.activeChannels[ch] = !state.activeChannels[ch];
      chip.classList.toggle('disabled', !state.activeChannels[ch]);
    });

    el.channelChips.appendChild(chip);
  }
}

// Build 16 Bar Meters
function buildBarMeters() {
  el.barMetersContainer.innerHTML = '';
  for (let ch = 0; ch < 16; ch++) {
    const col = document.createElement('div');
    col.className = 'bar-col';
    col.innerHTML = `
      <div id="barVal_${ch}" class="bar-val">0.00</div>
      <div class="bar-track">
        <div id="barFill_${ch}" class="bar-fill" style="height: 0%;"></div>
      </div>
      <div class="bar-label">C${ch + 1}</div>
    `;
    el.barMetersContainer.appendChild(col);
  }
}

// Build Stats Table Rows
function buildStatsTable() {
  el.statsTableBody.innerHTML = '';
  for (let ch = 0; ch < 16; ch++) {
    const tr = document.createElement('tr');
    tr.id = `statsRow_${ch}`;
    tr.innerHTML = `
      <td style="color: ${CHANNEL_COLORS[ch]}; font-weight: bold;">Ch ${ch + 1}</td>
      <td id="stCur_${ch}">0.000</td>
      <td id="stMean_${ch}">0.000</td>
      <td id="stMin_${ch}">0.000</td>
      <td id="stMax_${ch}">0.000</td>
      <td id="stPP_${ch}">0.000</td>
      <td id="stRms_${ch}">0.000</td>
    `;
    el.statsTableBody.appendChild(tr);
  }
}

// Resize Canvas Buffer
function resizeCanvas() {
  const rect = el.waveformCanvas.parentElement.getBoundingClientRect();
  el.waveformCanvas.width = rect.width;
  el.waveformCanvas.height = rect.height;
}

let fpsFrameCount = 0;
let lastFpsTime = performance.now();
let currentFps = 0;

// Setup Event Listeners
function setupEventListeners() {
  // Connect / Disconnect
  el.btnConnect.addEventListener('click', handleConnect);
  el.btnDisconnect.addEventListener('click', handleDisconnect);

  // Scan Controls
  el.btnStartScan.addEventListener('click', async () => {
    try {
      await apiPost('start-scan');
      updateSystemState({ isConnected: true, isScanning: true });
      appendTerminalLog('Data acquisition started (Port 503 binary stream).');
    } catch (e) {
      alert(`스캔 시작 실패: ${e.message}`);
    }
  });

  el.btnStopScan.addEventListener('click', async () => {
    try {
      await apiPost('stop-scan');
      updateSystemState({ isConnected: true, isScanning: false });
      appendTerminalLog('Data acquisition stopped.');
    } catch (e) {
      alert(`스캔 중지 실패: ${e.message}`);
    }
  });

  el.btnCalz.addEventListener('click', handleCalz);

  // Valve Mode
  el.valveBtns.forEach(btn => {
    btn.addEventListener('click', async () => {
      const mode = btn.dataset.valve;
      try {
        await apiPost('valve', { mode });
        updateValveUI(mode);
        appendTerminalLog(`Solenoid valve mode switched to ${mode}.`);
      } catch (e) {
        alert(`밸브 전환 실패: ${e.message}`);
      }
    });
  });

  // Data Logging
  el.btnStartLog.addEventListener('click', async () => {
    const fileName = el.logFileName.value.trim();
    try {
      const res = await apiPost('logging/start', { fileName });
      updateLoggerUI({
        isRecording: true,
        fileName: fileName || 'Scan',
        recordedFrames: 0,
        recordedBytes: 0
      });
      appendTerminalLog(`CSV logging started: ${res.filePath || 'Active'}`);
    } catch (e) {
      alert(`로깅 시작 실패: ${e.message}`);
    }
  });

  el.btnStopLog.addEventListener('click', async () => {
    try {
      const res = await apiPost('logging/stop');
      updateLoggerUI({ isRecording: false });
      appendTerminalLog(`CSV logging stopped and saved: ${res.filePath || 'Complete'}`);
    } catch (e) {
      alert(`로깅 중지 실패: ${e.message}`);
    }
  });

  // Chart Controls
  el.selTimeRange.addEventListener('change', (e) => {
    state.timeRangeSec = parseFloat(e.target.value);
  });
  el.chkAutoScale.addEventListener('change', (e) => {
    state.autoScale = e.target.checked;
  });
  el.btnPauseChart.addEventListener('click', () => {
    state.isPaused = !state.isPaused;
    el.btnPauseChart.textContent = state.isPaused ? '계속 (Resume)' : '일시 정지';
    el.btnPauseChart.classList.toggle('btn-warning', state.isPaused);
  });
  el.btnResetChart.addEventListener('click', resetChartData);

  // Simulator checkbox auto-updates port defaults
  el.chkSimulator.addEventListener('change', (e) => {
    if (e.target.checked) {
      el.inputHost.value = '127.0.0.1';
      el.inputCmdPort.value = '2323';
      el.inputDataPort.value = '5503';
    } else {
      el.inputHost.value = '192.168.1.100';
      el.inputCmdPort.value = '23';
      el.inputDataPort.value = '503';
    }
  });

  // Modals
  el.btnOpenTerminal.addEventListener('click', () => el.modalTerminal.classList.add('active'));
  el.btnCloseTerminal.addEventListener('click', () => el.modalTerminal.classList.remove('active'));
  el.btnSendTerminal.addEventListener('click', sendTerminalCommand);
  el.terminalInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') sendTerminalCommand();
  });

  // Terminal Quick Buttons
  document.querySelectorAll('.terminal-quick-btns button').forEach(btn => {
    btn.addEventListener('click', () => {
      el.terminalInput.value = btn.dataset.cmd;
      sendTerminalCommand();
    });
  });

  el.btnOpenSettings.addEventListener('click', () => el.modalSettings.classList.add('active'));
  el.btnCloseSettings.addEventListener('click', () => el.modalSettings.classList.remove('active'));
  el.btnCancelSettings.addEventListener('click', () => el.modalSettings.classList.remove('active'));
  el.btnApplySettings.addEventListener('click', applySettings);
}

// WebSocket Connection
function connectWebSocket() {
  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const wsUrl = `${protocol}//${location.host}/ws`;
  state.ws = new WebSocket(wsUrl);

  state.ws.onopen = () => {
    console.log('WebSocket connected to Rust backend.');
  };

  state.ws.onmessage = (event) => {
    try {
      const msg = JSON.parse(event.data);
      handleWsMessage(msg);
    } catch (_) {}
  };

  state.ws.onclose = () => {
    setTimeout(connectWebSocket, 2000);
  };
}

// Handle WebSocket incoming messages
function handleWsMessage(msg) {
  switch (msg.type) {
    case 'init':
    case 'status':
      updateSystemState(msg.data);
      break;

    case 'frame':
      processIncomingFrame(msg.data);
      if (msg.stats) updateStatsUI(msg.stats);
      if (msg.logger) updateLoggerUI(msg.logger);
      break;

    case 'stats':
      updateStatsUI(msg.data);
      if (msg.logger) updateLoggerUI(msg.logger);
      break;

    case 'log':
      appendTerminalLog(`[${msg.data.level.toUpperCase()}] ${msg.data.msg}`);
      break;

    case 'valveChange':
      updateValveUI(msg.data.mode);
      break;

    case 'calzProgress':
      handleCalzProgress(msg.data);
      break;
  }
}

// Process Data Frame
function processIncomingFrame(packet) {
  if (!packet || !packet.modules || packet.modules.length === 0) return;

  // Live frame counter & FPS calculation
  fpsFrameCount++;
  const nowMs = performance.now();
  if (nowMs - lastFpsTime >= 1000) {
    currentFps = Math.round((fpsFrameCount * 1000) / (nowMs - lastFpsTime));
    fpsFrameCount = 0;
    lastFpsTime = nowMs;
    el.statFps.textContent = currentFps;
  }
  if (packet.frameNumber !== undefined) {
    el.statFrames.textContent = packet.frameNumber.toLocaleString();
  }

  const module0 = packet.modules[0];
  const pressures = module0.pressures;
  const now = packet.timestampMs || Date.now();

  // Add to chart buffer if not paused
  if (!state.isPaused) {
    state.dataBuffer.push({ time: now, pressures });

    // Prune buffer older than 35 seconds
    const cutoff = now - 35000;
    while (state.dataBuffer.length > 0 && state.dataBuffer[0].time < cutoff) {
      state.dataBuffer.shift();
    }
  }

  // Update Bar Meters & Running Stats
  for (let ch = 0; ch < 16; ch++) {
    const val = pressures[ch];
    const s = state.channelStats[ch];

    s.current = val;
    s.sum += val;
    s.count++;
    s.sumSq += val * val;
    if (val < s.min) s.min = val;
    if (val > s.max) s.max = val;

    // Update Bar Height
    const fillEl = document.getElementById(`barFill_${ch}`);
    const valEl = document.getElementById(`barVal_${ch}`);
    if (fillEl && valEl) {
      valEl.textContent = val.toFixed(2);
      const span = (state.yMax - state.yMin) || 1;
      const pct = Math.max(0, Math.min(100, ((val - state.yMin) / span) * 100));
      fillEl.style.height = `${pct}%`;
    }

    // Update Table (throttled)
    if (packet.frameNumber % 5 === 0) {
      const mean = s.count > 0 ? (s.sum / s.count) : val;
      const pp = (s.max !== -Infinity && s.min !== Infinity) ? (s.max - s.min) : 0;
      const variance = s.count > 1 ? Math.max(0, (s.sumSq / s.count) - (mean * mean)) : 0;
      const rms = Math.sqrt(variance);

      document.getElementById(`stCur_${ch}`).textContent = val.toFixed(3);
      document.getElementById(`stMean_${ch}`).textContent = mean.toFixed(3);
      document.getElementById(`stMin_${ch}`).textContent = (s.min === Infinity ? 0 : s.min).toFixed(3);
      document.getElementById(`stMax_${ch}`).textContent = (s.max === -Infinity ? 0 : s.max).toFixed(3);
      document.getElementById(`stPP_${ch}`).textContent = pp.toFixed(3);
      document.getElementById(`stRms_${ch}`).textContent = rms.toFixed(4);
    }
  }
}

// Render Oscilloscope-Style Chart
function renderChart() {
  requestAnimationFrame(renderChart);

  const w = el.waveformCanvas.width;
  const h = el.waveformCanvas.height;
  if (w === 0 || h === 0) return;

  // Background
  ctx.fillStyle = '#0f141c';
  ctx.fillRect(0, 0, w, h);

  // Margins
  const padLeft = 55;
  const padRight = 20;
  const padTop = 20;
  const padBottom = 30;
  const plotW = w - padLeft - padRight;
  const plotH = h - padTop - padBottom;

  // Compute Time Bounds
  const now = state.dataBuffer.length > 0 ? state.dataBuffer[state.dataBuffer.length - 1].time : Date.now();
  const tMin = now - (state.timeRangeSec * 1000);
  const tMax = now;

  // Compute Auto-Scale Y Bounds
  if (state.autoScale && state.dataBuffer.length > 0) {
    let minVal = Infinity;
    let maxVal = -Infinity;

    for (let pt of state.dataBuffer) {
      if (pt.time < tMin) continue;
      for (let ch = 0; ch < 16; ch++) {
        if (!state.activeChannels[ch]) continue;
        const v = pt.pressures[ch];
        if (v < minVal) minVal = v;
        if (v > maxVal) maxVal = v;
      }
    }

    if (minVal !== Infinity && maxVal !== -Infinity) {
      const margin = (maxVal - minVal) * 0.15 || 0.5;
      state.yMin = minVal - margin;
      state.yMax = maxVal + margin;
    }
  }

  // Draw Grid Lines
  ctx.strokeStyle = '#1e293b';
  ctx.lineWidth = 1;
  const gridRows = 6;
  const gridCols = 5;

  // Horizontal Grid & Labels
  ctx.fillStyle = '#8b9bb4';
  ctx.font = '10px Consolas, monospace';
  ctx.textAlign = 'right';
  ctx.textBaseline = 'middle';

  for (let i = 0; i <= gridRows; i++) {
    const y = padTop + (plotH / gridRows) * i;
    ctx.beginPath();
    ctx.moveTo(padLeft, y);
    ctx.lineTo(padLeft + plotW, y);
    ctx.stroke();

    const val = state.yMax - ((state.yMax - state.yMin) / gridRows) * i;
    ctx.fillText(val.toFixed(2), padLeft - 6, y);
  }

  // Vertical Grid & Time Labels
  ctx.textAlign = 'center';
  ctx.textBaseline = 'top';
  for (let i = 0; i <= gridCols; i++) {
    const x = padLeft + (plotW / gridCols) * i;
    ctx.beginPath();
    ctx.moveTo(x, padTop);
    ctx.lineTo(x, padTop + plotH);
    ctx.stroke();

    const secAgo = ((gridCols - i) / gridCols) * state.timeRangeSec;
    const label = secAgo === 0 ? '0s' : `-${secAgo.toFixed(1)}s`;
    ctx.fillText(label, x, padTop + plotH + 6);
  }

  // Draw Waveform Traces
  if (state.dataBuffer.length > 1) {
    ctx.save();
    ctx.beginPath();
    ctx.rect(padLeft, padTop, plotW, plotH);
    ctx.clip();

    for (let ch = 0; ch < 16; ch++) {
      if (!state.activeChannels[ch]) continue;

      ctx.strokeStyle = CHANNEL_COLORS[ch];
      ctx.lineWidth = 1.5;
      ctx.beginPath();

      let started = false;
      for (let i = 0; i < state.dataBuffer.length; i++) {
        const pt = state.dataBuffer[i];
        if (pt.time < tMin) continue;

        const x = padLeft + ((pt.time - tMin) / (tMax - tMin)) * plotW;
        const yNorm = (pt.pressures[ch] - state.yMin) / (state.yMax - state.yMin);
        const y = padTop + (1.0 - yNorm) * plotH;

        if (!started) {
          ctx.moveTo(x, y);
          started = true;
        } else {
          ctx.lineTo(x, y);
        }
      }
      ctx.stroke();
    }

    ctx.restore();
  }
}

// Reset Chart
function resetChartData() {
  state.dataBuffer = [];
  for (let ch = 0; ch < 16; ch++) {
    state.channelStats[ch] = {
      current: 0,
      sum: 0,
      count: 0,
      min: Infinity,
      max: -Infinity,
      sumSq: 0
    };
  }
}

// Connection Handlers
async function handleConnect() {
  const isSim = el.chkSimulator.checked;
  el.btnConnect.disabled = true;

  if (isSim) {
    appendTerminalLog('Starting internal DSA5000 Simulator (Rust) on 127.0.0.1:2323 / 5503...');
    await apiPost('simulator/start');
  }

  const payload = {
    host: el.inputHost.value.trim(),
    cmdPort: parseInt(el.inputCmdPort.value, 10),
    dataPort: parseInt(el.inputDataPort.value, 10)
  };

  try {
    const res = await apiPost('connect', payload);
    if (res.success) {
      appendTerminalLog(`Successfully connected to ${payload.host}`);
      updateSystemState({
        isConnected: true,
        isScanning: false,
        systemInfo: res.systemInfo
      });
    }
  } catch (err) {
    alert(`연결 실패: ${err.message}`);
    el.btnConnect.disabled = false;
  }
}

async function handleDisconnect() {
  el.btnDisconnect.disabled = true;
  await apiPost('disconnect');
  if (el.chkSimulator.checked) {
    await apiPost('simulator/stop');
  }
  updateSystemState({
    isConnected: false,
    isScanning: false
  });
  appendTerminalLog('Disconnected and sockets released.');
}

// Zero Calibration (CALZ)
async function handleCalz() {
  el.modalCalz.classList.add('active');
  el.calzStatusText.textContent = '영점 교정 명령 전송 중...';
  el.calzProgressBar.style.width = '10%';
  await apiPost('calz');
}

function handleCalzProgress(data) {
  if (data.status) el.calzStatusText.textContent = data.status;
  if (data.percent !== undefined) el.calzProgressBar.style.width = `${data.percent}%`;
  if (data.percent === 100) {
    setTimeout(() => {
      el.modalCalz.classList.remove('active');
    }, 800);
  }
}

// Update System State in UI
function updateSystemState(data) {
  state.isConnected = data.isConnected;
  state.isScanning = data.isScanning;

  el.connBadge.className = `status-badge ${data.isConnected ? (data.isScanning ? 'scanning' : 'connected') : 'disconnected'}`;
  el.connBadge.textContent = data.isConnected ? (data.isScanning ? 'ACQUIRING DATA' : 'CONNECTED') : 'DISCONNECTED';

  el.btnConnect.disabled = data.isConnected;
  el.btnDisconnect.disabled = !data.isConnected;
  el.btnStartScan.disabled = !data.isConnected || data.isScanning;
  el.btnStopScan.disabled = !data.isConnected || !data.isScanning;
  el.btnCalz.disabled = !data.isConnected || data.isCalibrating;

  if (data.systemInfo) {
    el.infoModel.textContent = data.systemInfo.model || '--';
    el.infoSN.textContent = data.systemInfo.serialNumber || '--';
    el.infoRate.textContent = data.systemInfo.scanRateHz || '--';
    el.infoUnits.textContent = data.systemInfo.engineeringUnits || 'PSI';
    updateValveUI(data.systemInfo.valveMode);
  }
}

function updateValveUI(mode) {
  const upper = (mode || 'PX').toUpperCase();
  el.valveBtns.forEach(btn => {
    btn.classList.toggle('active', btn.dataset.valve === upper);
  });
  const names = { PX: 'Px (계측 모드)', CAL: 'Cal (교정 모드)', PRG: 'Prg (퍼지 모드)', OFF: 'Off (차단)' };
  el.valveStatusText.textContent = names[upper] || upper;
}

function updateStatsUI(stats) {
  el.statFps.textContent = stats.currentFps || 0;
  el.statFrames.textContent = stats.totalFrames ? stats.totalFrames.toLocaleString() : '0';
  el.statDropped.textContent = stats.droppedFrames ? stats.droppedFrames.toLocaleString() : '0';
  el.statDropped.className = `stat-val ${(stats.droppedFrames > 0) ? 'stat-bad' : 'stat-good'}`;
}

function updateLoggerUI(logger) {
  el.btnStartLog.disabled = !state.isConnected || logger.isRecording;
  el.btnStopLog.disabled = !logger.isRecording;

  if (logger.isRecording) {
    el.logStatusText.textContent = `기록 중: ${logger.fileName}`;
    el.logFrameCount.textContent = `(${logger.recordedFrames.toLocaleString()} frames, ${(logger.recordedBytes / 1024).toFixed(1)} KB)`;
  } else {
    el.logStatusText.textContent = '대기 중 (Ready)';
    el.logFrameCount.textContent = '';
  }
}

// Terminal Commands
async function sendTerminalCommand() {
  const cmd = el.terminalInput.value.trim();
  if (!cmd) return;

  appendTerminalLog(`> ${cmd}`);
  el.terminalInput.value = '';

  try {
    const res = await apiPost('terminal', { command: cmd });
    if (res.response) {
      appendTerminalLog(res.response);
    }
  } catch (err) {
    appendTerminalLog(`ERROR: ${err.message}`);
  }
}

function appendTerminalLog(text) {
  el.terminalOutput.textContent += text + '\n';
  el.terminalOutput.scrollTop = el.terminalOutput.scrollHeight;
}

// Settings Apply
async function applySettings() {
  const rateHz = parseInt(el.cfgRate.value, 10);
  const units = el.cfgUnits.value;

  await apiPost('settings', { rateHz, units });
  el.modalSettings.classList.remove('active');
  appendTerminalLog(`Settings applied: Rate=${rateHz}Hz, Units=${units}`);
}

// REST API Helper
async function apiPost(endpoint, data = {}) {
  const res = await fetch(`/api/${endpoint}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(data)
  });
  const json = await res.json();
  if (!res.ok) throw new Error(json.error || 'Server error');
  return json;
}

// Start application
window.addEventListener('DOMContentLoaded', init);
