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
  departed_members,
  deviceFromJson,
  encodeAppMessage,
  encodeClientFrame,
  encodeEnvelope,
  encryptFile,
  newFileKey,
  new_device,
  plan_roster,
  type DeviceState,
} from "./wasm/thrum_wasm.js";
import { initWasm } from "./wasmModule";

import { Api, type Session } from "./api";
import {
  getMessage,
  loadConversations,
  loadDeviceJson,
  loadMeta,
  saveDeviceJson,
  saveConversation,
  saveMessage,
  saveMeta,
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
  /**
   * Waits until incoming frames already received are applied; set by the
   * live connection. Sending first could encrypt in an epoch a removal
   * commit has already ended, which the removed device can still read.
   */
  beforeSend: (() => Promise<void>) | null = null;
  /** Server roster JSON per conversation, for checking commits and welcomes. */
  private rosters = new Map<string, string>();
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

  /** Keep at least `keyPackageTarget` one-time packages, plus one last-resort package. */
  async topUpKeyPackages(): Promise<void> {
    if (!this.device) throw new Error("not logged in");
    const stock = await this.api.keyPackageStock();
    if (!stock.lastResort) {
      await this.api.uploadKeyPackage(this.device.new_last_resort_key_package(), true);
    }
    for (let i = stock.count; i < this.keyPackageTarget; i += 1) {
      await this.api.uploadKeyPackage(this.device.new_key_package());
    }
    await this.persist();
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
    await this.syncGroup(created.id, created.members, new Set(created.members));
    return created.id;
  }

  /** Join a conversation through an invite token. */
  async joinConversation(inviteToken: string): Promise<string> {
    const joined = await this.api.redeemInvite(inviteToken);
    await saveConversation({ id: joined.id, members: joined.members, name: null });
    return joined.id;
  }

  /**
   * Bring one conversation's MLS group in line with the server.
   *
   * Usernames in `admit` have their current devices added without a prior
   * approval: the user just named them. Any other new device is kept pending
   * until `trustDevice`. Revoked devices and departed accounts are removed.
   */
  async syncGroup(
    conversationId: string,
    members: string[],
    admit: ReadonlySet<string> = new Set(),
  ): Promise<void> {
    if (!this.device || !this.session) throw new Error("not logged in");
    const roster = [...(this.device.group_members(conversationId) as unknown as string[])];
    const trusted = await loadTrusted();
    const known = await loadKnown(conversationId);
    const knownAnywhere = await loadKnownAnywhere();
    const packages: Uint8Array[] = [];
    const added: string[] = [];
    const pending: string[] = [];
    const removals = [
      ...(departed_members(this.identity, roster, members) as unknown as string[]),
    ];
    const accounts: Record<string, Record<string, string | null>> = {};
    for (const member of members) {
      accounts[member] = await this.api.activeDevicesWithKeys(member);
    }
    const changedKeys = await applyPins(accounts);
    const serverRoster = JSON.stringify(accounts);
    this.rosters.set(conversationId, serverRoster);
    for (const member of members) {
      const active = Object.keys(accounts[member] ?? {});
      const admitMember = [...admit].some((name) => name.toLowerCase() === member.toLowerCase());
      // Trust on first use is per account: a new conversation with an
      // account this browser already knows does not admit new devices.
      const seen = [...known, ...knownAnywhere].some(
        (name) => name.toLowerCase() === member.toLowerCase(),
      );
      const actions = plan_roster(
        this.identity,
        roster,
        member,
        active,
        [...trusted],
        admitMember,
        seen,
      ) as unknown as { kind: string; identity: string }[];
      for (const action of actions) {
        if (action.kind === "remove") {
          // A device that left the server's list is no longer trusted: if
          // it is listed again, it waits for the user.
          trusted.delete(action.identity);
          removals.push(action.identity);
        }
        else if (action.kind === "pending") pending.push(action.identity);
        else if (action.kind === "add") {
          if (changedKeys.has(action.identity)) {
            // The key changed since this browser pinned it: like a new
            // device, it waits for the user.
            pending.push(action.identity);
            continue;
          }
          trusted.add(action.identity);
          const deviceId = action.identity.slice(action.identity.indexOf("#") + 1);
          const packageBytes = await this.api.fetchKeyPackage(deviceId);
          if (!packageBytes) continue;
          try {
            this.device.check_key_package(packageBytes, action.identity, serverRoster);
          } catch (error) {
            // One bad package must not stop the rest of the sync, which
            // may hold removals and other conversations.
            console.warn("not adding", action.identity, error);
            if ((await loadPins())[action.identity]) pending.push(action.identity);
            continue;
          }
          packages.push(packageBytes);
          added.push(action.identity);
        }
      }
      const inGroup = roster.some((identity) => identity.toLowerCase().startsWith(`${member.toLowerCase()}#`));
      if (active.length > 0 || inGroup) known.add(member);
    }
    await saveTrusted(trusted);
    await saveKnown(conversationId, known);
    await savePending(conversationId, pending);
    for (const identity of removals) {
      try {
        const commit = this.device.remove_member(conversationId, identity);
        await this.persist();
        await this.api.sendMessage(conversationId, encodeEnvelope("mls", commit));
        await this.noteRoster(conversationId, `${identity} left this conversation`);
      } catch {
        // Another member may already have removed this device.
      }
    }
    if (packages.length === 0) return;
    const result = this.device.add_members(conversationId, packages) as unknown as {
      commit: Uint8Array;
      welcome: Uint8Array | null;
    };
    await this.persist();
    await this.pinGroupKeys(conversationId);
    if (result.welcome) {
      await this.api.sendMessage(conversationId, encodeEnvelope("welcome", result.welcome));
    }
    await this.api.sendMessage(conversationId, encodeEnvelope("mls", result.commit));
    for (const identity of added) {
      await this.noteRoster(conversationId, `${identity} joined this conversation`);
    }
  }

  /** Device identities waiting for approval, across conversations. */
  async pendingDevices(): Promise<{ conversationId: string; identity: string }[]> {
    const raw = await loadMeta("pending-devices");
    if (!raw) return [];
    try {
      const parsed = JSON.parse(raw) as Record<string, string[]>;
      const rows: { conversationId: string; identity: string }[] = [];
      for (const [conversationId, identities] of Object.entries(parsed)) {
        for (const identity of identities) rows.push({ conversationId, identity });
      }
      return rows;
    } catch {
      return [];
    }
  }

  /** Approve a device identity and add it to the groups it is waiting on. */
  async trustDevice(identity: string): Promise<void> {
    const trusted = await loadTrusted();
    trusted.add(identity);
    await saveTrusted(trusted);
    // The user accepts whatever key the device presents next.
    const pins = await loadPins();
    delete pins[identity];
    await savePins(pins);
    await this.sync();
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
        await this.syncGroup(conversation.id, conversation.members, new Set());
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
    await this.beforeSend?.();
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
    await this.syncGroup(conversationId, all, new Set(members));
    return all;
  }

  /** Remove a member (admin). */
  async removeMember(conversationId: string, username: string): Promise<void> {
    await this.api.removeMember(conversationId, username);
    await this.refreshConversation(conversationId);
    await this.syncGroup(conversationId, await this.memberList(conversationId), new Set());
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

  /** Pin the signature key of each current leaf that has none yet. */
  private async pinGroupKeys(conversationId: string): Promise<void> {
    if (!this.device) return;
    const leaves = this.device.group_leaves(conversationId) as unknown as {
      identity: string;
      signatureKey: string;
    }[];
    const pins = await loadPins();
    let added = false;
    for (const leaf of leaves) {
      if (!pins[leaf.identity]) {
        pins[leaf.identity] = leaf.signatureKey;
        added = true;
      }
    }
    if (added) await savePins(pins);
  }

  /**
   * The server's view of a conversation, as the wasm core's roster JSON:
   * every member account with its active devices and their bound keys.
   * Cached per conversation; `refresh` fetches it again.
   */
  private async serverRoster(conversationId: string, refresh: boolean): Promise<string> {
    const cached = this.rosters.get(conversationId);
    if (cached && !refresh) return cached;
    const conversations = await this.api.listConversations();
    const conversation = conversations.find((entry) => entry.id === conversationId);
    const accounts: Record<string, Record<string, string | null>> = {};
    for (const member of conversation?.members ?? []) {
      accounts[member] = await this.api.activeDevicesWithKeys(member);
    }
    await applyPins(accounts);
    const roster = JSON.stringify(accounts);
    this.rosters.set(conversationId, roster);
    return roster;
  }

  /**
   * Turn one relay payload into store updates. Mirrors the CLI's
   * `interpret`/`apply_app_message`. Returns whether the frame may be
   * ack'd: it was applied, or it never can be (bad bytes, a refused
   * commit), so offering it again would only replay it. False leaves it
   * queued on the server for re-delivery after the next reconnect, which
   * only helps a message that arrived before its group's welcome.
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
      // Undecodable payload: no later state can read it, so drop it.
      console.warn("message", messageId, "has an undecodable envelope; dropped");
      return true;
    }
    if (envelope.kind === "welcome") {
      // A welcome only counts for the conversation it arrived in, and
      // only when every member it names is on the server's roster with
      // its bound key. A refusal leaves the device unchanged, so retry
      // once with a fresh roster in case a member was just added.
      for (const refresh of [false, true]) {
        try {
          const roster = await this.serverRoster(conversationId, refresh);
          this.device.process_welcome(conversationId, envelope.data, roster);
          await this.persist();
          await this.pinGroupKeys(conversationId);
          break;
        } catch (error) {
          // Stale or foreign welcome; ignore like the CLI does.
          if (refresh) console.warn("welcome rejected", error);
        }
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
    const before = [...(this.device.group_members(conversationId) as unknown as string[])];
    type Processed = { kind: string; sender?: string; payload?: Uint8Array; reason?: string };
    let incoming: Processed = { kind: "ignored" };
    // Commits are checked against the server's roster; a refusal leaves
    // the device unchanged, so retry once with a fresh roster in case the
    // commit follows a membership change this device has not seen. A
    // failed roster fetch throws, which leaves the frame queued.
    for (const refresh of [false, true]) {
      const roster = await this.serverRoster(conversationId, refresh);
      try {
        incoming = this.device.process_incoming(
          conversationId,
          envelope.data,
          roster,
        ) as unknown as Processed;
      } catch (error) {
        // The device has the group but cannot apply this message: bytes
        // that are not MLS, or a message for an epoch it will never be in.
        // Frames arrive in order, so a replay would fail the same way;
        // drop it like the CLI does.
        console.warn("message", messageId, "could not be applied; dropped", error);
        return true;
      }
      if (incoming.kind !== "rejected") break;
    }
    if (incoming.kind === "rejected") {
      console.warn("commit refused:", incoming.reason);
      return true;
    }
    if (incoming.kind === "commit") {
      const after = [...(this.device.group_members(conversationId) as unknown as string[])];
      await this.persist();
      await this.pinGroupKeys(conversationId);
      await this.noteRosterDiff(conversationId, before, after);
      this.notify();
      return true;
    }
    if (incoming.kind !== "application" || !incoming.payload || !incoming.sender) return true;
    // The sender is the MLS credential that signed the message. The relay's
    // label is only a hint; one that disagrees means the frame was tampered
    // with, so it is dropped.
    const hash = incoming.sender.indexOf("#");
    const author = hash > 0 ? incoming.sender.slice(0, hash) : "";
    if (!author || author.toLowerCase() !== sender.toLowerCase()) {
      console.warn("dropped a message labeled", sender, "that", incoming.sender, "sent");
      return true;
    }
    let app: Record<string, unknown>;
    try {
      app = (await decodeAppMessage(incoming.payload)) as unknown as Record<string, unknown>;
    } catch (error) {
      console.warn("message", messageId, "carries an unreadable app message; dropped", error);
      return true;
    }
    await this.applyAppMessage(conversationId, messageId, author, app);
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

  private async noteRoster(conversationId: string, text: string): Promise<void> {
    await saveMessage({
      conversationId,
      messageId: `local-${text}`,
      sender: "thrum",
      text,
      edited: false,
      deleted: false,
      createdAt: Date.now(),
      replyTo: null,
      attachment: null,
      readBy: [],
    });
  }

  private async noteRosterDiff(
    conversationId: string,
    before: string[],
    after: string[],
  ): Promise<void> {
    for (const identity of after) {
      if (!before.includes(identity)) {
        await this.noteRoster(conversationId, `${identity} joined this conversation`);
      }
    }
    for (const identity of before) {
      if (!after.includes(identity)) {
        await this.noteRoster(conversationId, `${identity} left this conversation`);
      }
    }
  }
}

async function loadTrusted(): Promise<Set<string>> {
  const raw = await loadMeta("trusted-devices");
  if (!raw) return new Set();
  try {
    return new Set(JSON.parse(raw) as string[]);
  } catch {
    return new Set();
  }
}

async function saveTrusted(trusted: Set<string>): Promise<void> {
  await saveMeta("trusted-devices", JSON.stringify([...trusted]));
}

async function loadKnown(conversationId: string): Promise<Set<string>> {
  const raw = await loadMeta("known-accounts");
  if (!raw) return new Set();
  try {
    const parsed = JSON.parse(raw) as Record<string, string[]>;
    return new Set(parsed[conversationId] ?? []);
  } catch {
    return new Set();
  }
}

async function saveKnown(conversationId: string, names: Set<string>): Promise<void> {
  const raw = await loadMeta("known-accounts");
  let known: Record<string, string[]> = {};
  if (raw) {
    try {
      known = JSON.parse(raw) as Record<string, string[]>;
    } catch {
      known = {};
    }
  }
  if (names.size === 0) delete known[conversationId];
  else known[conversationId] = [...names];
  await saveMeta("known-accounts", JSON.stringify(known));
}

async function savePending(conversationId: string, identities: string[]): Promise<void> {
  const raw = await loadMeta("pending-devices");
  let pending: Record<string, string[]> = {};
  if (raw) {
    try {
      pending = JSON.parse(raw) as Record<string, string[]>;
    } catch {
      pending = {};
    }
  }
  if (identities.length === 0) delete pending[conversationId];
  else pending[conversationId] = identities;
  await saveMeta("pending-devices", JSON.stringify(pending));
}

/** Accounts this browser has introduced in any conversation. */
async function loadKnownAnywhere(): Promise<Set<string>> {
  const raw = await loadMeta("known-accounts");
  if (!raw) return new Set();
  try {
    const parsed = JSON.parse(raw) as Record<string, string[]>;
    return new Set(Object.values(parsed).flat());
  } catch {
    return new Set();
  }
}

/** Device identity → the signature key (base64) first seen for it. */
async function loadPins(): Promise<Record<string, string>> {
  const raw = await loadMeta("pinned-keys");
  if (!raw) return {};
  try {
    return JSON.parse(raw) as Record<string, string>;
  } catch {
    return {};
  }
}

async function savePins(pins: Record<string, string>): Promise<void> {
  await saveMeta("pinned-keys", JSON.stringify(pins));
}

/**
 * Hold every listed device in a roster to the key this browser pinned for
 * it, whatever key the server reports. Returns the identities whose key
 * the server now reports differently.
 */
async function applyPins(
  accounts: Record<string, Record<string, string | null>>,
): Promise<Set<string>> {
  const pins = await loadPins();
  const changed = new Set<string>();
  for (const [username, devices] of Object.entries(accounts)) {
    for (const [deviceId, reported] of Object.entries(devices)) {
      const identity = `${username}#${deviceId}`;
      const pinned = pins[identity];
      if (!pinned) continue;
      if (reported !== null && reported !== pinned) {
        console.warn("the server reports a new signature key for", identity);
        changed.add(identity);
      }
      devices[deviceId] = pinned;
    }
  }
  return changed;
}
