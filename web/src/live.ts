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
  /**
   * Whether the current connection has applied the queue the server held
   * when it authenticated: the server's `caught_up` frame was reached in
   * the processing chain, so every frame before it has been handled.
   */
  private drained = false;

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
   * Resolves once the server's queue for this device has been applied: the
   * current connection's `caught_up` frame has been reached and every
   * frame before it handled. While the socket is still connecting or
   * reconnecting it keeps waiting. It gives up after a few seconds, so a
   * server that cannot be reached does not stall callers.
   */
  async caughtUp(): Promise<void> {
    const giveUpAt = Date.now() + 5000;
    while (!this.closed && Date.now() < giveUpAt) {
      const socket = this.socket;
      if (this.drained && socket && socket.readyState === WebSocket.OPEN) {
        await this.processing;
        return;
      }
      await new Promise((resolve) => setTimeout(resolve, 50));
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
      this.drained = false;
      this.reconnectDelayMs = 1000;
      this.onChange(true);
      socket.send(this.session.authFrame());
    };
    socket.onmessage = (event) => {
      if (typeof event.data !== "string") return;
      const text = event.data;
      this.processing = this.processing.then(() => this.deliver(socket, text));
    };
    socket.onclose = () => {
      if (this.socket === socket) {
        this.socket = null;
        this.drained = false;
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
  private async deliver(from: WebSocket, text: string): Promise<void> {
    if (isCaughtUp(text)) {
      if (this.socket === from) this.drained = true;
      return;
    }
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

/** Whether a server frame is the `caught_up` marker that ends the queue. */
function isCaughtUp(text: string): boolean {
  try {
    return (JSON.parse(text) as { type?: unknown }).type === "caught_up";
  } catch {
    return false;
  }
}
