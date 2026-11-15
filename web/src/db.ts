/// IndexedDB persistence for the web client.
///
/// The database stays on this device. It holds three things: the session
/// (token and device id, which the server also knows), the conversation and
/// message history the server queues, and — under the `keys` store — the
/// device's MLS key material. The key store is the only secret here and it
/// never leaves the page.

import type { Session } from "./api";

const DB_NAME = "thrum";
const DB_VERSION = 2;

export interface StoredMessage {
  conversationId: string;
  messageId: string;
  sender: string;
  text: string | null;
  edited: boolean;
  deleted: boolean;
  createdAt: number;
  /** Set when the message quotes another message by relay id. */
  replyTo: string | null;
  /** Attachment metadata, when the message carries one. */
  attachment: StoredAttachment | null;
  /** Usernames confirmed to have read this message. */
  readBy: string[];
}

export interface StoredAttachment {
  blobId: string;
  keyB64: string;
  nonceB64: string;
  name: string;
  mime: string;
  size: number;
  sha256: string;
}

export interface StoredConversation {
  id: string;
  members: string[];
  name: string | null;
}

let databasePromise: Promise<IDBDatabase> | null = null;

function open(): Promise<IDBDatabase> {
  if (databasePromise) return databasePromise;
  databasePromise = new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, DB_VERSION);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains("session")) db.createObjectStore("session");
      if (!db.objectStoreNames.contains("keys")) db.createObjectStore("keys");
      if (!db.objectStoreNames.contains("conversations")) {
        const store = db.createObjectStore("conversations", { keyPath: "id" });
        store.createIndex("byName", "name");
      }
      if (!db.objectStoreNames.contains("messages")) {
        const store = db.createObjectStore("messages", { keyPath: ["conversationId", "messageId"] });
        store.createIndex("byConversation", "conversationId");
      }
      if (!db.objectStoreNames.contains("meta")) db.createObjectStore("meta");
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  return databasePromise;
}

async function withStore<T>(
  storeName: string,
  mode: IDBTransactionMode,
  run: (store: IDBObjectStore) => IDBRequest<T> | void,
): Promise<T> {
  const db = await open();
  return await new Promise<T>((resolve, reject) => {
    const tx = db.transaction(storeName, mode);
    const store = tx.objectStore(storeName);
    let request: IDBRequest<T> | void;
    try {
      request = run(store);
    } catch (error) {
      reject(error);
      return;
    }
    tx.oncomplete = () => {
      if (request && "result" in request) resolve(request.result);
      else resolve(undefined as T);
    };
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error);
  });
}

// --- session ---

export async function loadSession(): Promise<Session | null> {
  return (await withStore<Session | undefined>("session", "readonly", (store) =>
    store.get("current"),
  )) ?? null;
}

export async function saveSession(session: Session): Promise<void> {
  await withStore("session", "readwrite", (store) => {
    store.put(session, "current");
  });
}

export async function clearSession(): Promise<void> {
  await withStore("session", "readwrite", (store) => {
    store.clear();
  });
}

// --- device keys (never leaves the page) ---

export async function loadDeviceJson(username: string): Promise<string | null> {
  return (await withStore<string | undefined>("keys", "readonly", (store) =>
    store.get(username),
  )) ?? null;
}

export async function saveDeviceJson(username: string, json: string): Promise<void> {
  await withStore("keys", "readwrite", (store) => {
    store.put(json, username);
  });
}

export async function clearDeviceJson(username: string): Promise<void> {
  await withStore("keys", "readwrite", (store) => {
    store.delete(username);
  });
}

// --- conversations ---

export async function saveConversation(conversation: StoredConversation): Promise<void> {
  await withStore("conversations", "readwrite", (store) => {
    store.put(conversation);
  });
}

export async function loadConversations(): Promise<StoredConversation[]> {
  return await withStore<StoredConversation[]>("conversations", "readonly", (store) =>
    store.getAll(),
  );
}

export async function deleteConversation(id: string): Promise<void> {
  await withStore("conversations", "readwrite", (store) => {
    store.delete(id);
  });
}

// --- messages ---

export async function saveMessage(message: StoredMessage): Promise<void> {
  await withStore("messages", "readwrite", (store) => {
    store.put(message);
  });
}

export async function getMessage(
  conversationId: string,
  messageId: string,
): Promise<StoredMessage | null> {
  return (await withStore<StoredMessage | undefined>("messages", "readonly", (store) =>
    store.get([conversationId, messageId]),
  )) ?? null;
}

export async function loadMessages(conversationId: string): Promise<StoredMessage[]> {
  return await withStore<StoredMessage[]>("messages", "readonly", (store) =>
    store.index("byConversation").getAll(conversationId),
  );
}

export async function loadMeta(key: string): Promise<string | null> {
  return (await withStore<string | undefined>("meta", "readonly", (store) => store.get(key))) ?? null;
}

export async function saveMeta(key: string, value: string): Promise<void> {
  await withStore("meta", "readwrite", (store) => {
    store.put(value, key);
  });
}

export async function wipeEverything(): Promise<void> {
  const db = await open();
  await new Promise<void>((resolve, reject) => {
    const tx = db.transaction(
      ["session", "keys", "conversations", "messages", "meta"],
      "readwrite",
    );
    for (const name of ["session", "keys", "conversations", "messages", "meta"]) {
      tx.objectStore(name).clear();
    }
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error);
  });
}
