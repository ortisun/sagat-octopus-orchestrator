package main

// awaitJournal reads the whole journal of a run: an event on a later page than the first is found, and the
// runner asks for each page with the cursor the page before it gave.

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
)

const testRun = "0190aaaa-0000-7000-8000-0000000000d1"

// journalServer answers the journal of testRun in two pages: the first holds only a step.leased of step held, the
// second the step.started of that step. It keeps the cursors it was asked for.
func journalServer(t *testing.T) (c *client.Client, cursors func() []string) {
	t.Helper()
	var mu sync.Mutex
	var asked []string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/runs/"+testRun+"/events" {
			http.NotFound(w, r)
			return
		}
		cursor := r.URL.Query().Get("cursor")
		mu.Lock()
		asked = append(asked, cursor)
		mu.Unlock()
		page := map[string]any{
			"items":      []any{map[string]any{"sequence": 1, "eventType": "step.leased", "stepId": "held", "attempt": 1}},
			"nextCursor": "second",
		}
		if cursor == "second" {
			page = map[string]any{
				"items":      []any{map[string]any{"sequence": 2, "eventType": "step.started", "stepId": "held", "attempt": 1}},
				"nextCursor": nil,
			}
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(page)
	}))
	t.Cleanup(server.Close)
	c, err := client.New(server.URL+"/v1", "payments", sagat.StaticToken("tok-1"))
	if err != nil {
		t.Fatal(err)
	}
	return c, func() []string {
		mu.Lock()
		defer mu.Unlock()
		return append([]string(nil), asked...)
	}
}

func TestJournalHasReadsEveryPageWithTheCursorOfThePageBefore(t *testing.T) {
	c, cursors := journalServer(t)
	found, _, err := journalHas(context.Background(), c, testRun, "step.started", "held")
	if err != nil {
		t.Fatal(err)
	}
	if !found {
		t.Fatal("the step.started on the second page was not found")
	}
	if got := cursors(); len(got) != 2 || got[0] != "" || got[1] != "second" {
		t.Fatalf("the pages asked for: %q, want the first page and then the cursor \"second\"", got)
	}
}

func TestJournalHasReportsAnEventThatNoPageHas(t *testing.T) {
	c, cursors := journalServer(t)
	found, ofStep, err := journalHas(context.Background(), c, testRun, "step.completed", "held")
	if err != nil {
		t.Fatal(err)
	}
	if found {
		t.Fatal("a step.completed was found in a journal that has none")
	}
	// What a barrier that times out prints: the events of the step on every page, with their attempts.
	if strings.Join(ofStep, " ") != "step.leased/1 step.started/1" {
		t.Fatalf("the events of the step: %q", ofStep)
	}
	if got := len(cursors()); got != 2 {
		t.Fatalf("%d pages were read, want both", got)
	}
}
