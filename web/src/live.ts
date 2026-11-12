/// The live WebSocket connection: authenticates, delivers incoming frames
/// to the session, and acks them. Reconnects with a short backoff.

import type { CryptoSession } from "./session";

export class LiveConnection {
  private socket: WebSocket | null = null;
  private closed = false;
  private reconnectDelayMs = 1000;
  private onChange: (connected: boolean) => void;

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
      this.reconnectDelayMs = 1000;
      this.onChange(true);
      socket.send(this.session.authFrame());
    };
    socket.onmessage = async (event) => {
      if (typeof event.data !== "string") return;
      try {
        const ackId = await this.session.handleServerFrame(event.data);
        if (ackId && socket.readyState === WebSocket.OPEN) {
          socket.send(JSON.stringify({ type: "ack", message_id: ackId }));
        }
      } catch (error) {
        console.warn("frame failed", error);
      }
    };
    socket.onclose = () => {
      if (this.socket === socket) this.socket = null;
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
}
