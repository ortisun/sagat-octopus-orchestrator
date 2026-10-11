import { describe, expect, it } from 'vitest';
import { clipLabel, clipText, formatBytes, formatDuration, isUuid, revealInvisible, sanitizeLog, shortDigest, stripAnsi, utf8Length } from './text';

const ESC = String.fromCharCode(27);
const BEL = String.fromCharCode(7);
const CSI = String.fromCharCode(0x9b);

describe('stripAnsi', () => {
  it('removes colours and cursor movements but keeps the words', () => {
    expect(stripAnsi(`${ESC}[31mred${ESC}[0m and ${ESC}[1;4;32mgreen${ESC}[m`)).toBe('red and green');
    expect(stripAnsi(`go${ESC}[2K${ESC}[1Gback${ESC}[10;20H!`)).toBe('goback!');
  });

  it('removes operating-system commands ended by BEL or by ST, such as a window title or a hyperlink', () => {
    expect(stripAnsi(`${ESC}]0;fake title${BEL}text`)).toBe('text');
    expect(stripAnsi(`${ESC}]8;;https://evil.example${ESC}\\click${ESC}]8;;${ESC}\\`)).toBe('click');
  });

  it('removes the one-byte CSI and two-byte escapes', () => {
    expect(stripAnsi(`a${CSI}31mb`)).toBe('ab');
    expect(stripAnsi(`a${ESC}cb${ESC}7c${ESC}Md`)).toBe('abcd');
  });

  it('removes an escape that the platform cut at the end of the log', () => {
    expect(stripAnsi(`done${ESC}[31`)).toBe('done');
    expect(stripAnsi(`done${ESC}]0;title without end`)).toBe('done');
  });

  it('leaves text without escapes alone', () => {
    expect(stripAnsi('plain [31m looks like one but is not')).toBe('plain [31m looks like one but is not');
  });
});

describe('sanitizeLog', () => {
  it('turns a carriage return, alone or with a line feed, into a line end', () => {
    expect(sanitizeLog('10%\r20%\r\n30%')).toBe('10%\n20%\n30%');
  });

  it('removes control characters except tab and newline', () => {
    const bell = String.fromCharCode(7);
    const nul = String.fromCharCode(0);
    const del = String.fromCharCode(127);
    expect(sanitizeLog(`a${bell}b${nul}c${del}d\te\nf`)).toBe('abcd\te\nf');
  });

  it('shows the characters that reorder or hide text instead of acting on them', () => {
    const override = String.fromCharCode(0x202e);
    const zeroWidth = String.fromCharCode(0x200b);
    const out = sanitizeLog(`safe${override}txt.exe${zeroWidth}`);
    expect(out).toBe('safe<U+202E>txt.exe<U+200B>');
    expect(out).not.toContain(override);
    expect(revealInvisible(String.fromCharCode(0xfeff))).toBe('<U+FEFF>');
  });
});

describe('clipText', () => {
  const limits = { maxChars: 1000, maxLines: 3, maxLineChars: 10 };

  it('does not touch text within the limits', () => {
    expect(clipText('a\nb', limits)).toEqual({ text: 'a\nb', clipped: false, totalChars: 3, totalLines: 2 });
  });

  it('keeps the first lines and says it left the others out', () => {
    const out = clipText('1\n2\n3\n4\n5', limits);
    expect(out.text).toBe('1\n2\n3');
    expect(out.clipped).toBe(true);
    expect(out.totalLines).toBe(5);
  });

  it('shortens a very long line and says how much was left out of it', () => {
    const out = clipText('x'.repeat(50), limits);
    expect(out.text.startsWith('x'.repeat(10))).toBe(true);
    expect(out.text).toContain('40 more characters on this line');
    expect(out.clipped).toBe(true);
  });

  it('stops at the character budget', () => {
    const out = clipText('abcdef\nghijkl\nmnopqr', { maxChars: 10, maxLines: 100, maxLineChars: 100 });
    expect(out.text.length).toBeLessThanOrEqual(10);
    expect(out.clipped).toBe(true);
  });

  it('never cuts a character in half', () => {
    const emoji = String.fromCodePoint(0x1f600);
    const out = clipText(`${emoji}${emoji}${emoji}`, { maxChars: 100, maxLines: 10, maxLineChars: 3 });
    // Three code units would end in the middle of the second emoji; the cut moves back to the first.
    expect(out.text.startsWith(`${emoji} `)).toBe(true);
    const lone = Array.from(out.text).filter((char) => char.length === 1 && char.charCodeAt(0) >= 0xd800 && char.charCodeAt(0) <= 0xdfff);
    expect(lone).toHaveLength(0);
  });

  it('copes with a megabyte of text quickly', () => {
    const big = 'line of text\n'.repeat(80_000);
    const started = performance.now();
    const out = clipText(big, { maxChars: 32 * 1024, maxLines: 400, maxLineChars: 2000 });
    expect(performance.now() - started).toBeLessThan(500);
    expect(out.text.split('\n')).toHaveLength(400);
  });
});

describe('utf8Length', () => {
  it('agrees with the encoder for ASCII, accents, CJK, emoji and a lone surrogate', () => {
    const lone = String.fromCharCode(0xd800);
    for (const sample of ['', 'plain', 'café', '你好', String.fromCodePoint(0x1f600), `a${lone}b`, 'mix é你' + String.fromCodePoint(0x1f4a9)]) {
      expect(utf8Length(sample)).toBe(new TextEncoder().encode(sample).length);
    }
  });
});

describe('formatting', () => {
  it('writes sizes and durations for people', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(1023)).toBe('1023 B');
    expect(formatBytes(1536)).toBe('1.5 KiB');
    expect(formatBytes(256 * 1024)).toBe('256 KiB');
    expect(formatBytes(5 * 1024 * 1024)).toBe('5 MiB');
    expect(formatBytes(-1)).toBe('—');
    expect(formatDuration(undefined)).toBe('—');
    expect(formatDuration(85)).toBe('85 ms');
    expect(formatDuration(1200)).toBe('1.2 s');
    expect(formatDuration(65_000)).toBe('1 min 5 s');
  });

  it('shortens a digest and clips a label', () => {
    const digest = `sha256:${'ab'.repeat(32)}`;
    expect(shortDigest(digest)).toBe(`sha256:${'ab'.repeat(6)}…`);
    expect(clipLabel('x'.repeat(100), 10)).toBe(`${'x'.repeat(10)}…`);
    expect(clipLabel(`ok${String.fromCharCode(0)}${String.fromCharCode(0x202e)}`)).toBe('ok<U+202E>');
  });

  it('knows an id from the address bar from something else', () => {
    expect(isUuid('22222222-2222-4222-8222-222222222222')).toBe(true);
    expect(isUuid('../../etc/passwd')).toBe(false);
    expect(isUuid('22222222-2222-4222-8222-22222222222')).toBe(false);
    expect(isUuid(undefined)).toBe(false);
  });
});
