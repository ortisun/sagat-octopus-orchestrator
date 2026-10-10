package ids

import (
	"regexp"
	"testing"
)

func TestUUIDIsARandomVersion4InCanonicalForm(t *testing.T) {
	pattern := regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$`)
	seen := map[string]bool{}
	for i := 0; i < 1000; i++ {
		id := UUID()
		if !pattern.MatchString(id) {
			t.Fatalf("%q", id)
		}
		if seen[id] {
			t.Fatalf("%q twice", id)
		}
		seen[id] = true
	}
}
