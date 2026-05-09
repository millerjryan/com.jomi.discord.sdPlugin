let websocket = null;
let uuid = "";
let actionInfo = {};
let context = "";
let action = "";
let settings = {};
let globalSettings = {};
let allSounds = [];
let guilds = [];
let soundsForSelectedGuild = [];
let autoLoadedAuthorizedState = false;
let soundsRequestInFlight = false;
let soundsRequestTimeoutId = null;
let debugLines = [];

const REQUEST_TIMEOUT_MS = 30000;

const clientIdInput = () => document.getElementById("client-id");
const clientSecretInput = () => document.getElementById("client-secret");
const redirectUriInput = () => document.getElementById("redirect-uri");
const guildSelect = () => document.getElementById("guild-select");
const soundSelect = () => document.getElementById("sound-select");
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
  settings = (actionInfo.payload && actionInfo.payload.settings) || {};

  websocket = new WebSocket(`ws://127.0.0.1:${inPort}`);
  websocket.onopen = () => {
    websocket.send(JSON.stringify({ event: inRegisterEvent, uuid }));
    sendToPlugin({ type: "loadState" });
    requestGlobalSettings();
    hydrateActionSettings();
    bindUi();
  };

  websocket.onmessage = (event) => {
    const message = safeJson(event.data);
    if (!message || !message.event) {
      return;
    }

    if (message.event === "didReceiveSettings") {
      settings = (message.payload && message.payload.settings) || {};
      hydrateActionSettings();
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
    sendToPlugin({
      type: "saveCredentials",
      ...buildCredentialPayload(),
    });
    setStatus("Credentials saved locally in the plugin.", "success");
  });

  document.getElementById("connect-discord").addEventListener("click", () => {
    if (globalSettings.isAuthorized) {
      requestSounds("Using existing Discord authorization...");
      return;
    }

    const creds = buildCredentialPayload();
    if (!creds.clientId.trim() || !creds.clientSecret.trim()) {
      setStatus(
        "Error: Client ID and Client Secret are required. Please enter them and click Save Credentials first.",
        "error",
      );
      return;
    }

    setStatus("Opening Discord authorization if needed...", "");
    clearSoundsRequestTimeout();
    soundsRequestInFlight = true;
    soundsRequestTimeoutId = setTimeout(() => {
      soundsRequestInFlight = false;
      setStatus("Discord authorization timed out. Please try Connect Discord again.", "error");
    }, REQUEST_TIMEOUT_MS);

    sendToPlugin({
      type: "connectDiscord",
      ...creds,
    });
  });

  document.getElementById("refresh-sounds").addEventListener("click", () => {
    requestSounds("Refreshing Discord soundboard sounds...");
  });

  guildSelect().addEventListener("change", () => {
    const selectedGuildId = guildSelect().value;
    const selectedGuild = guilds.find((guild) => guild.id === selectedGuildId);

    if (!selectedGuild) {
      soundsForSelectedGuild = [];
      replaceSelectOptions(soundSelect(), [{ value: "", label: "Choose a server first" }]);
      appendDebugLog("[Guild Change] No guild selected");
      return;
    }

    soundsForSelectedGuild = allSounds.filter((sound) => sound.guildId === selectedGuildId);
    appendDebugLog(`[Guild Change] Selected guild: ${selectedGuildId} (${selectedGuild.name})`);
    appendDebugLog(`[Guild Change] Total sounds available: ${allSounds.length}`);
    appendDebugLog(`[Guild Change] Sounds for selected guild: ${soundsForSelectedGuild.length}`);
    appendDebugLog(`[Guild Change] Filtered list: ${summarizeSounds(soundsForSelectedGuild)}`);
    
    replaceSelectOptions(
      soundSelect(),
      soundsForSelectedGuild.length
        ? soundsForSelectedGuild.map((sound) => ({ value: sound.soundId, label: sound.soundName }))
        : [{ value: "", label: "No sounds in this server" }],
    );

    if (settings.guildId === selectedGuildId && settings.soundId) {
      soundSelect().value = settings.soundId;
    }
  });

  soundSelect().addEventListener("change", () => {
    const selectedSoundId = soundSelect().value;
    const selectedSound = soundsForSelectedGuild.find((s) => s.soundId === selectedSoundId);

    if (selectedSound) {
      appendDebugLog(
        `[Sound Change] Selected sound: ${selectedSound.soundId} (${selectedSound.soundName}) in guild ${selectedSound.guildId}`,
      );
      updateActionSettings({
        guildId: selectedSound.guildId,
        guildName: selectedSound.guildName,
        soundId: selectedSound.soundId,
        soundName: selectedSound.soundName,
      });
      setStatus(`Selected ${selectedSound.soundName}.`, "success");
    }
  });
}

function handlePluginMessage(payload) {
  appendDebugLog(`< ${payload.type || "unknown"}`);

  if (payload.type === "globalState") {
    globalSettings = payload.settings || {};
    hydrateGlobalSettings();
    return;
  }

  if (payload.type === "soundboardSounds") {
    clearSoundsRequestTimeout();
    soundsRequestInFlight = false;
    allSounds = Array.isArray(payload.sounds) ? payload.sounds : [];

    appendDebugLog("[Plugin Message] Received soundboardSounds");
    appendDebugLog(`[Plugin Message] Total sounds: ${allSounds.length}`);
    appendDebugLog(`[Plugin Message] Sound list: ${summarizeSounds(allSounds)}`);
    
    // Extract unique guilds from sounds
    const guildMap = new Map();
    allSounds.forEach((sound) => {
      if (!guildMap.has(sound.guildId)) {
        guildMap.set(sound.guildId, { id: sound.guildId, name: sound.guildName || "Unknown Server" });
      }
    });
    guilds = Array.from(guildMap.values()).sort((a, b) => a.name.localeCompare(b.name));
    appendDebugLog(`[Plugin Message] Guilds: ${summarizeGuilds(guilds)}`);

    replaceSelectOptions(
      guildSelect(),
      guilds.length
        ? guilds.map((guild) => ({ value: guild.id, label: guild.name }))
        : [{ value: "", label: "No servers with sounds" }],
    );

    if (settings.guildId && guilds.find((g) => g.id === settings.guildId)) {
      guildSelect().value = settings.guildId;
      guildSelect().dispatchEvent(new Event("change"));
    } else if (guilds.length) {
      guildSelect().value = guilds[0].id;
      guildSelect().dispatchEvent(new Event("change"));
    }

    setStatus(`Loaded ${allSounds.length} soundboard sound${allSounds.length === 1 ? "" : "s"} from ${guilds.length} server${guilds.length === 1 ? "" : "s"}.`, "success");
    return;
  }

  if (payload.type === "status") {
    if ((payload.level || "") === "error") {
      clearSoundsRequestTimeout();
      soundsRequestInFlight = false;
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
    maybeAutoLoadAuthorizedState();
  }
}

function maybeAutoLoadAuthorizedState() {
  if (autoLoadedAuthorizedState || !globalSettings.isAuthorized) {
    return;
  }

  autoLoadedAuthorizedState = true;
  requestSounds("Refreshing Discord soundboard sounds...");
}

function hydrateActionSettings() {
  if (!settings.guildId || !settings.soundId) {
    return;
  }

  if (guilds.find((g) => g.id === settings.guildId)) {
    guildSelect().value = settings.guildId;
    guildSelect().dispatchEvent(new Event("change"));
    setTimeout(() => {
      soundSelect().value = settings.soundId;
    }, 0);
  }
}

function requestSounds(message) {
  if (soundsRequestInFlight) {
    return;
  }

  setStatus(message, "");
  soundsRequestInFlight = true;
  clearSoundsRequestTimeout();
  soundsRequestTimeoutId = setTimeout(() => {
    soundsRequestInFlight = false;
    setStatus("Timed out while loading soundboard sounds. Try Refresh Sounds again.", "error");
  }, REQUEST_TIMEOUT_MS);
  sendToPlugin({ type: "loadSoundboardSounds" });
}

function clearSoundsRequestTimeout() {
  if (!soundsRequestTimeoutId) {
    return;
  }

  clearTimeout(soundsRequestTimeoutId);
  soundsRequestTimeoutId = null;
}

function requestGlobalSettings() {
  send({ event: "getGlobalSettings", context: uuid });
}

function updateActionSettings(patch) {
  settings = { ...settings, ...patch };
  sendToPlugin({
    type: "saveSoundboardSettings",
    guildId: settings.guildId || "",
    guildName: settings.guildName || "",
    soundId: settings.soundId || "",
    soundName: settings.soundName || "",
  });
}

function sendToPlugin(payload) {
  appendDebugLog(`> ${payload.type || "unknown"}`);
  send({
    action,
    context,
    event: "sendToPlugin",
    payload,
  });
}

function send(message) {
  if (!websocket || websocket.readyState !== WebSocket.OPEN) {
    return;
  }
  websocket.send(JSON.stringify(message));
}

function replaceSelectOptions(select, entries) {
  select.innerHTML = "";
  entries.forEach((entry) => {
    const option = document.createElement("option");
    option.value = entry.value;
    option.textContent = entry.label;
    select.appendChild(option);
  });
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

function summarizeSounds(sounds) {
  if (!Array.isArray(sounds) || sounds.length === 0) {
    return "none";
  }

  const preview = sounds
    .slice(0, 15)
    .map((sound) => `${sound.guildId || "?"}:${sound.soundId || "?"}:${sound.soundName || "(unnamed)"}`)
    .join(" | ");
  if (sounds.length > 15) {
    return `${preview} | ... (+${sounds.length - 15} more)`;
  }
  return preview;
}

function summarizeGuilds(entries) {
  if (!Array.isArray(entries) || entries.length === 0) {
    return "none";
  }

  return entries.map((guild) => `${guild.id || "?"}:${guild.name || "(unnamed)"}`).join(" | ");
}

function safeJson(value) {
  try {
    return JSON.parse(value);
  } catch {
    return null;
  }
}
