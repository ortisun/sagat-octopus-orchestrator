import { describe, expect, it } from 'vitest';
import { SseParser, readSse } from './sse';

const parse = (...chunks: string[]) => {
  const parser = new SseParser();
  return chunks.flatMap((c) => parser.push(c));
};

describe('SseParser', () => {
  it('dispatches an event at a blank line, with its name, id and data', () => {
    expect(parse('event: graph\nid: 12\ndata: {"a":1}\n\n')).toEqual([{ event: 'graph', id: '12', data: '{"a":1}' }]);
  });

  it('names an event with no name "message", joins repeated data lines and drops one space after the colon only', () => {
    expect(parse('data:  x\ndata: y\n\n')).toEqual([{ event: 'message', id: undefined, data: ' x\ny' }]);
  });

  it('ignores comments (the keep-alive) and fields it does not know', () => {
    expect(parse(': ping\n\nretry: 100\nunknown: 1\nevent: end\ndata: {}\n\n')).toEqual([{ event: 'end', id: undefined, data: '{}' }]);
  });

  it('does not dispatch an event that has no data', () => {
    expect(parse('event: graph\n\nid: 3\n\n')).toEqual([]);
  });

  it('reads \\n, \\r and \\r\\n alike, even when the pair is cut between two chunks', () => {
    expect(parse('event: a\r\ndata: 1\r\n\r\n')).toHaveLength(1);
    expect(parse('event: a\rdata: 1\r\r')).toHaveLength(1);
    const split = parse('event: a\r', '\ndata: 1\r', '\n\r', '\n');
    expect(split).toEqual([{ event: 'a', id: undefined, data: '1' }]);
  });

  it('is not troubled by chunks that cut a line anywhere', () => {
    const text = 'id: 7\nevent: graph\ndata: {"x":"é"}\n\nid: 8\nevent: end\ndata: {}\n\n';
    for (let cut = 1; cut < text.length; cut++) {
      const got = parse(text.slice(0, cut), text.slice(cut));
      expect(got.map((m) => m.id)).toEqual(['7', '8']);
      expect(got[0]?.data).toBe('{"x":"é"}');
    }
  });

  it('remembers the id for the next event as the format says, and ignores one with a NUL', () => {
    expect(parse('id: 1\ndata: a\n\ndata: b\n\n').map((m) => m.id)).toEqual(['1', '1']);
    expect(parse('id: a\0b\ndata: x\n\n')[0]?.id).toBeUndefined();
  });

  it('skips a byte order mark at the start of the stream', () => {
    expect(parse('﻿data: x\n\n')).toEqual([{ event: 'message', id: undefined, data: 'x' }]);
  });
});

function bodyOf(bytes: Uint8Array[]): ReadableStream<Uint8Array> {
  return new ReadableStream({
    start(controller) {
      for (const chunk of bytes) controller.enqueue(chunk);
      controller.close();
    },
  });
}

describe('readSse', () => {
  it('decodes a character that a chunk boundary cuts in two', async () => {
    const bytes = new TextEncoder().encode('event: graph\ndata: {"n":"日本語 😀"}\n\n');
    for (let cut = 1; cut < bytes.length; cut++) {
      const got: string[] = [];
      for await (const message of readSse(bodyOf([bytes.slice(0, cut), bytes.slice(cut)]))) got.push(message.data);
      expect(got).toEqual(['{"n":"日本語 😀"}']);
    }
  });

  it('stops reading when the signal aborts', async () => {
    const controller = new AbortController();
    let cancelled = false;
    const body = new ReadableStream<Uint8Array>({
      pull() {
        return new Promise(() => undefined); // never any data
      },
      cancel() {
        cancelled = true;
      },
    });
    const reading = (async () => {
      for await (const message of readSse(body, controller.signal)) void message;
    })();
    controller.abort();
    await reading;
    expect(cancelled).toBe(true);
  });
});
