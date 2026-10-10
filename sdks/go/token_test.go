package sagat

import (
	"bytes"
	"encoding/json"
	"fmt"
	"log/slog"
	"strings"
	"testing"
)

func TestAStaticTokenNeverPrintsItselfInAnyFormat(t *testing.T) {
	const canary = "CANARY-TOKEN-5c1e"
	token := StaticToken(canary)
	holder := struct {
		Token TokenSource
		T     *struct{ Token TokenSource }
	}{Token: token, T: &struct{ Token TokenSource }{token}}
	var logged bytes.Buffer
	slog.New(slog.NewTextHandler(&logged, nil)).Info("x", "token", token, "holder", holder)
	slog.New(slog.NewJSONHandler(&logged, nil)).Info("x", "token", token, "holder", holder)
	encoded, err := json.Marshal(map[string]any{"token": token, "holder": holder})
	if err != nil {
		t.Fatal(err)
	}
	outputs := []string{logged.String(), string(encoded)}
	for _, verb := range []string{"%v", "%+v", "%#v", "%s", "%q", "%d", "%x", "%X", "%T", "%p"} {
		outputs = append(outputs, fmt.Sprintf(verb, token), fmt.Sprintf(verb, holder), fmt.Sprintf(verb, &holder), fmt.Sprintf(verb, []any{token}), fmt.Sprintf(verb, map[string]any{"k": token}))
	}
	outputs = append(outputs, fmt.Sprint(token), fmt.Sprintln(token))
	for _, out := range outputs {
		if strings.Contains(out, canary) {
			t.Errorf("the token is in %q", out)
		}
	}
	if fmt.Sprint(token) != "<redacted>" || fmt.Sprintf("%#v", token) != "<redacted>" {
		t.Errorf("%q %q", fmt.Sprint(token), fmt.Sprintf("%#v", token))
	}
	// It still works.
	got, err := token.Token(nil) //nolint:staticcheck // a static token ignores its context
	if err != nil || got != canary {
		t.Errorf("%q %v", got, err)
	}
	if CanRefresh(token) {
		t.Error("a static token cannot be refreshed")
	}
}
