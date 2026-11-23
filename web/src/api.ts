/// The server's JSON API, mirroring the CLI client (`crates/cli/src/api.rs`).
/// Every request carries the session token; errors surface as `ApiError`
/// with the server's stable error code.

export class ApiError extends Error {
  status: number;
  code: string;
  constructor(status: number, code: string) {
    super(`server said ${status}: ${code}`);
    this.status = status;
    this.code = code;
  }
}

export interface Session {
  server: string;
  username: string;
  token: string;
  device_id: string;
}

export class Api {
  readonly server: string;
  private token: string | null = null;

  constructor(server: string) {
    this.server = server.replace(/\/$/, "");
  }

  setToken(token: string | null) {
    this.token = token;
  }

  private async call(
    method: string,
    path: string,
    options: { body?: BodyInit; json?: unknown; headers?: Record<string, string> } = {},
  ): Promise<Response> {
    const headers: Record<string, string> = { ...options.headers };
    if (this.token) headers["authorization"] = `Bearer ${this.token}`;
    if (options.json !== undefined) headers["content-type"] = "application/json";
    let response: Response;
    try {
      response = await fetch(`${this.server}${path}`, {
        method,
        headers,
        body: options.json !== undefined ? JSON.stringify(options.json) : options.body,
      });
    } catch (error) {
      throw new ApiError(0, `request failed: ${String(error)}`);
    }
    return response;
  }

  private async expectOk(response: Response): Promise<Response> {
    if (response.ok) return response;
    let code = "unknown";
    try {
      const body = (await response.json()) as { error?: string };
      if (body.error) code = body.error;
    } catch {
      // keep the fallback code
    }
    throw new ApiError(response.status, code);
  }

  private async expectJson<T>(response: Response): Promise<T> {
    await this.expectOk(response);
    return (await response.json()) as T;
  }

  async register(username: string, password: string): Promise<void> {
    await this.expectOk(
      await this.call("POST", "/v1/accounts", { json: { username, password } }),
    );
  }

  async login(username: string, password: string, deviceLabel: string): Promise<Session> {
    const body = await this.expectJson<{
      username: string;
      token: string;
      device_id: string;
    }>(
      await this.call("POST", "/v1/sessions", {
        json: { username, password, device_label: deviceLabel },
      }),
    );
    return { server: this.server, username: body.username, token: body.token, device_id: body.device_id };
  }

  async logout(): Promise<void> {
    await this.expectOk(await this.call("DELETE", "/v1/sessions/current"));
  }

  async listDevices(): Promise<{ id: string; label: string; created_at: number; active: boolean }[]> {
    const body = await this.expectJson<
      { id: string; label: string; created_at: number; active: boolean }[]
    >(await this.call("GET", "/v1/devices"));
    return body ?? [];
  }

  async revokeDevice(deviceId: string): Promise<void> {
    await this.expectOk(await this.call("DELETE", `/v1/devices/${encodeURIComponent(deviceId)}`));
  }

  async uploadKeyPackage(bytes: Uint8Array, lastResort = false): Promise<void> {
    const headers: Record<string, string> = { "content-type": "application/octet-stream" };
    if (lastResort) headers["x-thrum-key-package-role"] = "last-resort";
    await this.expectOk(
      await this.call("POST", "/v1/key-packages", {
        body: bytes as unknown as BodyInit,
        headers,
      }),
    );
  }

  /** The key packages the server holds for this device (content base64). */
  async ownKeyPackages(): Promise<{ id: string; content: string }[]> {
    const body = await this.expectJson<{ packages: { id: string; content: string }[] }>(
      await this.call("GET", "/v1/key-packages"),
    );
    return body.packages ?? [];
  }

  /** Withdraw one of this device's stored key packages; one already gone is fine. */
  async deleteKeyPackage(id: string): Promise<void> {
    const response = await this.call("DELETE", `/v1/key-packages/${encodeURIComponent(id)}`);
    if (response.status === 404) return;
    await this.expectOk(response);
  }

  async keyPackageStock(): Promise<{ count: number; lastResort: boolean }> {
    const body = await this.expectJson<{ count: number; last_resort?: boolean }>(
      await this.call("GET", "/v1/key-packages/count"),
    );
    return { count: body.count, lastResort: body.last_resort === true };
  }

  async activeDevices(username: string): Promise<string[]> {
    return Object.keys(await this.activeDevicesWithKeys(username));
  }

  /**
   * The account's active device ids, each with the signature key the
   * server bound to it (base64), or null while the device has not uploaded
   * a key package yet.
   */
  async activeDevicesWithKeys(username: string): Promise<Record<string, string | null>> {
    const body = await this.expectJson<{
      devices: string[];
      signature_keys?: Record<string, string | null>;
    }>(await this.call("GET", `/v1/users/${encodeURIComponent(username)}/devices`));
    const keys = body.signature_keys ?? {};
    const devices: Record<string, string | null> = {};
    for (const device of body.devices ?? []) devices[device] = keys[device] ?? null;
    return devices;
  }

  async serverInfo(): Promise<{ sourceUrl: string }> {
    const body = await this.expectJson<{ source_url?: string }>(await this.call("GET", "/v1/meta"));
    return { sourceUrl: body.source_url ?? "" };
  }

  async devicesWithKeyPackages(username: string): Promise<string[]> {
    const body = await this.expectJson<{ devices: string[] }>(
      await this.call("GET", `/v1/users/${encodeURIComponent(username)}/key-package-devices`),
    );
    return body.devices ?? [];
  }

  async fetchKeyPackage(deviceId: string): Promise<Uint8Array | null> {
    const response = await this.call(
      "GET",
      `/v1/devices/${encodeURIComponent(deviceId)}/key-package`,
    );
    if (response.status === 404) return null;
    await this.expectOk(response);
    return new Uint8Array(await response.arrayBuffer());
  }

  async createConversation(
    members: string[],
    name?: string,
  ): Promise<{ id: string; members: string[] }> {
    const body = await this.expectJson<{ id: string; members: string[] }>(
      await this.call("POST", "/v1/conversations", { json: name ? { members, name } : { members } }),
    );
    return body;
  }

  async listConversations(): Promise<{ id: string; members: string[]; name?: string }[]> {
    const body = await this.expectJson<
      { id: string; members: string[]; name?: string }[]
    >(await this.call("GET", "/v1/conversations"));
    return body ?? [];
  }

  async addMembers(conversationId: string, members: string[]): Promise<string[]> {
    const body = await this.expectJson<{ members: string[] }>(
      await this.call("POST", `/v1/conversations/${encodeURIComponent(conversationId)}/members`, {
        json: { members },
      }),
    );
    return body.members ?? [];
  }

  async removeMember(conversationId: string, username: string): Promise<void> {
    await this.expectOk(
      await this.call(
        "DELETE",
        `/v1/conversations/${encodeURIComponent(conversationId)}/members/${encodeURIComponent(username)}`,
      ),
    );
  }

  async renameConversation(conversationId: string, name: string): Promise<void> {
    await this.expectOk(
      await this.call("PATCH", `/v1/conversations/${encodeURIComponent(conversationId)}/name`, {
        json: { name },
      }),
    );
  }

  async sendMessage(conversationId: string, payload: Uint8Array): Promise<string> {
    const body = await this.expectJson<{ id: string }>(
      await this.call("POST", `/v1/conversations/${encodeURIComponent(conversationId)}/messages`, {
        body: payload as unknown as BodyInit,
        headers: { "content-type": "application/octet-stream" },
      }),
    );
    return body.id;
  }

  async createInvite(conversationId: string): Promise<string> {
    const body = await this.expectJson<{ token: string }>(
      await this.call("POST", `/v1/conversations/${encodeURIComponent(conversationId)}/invites`, {
        json: {},
      }),
    );
    return body.token;
  }

  async revokeInvite(conversationId: string, inviteToken: string): Promise<void> {
    await this.expectOk(
      await this.call(
        "POST",
        `/v1/conversations/${encodeURIComponent(conversationId)}/invites/revoke`,
        { json: { token: inviteToken } },
      ),
    );
  }

  async redeemInvite(inviteToken: string): Promise<{ id: string; members: string[] }> {
    return await this.expectJson<{ id: string; members: string[] }>(
      await this.call("POST", "/v1/invites/redeem", { json: { token: inviteToken } }),
    );
  }

  async block(username: string): Promise<void> {
    await this.expectOk(
      await this.call("POST", "/v1/blocks", { json: { username } }),
    );
  }

  async unblock(username: string): Promise<void> {
    await this.expectOk(await this.call("DELETE", `/v1/blocks/${encodeURIComponent(username)}`));
  }

  async uploadBlob(blobId: string, bytes: Uint8Array): Promise<void> {
    await this.expectOk(
      await this.call("POST", "/v1/blobs", {
        body: bytes as unknown as BodyInit,
        headers: {
          "content-type": "application/octet-stream",
          "x-thrum-blob-id": blobId,
        },
      }),
    );
  }

  async fetchBlob(blobId: string): Promise<Uint8Array | null> {
    const response = await this.call("GET", `/v1/blobs/${encodeURIComponent(blobId)}`);
    if (response.status === 404) return null;
    await this.expectOk(response);
    return new Uint8Array(await response.arrayBuffer());
  }
}
