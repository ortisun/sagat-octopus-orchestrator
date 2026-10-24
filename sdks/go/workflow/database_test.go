package workflow

import (
	"encoding/json"
	"os"
	"reflect"
	"strings"
	"testing"
)

func TestDatabaseSharedTypedFixture(t *testing.T) {
	raw, err := os.ReadFile("../../../tests/contract/database-connections/typed-parameters.workflow.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Metadata struct{ Name string }
		Nodes    []struct {
			ID string
			DatabaseOptions
		}
	}
	if err = json.Unmarshal(raw, &fixture); err != nil {
		t.Fatal(err)
	}
	text, err := New(fixture.Metadata.Name).Database(fixture.Nodes[0].ID, fixture.Nodes[0].DatabaseOptions).JSON()
	if err != nil {
		t.Fatal(err)
	}
	var expected, actual any
	if err = json.Unmarshal(raw, &expected); err != nil {
		t.Fatal(err)
	}
	if err = json.Unmarshal([]byte(text), &actual); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(expected, actual) {
		t.Fatal("builder diverged from shared database IR")
	}
}

func TestDatabaseRejectsUnsafeShapesWithoutEchoingValues(t *testing.T) {
	base := DatabaseOptions{ConnectionRef: ConnectionRef{Name: "orders-db"}, Operation: DatabaseQuery, Statement: "SELECT $1"}
	invalid := []DatabaseOptions{base, base, base, base, base}
	invalid[0].ConnectionRef.Name = "postgres://credential-canary@host/db"
	invalid[1].Statement = ""
	invalid[2].Operation = "credential-canary"
	invalid[3].Parameters = []DatabaseParameter{{Literal: &DatabaseValue{Type: DatabaseInteger, Value: 123}}}
	invalid[4].Parameters = []DatabaseParameter{{Literal: &DatabaseValue{Type: DatabaseDecimal, Value: "credential-canary"}}}
	for _, options := range invalid {
		err := New("database-invalid").Database("q", options).Err()
		if err == nil || strings.Contains(err.Error(), "credential-canary") {
			t.Fatal("database validation did not fail with a safe message")
		}
	}
	base.Parameters = []DatabaseParameter{{Literal: &DatabaseValue{Type: DatabaseInteger, Value: "9223372036854775807"}}}
	builder := New("database-snapshot").Database("q", base).End("done")
	base.Parameters[0].Literal.Value = "credential-canary"
	text, err := builder.JSON()
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(text, "9223372036854775807") || strings.Contains(text, "credential-canary") {
		t.Fatal("caller mutation changed retained builder")
	}
}
