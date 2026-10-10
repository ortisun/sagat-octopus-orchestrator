package sagat

import (
	"crypto/rand"
	"encoding/hex"
	"strings"
)

// A W3C traceparent of version 00: 00-<32 hex>-<16 hex>-<2 hex>, lowercase, with a non-zero trace
// id and parent id.
func parseTraceparent(value string) (traceID string, ok bool) {
	if len(value) != 55 || value[2] != '-' || value[35] != '-' || value[52] != '-' || value[:2] != "00" {
		return "", false
	}
	traceID, parentID, flags := value[3:35], value[36:52], value[53:]
	if !isLowerHex(traceID) || !isLowerHex(parentID) || !isLowerHex(flags) {
		return "", false
	}
	if traceID == strings.Repeat("0", 32) || parentID == strings.Repeat("0", 16) {
		return "", false
	}
	return traceID, true
}

func isLowerHex(s string) bool {
	for i := 0; i < len(s); i++ {
		c := s[i]
		if (c < '0' || c > '9') && (c < 'a' || c > 'f') {
			return false
		}
	}
	return true
}

// ValidTraceparent reports whether value is a W3C traceparent of version 00.
func ValidTraceparent(value string) bool {
	_, ok := parseTraceparent(value)
	return ok
}

// CheckTraceparent returns a BAD_REQUEST error for a malformed traceparent, before it is sent.
func CheckTraceparent(value string) error {
	if !ValidTraceparent(value) {
		return NewError(CodeBadRequest, "traceparent is not a W3C trace context (00-<32 hex>-<16 hex>-<2 hex>)")
	}
	return nil
}

// TraceIDOf is the trace id of a valid traceparent, and "" otherwise.
func TraceIDOf(value string) string {
	id, _ := parseTraceparent(value)
	return id
}

// NewTraceparent is a fresh sampled trace context, for an application that has none.
func NewTraceparent() string {
	var raw [24]byte
	_, _ = rand.Read(raw[:])
	return "00-" + hex.EncodeToString(raw[:16]) + "-" + hex.EncodeToString(raw[16:]) + "-01"
}
