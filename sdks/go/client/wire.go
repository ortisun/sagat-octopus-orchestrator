package client

import (
	"context"
	"encoding/json"
	"iter"
	"net/url"
	"strconv"
	"strings"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/jsonx"
)

// Small checks and readers shared by the groups of feature 018 (schedules, batch jobs, business
// identifiers) and by the reset calls. An answer that does not have the shape the contract promises is a
// PROTOCOL error, never a guess; a value of an enumeration this SDK does not know is kept as the string
// it came as.

// Page is one page of a list, in the order the server gave it.
type Page[T any] struct {
	Items []T
	// NextCursor is empty on the last page.
	NextCursor string
	Raw        map[string]json.RawMessage
}

// mutation checks what a call that changes something carries and returns it: one Idempotency-Key, given by
// the caller or generated here once, and the traceparent. Every retry of the call reuses both.
func (c *Client) mutation(ctx context.Context, o callOptions) (idempotencyKey, traceparent string, err error) {
	if idempotencyKey, err = key(o.idempotencyKey); err != nil {
		return "", "", err
	}
	if traceparent, err = c.trace(ctx, o.traceparent); err != nil {
		return "", "", err
	}
	return idempotencyKey, traceparent, nil
}

// namespacePath is the path of a namespace by its id: a UUID, since it becomes a segment of the path.
func namespacePath(namespaceID string) (string, error) {
	if !uuidPattern.MatchString(namespaceID) {
		return "", sagat.Invalid("a namespace id is a UUID")
	}
	return "/namespaces/" + lower(namespaceID), nil
}

// idSegment is an id that becomes a path segment (schedule, backfill, batch job, reset): a UUID, so that it
// can only name one segment.
func idSegment(value, what string) (string, error) {
	if !uuidPattern.MatchString(value) {
		return "", sagat.Invalid("%s is a UUID", what)
	}
	return lower(value), nil
}

// escapeSegment percent-encodes text for one path segment: only the unreserved characters of RFC 3986 stay as
// they are. A segment that is exactly "." or ".." never gets here: every URL resolver takes it as a step in the
// path, whatever its encoding, so the caller refuses it.
func escapeSegment(value string) string {
	var out strings.Builder
	for i := 0; i < len(value); i++ {
		b := value[i]
		switch {
		case b >= 'a' && b <= 'z', b >= 'A' && b <= 'Z', b >= '0' && b <= '9', b == '-', b == '_', b == '~', b == '.':
			out.WriteByte(b)
		default:
			out.WriteByte('%')
			out.WriteByte("0123456789ABCDEF"[b>>4])
			out.WriteByte("0123456789ABCDEF"[b&0x0f])
		}
	}
	return out.String()
}

func positive(query url.Values, name string, n int) {
	if n > 0 {
		query.Set(name, strconv.Itoa(n))
	}
}

// stringList reads a JSON array of strings; anything else is an empty list, and a member that is not a
// string is kept as its JSON text. It is never nil.
func stringList(raw json.RawMessage) []string {
	var items []json.RawMessage
	out := []string{}
	if json.Unmarshal(raw, &items) != nil {
		return out
	}
	for _, item := range items {
		var s string
		if json.Unmarshal(item, &s) == nil {
			out = append(out, s)
		} else {
			out = append(out, string(item))
		}
	}
	return out
}

// rawList reads a JSON array as its members, each kept as the JSON it was. It is never nil.
func rawList(raw json.RawMessage) []json.RawMessage {
	var items []json.RawMessage
	if json.Unmarshal(raw, &items) != nil || items == nil {
		return []json.RawMessage{}
	}
	return items
}

// counters reads an object of numbers; a member that is not a number is left out. It is never nil.
func counters(raw json.RawMessage) map[string]int64 {
	out := map[string]int64{}
	members, err := jsonx.Object(raw)
	if err != nil {
		return out
	}
	for name := range members {
		if n, ok := jsonx.Int(members, name); ok {
			out[name] = n
		}
	}
	return out
}

func requiredTime(m map[string]json.RawMessage, key string) (time.Time, error) {
	t, ok := jsonx.Time(m, key)
	if !ok {
		return time.Time{}, protocolError("%s is missing from the answer", key)
	}
	return t, nil
}

// optionalInt reads a member that is a JSON number, or nil when it is absent, null or another type.
func optionalInt(m map[string]json.RawMessage, key string) *int64 {
	if n, ok := jsonx.Int(m, key); ok {
		return &n
	}
	return nil
}

// optionalFloat reads a member that is a JSON number, or nil when it is absent, null or another type.
func optionalFloat(m map[string]json.RawMessage, key string) *float64 {
	raw, ok := m[key]
	if !ok || jsonx.IsNull(raw) {
		return nil
	}
	var f float64
	if json.Unmarshal(raw, &f) != nil {
		return nil
	}
	return &f
}

func intOr(m map[string]json.RawMessage, key string, fallback int64) int64 {
	if n, ok := jsonx.Int(m, key); ok {
		return n
	}
	return fallback
}

// parsePage reads {items, nextCursor} and each item with read.
func parsePage[T any](body []byte, what string, read func(map[string]json.RawMessage) (T, error)) (*Page[T], error) {
	raw, err := object(body, what)
	if err != nil {
		return nil, err
	}
	var members []json.RawMessage
	if encoded, ok := raw["items"]; !ok || json.Unmarshal(encoded, &members) != nil || members == nil {
		return nil, protocolError("items is missing from the page")
	}
	page := &Page[T]{Items: make([]T, 0, len(members)), Raw: raw}
	for _, member := range members {
		fields, err := object(member, what)
		if err != nil {
			return nil, err
		}
		item, err := read(fields)
		if err != nil {
			return nil, err
		}
		page.Items = append(page.Items, item)
	}
	page.NextCursor, _ = jsonx.String(raw, "nextCursor")
	return page, nil
}

// parseItems reads {items} (a list with no cursor) and each item with read. It is never nil.
func parseItems[T any](body []byte, what string, read func(map[string]json.RawMessage) (T, error)) ([]T, error) {
	raw, err := object(body, what)
	if err != nil {
		return nil, err
	}
	members := rawList(raw["items"])
	out := make([]T, 0, len(members))
	for _, member := range members {
		fields, err := object(member, what)
		if err != nil {
			return nil, err
		}
		item, err := read(fields)
		if err != nil {
			return nil, err
		}
		out = append(out, item)
	}
	return out, nil
}

// allPages yields every item of a list, page after page, from the cursor the first fetch is given. The
// iteration ends with the first error, which is yielded last.
func allPages[T any](start string, fetch func(cursor string) (*Page[T], error)) iter.Seq2[*T, error] {
	return func(yield func(*T, error) bool) {
		cursor := start
		for {
			page, err := fetch(cursor)
			if err != nil {
				yield(nil, err)
				return
			}
			for index := range page.Items {
				if !yield(&page.Items[index], nil) {
					return
				}
			}
			if page.NextCursor == "" {
				return
			}
			cursor = page.NextCursor
		}
	}
}
