let websocket = null;
let uuid = "";
let actionInfo = {};
let context = "";
let action = "";
let settings = {};
let globalSettings = {};
let guilds = [];
let channels = [];
let channelsGuildId = "";
let autoLoadedAuthorizedState = false;
let guildRequestInFlight = false;
let guildRequestTimeoutId = null;
let channelsRequestGuildId = "";
let channelsRequestTimeoutId = null;
let debugLines = [];

const REQUEST_TIMEOUT_MS = 30000;

const clientIdInput = () => document.getElementById("client-id");
const clientSecretInput = () => document.getElementById("client-secret");
const redirectUriInput = () => document.getElementById("redirect-uri");
const guildSelect = () => document.getElementById("guild-select");
const channelSelect = () => document.getElementById("channel-select");
const statusBox = () => document.getElementById("status");
const debugLogBox = () => document.getElementById("debug-log");

function resolveGuildIconUrl(guild) {
  if (!guild) {
    return "";
  }

  if (guild.iconUrl) {
    return guild.iconUrl;
  }

  if (guild.icon) {
    return `https://cdn.discordapp.com/icons/${guild.id}/${guild.icon}.png?size=128`;
  }

  return "";
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
      maybeLoadChannelsForSelectedGuild();
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

  document.getElementById("save-credentials").addEventListener("click", () => {
    sendToPlugin({
      type: "saveCredentials",
      clientId: clientIdInput().value.trim(),
      clientSecret: clientSecretInput().value.trim(),
      redirectUri: redirectUriInput().value.trim(),
    });
    setStatus("Credentials saved locally in the plugin.", "success");
  });

  document.getElementById("connect-discord").addEventListener("click", () => {
    if (globalSettings.isAuthorized) {
      requestGuilds("Using existing Discord authorization...");
      return;
    }

    setStatus("Opening Discord authorization if needed...", "");
    clearGuildRequestTimeout();
    guildRequestInFlight = true;
    guildRequestTimeoutId = setTimeout(() => {
      guildRequestInFlight = false;
      setStatus("Discord authorization timed out. Please try Connect Discord again.", "error");
    }, REQUEST_TIMEOUT_MS);
    sendToPlugin({
      type: "connectDiscord",
      clientId: clientIdInput().value.trim(),
      clientSecret: clientSecretInput().value.trim(),
      redirectUri: redirectUriInput().value.trim(),
    });
  });

  document.getElementById("refresh-servers").addEventListener("click", () => {
    requestGuilds("Refreshing Discord servers...");
  });

  guildSelect().addEventListener("change", () => {
    const selectedGuild = guilds.find((guild) => guild.id === guildSelect().value);
    const nextGuildId = selectedGuild ? selectedGuild.id : "";
    const guildChanged = (settings.guildId || "") !== nextGuildId;
    const guildIconUrl = resolveGuildIconUrl(selectedGuild);

    updateActionSettings({
      guildId: nextGuildId,
      guildName: selectedGuild ? selectedGuild.name : "",
      guildIconUrl,
      channelId: guildChanged ? "" : settings.channelId || "",
      channelName: guildChanged ? "" : settings.channelName || "",
    });

    appendDebugLog(
      `resolved guild icon URL for ${selectedGuild ? selectedGuild.name : "(none)"}: ${guildIconUrl || "(empty)"}`,
    );

    if (!selectedGuild) {
      clearChannelsRequestTimeout();
      channelsRequestGuildId = "";
      channelsGuildId = "";
      replaceSelectOptions(channelSelect(), [{ value: "", label: "Choose a server first" }]);
      return;
    }

    requestChannelsForGuild(selectedGuild.id, selectedGuild.name);
  });

  channelSelect().addEventListener("change", () => {
    const selectedChannel = channels.find((channel) => channel.id === channelSelect().value);
    updateActionSettings({
      channelId: selectedChannel ? selectedChannel.id : "",
      channelName: selectedChannel ? selectedChannel.name : "",
    });
    if (selectedChannel) {
      setStatus(`Selected ${selectedChannel.name}.`, "success");
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

  if (payload.type === "guilds") {
    clearGuildRequestTimeout();
    guildRequestInFlight = false;
    guilds = Array.isArray(payload.guilds) ? payload.guilds : [];
    replaceSelectOptions(
      guildSelect(),
      guilds.length
        ? guilds.map((guild) => ({ value: guild.id, label: guild.name }))
        : [{ value: "", label: "No servers returned" }],
    );

    if (guilds.length) {
      appendDebugLog(
        `first guild icon payload: iconUrl='${guilds[0].iconUrl || ""}' icon='${guilds[0].icon || ""}'`,
      );
    }

    if (settings.guildId) {
      guildSelect().value = settings.guildId;
    }

    maybeLoadChannelsForSelectedGuild();

    setStatus(`Loaded ${guilds.length} server${guilds.length === 1 ? "" : "s"}.`, "success");
    return;
  }

  if (payload.type === "channels") {
    clearChannelsRequestTimeout();
    channels = Array.isArray(payload.channels) ? payload.channels : [];
    channelsGuildId = payload.guildId || "";
    channelsRequestGuildId = "";
    replaceSelectOptions(
      channelSelect(),
      channels.length
        ? channels.map((channel) => ({ value: channel.id, label: channel.name }))
        : [{ value: "", label: "No voice channels returned" }],
    );

    if (settings.channelId) {
      channelSelect().value = settings.channelId;
    }

    const selectedChannel =
      channels.find((channel) => channel.id === channelSelect().value) ||
      channels[0] ||
      null;

    if (selectedChannel) {
      channelSelect().value = selectedChannel.id;
      if ((settings.channelId || "") !== selectedChannel.id) {
        updateActionSettings({
          channelId: selectedChannel.id,
          channelName: selectedChannel.name || "",
        });
      }
    } else if (settings.channelId || settings.channelName) {
      updateActionSettings({
        channelId: "",
        channelName: "",
      });
    }

    setStatus(`Loaded ${channels.length} voice channel${channels.length === 1 ? "" : "s"}.`, "success");
    return;
  }

  if (payload.type === "status") {
    const level = payload.level || "";
    if (level === "error") {
      clearGuildRequestTimeout();
      guildRequestInFlight = false;
      clearChannelsRequestTimeout();
      channelsRequestGuildId = "";
    }
    setStatus(payload.message || "", payload.level || "");
  }

  if (payload.type === "log") {
    appendDebugLog(payload.message || "(empty log message)");
  }
}

function hydrateGlobalSettings() {
  clientIdInput().value = globalSettings.clientId || "";
  clientSecretInput().value = globalSettings.clientSecret || "";
  redirectUriInput().value = globalSettings.redirectUri || "http://localhost";
  if (globalSettings.isAuthorized) {
    setStatus("Discord is already authorized for this plugin.", "success");
    maybeAutoLoadAuthorizedState();
  }
}

function maybeAutoLoadAuthorizedState() {
  if (autoLoadedAuthorizedState || !globalSettings.isAuthorized) {
    return;
  }

  autoLoadedAuthorizedState = true;
  requestGuilds("Refreshing Discord servers...");
}

function hydrateActionSettings() {
  if (settings.guildId) {
    guildSelect().value = settings.guildId;
  }
  if (settings.channelId) {
    channelSelect().value = settings.channelId;
  }
}

function maybeLoadChannelsForSelectedGuild() {
  if (!settings.guildId) {
    return;
  }

  if (guildSelect().value !== settings.guildId) {
    return;
  }

  if (channelsGuildId === settings.guildId) {
    return;
  }

  const selectedGuild = guilds.find((guild) => guild.id === settings.guildId);
  requestChannelsForGuild(settings.guildId, selectedGuild ? selectedGuild.name : "selected server");
}

function requestGuilds(message) {
  if (guildRequestInFlight) {
    return;
  }

  setStatus(message, "");
  guildRequestInFlight = true;
  clearGuildRequestTimeout();
  guildRequestTimeoutId = setTimeout(() => {
    guildRequestInFlight = false;
    setStatus("Timed out while loading Discord servers. Try Refresh Servers again.", "error");
  }, REQUEST_TIMEOUT_MS);
  sendToPlugin({ type: "loadGuilds" });
}

function requestChannelsForGuild(guildId, guildName) {
  if (!guildId) {
    return;
  }

  if (channelsRequestGuildId === guildId) {
    return;
  }

  channelsRequestGuildId = guildId;
  clearChannelsRequestTimeout();
  setStatus(`Loading channels for ${guildName}...`, "");
  channelsRequestTimeoutId = setTimeout(() => {
    channelsRequestGuildId = "";
    setStatus(`Timed out while loading channels for ${guildName}.`, "error");
  }, REQUEST_TIMEOUT_MS);
  sendToPlugin({ type: "loadChannels", guildId });
}

function clearGuildRequestTimeout() {
  if (!guildRequestTimeoutId) {
    return;
  }

  clearTimeout(guildRequestTimeoutId);
  guildRequestTimeoutId = null;
}

function clearChannelsRequestTimeout() {
  if (!channelsRequestTimeoutId) {
    return;
  }

  clearTimeout(channelsRequestTimeoutId);
  channelsRequestTimeoutId = null;
}

function requestGlobalSettings() {
  send({ event: "getGlobalSettings", context: uuid });
}

function updateActionSettings(patch) {
  settings = { ...settings, ...patch };
  sendToPlugin({
    type: "saveActionSettings",
    guildId: settings.guildId || "",
    guildName: settings.guildName || "",
    guildIconUrl: settings.guildIconUrl || "",
    channelId: settings.channelId || "",
    channelName: settings.channelName || "",
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

function safeJson(value) {
  try {
    return JSON.parse(value);
  } catch {
    return null;
  }
}
