/// Crypto session: the web client's equivalent of the CLI's `Chat` struct.
///
/// Owns the device's MLS state (`DeviceState`, from the wasm core), the
/// IndexedDB stores, and the API client. Every message leaves this module
/// already encrypted; every incoming message is decrypted here.

import {
  decodeAppMessage,
  decodeEnvelope,
  decodePayload,
  decodeServerFrame,
  decryptFile,
  deviceFromJson,
  encodeAppMessage,
  encodeClientFrame,
  encodeEnvelope,
  encryptFile,
  newFileKey,
  new_device,
  type DeviceState,
} from "./wasm/geist3_wasm.js";
import { initWasm } from "./wasmModule";

import { Api, type Session } from "./api";
import {
  getMessage,
  loadConversations,
  loadDeviceJson,
  saveDeviceJson,
  saveConversation,
  saveMessage,
  type StoredAttachment,
  type StoredMessage,
} from "./db";

/** Bytes of randomness in a blob id: 16 bytes, hex encoded. */
function newBlobId(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** SHA-256 of some bytes, hex encoded. */
export async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes as unknown as BufferSource);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** Base64 helpers matching the core's STANDARD engine. */
function toBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}

export class CryptoSession {
  readonly api: Api;
  private device: DeviceState | null = null;
  private session: Session | null = null;
  private keyPackageTarget = 2;

  private constructor(api: Api) {
    this.api = api;
  }

  /** The logged-in session, or null when logged out. */
  get current(): Session | null {
    return this.session;
  }

  /** The device identity (`username#device-id`). */
  get identity(): string {
    if (!this.device) throw new Error("not logged in");
    return this.device.identity;
  }

  /**
   * Restore a logged-in session: load the stored device key material and
   * re-arm the API token. Returns null when there is nothing to restore.
   */
  static async restore(server: string): Promise<CryptoSession | null> {
    await initWasm();
    const { loadSession } = await import("./db");
    const stored = await loadSession();
    if (!stored) return null;
    const api = new Api(server);
    api.setToken(stored.token);
    const session = new CryptoSession(api);
    session.session = stored;
    const json = await loadDeviceJson(stored.username);
    if (!json) return null;
    session.device = deviceFromJson(json);
    return session;
  }

  /** Register an account, log in, and create this browser's device keys. */
  static async register(
    server: string,
    username: string,
    password: string,
  ): Promise<CryptoSession> {
    await initWasm();
    const api = new Api(server);
    await api.register(username, password);
    return await CryptoSession.login(server, username, password);
  }

  /** Log in, creating a fresh device in the browser with fresh MLS keys. */
  static async login(
    server: string,
    username: string,
    password: string,
  ): Promise<CryptoSession> {
    await initWasm();
    const api = new Api(server);
    const session = await api.login(username, password, "web");
    api.setToken(session.token);
    const crypto = new CryptoSession(api);
    crypto.session = session;
    crypto.device = new_device(`${session.username}#${session.device_id}`);
    await crypto.topUpKeyPackages();
    await crypto.persist();
    return crypto;
  }

  /** Store the session and the serialized device keys in IndexedDB. */
  private async persist(): Promise<void> {
    if (!this.session || !this.device) return;
    const { saveSession } = await import("./db");
    await saveSession(this.session);
    await saveDeviceJson(this.session.username, this.device.toJSON());
  }

  /** Keep at least `keyPackageTarget` key packages with the server. */
  async topUpKeyPackages(): Promise<void> {
    if (!this.device) throw new Error("not logged in");
    const count = await this.api.keyPackageCount();
    for (let i = count; i < this.keyPackageTarget; i += 1) {
      await this.api.uploadKeyPackage(this.device.new_key_package());
    }
  }

  /** Log out: revoke the session on the server and drop the keys. */
  async logout(): Promise<void> {
    if (!this.session) return;
    const { clearSession, clearDeviceJson, wipeEverything } = await import("./db");
    try {
      await this.api.logout();
    } catch {
      // The session may already be gone; clear locally regardless.
    }
    const username = this.session.username;
    await clearSession();
    await clearDeviceJson(username);
    await wipeEverything();
    this.session = null;
    this.device = null;
  }

  /** Create a conversation and its MLS group; the creator is admin. */
  async createConversation(members: string[], name?: string): Promise<string> {
    if (!this.device) throw new Error("not logged in");
    const created = await this.api.createConversation(members, name);
    this.device.create_group(created.id);
    await this.persist();
    await saveConversation({ id: created.id, members: created.members, name: name ?? null });
    await this.syncGroup(created.id, created.members);
    return created.id;
  }

  /** Join a conversation through an invite token. */
  async joinConversation(inviteToken: string): Promise<string> {
    const joined = await this.api.redeemInvite(inviteToken);
    await saveConversation({ id: joined.id, members: joined.members, name: null });
    return joined.id;
  }

  /**
   * Bring one conversation's MLS group in line with the server's member
   * list: every device of every member missing from the group gets added
   * with one commit and one welcome.
   */
  async syncGroup(conversationId: string, members: string[]): Promise<void> {
    if (!this.device || !this.session) throw new Error("not logged in");
    const roster = this.device.group_members(conversationId) as unknown as string[];
    const packages: { deviceId: string; bytes: Uint8Array; identity: string }[] = [];
    for (const member of members) {
      const deviceIds = await this.api.devicesWithKeyPackages(member);
      for (const deviceId of deviceIds) {
        const identity = `${member}#${deviceId}`;
        if (roster.includes(identity) || identity === this.identity) continue;
        const packageBytes = await this.api.fetchKeyPackage(deviceId);
        if (!packageBytes) continue;
        const claimed = this.device.key_package_identity(packageBytes);
        if (claimed !== identity) {
          throw new Error(`server returned a key package for ${claimed} instead of ${identity}`);
        }
        packages.push({ deviceId, bytes: packageBytes, identity });
      }
    }
    if (packages.length === 0) return;
    const result = this.device.add_members(
      conversationId,
      packages.map((entry) => entry.bytes),
    ) as unknown as { commit: Uint8Array; welcome: Uint8Array | null };
    await this.persist();
    if (result.welcome) {
      await this.api.sendMessage(
        conversationId,
        encodeEnvelope("welcome", result.welcome),
      );
    }
    await this.api.sendMessage(conversationId, encodeEnvelope("mls", result.commit));
  }

  /** Refresh the conversation list from the server and sync every group. */
  async sync(): Promise<void> {
    if (!this.device) throw new Error("not logged in");
    await this.topUpKeyPackages();
    const conversations = await this.api.listConversations();
    for (const conversation of conversations) {
      await saveConversation({
        id: conversation.id,
        members: conversation.members,
        name: conversation.name ?? null,
      });
      if (this.device.has_group(conversation.id)) {
        await this.syncGroup(conversation.id, conversation.members);
      }
    }
  }

  /** Ensure the local MLS group exists for a conversation we just joined. */
  async ensureGroup(conversationId: string, members: string[]): Promise<void> {
    if (!this.device) throw new Error("not logged in");
    if (this.device.has_group(conversationId)) return;
    // A welcome will arrive over the WebSocket; but if this device created
    // the conversation on another client, it can also build the group by
    // asking an existing member to re-send. Here we simply wait for that
    // welcome; `create_group` is for the creator only.
    void members;
  }

  /** Encrypt and send any application message; returns the relay message id. */
  private async sendApp(conversationId: string, app: unknown): Promise<string> {
    if (!this.device) throw new Error("not logged in");
    if (!this.device.has_group(conversationId)) {
      throw new Error("this device has not joined this conversation's group yet");
    }
    const appBytes = encodeAppMessage(app);
    const wire = this.device.encrypt(conversationId, appBytes);
    await this.persist();
    const payload = encodeEnvelope("mls", wire);
    const id = await this.api.sendMessage(conversationId, payload);
    await this.recordOwnMessage(conversationId, id, app as Record<string, unknown>);
    return id;
  }

  /** Remember what this device said, so edits, deletes and receipts work. */
  private async recordOwnMessage(
    conversationId: string,
    messageId: string,
    app: Record<string, unknown>,
  ): Promise<void> {
    const type = app["type"];
    const known = (await getMessage(conversationId, messageId)) !== null;
    if (known) return;
    if (type === "text" || type === "reply") {
      await saveMessage({
        conversationId,
        messageId,
        sender: this.session?.username ?? "",
        text: String(app["text"] ?? ""),
        edited: false,
        deleted: false,
        createdAt: Date.now(),
        replyTo: type === "reply" ? String(app["reply_to"] ?? "") : null,
        attachment: null,
        readBy: [],
      });
    } else if (type === "attachment") {
      await saveMessage({
        conversationId,
        messageId,
        sender: this.session?.username ?? "",
        text: null,
        edited: false,
        deleted: false,
        createdAt: Date.now(),
        replyTo: null,
        attachment: {
          blobId: String(app["blob_id"] ?? ""),
          keyB64: String(app["key_b64"] ?? ""),
          nonceB64: String(app["nonce_b64"] ?? ""),
          name: String(app["name"] ?? ""),
          mime: String(app["mime"] ?? ""),
          size: Number(app["size"] ?? 0),
          sha256: String(app["sha256"] ?? ""),
        },
        readBy: [],
      });
    }
  }

  /** Send a chat text. */
  async sendText(conversationId: string, text: string): Promise<string> {
    return await this.sendApp(conversationId, { type: "text", text });
  }

  /** Edit a message this device sent. */
  async sendEdit(conversationId: string, messageId: string, text: string): Promise<string> {
    return await this.sendApp(conversationId, { type: "edit", message_id: messageId, text });
  }

  /** Delete a message this device sent. */
  async sendDelete(conversationId: string, messageId: string): Promise<string> {
    return await this.sendApp(conversationId, { type: "delete", message_id: messageId });
  }

  /** Send read receipts for the given message ids. */
  async sendRead(conversationId: string, messageIds: string[]): Promise<void> {
    if (messageIds.length === 0) return;
    await this.sendApp(conversationId, { type: "read", message_ids: messageIds });
  }

  /**
   * Encrypt a file and upload it: images are sent as-is (the browser
   * downscales nothing), the ciphertext goes to the blob store under a
   * fresh id, and the key travels inside the MLS message.
   */
  async sendAttachment(
    conversationId: string,
    file: File,
  ): Promise<string> {
    const plaintext = new Uint8Array(await file.arrayBuffer());
    const key = (await newFileKey()) as unknown as { key_base64: string; nonce_base64: string };
    const blobId = newBlobId();
    const ciphertext = encryptFile(key.key_base64, key.nonce_base64, blobId, plaintext);
    await this.api.uploadBlob(blobId, ciphertext);
    return await this.sendApp(conversationId, {
      type: "attachment",
      blob_id: blobId,
      key_b64: key.key_base64,
      nonce_b64: key.nonce_base64,
      name: file.name,
      mime: file.type || "application/octet-stream",
      size: plaintext.length,
      sha256: await sha256Hex(plaintext),
    });
  }

  /**
   * Fetch, decrypt, checksum-verify and return the attachment of a stored
   * message. Throws when the blob is gone or the checksum fails.
   */
  async fetchAttachment(messageId: string): Promise<{ meta: StoredAttachment; bytes: Uint8Array }> {
    const conversation = await loadConversations();
    let found: StoredMessage | null = null;
    for (const entry of conversation) {
      found = (await getMessage(entry.id, messageId)) ?? found;
    }
    if (!found || !found.attachment) throw new Error(`no attachment for message ${messageId}`);
    const meta = found.attachment;
    const ciphertext = await this.api.fetchBlob(meta.blobId);
    if (!ciphertext) throw new Error("the attachment has expired on the server");
    const plaintext = decryptFile(meta.keyB64, meta.nonceB64, meta.blobId, ciphertext);
    const digest = await sha256Hex(plaintext);
    if (digest !== meta.sha256) {
      throw new Error("the attachment failed its checksum; not saving");
    }
    return { meta, bytes: plaintext };
  }

  /** Add members (admin); returns the new member list. */
  async addMembers(conversationId: string, members: string[]): Promise<string[]> {
    const all = await this.api.addMembers(conversationId, members);
    await this.refreshConversation(conversationId);
    await this.syncGroup(conversationId, all);
    return all;
  }

  /** Remove a member (admin). */
  async removeMember(conversationId: string, username: string): Promise<void> {
    await this.api.removeMember(conversationId, username);
    await this.refreshConversation(conversationId);
    await this.syncGroup(conversationId, await this.memberList(conversationId));
  }

  /** Rename a conversation (admin). */
  async renameConversation(conversationId: string, name: string): Promise<void> {
    await this.api.renameConversation(conversationId, name);
    await this.refreshConversation(conversationId);
  }

  /** Mint an invite token (admin). */
  async createInvite(conversationId: string): Promise<string> {
    return await this.api.createInvite(conversationId);
  }

  /** Revoke an invite token (admin). */
  async revokeInvite(conversationId: string, inviteToken: string): Promise<void> {
    await this.api.revokeInvite(conversationId, inviteToken);
  }

  /** Block a user; blocks apply to new shared conversations. */
  async block(username: string): Promise<void> {
    await this.api.block(username);
  }

  async unblock(username: string): Promise<void> {
    await this.api.unblock(username);
  }

  private async memberList(conversationId: string): Promise<string[]> {
    const conversations = await loadConversations();
    const conversation = conversations.find((entry) => entry.id === conversationId);
    return conversation?.members ?? [];
  }

  private async refreshConversation(conversationId: string): Promise<void> {
    const conversations = await this.api.listConversations();
    const conversation = conversations.find((entry) => entry.id === conversationId);
    if (conversation) {
      await saveConversation({
        id: conversation.id,
        members: conversation.members,
        name: conversation.name ?? null,
      });
    }
  }

  /**
   * Observer notified (fire-and-forget) whenever incoming frames change
   * local state: a welcome joined a group, a commit landed, or an
   * application message was applied. The UI re-renders on this.
   */
  onActivity: (() => void) | null = null;

  /** Fire the activity observer without ever breaking message delivery. */
  private notify(): void {
    try {
      this.onActivity?.();
    } catch {
      // Observer errors must not affect the frame pipeline.
    }
  }

  /**
   * Make sure a conversation that just produced traffic is in the local
   * list: a device that was added to a group only learns the group's
   * metadata from the server, never from the welcome itself.
   */
  private async ensureConversationKnown(conversationId: string): Promise<void> {
    const conversations = await loadConversations();
    if (conversations.some((entry) => entry.id === conversationId)) return;
    const list = await this.api.listConversations();
    const found = list.find((entry) => entry.id === conversationId);
    if (found) {
      await saveConversation({
        id: found.id,
        members: found.members,
        name: found.name ?? null,
      });
    }
  }

  /**
   * Handle one WebSocket server frame. Returns the ack'd message id when
   * the frame was a message that was fully applied (the caller must send
   * the ack), or null when there is nothing to ack — including messages
   * that could not be applied yet, which the server will offer again on
   * the next connection.
   */
  async handleServerFrame(text: string): Promise<string | null> {
    const frame = decodeServerFrame(text) as unknown as
      | { type: "auth_ok"; device_id: string }
      | { type: "ack_ok"; message_id: string }
      | {
          type: "message";
          conversation_id: string;
          message_id: string;
          sender_username: string;
          payload_base64: string;
          created_at: number;
        };
    if (frame.type === "message") {
      const applied = await this.interpret(
        frame.conversation_id,
        frame.message_id,
        frame.sender_username,
        decodePayload(frame.payload_base64),
      );
      return applied ? frame.message_id : null;
    }
    return null;
  }

  /** The auth frame to send first on a WebSocket connection. */
  authFrame(): string {
    if (!this.session) throw new Error("not logged in");
    return encodeClientFrame({ type: "auth", token: this.session.token });
  }

  /**
   * Turn one relay payload into store updates. Mirrors the CLI's
   * `interpret`/`apply_app_message`. Returns whether the frame was fully
   * applied and may be ack'd; false leaves it queued on the server for
   * re-delivery after the next reconnect.
   */
  private async interpret(
    conversationId: string,
    messageId: string,
    sender: string,
    payload: Uint8Array,
  ): Promise<boolean> {
    if (!this.device) throw new Error("not logged in");
    const envelope = (await decodeEnvelope(payload)) as unknown as
      | { kind: "mls" | "welcome"; data: Uint8Array }
      | null;
    if (!envelope) {
      // Undecodable payload. Not acking keeps it on the server so a
      // later connection can try again with fresher state.
      console.warn("message", messageId, "has an undecodable envelope");
      return false;
    }
    if (envelope.kind === "welcome") {
      try {
        this.device.process_welcome(envelope.data);
        await this.persist();
      } catch (error) {
        // Stale or foreign welcome; ignore like the CLI does.
        console.warn("welcome rejected", error);
      }
      await this.ensureConversationKnown(conversationId);
      this.notify();
      return true;
    }
    if (!this.device.has_group(conversationId)) {
      // The group has not joined this device yet (its welcome is still
      // in flight). Leave the message queued; the next connection will
      // offer it again once the welcome has been applied.
      console.warn("message", messageId, "arrived before its group; leaving un-acked");
      return false;
    }
    let incoming: { kind: string; sender?: string; payload?: Uint8Array };
    try {
      incoming = (await this.device.process_incoming(
        conversationId,
        envelope.data,
      )) as unknown as { kind: string; sender?: string; payload?: Uint8Array };
    } catch (error) {
      // The device has the group but could not apply this message;
      // re-delivery after a reconnect may succeed with fresher state.
      console.warn("message", messageId, "could not be applied", error);
      return false;
    }
    if (incoming.kind === "commit") {
      await this.persist();
      this.notify();
      return true;
    }
    if (incoming.kind !== "application" || !incoming.payload) return true;
    const app = (await decodeAppMessage(incoming.payload)) as unknown as Record<string, unknown>;
    await this.applyAppMessage(conversationId, messageId, incoming.sender ?? sender, app);
    await this.ensureConversationKnown(conversationId);
    this.notify();
    return true;
  }

  /** Apply a decrypted application message to the local store. */
  private async applyAppMessage(
    conversationId: string,
    messageId: string,
    sender: string,
    app: Record<string, unknown>,
  ): Promise<void> {
    const type = app["type"];
    const existing = await getMessage(conversationId, messageId);
    await this.persist();
    if (type === "text" || type === "reply") {
      const text = String(app["text"] ?? "");
      if (!existing) {
        await saveMessage({
          conversationId,
          messageId,
          sender,
          text,
          edited: false,
          deleted: false,
          createdAt: Date.now(),
          replyTo: type === "reply" ? String(app["reply_to"] ?? "") : null,
          attachment: null,
          readBy: [],
        });
      }
    } else if (type === "edit") {
      const target = String(app["message_id"] ?? "");
      const message = await getMessage(conversationId, target);
      if (message && message.sender === sender) {
        await saveMessage({ ...message, text: String(app["text"] ?? ""), edited: true });
      }
    } else if (type === "delete") {
      const target = String(app["message_id"] ?? "");
      const message = await getMessage(conversationId, target);
      if (message && message.sender === sender) {
        await saveMessage({ ...message, deleted: true });
      }
    } else if (type === "read") {
      const ids = (app["message_ids"] as string[]) ?? [];
      for (const id of ids) {
        const message = await getMessage(conversationId, id);
        if (message && !message.readBy.includes(sender) && message.sender !== sender) {
          await saveMessage({ ...message, readBy: [...message.readBy, sender] });
        }
      }
    } else if (type === "attachment") {
      if (!existing) {
        await saveMessage({
          conversationId,
          messageId,
          sender,
          text: null,
          edited: false,
          deleted: false,
          createdAt: Date.now(),
          replyTo: null,
          attachment: {
            blobId: String(app["blob_id"] ?? ""),
            keyB64: String(app["key_b64"] ?? ""),
            nonceB64: String(app["nonce_b64"] ?? ""),
            name: String(app["name"] ?? ""),
            mime: String(app["mime"] ?? ""),
            size: Number(app["size"] ?? 0),
            sha256: String(app["sha256"] ?? ""),
          },
          readBy: [],
        });
      }
    }
  }

  /** A short display label for a conversation: its name or member list. */
  async conversationLabel(conversationId: string): Promise<string> {
    const conversations = await loadConversations();
    const conversation = conversations.find((entry) => entry.id === conversationId);
    if (!conversation) return conversationId;
    if (conversation.name) return conversation.name;
    const others = conversation.members.filter(
      (member) => member !== this.session?.username,
    );
    return others.length > 0 ? others.join(", ") : conversation.members.join(", ");
  }

  /** Base64 of an attachment's bytes for data-URL downloads. */
  static toBase64(bytes: Uint8Array): string {
    return toBase64(bytes);
  }
}
