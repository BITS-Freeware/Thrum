/// The live WebSocket connection: authenticates, delivers incoming frames
/// to the session, and acks them. Reconnects with a short backoff.

import type { CryptoSession } from "./session";

export class LiveConnection {
  private socket: WebSocket | null = null;
  private closed = false;
  private reconnectDelayMs = 1000;
  private onChange: (connected: boolean) => void;
  /**
   * Frames are handled strictly one at a time, in arrival order. Two
   * reasons: the server may deliver the same message both live and from
   * the catch-up queue (clients deduplicate by message id, which only
   * works when handling is serialized), and MLS state must see the
   * welcome before the commit and the application messages that follow.
   */
  private processing: Promise<void> = Promise.resolve();
  /** When the current socket opened, and when its last frame arrived. */
  private openedAt: number | null = null;
  private lastFrameAt = 0;

  constructor(
    private readonly session: CryptoSession,
    onChange: (connected: boolean) => void,
  ) {
    this.session = session;
    this.onChange = onChange;
  }

  /** Open (or re-open) the WebSocket and pump frames until closed. */
  start(): void {
    this.closed = false;
    this.connect();
  }

  /** Resolves once every frame received so far has been handled. */
  idle(): Promise<void> {
    return this.processing;
  }

  /**
   * Resolves once the server's queue for this device has been applied:
   * the connection is authenticated, every frame received is handled, and
   * no frame has arrived for a short quiet period (the server sends the
   * queue right after authentication). Resolves at once when offline,
   * and gives up after a few seconds so a busy group cannot stall callers.
   */
  async caughtUp(): Promise<void> {
    const quietMs = 700;
    const giveUpAt = Date.now() + 5000;
    for (;;) {
      await this.processing;
      const socket = this.socket;
      if (!socket || socket.readyState !== WebSocket.OPEN) return;
      const now = Date.now();
      const quiet =
        this.openedAt !== null &&
        now - this.openedAt >= quietMs &&
        now - this.lastFrameAt >= quietMs;
      if (quiet) return;
      if (now >= giveUpAt) return;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }

  stop(): void {
    this.closed = true;
    this.socket?.close();
    this.socket = null;
    this.onChange(false);
  }

  private connect(): void {
    if (this.closed) return;
    const session = this.session.current;
    if (!session) return;
    const url = new URL(session.server.replace(/^http/, "ws"));
    url.pathname = "/v1/ws";
    const socket = new WebSocket(url);
    this.socket = socket;
    socket.onopen = () => {
      this.openedAt = Date.now();
      this.reconnectDelayMs = 1000;
      this.onChange(true);
      socket.send(this.session.authFrame());
    };
    socket.onmessage = (event) => {
      if (typeof event.data !== "string") return;
      const text = event.data;
      this.lastFrameAt = Date.now();
      this.processing = this.processing.then(() => this.deliver(text));
    };
    socket.onclose = () => {
      if (this.socket === socket) {
        this.socket = null;
        this.openedAt = null;
      }
      this.onChange(false);
      if (!this.closed) {
        setTimeout(() => this.connect(), this.reconnectDelayMs);
        this.reconnectDelayMs = Math.min(this.reconnectDelayMs * 2, 15000);
      }
    };
    socket.onerror = () => {
      socket.close();
    };
  }

  /**
   * Handle one frame: apply it, then ack it. A frame whose handling
   * failed (or that the session could not apply yet) is left un-acked,
   * so the server offers it again on the next connection instead of
   * losing it. This method never rejects: the processing chain must
   * keep flowing for later frames.
   */
  private async deliver(text: string): Promise<void> {
    try {
      const ackId = await this.session.handleServerFrame(text);
      const socket = this.socket;
      if (ackId && socket && socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify({ type: "ack", message_id: ackId }));
      }
    } catch (error) {
      console.warn("frame failed", error);
    }
  }
}
