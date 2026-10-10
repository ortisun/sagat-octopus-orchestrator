// Package safelog makes a logger the SDK can call without being taken down by it: a slog.Handler of
// the application that panics is the application's bug, but the heartbeat of a worker and the retry
// of a call must go on (sdks/SPEC.md, section 10).
package safelog

import (
	"context"
	"log/slog"
)

// New wraps the handler of l so that a panic in it is swallowed. A nil logger is a logger that discards.
func New(l *slog.Logger) *slog.Logger {
	if l == nil {
		return slog.New(slog.DiscardHandler)
	}
	if _, already := l.Handler().(handler); already {
		return l
	}
	return slog.New(handler{inner: l.Handler()})
}

type handler struct{ inner slog.Handler }

func (h handler) Enabled(ctx context.Context, level slog.Level) (enabled bool) {
	defer func() {
		if recover() != nil {
			enabled = false
		}
	}()
	return h.inner.Enabled(ctx, level)
}

func (h handler) Handle(ctx context.Context, record slog.Record) (err error) {
	defer func() {
		if recover() != nil {
			err = nil
		}
	}()
	return h.inner.Handle(ctx, record)
}

func (h handler) WithAttrs(attrs []slog.Attr) (next slog.Handler) {
	defer func() {
		if recover() != nil {
			next = h
		}
	}()
	return handler{inner: h.inner.WithAttrs(attrs)}
}

func (h handler) WithGroup(name string) (next slog.Handler) {
	defer func() {
		if recover() != nil {
			next = h
		}
	}()
	return handler{inner: h.inner.WithGroup(name)}
}
