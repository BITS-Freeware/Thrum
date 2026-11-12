/* @ts-self-types="./thrum_wasm.d.ts" */

/**
 * A device's MLS state. Create with [`new_device`] or
 * [`DeviceState::from_json`]; persist with [`DeviceState::to_json`].
 */
export class DeviceState {
    static __wrap(ptr) {
        const obj = Object.create(DeviceState.prototype);
        obj.__wbg_ptr = ptr;
        DeviceStateFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        DeviceStateFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_devicestate_free(ptr, 0);
    }
    /**
     * Add members from serialized key packages. Returns
     * `{ commit: Uint8Array, welcome: Uint8Array | null }`.
     * @param {string} conversation_id
     * @param {Uint8Array[]} key_packages
     * @returns {any}
     */
    add_members(conversation_id, key_packages) {
        let ptr0, len0, ptr1, len1;
        try {
            ptr0 = passStringToWasm0(conversation_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            len0 = WASM_VECTOR_LEN;
            ptr1 = passArrayJsValueToWasm0(key_packages, wasm.__wbindgen_malloc);
            len1 = WASM_VECTOR_LEN;
        } catch (e) {
            if (ptr1) { wasm.__externref_drop_slice(ptr1, len1); wasm.__wbindgen_free(ptr1, len1 * 4, 4); }
            if (ptr0) { wasm.__wbindgen_free(ptr0, len0 * 1, 1); }
            throw e;
        }
        const ret = wasm.devicestate_add_members(this.__wbg_ptr, ptr0, len0, ptr1, len1);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return takeFromExternrefTable0(ret[0]);
    }
    /**
     * Create the MLS group for a conversation. Idempotent.
     * @param {string} conversation_id
     */
    create_group(conversation_id) {
        const ptr0 = passStringToWasm0(conversation_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.devicestate_create_group(this.__wbg_ptr, ptr0, len0);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Encrypt an application payload for the conversation's group.
     * @param {string} conversation_id
     * @param {Uint8Array} payload
     * @returns {Uint8Array}
     */
    encrypt(conversation_id, payload) {
        const ptr0 = passStringToWasm0(conversation_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.devicestate_encrypt(this.__wbg_ptr, ptr0, len0, payload);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return takeFromExternrefTable0(ret[0]);
    }
    /**
     * The credential identities of all current group members.
     * @param {string} conversation_id
     * @returns {Array<any>}
     */
    group_members(conversation_id) {
        const ptr0 = passStringToWasm0(conversation_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.devicestate_group_members(this.__wbg_ptr, ptr0, len0);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return takeFromExternrefTable0(ret[0]);
    }
    /**
     * True when this device has joined the conversation's MLS group.
     * @param {string} conversation_id
     * @returns {boolean}
     */
    has_group(conversation_id) {
        const ptr0 = passStringToWasm0(conversation_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.devicestate_has_group(this.__wbg_ptr, ptr0, len0);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return ret[0] !== 0;
    }
    /**
     * The device identity (`username#device-id`).
     * @returns {string}
     */
    get identity() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.devicestate_identity(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Validate a fetched key package and return the identity it claims.
     * @param {Uint8Array} _package
     * @returns {string}
     */
    key_package_identity(_package) {
        let deferred2_0;
        let deferred2_1;
        try {
            const ret = wasm.devicestate_key_package_identity(this.__wbg_ptr, _package);
            var ptr1 = ret[0];
            var len1 = ret[1];
            if (ret[3]) {
                ptr1 = 0; len1 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred2_0 = ptr1;
            deferred2_1 = len1;
            return getStringFromWasm0(ptr1, len1);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * A fresh serialized key package for this device.
     * @returns {Uint8Array}
     */
    new_key_package() {
        const ret = wasm.devicestate_new_key_package(this.__wbg_ptr);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return takeFromExternrefTable0(ret[0]);
    }
    /**
     * Process one serialized MLS message. Returns
     * `{ kind: "application", sender, payload }`,
     * `{ kind: "commit" }` or `{ kind: "ignored" }`.
     * @param {string} conversation_id
     * @param {Uint8Array} message
     * @returns {any}
     */
    process_incoming(conversation_id, message) {
        const ptr0 = passStringToWasm0(conversation_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.devicestate_process_incoming(this.__wbg_ptr, ptr0, len0, message);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return takeFromExternrefTable0(ret[0]);
    }
    /**
     * Process a serialized welcome: join the group. Returns the group's
     * conversation id.
     * @param {Uint8Array} welcome
     * @returns {string}
     */
    process_welcome(welcome) {
        let deferred2_0;
        let deferred2_1;
        try {
            const ret = wasm.devicestate_process_welcome(this.__wbg_ptr, welcome);
            var ptr1 = ret[0];
            var len1 = ret[1];
            if (ret[3]) {
                ptr1 = 0; len1 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred2_0 = ptr1;
            deferred2_1 = len1;
            return getStringFromWasm0(ptr1, len1);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * Remove a member by identity; returns the serialized commit.
     * @param {string} conversation_id
     * @param {string} identity
     * @returns {Uint8Array}
     */
    remove_member(conversation_id, identity) {
        let ptr0, len0, ptr1, len1;
        try {
            ptr0 = passStringToWasm0(conversation_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            len0 = WASM_VECTOR_LEN;
            ptr1 = passStringToWasm0(identity, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            len1 = WASM_VECTOR_LEN;
        } catch (e) {
            if (ptr1) { wasm.__wbindgen_free(ptr1, len1 * 1, 1); }
            if (ptr0) { wasm.__wbindgen_free(ptr0, len0 * 1, 1); }
            throw e;
        }
        const ret = wasm.devicestate_remove_member(this.__wbg_ptr, ptr0, len0, ptr1, len1);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return takeFromExternrefTable0(ret[0]);
    }
    /**
     * Serialize the device's private key material to JSON for IndexedDB.
     * @returns {string}
     */
    toJSON() {
        let deferred2_0;
        let deferred2_1;
        try {
            const ret = wasm.devicestate_toJSON(this.__wbg_ptr);
            var ptr1 = ret[0];
            var len1 = ret[1];
            if (ret[3]) {
                ptr1 = 0; len1 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred2_0 = ptr1;
            deferred2_1 = len1;
            return getStringFromWasm0(ptr1, len1);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * The username part of the device identity.
     * @returns {string}
     */
    get username() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.devicestate_username(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
}
if (Symbol.dispose) DeviceState.prototype[Symbol.dispose] = DeviceState.prototype.free;

/**
 * Decode payload bytes into an application message object.
 * @param {Uint8Array} payload
 * @returns {any}
 */
export function decodeAppMessage(payload) {
    const ret = wasm.decodeAppMessage(payload);
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return takeFromExternrefTable0(ret[0]);
}

/**
 * Parse a relay payload into `{ kind, data: Uint8Array }`, or null.
 * @param {Uint8Array} payload
 * @returns {any}
 */
export function decodeEnvelope(payload) {
    const ret = wasm.decodeEnvelope(payload);
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return takeFromExternrefTable0(ret[0]);
}

/**
 * Decode relay base64; throws on malformed input.
 * @param {string} encoded
 * @returns {Uint8Array}
 */
export function decodePayload(encoded) {
    const ptr0 = passStringToWasm0(encoded, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.decodePayload(ptr0, len0);
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return takeFromExternrefTable0(ret[0]);
}

/**
 * Parse a WebSocket text frame into a server frame object.
 * @param {string} text
 * @returns {any}
 */
export function decodeServerFrame(text) {
    const ptr0 = passStringToWasm0(text, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.decodeServerFrame(ptr0, len0);
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return takeFromExternrefTable0(ret[0]);
}

/**
 * Decrypt a fetched blob; throws when the key, id or bytes do not match.
 * @param {string} key_base64
 * @param {string} nonce_base64
 * @param {string} blob_id
 * @param {Uint8Array} ciphertext
 * @returns {Uint8Array}
 */
export function decryptFile(key_base64, nonce_base64, blob_id, ciphertext) {
    let ptr0, len0, ptr1, len1, ptr2, len2;
    try {
        ptr0 = passStringToWasm0(key_base64, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        len0 = WASM_VECTOR_LEN;
        ptr1 = passStringToWasm0(nonce_base64, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        len1 = WASM_VECTOR_LEN;
        ptr2 = passStringToWasm0(blob_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        len2 = WASM_VECTOR_LEN;
    } catch (e) {
        if (ptr2) { wasm.__wbindgen_free(ptr2, len2 * 1, 1); }
        if (ptr1) { wasm.__wbindgen_free(ptr1, len1 * 1, 1); }
        if (ptr0) { wasm.__wbindgen_free(ptr0, len0 * 1, 1); }
        throw e;
    }
    const ret = wasm.decryptFile(ptr0, len0, ptr1, len1, ptr2, len2, ciphertext);
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return takeFromExternrefTable0(ret[0]);
}

/**
 * Reconstruct a device from [`DeviceState::toJSON`] output.
 * @param {string} json
 * @returns {DeviceState}
 */
export function deviceFromJson(json) {
    const ptr0 = passStringToWasm0(json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.deviceFromJson(ptr0, len0);
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return DeviceState.__wrap(ret[0]);
}

/**
 * Encode an application message object (same JSON shape as the Rust enum)
 * into the payload bytes that travel inside an MLS message.
 * @param {any} message
 * @returns {Uint8Array}
 */
export function encodeAppMessage(message) {
    const ret = wasm.encodeAppMessage(message);
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return takeFromExternrefTable0(ret[0]);
}

/**
 * Encode a client frame object into the JSON string sent over WebSocket.
 * @param {any} frame
 * @returns {string}
 */
export function encodeClientFrame(frame) {
    let deferred2_0;
    let deferred2_1;
    try {
        const ret = wasm.encodeClientFrame(frame);
        var ptr1 = ret[0];
        var len1 = ret[1];
        if (ret[3]) {
            ptr1 = 0; len1 = 0;
            throw takeFromExternrefTable0(ret[2]);
        }
        deferred2_0 = ptr1;
        deferred2_1 = len1;
        return getStringFromWasm0(ptr1, len1);
    } finally {
        wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
    }
}

/**
 * Wrap serialized MLS bytes as the relay payload (JSON envelope string).
 * @param {string} kind
 * @param {Uint8Array} data
 * @returns {Uint8Array}
 */
export function encodeEnvelope(kind, data) {
    const ptr0 = passStringToWasm0(kind, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.encodeEnvelope(ptr0, len0, data);
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return takeFromExternrefTable0(ret[0]);
}

/**
 * Encode bytes as the base64 the relay protocol expects.
 * @param {Uint8Array} data
 * @returns {string}
 */
export function encodePayload(data) {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.encodePayload(data);
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * Encrypt a file for upload; returns the ciphertext with the GCM tag.
 * @param {string} key_base64
 * @param {string} nonce_base64
 * @param {string} blob_id
 * @param {Uint8Array} plaintext
 * @returns {Uint8Array}
 */
export function encryptFile(key_base64, nonce_base64, blob_id, plaintext) {
    let ptr0, len0, ptr1, len1, ptr2, len2;
    try {
        ptr0 = passStringToWasm0(key_base64, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        len0 = WASM_VECTOR_LEN;
        ptr1 = passStringToWasm0(nonce_base64, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        len1 = WASM_VECTOR_LEN;
        ptr2 = passStringToWasm0(blob_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        len2 = WASM_VECTOR_LEN;
    } catch (e) {
        if (ptr2) { wasm.__wbindgen_free(ptr2, len2 * 1, 1); }
        if (ptr1) { wasm.__wbindgen_free(ptr1, len1 * 1, 1); }
        if (ptr0) { wasm.__wbindgen_free(ptr0, len0 * 1, 1); }
        throw e;
    }
    const ret = wasm.encryptFile(ptr0, len0, ptr1, len1, ptr2, len2, plaintext);
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return takeFromExternrefTable0(ret[0]);
}

/**
 * Generate a fresh per-file key and nonce as
 * `{ keyBase64, nonceBase64 }`, ready to travel inside an
 * `AppMessage::Attachment`.
 * @returns {any}
 */
export function newFileKey() {
    const ret = wasm.newFileKey();
    if (ret[2]) {
        throw takeFromExternrefTable0(ret[1]);
    }
    return takeFromExternrefTable0(ret[0]);
}

/**
 * Output of `add_members` is built inline; no struct needed.
 * Generate fresh key material for a new device identity
 * (`username#device-id`).
 * @param {string} identity
 * @returns {DeviceState}
 */
export function new_device(identity) {
    const ptr0 = passStringToWasm0(identity, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.new_device(ptr0, len0);
    return DeviceState.__wrap(ret);
}

/**
 * Validate and canonicalize an account name; returns the lowercase form.
 * @param {string} raw
 * @returns {string}
 */
export function validateUserName(raw) {
    let deferred3_0;
    let deferred3_1;
    try {
        const ptr0 = passStringToWasm0(raw, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.validateUserName(ptr0, len0);
        var ptr2 = ret[0];
        var len2 = ret[1];
        if (ret[3]) {
            ptr2 = 0; len2 = 0;
            throw takeFromExternrefTable0(ret[2]);
        }
        deferred3_0 = ptr2;
        deferred3_1 = len2;
        return getStringFromWasm0(ptr2, len2);
    } finally {
        wasm.__wbindgen_free(deferred3_0, deferred3_1, 1);
    }
}
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg_Error_208446807d0d1e30: function(arg0, arg1) {
            const ret = Error(getStringFromWasm0(arg0, arg1));
            return ret;
        },
        __wbg_String_8564e559799eccda: function(arg0, arg1) {
            const ret = String(arg1);
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32((arg0 >>> 0) + 4 * 1, len1, true);
            getDataViewMemory0().setInt32((arg0 >>> 0) + 4 * 0, ptr1, true);
        },
        __wbg___wbindgen_bigint_get_as_i64_11da4b52cd0aedef: function(arg0, arg1) {
            const v = arg1;
            const ret = typeof(v) === 'bigint' ? v : undefined;
            getDataViewMemory0().setBigInt64((arg0 >>> 0) + 8 * 1, isLikeNone(ret) ? BigInt(0) : ret, true);
            getDataViewMemory0().setInt32((arg0 >>> 0) + 4 * 0, !isLikeNone(ret), true);
        },
        __wbg___wbindgen_boolean_get_4f1baf9a00be3c74: function(arg0) {
            const v = arg0;
            const ret = typeof(v) === 'boolean' ? v : undefined;
            return isLikeNone(ret) ? 0xFFFFFF : ret ? 1 : 0;
        },
        __wbg___wbindgen_debug_string_f249a2a76caa9a55: function(arg0, arg1) {
            const ret = debugString(arg1);
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32((arg0 >>> 0) + 4 * 1, len1, true);
            getDataViewMemory0().setInt32((arg0 >>> 0) + 4 * 0, ptr1, true);
        },
        __wbg___wbindgen_in_3f58a9d3b3a7eed2: function(arg0, arg1) {
            const ret = arg0 in arg1;
            return ret;
        },
        __wbg___wbindgen_is_bigint_5fa10641e039d3ef: function(arg0) {
            const ret = typeof(arg0) === 'bigint';
            return ret;
        },
        __wbg___wbindgen_is_function_741ff2bb612836f5: function(arg0) {
            const ret = typeof(arg0) === 'function';
            return ret;
        },
        __wbg___wbindgen_is_object_8b968c3ab8e96595: function(arg0) {
            const val = arg0;
            const ret = typeof(val) === 'object' && val !== null;
            return ret;
        },
        __wbg___wbindgen_is_string_fc456fee5d3317de: function(arg0) {
            const ret = typeof(arg0) === 'string';
            return ret;
        },
        __wbg___wbindgen_is_undefined_be5b34249ff1fc15: function(arg0) {
            const ret = arg0 === undefined;
            return ret;
        },
        __wbg___wbindgen_jsval_eq_38d7bfbe4cc84039: function(arg0, arg1) {
            const ret = arg0 === arg1;
            return ret;
        },
        __wbg___wbindgen_jsval_loose_eq_b2f214e2e4b30e36: function(arg0, arg1) {
            const ret = arg0 == arg1;
            return ret;
        },
        __wbg___wbindgen_number_get_934b640d0889af78: function(arg0, arg1) {
            const obj = arg1;
            const ret = typeof(obj) === 'number' ? obj : undefined;
            getDataViewMemory0().setFloat64((arg0 >>> 0) + 8 * 1, isLikeNone(ret) ? 0 : ret, true);
            getDataViewMemory0().setInt32((arg0 >>> 0) + 4 * 0, !isLikeNone(ret), true);
        },
        __wbg___wbindgen_string_get_7ce8eee7fd327545: function(arg0, arg1) {
            const obj = arg1;
            const ret = typeof(obj) === 'string' ? obj : undefined;
            var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            var len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32((arg0 >>> 0) + 4 * 1, len1, true);
            getDataViewMemory0().setInt32((arg0 >>> 0) + 4 * 0, ptr1, true);
        },
        __wbg___wbindgen_throw_4c88d8bf264fd984: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg_call_40b008ba32f73991: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.call(arg1);
            return ret;
        }, arguments); },
        __wbg_call_a410eb4b59ed5965: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.call(arg1, arg2);
            return ret;
        }, arguments); },
        __wbg_crypto_23f565a907d30078: function(arg0) {
            const ret = arg0.crypto;
            return ret;
        },
        __wbg_done_57e08d0be6e30c02: function(arg0) {
            const ret = arg0.done;
            return ret;
        },
        __wbg_entries_7bb1459bd9b5c2b9: function(arg0) {
            const ret = Object.entries(arg0);
            return ret;
        },
        __wbg_getRandomValues_5d8c030b527203a6: function() { return handleError(function (arg0, arg1) {
            globalThis.crypto.getRandomValues(getArrayU8FromWasm0(arg0, arg1));
        }, arguments); },
        __wbg_getRandomValues_8b6f1f663e6f188d: function() { return handleError(function (arg0, arg1) {
            globalThis.crypto.getRandomValues(getArrayU8FromWasm0(arg0, arg1));
        }, arguments); },
        __wbg_getRandomValues_d2908e0b99612b91: function() { return handleError(function (arg0, arg1) {
            arg0.getRandomValues(arg1);
        }, arguments); },
        __wbg_get_635eea311fa1a291: function() { return handleError(function (arg0, arg1) {
            const ret = Reflect.get(arg0, arg1);
            return ret;
        }, arguments); },
        __wbg_get_d7176cfb76d71114: function(arg0, arg1) {
            const ret = arg0[arg1 >>> 0];
            return ret;
        },
        __wbg_get_unchecked_e6325ad374eff861: function(arg0, arg1) {
            const ret = arg0[arg1 >>> 0];
            return ret;
        },
        __wbg_instanceof_ArrayBuffer_12cdaa4161423e6b: function(arg0) {
            let result;
            try {
                result = arg0 instanceof ArrayBuffer;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_Map_0b4fc8b62e73f163: function(arg0) {
            let result;
            try {
                result = arg0 instanceof Map;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_Uint8Array_d7aeb0dcb3108fea: function(arg0) {
            let result;
            try {
                result = arg0 instanceof Uint8Array;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_isArray_bcf1376d0d732d08: function(arg0) {
            const ret = Array.isArray(arg0);
            return ret;
        },
        __wbg_isSafeInteger_1a35ba4d31e71d38: function(arg0) {
            const ret = Number.isSafeInteger(arg0);
            return ret;
        },
        __wbg_iterator_e4c5e8f68ada7b58: function() {
            const ret = Symbol.iterator;
            return ret;
        },
        __wbg_length_186163212ffe79c3: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_length_b03a0fabb109b1e9: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_msCrypto_d5f06fad6bf1409b: function(arg0) {
            const ret = arg0.msCrypto;
            return ret;
        },
        __wbg_new_13de428334042d26: function(arg0) {
            const ret = new Uint8Array(arg0);
            return ret;
        },
        __wbg_new_37eb45aaf5eeb820: function() {
            const ret = new Array();
            return ret;
        },
        __wbg_new_cd4348290afae586: function() {
            const ret = new Object();
            return ret;
        },
        __wbg_new_with_length_01b143a7e5dd5a3e: function(arg0) {
            const ret = new Uint8Array(arg0 >>> 0);
            return ret;
        },
        __wbg_next_178c1651c11432b3: function(arg0) {
            const ret = arg0.next;
            return ret;
        },
        __wbg_next_af5ac6a08bb0a717: function() { return handleError(function (arg0) {
            const ret = arg0.next();
            return ret;
        }, arguments); },
        __wbg_node_86166ff8ce1f2002: function(arg0) {
            const ret = arg0.node;
            return ret;
        },
        __wbg_now_463ac581d2fc7ea3: function() {
            const ret = Date.now();
            return ret;
        },
        __wbg_process_91f425a63f58e399: function(arg0) {
            const ret = arg0.process;
            return ret;
        },
        __wbg_prototypesetcall_13162965d1fe7aa3: function(arg0, arg1, arg2) {
            Uint8Array.prototype.set.call(getArrayU8FromWasm0(arg0, arg1), arg2);
        },
        __wbg_push_c9f4893fc209f768: function(arg0, arg1) {
            const ret = arg0.push(arg1);
            return ret;
        },
        __wbg_randomFillSync_34b023059e3363de: function() { return handleError(function (arg0, arg1) {
            arg0.randomFillSync(arg1);
        }, arguments); },
        __wbg_require_9a7d8cc2e1d275c0: function() { return handleError(function () {
            const ret = module.require;
            return ret;
        }, arguments); },
        __wbg_set_2793479888329cb6: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = Reflect.set(arg0, arg1, arg2);
            return ret;
        }, arguments); },
        __wbg_set_d2d33b52458e984a: function(arg0, arg1, arg2) {
            arg0[arg1 >>> 0] = arg2;
        },
        __wbg_set_e4b9a833d288759a: function(arg0, arg1, arg2) {
            arg0[arg1] = arg2;
        },
        __wbg_set_feb3714f475485ba: function(arg0, arg1, arg2) {
            arg0.set(getArrayU8FromWasm0(arg1, arg2));
        },
        __wbg_static_accessor_GLOBAL_THIS_003ee833af9699fc: function() {
            const ret = typeof globalThis === 'undefined' ? undefined : globalThis;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_static_accessor_GLOBAL_e4293fb22cbe77da: function() {
            const ret = typeof global === 'undefined' ? undefined : global;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_static_accessor_SELF_53bb20457401453d: function() {
            const ret = typeof self === 'undefined' ? undefined : self;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_static_accessor_WINDOW_b5e0810bfecb7feb: function() {
            const ret = typeof window === 'undefined' ? undefined : window;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_subarray_58f388a9247ac0bb: function(arg0, arg1, arg2) {
            const ret = arg0.subarray(arg1 >>> 0, arg2 >>> 0);
            return ret;
        },
        __wbg_value_e0f328965e184931: function(arg0) {
            const ret = arg0.value;
            return ret;
        },
        __wbg_versions_6e128a981b0df859: function(arg0) {
            const ret = arg0.versions;
            return ret;
        },
        __wbindgen_generic_0000000000000001: function(arg0) {
            // Cast intrinsic for `F64 -> Externref`.
            const ret = arg0;
            return ret;
        },
        __wbindgen_generic_0000000000000002: function(arg0) {
            // Cast intrinsic for `I64 -> Externref`.
            const ret = arg0;
            return ret;
        },
        __wbindgen_generic_0000000000000003: function(arg0, arg1) {
            // Cast intrinsic for `Ref(Slice(U8)) -> NamedExternref("Uint8Array")`.
            const ret = getArrayU8FromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_generic_0000000000000004: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_generic_0000000000000005: function(arg0) {
            // Cast intrinsic for `U64 -> Externref`.
            const ret = BigInt.asUintN(64, arg0);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            table.set(0, undefined);
            table.set(1024, undefined);
            table.set(1025, null);
            table.set(1026, true);
            table.set(1027, false);
        },
    };
    return {
        __proto__: null,
        "./thrum_wasm_bg.js": import0,
    };
}

const DeviceStateFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_devicestate_free(ptr, 1));

function addToExternrefTable0(obj) {
    const idx = wasm.__externref_table_alloc();
    wasm.__wbindgen_externrefs.set(idx, obj);
    return idx;
}

function debugString(val) {
    switch (typeof val) {
        // primitive types
        case 'number':
        case 'boolean':
        case 'undefined':
        return `${val}`;
        case 'string':
        return JSON.stringify(val);
        case 'bigint':
        return `${val}n`;
        case 'symbol':
        return val.description !== undefined
            ? `Symbol(${val.description})`
            : 'Symbol';
        case 'function':
        return typeof val.name === 'string' && val.name !== ''
            ? `Function(${val.name})`
            : 'Function';
        case 'object': {
            if (val === null) {
                return 'null';
            }
            // objects
            if (Array.isArray(val)) {
                return '[' + Array.from(val, debugString).join(', ') + ']';
            }
            // errors
            if (val instanceof Error) {
                return `${val.name}: ${val.message}\n${val.stack}`;
            }

            const toStringOutput = toString.call(val);
            // Test for built-in
            const className = /\[object ([^\]]+)\]/.exec(toStringOutput)?.[1];

            // Failed to match the standard '[object ClassName]'
            if (className === undefined) {
                return toStringOutput;
            }
            if (className === 'Object') {
                // we're a user defined class or Object
                // JSON.stringify avoids problems with cycles, and is generally much
                // easier than looping through ownProperties of `val`.
                try {
                    return 'Object(' + JSON.stringify(val) + ')';
                } catch (_) {
                    return 'Object';
                }
            }
            // TODO we could test for more things here, like `Set`s and `Map`s.
            return className;
        }
        // Per MDN: 'This list of values is exhaustive.'
        // https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Operators/typeof#description
    }
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

function getStringFromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return decodeText(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function handleError(f, args) {
    try {
        return f.apply(this, args);
    } catch (e) {
        const idx = addToExternrefTable0(e);
        wasm.__wbindgen_exn_store(idx);
    }
}

function isLikeNone(x) {
    return x === undefined || x === null;
}

function passArrayJsValueToWasm0(array, malloc) {
    const ptr = malloc(array.length * 4, 4) >>> 0;
    for (let i = 0; i < array.length; i++) {
        const add = addToExternrefTable0(array[i]);
        getDataViewMemory0().setUint32(ptr + 4 * i, add, true);
    }
    WASM_VECTOR_LEN = array.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(bytes) {
    numBytesDecoded += bytes.length;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true });
        cachedTextDecoder.decode();
        numBytesDecoded = bytes.length;
    }
    return cachedTextDecoder.decode(bytes);
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (!module.ok) {
            throw new Error(`failed to fetch Wasm: ${module.status} ${module.statusText} fetching '${module.url}'`);
        }

        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

let __wbg_init_promise;
async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;
    if (__wbg_init_promise === undefined) {
        __wbg_init_promise = __wbg_init_once(module_or_path);
        __wbg_init_promise.catch(() => { __wbg_init_promise = undefined; });
    }
    return __wbg_init_promise;
}

async function __wbg_init_once(module_or_path) {

    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('thrum_wasm_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
