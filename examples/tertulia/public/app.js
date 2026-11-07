// Tertulia browser client: plain JS on top of the gateway protocol
// ({ event, data, id? } JSON frames). See src/chat/chat_gateway.rs.
"use strict";

const $ = (id) => document.getElementById(id);
const state = {
  token: sessionStorage.getItem("tertulia.token"),
  user: null,
  ws: null,
  nextId: 1,
  pending: new Map(), // id -> { resolve, reject }
  rooms: new Map(), // name -> room (with members)
  current: null,
  typingUsers: new Map(), // username -> timeout
  typingSent: false,
  typingTimer: null,
  stats: null,
};

// ---------------------------------------------------------------- REST

async function api(path, { method = "GET", body } = {}) {
  const headers = { "content-type": "application/json" };
  if (state.token) headers.authorization = `Bearer ${state.token}`;
  const res = await fetch(`/api${path}`, { method, headers, body: body ? JSON.stringify(body) : undefined });
  const data = await res.json().catch(() => null);
  if (!res.ok) throw new Error(errorText(data) || `HTTP ${res.status}`);
  return data;
}

function errorText(data) {
  if (!data) return "";
  if (Array.isArray(data.errors) && data.errors.length) return data.errors.map((e) => e.message).join("; ");
  return Array.isArray(data.message) ? data.message.join("; ") : data.message;
}

// ----------------------------------------------------------- WebSocket

function connect() {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  const ws = new WebSocket(`${proto}//${location.host}/chat?token=${encodeURIComponent(state.token)}`);
  state.ws = ws;
  setConnection("Conectando…");
  ws.onmessage = (e) => handle(JSON.parse(e.data));
  ws.onclose = (e) => {
    state.ws = null;
    for (const p of state.pending.values()) p.reject(new Error("Conexión cerrada"));
    state.pending.clear();
    setComposer(false);
    if (e.code === 4401) return logout("Tu sesión no es válida; vuelve a entrar.");
    setConnection("Desconectado");
    showError("Se perdió la conexión. Reintentando en 3 s…");
    setTimeout(() => state.token && !state.ws && connect(), 3000);
  };
}

/** Sends a frame and resolves with its reply (matched by `id`), or rejects with the exception. */
function request(event, data) {
  return new Promise((resolve, reject) => {
    if (!state.ws || state.ws.readyState !== WebSocket.OPEN) return reject(new Error("Sin conexión"));
    const id = state.nextId++;
    state.pending.set(id, { resolve, reject });
    state.ws.send(JSON.stringify({ event, data, id }));
  });
}

function emit(event, data) {
  if (state.ws && state.ws.readyState === WebSocket.OPEN) state.ws.send(JSON.stringify({ event, data }));
}

function handle(frame) {
  const { event, data, id } = frame;
  if (id !== undefined && state.pending.has(id)) {
    const p = state.pending.get(id);
    state.pending.delete(id);
    return event === "exception" ? p.reject(new Error(errorText(data) || "Error")) : p.resolve(data);
  }
  switch (event) {
    case "welcome":
      setConnection("Conectado");
      showError("");
      setRooms(data.rooms);
      if (state.current) joinRoom(state.current);
      break;
    case "room_created":
      upsertRoom({ ...data.room, members: data.members });
      break;
    case "user_joined":
    case "user_left":
      updateMembers(data.room, data.members);
      if (data.room === state.current) {
        addNotice(`${data.user.username} ${event === "user_joined" ? "entró" : "salió"}.`);
        if (event === "user_left") stopTyping(data.user.username);
      }
      break;
    case "message":
      if (data.room === state.current) {
        stopTyping(data.username);
        addMessage(data);
      }
      break;
    case "typing":
      if (data.room === state.current) data.typing ? startTyping(data.user) : stopTyping(data.user);
      break;
    case "announcement":
      if (data.room === state.current) showBanner(`Anuncio de ${data.from}: ${data.text}`);
      break;
    case "cleared":
      if (data.room === state.current) {
        $("messages").replaceChildren();
        addNotice(`${data.by} borró el historial (${data.deleted} mensajes).`);
      }
      break;
    case "exception":
      showError(errorText(data));
      break;
  }
}

// --------------------------------------------------------------- rooms

function setRooms(list) {
  state.rooms = new Map(list.map((r) => [r.name, r]));
  renderRooms();
}

function upsertRoom(room) {
  state.rooms.set(room.name, room);
  renderRooms();
}

function updateMembers(name, members) {
  const room = state.rooms.get(name);
  if (!room) return;
  room.members = members;
  renderRooms();
  if (name === state.current) renderMembers();
}

function renderRooms() {
  const items = [...state.rooms.values()].map((room) => {
    const li = document.createElement("li");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.setAttribute("aria-current", String(room.name === state.current));
    const name = document.createElement("span");
    name.textContent = `#${room.name}`;
    const count = document.createElement("span");
    count.className = "count";
    count.textContent = `${room.members.length}/${room.maxMembers}`;
    count.title = "Conectados / capacidad";
    btn.append(name, count);
    btn.onclick = () => joinRoom(room.name);
    li.append(btn);
    return li;
  });
  $("rooms").replaceChildren(...items);
}

async function joinRoom(name) {
  if (state.current && state.current !== name) {
    await request("leave", { room: state.current }).catch(() => {});
  }
  try {
    const joined = await request("join", { room: name });
    state.current = name;
    upsertRoom({ ...joined.room, members: joined.members });
    $("room-title").textContent = `#${name}`;
    $("room-topic-line").textContent = joined.room.topic;
    $("messages").replaceChildren();
    if (!joined.history.length) addNotice("Todavía no hay mensajes. Escribe el primero.", "first-run");
    joined.history.forEach(addMessage);
    renderMembers();
    $("banner").hidden = true;
    state.typingUsers.forEach((t) => clearTimeout(t));
    state.typingUsers.clear();
    renderTyping();
    setComposer(true);
    showError("");
    $("text").focus();
  } catch (err) {
    showError(err.message);
  }
}

async function leaveRoom() {
  if (!state.current) return;
  await request("leave", { room: state.current }).catch((e) => showError(e.message));
  state.current = null;
  $("room-title").textContent = "Elige una sala";
  $("room-topic-line").textContent = "";
  $("members").textContent = "";
  $("messages").replaceChildren();
  addNotice("Únete a una sala de la izquierda para leer y escribir.");
  setComposer(false);
  renderRooms();
}

function renderMembers() {
  const room = state.rooms.get(state.current);
  const names = room ? room.members.map((m) => m.username) : [];
  $("members").textContent = names.length ? `En la sala: ${names.join(", ")}` : "";
}

// ------------------------------------------------------------ messages

function addMessage(msg, pending = false) {
  document.querySelector("#messages .first-run")?.remove();
  const li = document.createElement("li");
  if (msg.username === state.user.username) li.classList.add("own");
  if (pending) li.classList.add("pending");
  const who = document.createElement("span");
  who.className = "who";
  who.textContent = msg.username;
  const text = document.createElement("span");
  text.textContent = msg.text;
  li.append(who, text);
  if (msg.createdAt) {
    const time = document.createElement("time");
    time.dateTime = msg.createdAt;
    time.textContent = new Date(msg.createdAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    li.append(time);
  }
  $("messages").append(li);
  li.scrollIntoView({ block: "end" });
  return li;
}

function addNotice(text, extraClass) {
  const li = document.createElement("li");
  li.className = extraClass ? `notice ${extraClass}` : "notice";
  li.textContent = text;
  $("messages").append(li);
}

async function sendMessage(e) {
  e.preventDefault();
  const input = $("text");
  const text = input.value.trim();
  if (!text || !state.current) return;
  input.value = "";
  sendTyping(false);
  // Optimistic: shown at once, confirmed by the `sent` ack (same id).
  const li = addMessage({ username: state.user.username, text }, true);
  try {
    const saved = await request("message", { room: state.current, text });
    li.replaceWith(addMessage(saved));
  } catch (err) {
    li.remove();
    input.value = text;
    showError(err.message);
  }
}

// -------------------------------------------------------------- typing

function onInput() {
  if (!state.current) return;
  if (!state.typingSent) sendTyping(true);
  clearTimeout(state.typingTimer);
  state.typingTimer = setTimeout(() => sendTyping(false), 2000);
}

function sendTyping(typing) {
  if (!state.current || state.typingSent === typing) return;
  state.typingSent = typing;
  emit("typing", { room: state.current, typing });
}

function startTyping(user) {
  clearTimeout(state.typingUsers.get(user));
  state.typingUsers.set(user, setTimeout(() => stopTyping(user), 5000));
  renderTyping();
}

function stopTyping(user) {
  clearTimeout(state.typingUsers.get(user));
  state.typingUsers.delete(user);
  renderTyping();
}

function renderTyping() {
  const names = [...state.typingUsers.keys()];
  $("typing").textContent =
    names.length === 0 ? "" : names.length === 1 ? `${names[0]} está escribiendo…` : `${names.join(", ")} están escribiendo…`;
}

// ------------------------------------------------------------- helpers

function setConnection(text) {
  $("connection").textContent = text;
}

function setComposer(enabled) {
  $("text").disabled = !enabled;
  $("send").disabled = !enabled;
  $("leave").hidden = !enabled;
  const mod = enabled && state.user && state.user.role === "moderator";
  $("clear").hidden = !mod;
  $("announce").hidden = !mod;
}

function showError(text) {
  $("chat-error").textContent = text || "";
}

function showBanner(text) {
  const b = $("banner");
  b.textContent = text;
  b.hidden = false;
}

function renderStats(s) {
  const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
  $("stats").textContent = [
    `${plural(s.onlineUsers.length, "persona", "personas")} en línea`,
    plural(s.connections, "conexión", "conexiones"),
    plural(s.messages, "mensaje guardado", "mensajes guardados"),
  ].join(" · ") + " (vía SSE)";
}

// ------------------------------------------------------------ sessions

function enter(user) {
  state.user = user;
  $("me-name").textContent = user.username;
  $("me-role").textContent = user.role === "moderator" ? "moderador" : "miembro";
  $("login").hidden = true;
  $("app").hidden = false;
  connect();
}

function logout(message) {
  sessionStorage.removeItem("tertulia.token");
  state.token = null;
  state.current = null;
  if (state.ws) state.ws.close();
  $("app").hidden = true;
  $("login").hidden = false;
  $("login-error").textContent = message || "";
}

$("login-form").onsubmit = async (e) => {
  e.preventDefault();
  $("login-error").textContent = "";
  try {
    const { accessToken, user } = await api("/auth/login", { method: "POST", body: { username: $("username").value } });
    state.token = accessToken;
    sessionStorage.setItem("tertulia.token", accessToken);
    enter(user);
  } catch (err) {
    $("login-error").textContent = err.message;
  }
};

$("create-room").onsubmit = async (e) => {
  e.preventDefault();
  $("create-error").textContent = "";
  try {
    const room = await api("/rooms", { method: "POST", body: { name: $("room-name").value, topic: $("room-topic").value } });
    e.target.reset();
    joinRoom(room.name);
  } catch (err) {
    $("create-error").textContent = err.message;
  }
};

$("announce").onsubmit = async (e) => {
  e.preventDefault();
  const text = $("announce-text").value.trim();
  if (!text || !state.current) return;
  try {
    await api(`/rooms/${state.current}/announcements`, { method: "POST", body: { text } });
    $("announce-text").value = "";
  } catch (err) {
    showError(err.message);
  }
};

$("clear").onclick = () => {
  if (state.current && confirm(`¿Borrar todo el historial de #${state.current}?`)) {
    request("clear", { room: state.current }).catch((err) => showError(err.message));
  }
};
$("leave").onclick = leaveRoom;
$("logout").onclick = () => logout();
$("composer").onsubmit = sendMessage;
$("text").oninput = onInput;

// Live numbers over Server-Sent Events.
new EventSource("/api/stats/stream").addEventListener("stats", (e) => renderStats(JSON.parse(e.data)));

if (state.token) {
  api("/auth/me").then(enter, () => logout());
}
