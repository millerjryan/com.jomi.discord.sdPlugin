let websocket = null;
let uuid = "";
let actionInfo = {};
let context = "";
let action = "";
let globalSettings = {};
let guildRequestInFlight = false;
let guildRequestTimeoutId = null;
let debugLines = [];

const REQUEST_TIMEOUT_MS = 30000;

const clientIdInput = () => document.getElementById("client-id");
const clientSecretInput = () => document.getElementById("client-secret");
const redirectUriInput = () => document.getElementById("redirect-uri");
const statusBox = () => document.getElementById("status");
const debugLogBox = () => document.getElementById("debug-log");

function buildCredentialPayload() {
  return {
    clientId: clientIdInput().value.trim(),
    clientSecret: clientSecretInput().value.trim(),
    redirectUri: redirectUriInput().value.trim() || "http://localhost",
  };
}

function connectElgatoStreamDeckSocket(inPort, inUUID, inRegisterEvent, inInfo, inActionInfo) {
  uuid = inUUID;
  actionInfo = safeJson(inActionInfo) || {};
  context = actionInfo.context || "";
  action = actionInfo.action || "";

  console.log("connectElgatoStreamDeckSocket called:", { inPort, uuid, context, action });
  appendDebugLog(`connectElgatoStreamDeckSocket: uuid='${uuid}' action='${action}' context='${context}'`);

  websocket = new WebSocket(`ws://127.0.0.1:${inPort}`);
  websocket.onopen = () => {
    websocket.send(JSON.stringify({ event: inRegisterEvent, uuid }));
    sendToPlugin({ type: "loadState" });
    requestGlobalSettings();
    bindUi();
  };

  websocket.onmessage = (event) => {
    const message = safeJson(event.data);
    if (!message || !message.event) {
      return;
    }

    if (message.event === "didReceiveGlobalSettings") {
      globalSettings = (message.payload && message.payload.settings) || {};
      hydrateGlobalSettings();
      return;
    }

    if (message.event === "sendToPropertyInspector") {
      handlePluginMessage(message.payload || {});
    }
  };
}

function bindUi() {
  if (bindUi.done) {
    return;
  }
  bindUi.done = true;

  if (!redirectUriInput().value.trim()) {
    redirectUriInput().value = "http://localhost";
  }

  document.getElementById("save-credentials").addEventListener("click", () => {
    const creds = buildCredentialPayload();
    appendDebugLog(`Save Credentials clicked: action='${action}' context='${context}' clientId='${creds.clientId.substring(0, 8)}...'`);
    sendToPlugin({
      type: "saveCredentials",
      ...creds,
    });
    setStatus("Credentials saved locally in the plugin.", "success");
  });

  document.getElementById("connect-discord").addEventListener("click", () => {
    appendDebugLog(`Connect Discord clicked: action='${action}' context='${context}'`);
    if (guildRequestInFlight) {
      appendDebugLog("Guild request already in flight, ignoring click");
      return;
    }

    const creds = buildCredentialPayload();
    appendDebugLog(`Payload: clientId='${creds.clientId.substring(0, 8) || "(empty)"}...' clientSecret_present=${!!creds.clientSecret} redirectUri='${creds.redirectUri}'`);
    if (!creds.clientId.trim() || !creds.clientSecret.trim()) {
      setStatus("Error: Client ID and Client Secret are required. Please enter them and click Save Credentials first.", "error");
      appendDebugLog("Credentials validation failed");
      return;
    }

    setStatus("Opening Discord authorization if needed...", "");
    guildRequestInFlight = true;
    clearGuildRequestTimeout();
    guildRequestTimeoutId = setTimeout(() => {
      guildRequestInFlight = false;
      setStatus("Discord authorization timed out. Please try Connect Discord again.", "error");
      appendDebugLog("Connect Discord timed out");
    }, REQUEST_TIMEOUT_MS);

    appendDebugLog("Sending connectDiscord request");
    sendToPlugin({
      type: "connectDiscord",
      ...creds,
    });
  });
}

function handlePluginMessage(payload) {
  appendDebugLog(`< ${payload.type || "unknown"}`);

  if (payload.type === "globalState") {
    globalSettings = payload.settings || {};
    hydrateGlobalSettings();
    return;
  }

  if (payload.type === "guilds") {
    clearGuildRequestTimeout();
    guildRequestInFlight = false;
    const count = Array.isArray(payload.guilds) ? payload.guilds.length : 0;
    setStatus(`Connected. Loaded ${count} server${count === 1 ? "" : "s"}.`, "success");
    return;
  }

  if (payload.type === "status") {
    if ((payload.level || "") === "error") {
      clearGuildRequestTimeout();
      guildRequestInFlight = false;
    }
    setStatus(payload.message || "", payload.level || "");
    return;
  }

  if (payload.type === "log") {
    appendDebugLog(payload.message || "(empty log message)");
  }
}

function hydrateGlobalSettings() {
  const clientId = (globalSettings.clientId || "").trim();
  const clientSecret = (globalSettings.clientSecret || "").trim();
  const redirectUri = (globalSettings.redirectUri || "").trim();

  if (clientId) {
    clientIdInput().value = clientId;
  }
  if (clientSecret) {
    clientSecretInput().value = clientSecret;
  }
  redirectUriInput().value = redirectUri || "http://localhost";

  if (clientId && clientSecret) {
    setStatus("Using shared global credentials/token from other Discord actions", "success");
  }

  if (globalSettings.isAuthorized) {
    setStatus("Using shared global credentials/token from other Discord actions", "success");
  }
}

function clearGuildRequestTimeout() {
  if (!guildRequestTimeoutId) {
    return;
  }

  clearTimeout(guildRequestTimeoutId);
  guildRequestTimeoutId = null;
}

function requestGlobalSettings() {
  send({ event: "getGlobalSettings", context: uuid });
}

function sendToPlugin(payload) {
  appendDebugLog(`> ${payload.type || "unknown"}`);
  const message = {
    action,
    context,
    event: "sendToPlugin",
    payload,
  };
  appendDebugLog(`sendToPlugin: action='${action}' context='${context}'`);
  send(message);
}

function send(message) {
  if (!websocket) {
    appendDebugLog(`ERROR: websocket is null`);
    return;
  }
  if (websocket.readyState !== WebSocket.OPEN) {
    appendDebugLog(`ERROR: websocket not open (readyState=${websocket.readyState})`);
    return;
  }
  appendDebugLog(`send: sending message...`);
  websocket.send(JSON.stringify(message));
}

function setStatus(text, level) {
  const element = statusBox();
  element.textContent = text || "Idle.";
  element.className = `status${level ? ` ${level}` : ""}`;
}

function appendDebugLog(line) {
  const timestamp = new Date().toLocaleTimeString();
  debugLines.push(`[${timestamp}] ${line}`);
  if (debugLines.length > 120) {
    debugLines = debugLines.slice(debugLines.length - 120);
  }

  const box = debugLogBox();
  if (!box) {
    return;
  }

  box.textContent = debugLines.join("\n");
  box.scrollTop = box.scrollHeight;
}
