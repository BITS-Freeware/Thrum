/* tslint:disable */
/* eslint-disable */

/**
 * A device's MLS state. Create with [`new_device`] or
 * [`DeviceState::from_json`]; persist with [`DeviceState::to_json`].
 */
export class DeviceState {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Add members from serialized key packages. Returns
     * `{ commit: Uint8Array, welcome: Uint8Array | null }`.
     */
    add_members(conversation_id: string, key_packages: Uint8Array[]): any;
    /**
     * Create the MLS group for a conversation. Idempotent.
     */
    create_group(conversation_id: string): void;
    /**
     * Encrypt an application payload for the conversation's group.
     */
    encrypt(conversation_id: string, payload: Uint8Array): Uint8Array;
    /**
     * The credential identities of all current group members.
     */
    group_members(conversation_id: string): Array<any>;
    /**
     * True when this device has joined the conversation's MLS group.
     */
    has_group(conversation_id: string): boolean;
    /**
     * Validate a fetched key package and return the identity it claims.
     */
    key_package_identity(_package: Uint8Array): string;
    /**
     * A fresh serialized key package for this device.
     */
    new_key_package(): Uint8Array;
    /**
     * Process one serialized MLS message. Returns
     * `{ kind: "application", sender, payload }`,
     * `{ kind: "commit" }` or `{ kind: "ignored" }`.
     */
    process_incoming(conversation_id: string, message: Uint8Array): any;
    /**
     * Process a serialized welcome: join the group. Returns the group's
     * conversation id.
     */
    process_welcome(welcome: Uint8Array): string;
    /**
     * Remove a member by identity; returns the serialized commit.
     */
    remove_member(conversation_id: string, identity: string): Uint8Array;
    /**
     * Serialize the device's private key material to JSON for IndexedDB.
     */
    toJSON(): string;
    /**
     * The device identity (`username#device-id`).
     */
    readonly identity: string;
    /**
     * The username part of the device identity.
     */
    readonly username: string;
}

/**
 * Decode payload bytes into an application message object.
 */
export function decodeAppMessage(payload: Uint8Array): any;

/**
 * Parse a relay payload into `{ kind, data: Uint8Array }`, or null.
 */
export function decodeEnvelope(payload: Uint8Array): any;

/**
 * Decode relay base64; throws on malformed input.
 */
export function decodePayload(encoded: string): Uint8Array;

/**
 * Parse a WebSocket text frame into a server frame object.
 */
export function decodeServerFrame(text: string): any;

/**
 * Decrypt a fetched blob; throws when the key, id or bytes do not match.
 */
export function decryptFile(key_base64: string, nonce_base64: string, blob_id: string, ciphertext: Uint8Array): Uint8Array;

/**
 * Reconstruct a device from [`DeviceState::toJSON`] output.
 */
export function deviceFromJson(json: string): DeviceState;

/**
 * Encode an application message object (same JSON shape as the Rust enum)
 * into the payload bytes that travel inside an MLS message.
 */
export function encodeAppMessage(message: any): Uint8Array;

/**
 * Encode a client frame object into the JSON string sent over WebSocket.
 */
export function encodeClientFrame(frame: any): string;

/**
 * Wrap serialized MLS bytes as the relay payload (JSON envelope string).
 */
export function encodeEnvelope(kind: string, data: Uint8Array): Uint8Array;

/**
 * Encode bytes as the base64 the relay protocol expects.
 */
export function encodePayload(data: Uint8Array): string;

/**
 * Encrypt a file for upload; returns the ciphertext with the GCM tag.
 */
export function encryptFile(key_base64: string, nonce_base64: string, blob_id: string, plaintext: Uint8Array): Uint8Array;

/**
 * Generate a fresh per-file key and nonce as
 * `{ keyBase64, nonceBase64 }`, ready to travel inside an
 * `AppMessage::Attachment`.
 */
export function newFileKey(): any;

/**
 * Output of `add_members` is built inline; no struct needed.
 * Generate fresh key material for a new device identity
 * (`username#device-id`).
 */
export function new_device(identity: string): DeviceState;

/**
 * Validate and canonicalize an account name; returns the lowercase form.
 */
export function validateUserName(raw: string): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_devicestate_free: (a: number, b: number) => void;
    readonly decodeAppMessage: (a: any) => [number, number, number];
    readonly decodeEnvelope: (a: any) => [number, number, number];
    readonly decodePayload: (a: number, b: number) => [number, number, number];
    readonly decodeServerFrame: (a: number, b: number) => [number, number, number];
    readonly decryptFile: (a: number, b: number, c: number, d: number, e: number, f: number, g: any) => [number, number, number];
    readonly deviceFromJson: (a: number, b: number) => [number, number, number];
    readonly devicestate_add_members: (a: number, b: number, c: number, d: number, e: number) => [number, number, number];
    readonly devicestate_create_group: (a: number, b: number, c: number) => [number, number];
    readonly devicestate_encrypt: (a: number, b: number, c: number, d: any) => [number, number, number];
    readonly devicestate_group_members: (a: number, b: number, c: number) => [number, number, number];
    readonly devicestate_has_group: (a: number, b: number, c: number) => [number, number, number];
    readonly devicestate_identity: (a: number) => [number, number];
    readonly devicestate_key_package_identity: (a: number, b: any) => [number, number, number, number];
    readonly devicestate_new_key_package: (a: number) => [number, number, number];
    readonly devicestate_process_incoming: (a: number, b: number, c: number, d: any) => [number, number, number];
    readonly devicestate_process_welcome: (a: number, b: any) => [number, number, number, number];
    readonly devicestate_remove_member: (a: number, b: number, c: number, d: number, e: number) => [number, number, number];
    readonly devicestate_toJSON: (a: number) => [number, number, number, number];
    readonly devicestate_username: (a: number) => [number, number];
    readonly encodeAppMessage: (a: any) => [number, number, number];
    readonly encodeClientFrame: (a: any) => [number, number, number, number];
    readonly encodeEnvelope: (a: number, b: number, c: any) => [number, number, number];
    readonly encodePayload: (a: any) => [number, number];
    readonly encryptFile: (a: number, b: number, c: number, d: number, e: number, f: number, g: any) => [number, number, number];
    readonly newFileKey: () => [number, number, number];
    readonly new_device: (a: number, b: number) => number;
    readonly validateUserName: (a: number, b: number) => [number, number, number, number];
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __externref_drop_slice: (a: number, b: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
