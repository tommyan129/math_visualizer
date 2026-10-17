// WebSocket client: Hello / version check, request ids, reconnect with backoff.

import type { ClientMsg } from "../generated/ClientMsg";
import type { ClientRequest } from "../generated/ClientRequest";
import type { ServerMsg } from "../generated/ServerMsg";
import type { ServerResponse } from "../generated/ServerResponse";
import type { ServerRequest } from "../generated/ServerRequest";
import type { BrowserReply } from "../generated/BrowserReply";

export const PROTOCOL_VERSION = 7;

export type ConnStatus = "connecting" | "connected" | "disconnected" | "taken-over" | "reload";

export interface ClientEvents {
  status(s: ConnStatus): void;
  message(m: ServerMsg): void;
  /** Server asks something of the browser (e.g. the current view). */
  serverRequest(r: ServerRequest): BrowserReply;
}

export class Client {
  private ws: WebSocket | null = null;
  private nextId = 1;
  private pending = new Map<number, (r: ServerResponse) => void>();
  private retry = 500;
  private stopped = false;

  constructor(private url: string, private ev: ClientEvents) {}

  start(): void {
    this.stopped = false;
    this.open();
  }

  /** Take the active-tab role back after `TakenOver`. */
  reclaim(): void {
    if (this.ws?.readyState === WebSocket.OPEN) this.send({ type: "Hello", protocol: PROTOCOL_VERSION });
    else this.start();
  }

  private open(): void {
    this.ev.status("connecting");
    const ws = new WebSocket(this.url);
    this.ws = ws;
    ws.onopen = () => {
      this.retry = 500;
      this.send({ type: "Hello", protocol: PROTOCOL_VERSION });
    };
    ws.onmessage = (e) => this.onMessage(JSON.parse(e.data as string) as ServerMsg);
    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.ws = null;
      for (const resolve of this.pending.values()) resolve({ type: "Error", message: "연결 끊김" });
      this.pending.clear();
      if (this.stopped) return;
      this.ev.status("disconnected");
      setTimeout(() => this.open(), this.retry);
      this.retry = Math.min(this.retry * 2, 8000);
    };
  }

  private onMessage(m: ServerMsg): void {
    switch (m.type) {
      case "Welcome":
        this.ev.status("connected");
        break;
      case "ReloadRequired":
        this.stopped = true;
        this.ev.status("reload");
        this.ws?.close();
        return;
      case "TakenOver":
        this.ev.status("taken-over");
        break;
      case "Response": {
        const resolve = this.pending.get(m.id);
        this.pending.delete(m.id);
        resolve?.(m.response);
        return;
      }
      case "Request":
        this.send({ type: "Reply", id: m.id, reply: this.ev.serverRequest(m.request) });
        return;
    }
    this.ev.message(m);
  }

  send(m: ClientMsg): void {
    if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(m));
  }

  request(request: ClientRequest): Promise<ServerResponse> {
    const id = this.nextId++;
    return new Promise((resolve) => {
      if (this.ws?.readyState !== WebSocket.OPEN) {
        resolve({ type: "Error", message: "서버에 연결되지 않음" });
        return;
      }
      this.pending.set(id, resolve);
      this.send({ type: "Request", id, request });
    });
  }
}
