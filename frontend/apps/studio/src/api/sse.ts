/** One event of a `text/event-stream`. */
export interface SseMessage {
  event: string;
  id: string | undefined;
  data: string;
}

/**
 * The parser of the event-stream format (WHATWG HTML, "server-sent events"), fed with text as it
 * arrives: lines end in `\n`, `\r` or `\r\n` (even when the pair is split between two chunks), a
 * blank line dispatches, a line starting with `:` is a comment, repeated `data` lines are joined
 * with a newline, an `id` with a NUL in it is ignored. A message with no data is not dispatched.
 */
export class SseParser {
  #buffer = '';
  #started = false;
  #event = '';
  #data: string[] = [];
  #id: string | undefined;
  #lastWasCr = false;

  push(chunk: string): SseMessage[] {
    let text = chunk;
    if (!this.#started && text.length > 0) {
      this.#started = true;
      if (text.startsWith('﻿')) text = text.slice(1);
    }
    if (this.#lastWasCr && text.startsWith('\n')) text = text.slice(1);
    if (text.length > 0) this.#lastWasCr = text.endsWith('\r');
    this.#buffer += text;
    const out: SseMessage[] = [];
    let start = 0;
    for (let i = 0; i < this.#buffer.length; i++) {
      const c = this.#buffer[i];
      if (c !== '\n' && c !== '\r') continue;
      this.#line(this.#buffer.slice(start, i), out);
      if (c === '\r' && this.#buffer[i + 1] === '\n') i++;
      start = i + 1;
    }
    this.#buffer = this.#buffer.slice(start);
    return out;
  }

  #line(line: string, out: SseMessage[]): void {
    if (line === '') {
      if (this.#data.length > 0) out.push({ event: this.#event === '' ? 'message' : this.#event, id: this.#id, data: this.#data.join('\n') });
      this.#event = '';
      this.#data = [];
      return;
    }
    if (line.startsWith(':')) return;
    const colon = line.indexOf(':');
    const field = colon === -1 ? line : line.slice(0, colon);
    let value = colon === -1 ? '' : line.slice(colon + 1);
    if (value.startsWith(' ')) value = value.slice(1);
    if (field === 'event') this.#event = value;
    else if (field === 'data') this.#data.push(value);
    else if (field === 'id' && !value.includes('\0')) this.#id = value;
  }
}

/** Reads a response body as events until it ends or the signal aborts. */
export async function* readSse(body: ReadableStream<Uint8Array>, signal?: AbortSignal): AsyncGenerator<SseMessage> {
  const reader = body.getReader();
  const decoder = new TextDecoder('utf-8', { fatal: false });
  const parser = new SseParser();
  const stop = () => void reader.cancel().catch(() => undefined);
  signal?.addEventListener('abort', stop, { once: true });
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      for (const message of parser.push(decoder.decode(value, { stream: true }))) yield message;
    }
    for (const message of parser.push(decoder.decode())) yield message;
  } finally {
    signal?.removeEventListener('abort', stop);
    stop();
  }
}
