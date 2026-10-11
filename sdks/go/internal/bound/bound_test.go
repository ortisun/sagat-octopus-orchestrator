package bound

import (
	"strings"
	"testing"
	"time"
)

func TestWaitsTextAndTraceIDsAreBounded(t *testing.T) {
	if Wait(time.Hour) != time.Minute || Wait(time.Second) != time.Second || Wait(0) != 0 {
		t.Error("Wait")
	}
	if got := Text(strings.Repeat("é", 600), 512); len([]rune(got)) != 512 {
		t.Errorf("%d", len([]rune(got)))
	}
	if got := Text("a\xffb", 10); got != "a�b" {
		t.Errorf("%q", got)
	}
	if Text("short", 512) != "short" {
		t.Error("a short text is kept")
	}
	for id, want := range map[string]string{
		strings.Repeat("a", 32): strings.Repeat("a", 32), strings.Repeat("AB", 16): strings.Repeat("ab", 16), "": "", strings.Repeat("a", 31): "",
		strings.Repeat("a", 33): "", strings.Repeat("g", 32): "", strings.Repeat("a", 31) + "\n": "", "../../etc/passwd 12345678901234": "",
	} {
		if got := TraceID(id); got != want {
			t.Errorf("%q: %q, want %q", id, got, want)
		}
	}
}
