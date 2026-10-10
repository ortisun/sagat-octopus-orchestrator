// Package bound limits what the SDK takes from a server and repeats: a wait, a message, a trace id,
// a reason. A server (or whatever answers in its place) is not trusted with the size of a pause or of
// a string the SDK puts in an error or a log.
package bound

import (
	"strings"
	"time"
	"unicode/utf8"
)

// MaxServerWait is the longest wait the SDK takes from a server (Retry-After, retry_after_ms), whatever
// it says (sdks/SPEC.md, section 6.2).
const MaxServerWait = 60 * time.Second

// MaxMessage is the longest message of a server error the SDK repeats, in characters.
const MaxMessage = 512

// MaxReason is the longest reason (details.reason, x-sagat-error) the SDK repeats, in characters.
const MaxReason = 128

// Wait caps a wait that came from the server.
func Wait(d time.Duration) time.Duration {
	return min(d, MaxServerWait)
}

// Text makes s valid UTF-8 and cuts it to n characters.
func Text(s string, n int) string {
	if !utf8.ValidString(s) {
		s = strings.ToValidUTF8(s, "�")
	}
	if utf8.RuneCountInString(s) <= n {
		return s
	}
	return string([]rune(s)[:n])
}

// TraceID returns id, in lower case, when it is 32 hexadecimal characters, and "" otherwise: what a
// server sends as a trace id is repeated in errors, so it must be nothing but a trace id.
func TraceID(id string) string {
	if len(id) != 32 {
		return ""
	}
	for i := 0; i < len(id); i++ {
		c := id[i]
		if (c < '0' || c > '9') && (c < 'a' || c > 'f') && (c < 'A' || c > 'F') {
			return ""
		}
	}
	return strings.ToLower(id)
}
