/// Thrum web client: a small chat client over the same protocol as the
/// CLI. Screens: register/login, chat list, new chat, conversation, group
/// details, devices. All rendering is plain DOM, all state lives in
/// IndexedDB plus the CryptoSession.

import { ApiError } from "./api";
import { LiveConnection } from "./live";
import { CryptoSession } from "./session";
import {
  loadConversations,
  loadMessages,
  saveMessage,
  type StoredMessage,
} from "./db";

const screen = document.getElementById("screen") as HTMLElement;

let session: CryptoSession | null = null;
let live: LiveConnection | null = null;
let connected = false;
/** Unsent composer text per conversation, so live re-renders keep it. */
const drafts: Record<string, string> = {};

let renderChain: Promise<void> = Promise.resolve();

/**
 * Queue one screen render; only one runs at a time. A burst of triggers
 * (a click's navigation, a live-refresh after incoming frames) must not
 * interleave at await points and stack two partial screens — two headers
 * or two message lists — into the DOM. Each job reads the current hash
 * when it runs, so a queued render behind a newer one still shows the
 * newest state.
 */
function enqueueRender(job: () => Promise<void>): Promise<void> {
  const run = renderChain.then(job, job);
  renderChain = run.catch(() => {});
  return run;
}

/** Render the current route; replaces the screen's content. */
function render(): Promise<void> {
  return enqueueRender(async () => {
    const hash = window.location.hash || "#/";
    const parts = hash.slice(2).split("/"); // "#/x/y" -> ["x","y"]
    if (!session) {
      if (parts[0] === "login") renderAuth();
      else window.location.hash = "#/login";
      return;
    }
    switch (parts[0]) {
      case "":
        window.location.hash = "#/chats";
        break;
      case "chats":
        await renderChats();
        break;
      case "new":
        renderNewChat(parts[1] === "invite");
        break;
      case "chat":
        if (parts[1]) await renderConversation(parts[1]);
        break;
      case "group":
        if (parts[1]) await renderGroup(parts[1]);
        break;
      case "devices":
        await renderDevices();
        break;
      default:
        window.location.hash = "#/chats";
    }
  });
}

window.addEventListener("hashchange", () => {
  void render();
});

/** Error text for an API failure, for inline display near forms. */
function errorText(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.code === "invalid_credentials") return "Wrong username or password.";
    if (error.code === "username_taken") return "That name is already taken.";
    if (error.code === "too_large") return "That file is over this server's size limit.";
    if (error.code === "quota_exceeded") return "Daily upload quota reached on this server.";
    if (error.code === "forbidden") return "Only the group's admin can do that.";
    if (error.status === 0) return `Cannot reach the server (${error.code}).`;
    return error.message;
  }
  return String(error);
}

function header(title: string, actions: HTMLElement[] = []): HTMLElement {
  const bar = document.createElement("header");
  bar.className = "topbar";
  const heading = document.createElement("h1");
  heading.textContent = title;
  bar.append(heading);
  if (connected) {
    const status = document.createElement("span");
    status.className = "status ok";
    status.textContent = "connected";
    bar.append(status);
  } else {
    const status = document.createElement("span");
    status.className = "status";
    status.textContent = "offline";
    bar.append(status);
  }
  for (const action of actions) bar.append(action);
  return bar;
}

function navLink(label: string, hash: string): HTMLAnchorElement {
  const link = document.createElement("a");
  link.href = hash;
  link.textContent = label;
  return link;
}

function logoutLink(): HTMLAnchorElement {
  const link = document.createElement("a");
  link.href = "#";
  link.textContent = "Log out";
  link.addEventListener("click", (event) => {
    event.preventDefault();
    void (async () => {
      live?.stop();
      await session?.logout();
      session = null;
      window.location.hash = "#/login";
      render();
    })();
  });
  return link;
}

// --- register / login ---

function renderAuth(): void {
  screen.replaceChildren();
  const main = document.createElement("main");
  main.className = "auth";
  const heading = document.createElement("h1");
  heading.textContent = "Thrum";
  const tagline = document.createElement("p");
  tagline.textContent = "Small, end-to-end encrypted chat. Keys stay in this browser.";
  const form = document.createElement("form");
  const nameLabel = document.createElement("label");
  nameLabel.textContent = "Username";
  const nameInput = document.createElement("input");
  nameInput.name = "username";
  nameInput.autocomplete = "username";
  nameInput.required = true;
  nameInput.spellcheck = false;
  nameLabel.append(nameInput);
  const passLabel = document.createElement("label");
  passLabel.textContent = "Password";
  const passInput = document.createElement("input");
  passInput.name = "password";
  passInput.type = "password";
  passInput.autocomplete = "current-password";
  passInput.required = true;
  passLabel.append(passInput);
  const submit = document.createElement("button");
  submit.type = "submit";
  submit.textContent = "Log in";
  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "linkish";
  toggle.textContent = "Create an account";
  const error = document.createElement("p");
  error.className = "error";
  error.setAttribute("role", "alert");
  error.hidden = true;
  let registering = false;
  toggle.addEventListener("click", () => {
    registering = !registering;
    submit.textContent = registering ? "Create account" : "Log in";
    toggle.textContent = registering ? "I have an account" : "Create an account";
    passInput.autocomplete = registering ? "new-password" : "current-password";
    error.hidden = true;
  });
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    void (async () => {
      error.hidden = true;
      submit.disabled = true;
      const server = window.location.origin;
      try {
        if (registering) {
          session = await CryptoSession.register(
            server,
            nameInput.value.trim(),
            passInput.value,
          );
        } else {
          session = await CryptoSession.login(
            server,
            nameInput.value.trim(),
            passInput.value,
          );
        }
        await session.sync();
        startLive();
        window.location.hash = "#/chats";
        render();
      } catch (caught) {
        error.textContent = errorText(caught);
        error.hidden = false;
      } finally {
        submit.disabled = false;
      }
    })();
  });
  form.append(nameLabel, passLabel, submit);
  main.append(heading, tagline, form, toggle, error);
  screen.append(main);
  nameInput.focus();
}

// --- chat list ---

async function renderChats(): Promise<void> {
  screen.replaceChildren();
  const newChat = document.createElement("a");
  newChat.className = "button";
  newChat.href = "#/new";
  newChat.textContent = "New chat";
  const devices = navLink("Devices", "#/devices");
  screen.append(header("Chats", [devices, newChat, logoutLink()]));
  try {
    await session?.sync();
  } catch (error) {
    showNotice(errorText(error));
  }
  const conversations = await loadConversations();
  const list = document.createElement("ul");
  list.className = "chat-list";
  for (const conversation of conversations) {
    const item = document.createElement("li");
    const link = document.createElement("a");
    link.href = `#/chat/${conversation.id}`;
    const label = session
      ? conversation.name ||
        conversation.members.filter((member) => member !== session?.current?.username).join(", ") ||
        conversation.members.join(", ")
      : conversation.id;
    link.textContent = label;
    item.append(link);
    const details = document.createElement("a");
    details.className = "details-link";
    details.href = `#/group/${conversation.id}`;
    details.textContent = "details";
    details.setAttribute("aria-label", `details of ${label}`);
    item.append(details);
    list.append(item);
  }
  if (conversations.length === 0) {
    const empty = document.createElement("li");
    empty.textContent = "No conversations yet.";
    list.append(empty);
  }
  screen.append(list);
}

// --- new chat ---

function renderNewChat(inviteMode: boolean): void {
  screen.replaceChildren();
  const back = navLink("Back", "#/chats");
  screen.append(header(inviteMode ? "Join with invite" : "New chat", [back]));
  const main = document.createElement("div");
  main.className = "panel";
  const form = document.createElement("form");
  const error = document.createElement("p");
  error.className = "error";
  error.setAttribute("role", "alert");
  error.hidden = true;
  let membersInput: HTMLInputElement | null = null;
  let nameInput: HTMLInputElement | null = null;
  if (inviteMode) {
    const label = document.createElement("label");
    label.textContent = "Invite token";
    const input = document.createElement("input");
    input.name = "token";
    input.required = true;
    label.append(input);
    form.append(label);
  } else {
    const label = document.createElement("label");
    label.textContent = "Members (comma-separated usernames)";
    membersInput = document.createElement("input");
    membersInput.name = "members";
    membersInput.required = true;
    membersInput.spellcheck = false;
    label.append(membersInput);
    const nameLabel = document.createElement("label");
    nameLabel.textContent = "Group name (optional)";
    nameInput = document.createElement("input");
    nameInput.name = "name";
    nameLabel.append(nameInput);
    form.append(label, nameLabel);
  }
  const submit = document.createElement("button");
  submit.type = "submit";
  submit.textContent = inviteMode ? "Join" : "Create";
  form.append(submit);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    void (async () => {
      error.hidden = true;
      submit.disabled = true;
      try {
        if (inviteMode) {
          const token = (form.elements.namedItem("token") as HTMLInputElement).value.trim();
          const id = await session!.joinConversation(token);
          window.location.hash = `#/chat/${id}`;
          return;
        }
        const members = (membersInput?.value ?? "")
          .split(",")
          .map((member) => member.trim().toLowerCase())
          .filter(Boolean);
        const name = nameInput?.value.trim();
        const id = await session!.createConversation(
          members,
          name ? name : undefined,
        );
        window.location.hash = `#/chat/${id}`;
      } catch (caught) {
        error.textContent = errorText(caught);
        error.hidden = false;
      } finally {
        submit.disabled = false;
      }
    })();
  });
  const switchLine = document.createElement("p");
  const switchLink = document.createElement("a");
  switchLink.href = inviteMode ? "#/new" : "#/new/invite";
  switchLink.textContent = inviteMode ? "Start a new chat instead" : "Have an invite token?";
  switchLine.append(switchLink);
  main.append(form, switchLine, error);
  screen.append(main);
}

// --- conversation view ---

async function renderConversation(conversationId: string): Promise<void> {
  const details = navLink("Details", `#/group/${conversationId}`);
  screen.replaceChildren();
  const label = await labelFor(conversationId);
  screen.append(header(label, [navLink("Chats", "#/chats"), details, logoutLink()]));
  const messages = await loadMessages(conversationId);
  messages.sort((a, b) => a.createdAt - b.createdAt);
  const list = document.createElement("ol");
  list.className = "messages";
  list.setAttribute("aria-label", `messages in ${label}`);
  for (const message of messages) {
    list.append(messageItem(conversationId, message));
  }
  screen.append(list);
  const composer = document.createElement("form");
  composer.className = "composer";
  const input = document.createElement("input");
  input.id = "composer-input";
  input.name = "text";
  input.placeholder = "Write a message";
  input.setAttribute("aria-label", "message text");
  input.autocomplete = "off";
  // Drafts survive live re-renders triggered by incoming frames.
  input.value = drafts[conversationId] ?? "";
  input.addEventListener("input", () => {
    drafts[conversationId] = input.value;
  });
  const send = document.createElement("button");
  send.type = "submit";
  send.textContent = "Send";
  const attach = document.createElement("button");
  attach.type = "button";
  attach.textContent = "Attach";
  const fileInput = document.createElement("input");
  fileInput.type = "file";
  fileInput.hidden = true;
  fileInput.setAttribute("aria-label", "choose a file to attach");
  attach.addEventListener("click", () => fileInput.click());
  fileInput.addEventListener("change", () => {
    const file = fileInput.files?.[0];
    if (!file) return;
    void (async () => {
      attach.disabled = true;
      try {
        await session!.sendAttachment(conversationId, file);
        await render();
      } catch (caught) {
        showNotice(errorText(caught));
      } finally {
        attach.disabled = false;
      }
    })();
  });
  composer.addEventListener("submit", (event) => {
    event.preventDefault();
    const text = input.value.trim();
    if (!text) return;
    void (async () => {
      send.disabled = true;
      try {
        await session!.sendText(conversationId, text);
        delete drafts[conversationId];
        await render();
      } catch (caught) {
        showNotice(errorText(caught));
      } finally {
        send.disabled = false;
      }
    })();
  });
  composer.append(input, send, attach, fileInput);
  screen.append(composer);
  input.focus();
  input.setSelectionRange(input.value.length, input.value.length);
  // Send read receipts for what is now on screen, and remember locally
  // that this device read them so they are not re-sent on every render.
  const me = session?.current?.username ?? "";
  const unread = messages
    .filter((message) => message.sender !== me)
    .filter((message) => !(message.readBy ?? []).includes(me));
  if (unread.length > 0) {
    try {
      await session?.sendRead(
        conversationId,
        unread.map((message) => message.messageId),
      );
      for (const message of unread) {
        if (!message.readBy.includes(me)) {
          await saveMessage({ ...message, readBy: [...message.readBy, me] });
        }
      }
    } catch {
      // Receipts are best-effort.
    }
  }
}

function messageItem(conversationId: string, message: StoredMessage): HTMLLIElement {
  const item = document.createElement("li");
  if (message.sender === session?.current?.username) item.className = "own";
  const sender = document.createElement("span");
  sender.className = "sender";
  sender.textContent = message.sender;
  const body = document.createElement("p");
  body.className = "body";
  if (message.deleted) {
    body.textContent = "deleted";
    body.className = "body deleted";
  } else if (message.text !== null) {
    body.textContent = message.text + (message.edited ? " (edited)" : "");
  } else if (message.attachment) {
    const link = document.createElement("a");
    link.href = "#";
    link.textContent = `${message.attachment.name} (${message.attachment.size} bytes)`;
    link.setAttribute(
      "aria-label",
      `download attachment ${message.attachment.name}`,
    );
    link.addEventListener("click", (event) => {
      event.preventDefault();
      void (async () => {
        try {
          const { meta, bytes } = await session!.fetchAttachment(message.messageId);
          const binary = new Uint8Array(bytes);
          const blob = new Blob([binary], { type: meta.mime });
          const url = URL.createObjectURL(blob);
          const a = document.createElement("a");
          a.href = url;
          a.download = meta.name;
          a.click();
          URL.revokeObjectURL(url);
        } catch (caught) {
          showNotice(errorText(caught));
        }
      })();
    });
    body.replaceChildren(link);
  }
  const meta = document.createElement("span");
  meta.className = "meta";
  const readers = message.readBy ?? [];
  meta.textContent = readers.length > 0 ? `read by ${readers.join(", ")}` : "";
  item.append(sender, body);
  if (meta.textContent) item.append(meta);
  // Own text messages can be edited or deleted.
  if (
    !message.deleted &&
    message.text !== null &&
    message.sender === session?.current?.username
  ) {
    const edit = document.createElement("button");
    edit.type = "button";
    edit.className = "linkish small";
    edit.textContent = "edit";
    edit.setAttribute("aria-label", `edit message ${message.messageId}`);
    edit.addEventListener("click", () => {
      const replacement = window.prompt("New text", message.text ?? "");
      if (replacement === null) return;
      void (async () => {
        try {
          await session!.sendEdit(conversationId, message.messageId, replacement);
          await saveMessage({ ...message, text: replacement, edited: true });
          await render();
        } catch (caught) {
          showNotice(errorText(caught));
        }
      })();
    });
    const del = document.createElement("button");
    del.type = "button";
    del.className = "linkish small";
    del.textContent = "delete";
    del.setAttribute("aria-label", `delete message ${message.messageId}`);
    del.addEventListener("click", () => {
      void (async () => {
        try {
          await session!.sendDelete(conversationId, message.messageId);
          await saveMessage({ ...message, deleted: true });
          await render();
        } catch (caught) {
          showNotice(errorText(caught));
        }
      })();
    });
    item.append(edit, del);
  }
  return item;
}

async function labelFor(conversationId: string): Promise<string> {
  const conversations = await loadConversations();
  const conversation = conversations.find((entry) => entry.id === conversationId);
  if (!conversation) return conversationId;
  return (
    conversation.name ||
    conversation.members.filter((m) => m !== session?.current?.username).join(", ") ||
    conversationId
  );
}

// --- group details ---

async function renderGroup(conversationId: string): Promise<void> {
  screen.replaceChildren();
  const back = navLink("Back", `#/chat/${conversationId}`);
  screen.append(header("Group", [back]));
  const conversations = await loadConversations();
  const conversation = conversations.find((entry) => entry.id === conversationId);
  if (!conversation) {
    showNotice("Unknown conversation.");
    return;
  }
  const main = document.createElement("div");
  main.className = "panel";
  const heading = document.createElement("h2");
  heading.textContent = conversation.name || conversationId;
  const memberList = document.createElement("ul");
  memberList.className = "members";
  for (const member of conversation.members) {
    const item = document.createElement("li");
    item.textContent = member;
    memberList.append(item);
  }
  main.append(heading, memberList);

  const addForm = document.createElement("form");
  const addLabel = document.createElement("label");
  addLabel.textContent = "Add member by username";
  const addInput = document.createElement("input");
  addInput.name = "member";
  addLabel.append(addInput);
  const addSubmit = document.createElement("button");
  addSubmit.type = "submit";
  addSubmit.textContent = "Add";
  addForm.append(addLabel, addSubmit);
  addForm.addEventListener("submit", (event) => {
    event.preventDefault();
    void (async () => {
      try {
        await session!.addMembers(conversationId, [addInput.value.trim().toLowerCase()]);
        await render();
      } catch (caught) {
        showNotice(errorText(caught));
      }
    })();
  });

  const renameForm = document.createElement("form");
  const renameLabel = document.createElement("label");
  renameLabel.textContent = "Rename group";
  const renameInput = document.createElement("input");
  renameInput.name = "name";
  renameInput.value = conversation.name ?? "";
  renameLabel.append(renameInput);
  const renameSubmit = document.createElement("button");
  renameSubmit.type = "submit";
  renameSubmit.textContent = "Rename";
  renameForm.append(renameLabel, renameSubmit);
  renameForm.addEventListener("submit", (event) => {
    event.preventDefault();
    void (async () => {
      try {
        await session!.renameConversation(conversationId, renameInput.value.trim());
        await render();
      } catch (caught) {
        showNotice(errorText(caught));
      }
    })();
  });

  const inviteForm = document.createElement("form");
  const inviteSubmit = document.createElement("button");
  inviteSubmit.type = "submit";
  inviteSubmit.textContent = "Create invite link";
  inviteForm.append(inviteSubmit);
  const inviteOut = document.createElement("output");
  inviteForm.append(inviteOut);
  inviteForm.addEventListener("submit", (event) => {
    event.preventDefault();
    void (async () => {
      try {
        const token = await session!.createInvite(conversationId);
        inviteOut.textContent = token;
      } catch (caught) {
        showNotice(errorText(caught));
      }
    })();
  });
  main.append(addForm, renameForm, inviteForm);
  screen.append(main);
}

// --- devices ---

async function renderDevices(): Promise<void> {
  screen.replaceChildren();
  const back = navLink("Back", "#/chats");
  screen.append(header("Devices", [back]));
  const main = document.createElement("div");
  main.className = "panel";
  const list = document.createElement("ul");
  list.className = "members";
  try {
    const devices = (await session!.api.listDevices()) ?? [];
    for (const device of devices) {
      const item = document.createElement("li");
      const label = document.createElement("span");
      label.textContent = `${device.label} (${device.active ? "active" : "revoked"})`;
      item.append(label);
      if (device.active && device.id !== session?.current?.device_id) {
        const revoke = document.createElement("button");
        revoke.type = "button";
        revoke.textContent = "Revoke";
        revoke.setAttribute("aria-label", `revoke device ${device.label}`);
        revoke.addEventListener("click", () => {
          void (async () => {
            try {
              await session!.api.revokeDevice(device.id);
              await render();
            } catch (caught) {
              showNotice(errorText(caught));
            }
          })();
        });
        item.append(revoke);
      }
      list.append(item);
    }
  } catch (caught) {
    showNotice(errorText(caught));
  }
  main.append(list);
  screen.append(main);
}

// --- shared bits ---

function showNotice(text: string): void {
  const notice = document.createElement("p");
  notice.className = "notice";
  notice.setAttribute("role", "status");
  notice.textContent = text;
  screen.append(notice);
  setTimeout(() => notice.remove(), 8000);
}

let refreshTimer: number | null = null;

/** Coalesce bursts of incoming frames into one re-render. */
function scheduleLiveRefresh(): void {
  if (refreshTimer !== null) return;
  refreshTimer = window.setTimeout(() => {
    refreshTimer = null;
    render();
  }, 120);
}

function startLive(): void {
  live?.stop();
  if (!session) return;
  session.onActivity = scheduleLiveRefresh;
  live = new LiveConnection(session, (isConnected) => {
    connected = isConnected;
    const status = document.querySelector(".status");
    if (status) {
      status.textContent = isConnected ? "connected" : "offline";
      status.classList.toggle("ok", isConnected);
    }
    if (isConnected) {
      // Refresh whatever screen is open so queued messages appear.
      scheduleLiveRefresh();
    }
  });
  live.start();
}

/** Boot: restore a stored session, then render. */
void (async () => {
  try {
    session = await CryptoSession.restore(window.location.origin);
  } catch (error) {
    console.warn("could not restore session", error);
    session = null;
  }
  if (session) {
    try {
      await session.sync();
    } catch (error) {
      console.warn("sync failed", error);
    }
    startLive();
  }
  render();
})();
