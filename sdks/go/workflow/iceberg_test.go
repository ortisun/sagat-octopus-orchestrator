package workflow

import (
	"encoding/json"
	"os"
	"reflect"
	"strings"
	"testing"
)

func str(v string) *string { return &v }
func u32(v uint32) *uint32 { return &v }

func icebergBase() IcebergWriteOptions {
	revision := uint32(3)
	return IcebergWriteOptions{
		ConnectionRef: ConnectionRef{Name: "lake-main", Revision: &revision},
		Table:         IcebergTable{Namespace: []string{"sales"}, Name: "orders"},
		Records:       IcebergRecords{Inline: str("steps.extract.output.rows")},
	}
}

func icebergNode(t *testing.T, options IcebergWriteOptions) map[string]any {
	t.Helper()
	text, err := New("lake").IcebergWrite("w", options).End("done").JSON()
	if err != nil {
		t.Fatal(err)
	}
	var built struct{ Nodes []map[string]any }
	if err = json.Unmarshal([]byte(text), &built); err != nil {
		t.Fatal(err)
	}
	return built.Nodes[0]
}

func TestIcebergWriteMatchesTheSharedValidDocument(t *testing.T) {
	raw, err := os.ReadFile("../../../tests/contract/iceberg-writes/fixtures/ir/valid/append-inline.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct{ Metadata struct{ Name string } }
	if err = json.Unmarshal(raw, &fixture); err != nil {
		t.Fatal(err)
	}
	text, err := New(fixture.Metadata.Name).Step("extract", "orders.extract").IcebergWrite("load", icebergBase()).End("done").JSON()
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
		t.Fatal("builder diverged from the shared iceberg write document")
	}
}

func TestIcebergWriteModeIsAlwaysAppendAndAMinimalWriteCarriesNothingItWasNotGiven(t *testing.T) {
	node := icebergNode(t, icebergBase())
	if node["mode"] != "append" || node["type"] != "icebergWrite" {
		t.Fatalf("unexpected node %v", node)
	}
	for _, absent := range []string{"ref", "mapping", "limits", "compensation", "ignoreUnknownFields", "retry", "secrets", "overwrite"} {
		if _, found := node[absent]; found {
			t.Fatalf("%s must be absent", absent)
		}
	}
}

func TestIcebergWriteCarriesEveryOptionAndAcceptsTheThreeRecordSources(t *testing.T) {
	options := icebergBase()
	options.Ref = str("staging")
	options.Mapping = []IcebergMapping{{Field: "/amount", Column: "amount"}, {Field: "/customer/name", Column: "customer_name"}}
	yes := true
	options.IgnoreUnknownFields = &yes
	options.Limits = &IcebergLimits{MaxRecordBytes: u32(1024), MaxRecords: u32(100000000), TimeoutMs: u32(3600000)}
	options.Compensation = "revert"
	options.OnError = str("recover")
	text, err := New("lake").IcebergWrite("w", options).Step("recover", "orders.recover").End("done").JSON()
	if err != nil {
		t.Fatal(err)
	}
	for _, expected := range []string{`"ref":"staging"`, `"maxRecordBytes":1024`, `"maxRecords":100000000`, `"timeoutMs":3600000`, `"compensation":"revert"`, `"onError":"recover"`, `"ignoreUnknownFields":true`} {
		if !strings.Contains(text, expected) {
			t.Fatalf("missing %s in %s", expected, text)
		}
	}
	for _, records := range []IcebergRecords{{Inline: str("input.rows")}, {Blobs: str("steps.a.output.parts")}, {DatabaseResult: str("read")}} {
		other := icebergBase()
		other.Records = records
		if err := New("lake").IcebergWrite("w", other).End("done").Err(); err != nil {
			t.Fatal(err)
		}
	}
}

func TestIcebergWriteIsASnapshotLaterChangesToTheOptionsDoNotReachIt(t *testing.T) {
	options := icebergBase()
	options.Mapping = []IcebergMapping{{Field: "/a", Column: "a"}}
	builder := New("lake").IcebergWrite("w", options).End("done")
	options.Mapping[0].Column = "changed"
	options.Table.Namespace[0] = "changed"
	text, err := builder.JSON()
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(text, "changed") {
		t.Fatalf("a later change reached the built node: %s", text)
	}
}

func TestIcebergWriteRefusesWhatTheServerWouldRefuseWithOneFixedMessage(t *testing.T) {
	long := func(n int, s string) string { return strings.Repeat(s, n) }
	zero := uint32(0)
	cases := map[string]func(o *IcebergWriteOptions){
		"connection is a url": func(o *IcebergWriteOptions) {
			o.ConnectionRef = ConnectionRef{Name: "postgres://credential-canary@host/db"}
		},
		"revision zero":              func(o *IcebergWriteOptions) { o.ConnectionRef = ConnectionRef{Name: "lake-main", Revision: &zero} },
		"double dash":                func(o *IcebergWriteOptions) { o.ConnectionRef = ConnectionRef{Name: "lake--main"} },
		"no namespace":               func(o *IcebergWriteOptions) { o.Table.Namespace = nil },
		"nine levels":                func(o *IcebergWriteOptions) { o.Table.Namespace = strings.Split("abcdefghi", "") },
		"empty name":                 func(o *IcebergWriteOptions) { o.Table.Name = "" },
		"dot dot name":               func(o *IcebergWriteOptions) { o.Table.Name = ".." },
		"control in a level":         func(o *IcebergWriteOptions) { o.Table.Namespace = []string{"sales\x00"} },
		"empty ref":                  func(o *IcebergWriteOptions) { o.Ref = str("") },
		"long ref":                   func(o *IcebergWriteOptions) { o.Ref = str(long(256, "r")) },
		"no source":                  func(o *IcebergWriteOptions) { o.Records = IcebergRecords{} },
		"two sources":                func(o *IcebergWriteOptions) { o.Records = IcebergRecords{Inline: str("a"), Blobs: str("b")} },
		"empty expression":           func(o *IcebergWriteOptions) { o.Records = IcebergRecords{Inline: str("")} },
		"expression of 1025 bytes":   func(o *IcebergWriteOptions) { o.Records = IcebergRecords{Inline: str(long(1025, "x"))} },
		"513 two-byte characters":    func(o *IcebergWriteOptions) { o.Records = IcebergRecords{Inline: str(long(513, "é"))} },
		"empty database result":      func(o *IcebergWriteOptions) { o.Records = IcebergRecords{DatabaseResult: str("")} },
		"pointer without slash":      func(o *IcebergWriteOptions) { o.Mapping = []IcebergMapping{{Field: "amount", Column: "amount"}} },
		"pointer with empty segment": func(o *IcebergWriteOptions) { o.Mapping = []IcebergMapping{{Field: "/a//b", Column: "a"}} },
		"pointer with a bad escape":  func(o *IcebergWriteOptions) { o.Mapping = []IcebergMapping{{Field: "/a~2", Column: "a"}} },
		"pointer with 17 segments": func(o *IcebergWriteOptions) {
			o.Mapping = []IcebergMapping{{Field: "/" + long(16, "a/") + "a", Column: "a"}}
		},
		"empty column": func(o *IcebergWriteOptions) { o.Mapping = []IcebergMapping{{Field: "/a", Column: ""}} },
		"column twice": func(o *IcebergWriteOptions) {
			o.Mapping = []IcebergMapping{{Field: "/a", Column: "c"}, {Field: "/b", Column: "c"}}
		},
		"field twice": func(o *IcebergWriteOptions) {
			o.Mapping = []IcebergMapping{{Field: "/a", Column: "c"}, {Field: "/a", Column: "d"}}
		},
		"zero records":                func(o *IcebergWriteOptions) { o.Limits = &IcebergLimits{MaxRecords: u32(0)} },
		"too many records":            func(o *IcebergWriteOptions) { o.Limits = &IcebergLimits{MaxRecords: u32(100000001)} },
		"record bytes below the edge": func(o *IcebergWriteOptions) { o.Limits = &IcebergLimits{MaxRecordBytes: u32(1023)} },
		"timeout below the edge":      func(o *IcebergWriteOptions) { o.Limits = &IcebergLimits{TimeoutMs: u32(999)} },
		"timeout above the edge":      func(o *IcebergWriteOptions) { o.Limits = &IcebergLimits{TimeoutMs: u32(3600001)} },
		"unknown compensation":        func(o *IcebergWriteOptions) { o.Compensation = "undo" },
		"empty on error":              func(o *IcebergWriteOptions) { o.OnError = str("") },
	}
	for name, mutate := range cases {
		options := icebergBase()
		mutate(&options)
		err := New("lake").IcebergWrite("w", options).Err()
		if err == nil {
			t.Fatalf("%s: was accepted", name)
		}
		if !strings.Contains(err.Error(), "invalid iceberg write") || strings.Contains(err.Error(), "credential-canary") {
			t.Fatalf("%s: unsafe or unexpected message %q", name, err.Error())
		}
	}
}

func TestIcebergWriteAcceptsTheEdges(t *testing.T) {
	long := func(n int, s string) string { return strings.Repeat(s, n) }
	cases := map[string]func(o *IcebergWriteOptions){
		"eight levels and a 255 byte name": func(o *IcebergWriteOptions) {
			o.Table = IcebergTable{Namespace: strings.Split("abcdefgh", ""), Name: long(255, "n")}
		},
		"255 byte ref":         func(o *IcebergWriteOptions) { o.Ref = str(long(255, "r")) },
		"1024 byte expression": func(o *IcebergWriteOptions) { o.Records = IcebergRecords{Inline: str(long(1024, "x"))} },
		"512 two-byte letters": func(o *IcebergWriteOptions) { o.Records = IcebergRecords{Inline: str(long(512, "é"))} },
		"16 segment pointer": func(o *IcebergWriteOptions) {
			o.Mapping = []IcebergMapping{{Field: "/" + long(15, "a/") + "a", Column: long(255, "c")}}
		},
		"escapes":                func(o *IcebergWriteOptions) { o.Mapping = []IcebergMapping{{Field: "/a~0b/c~1d", Column: "c"}} },
		"supplied empty mapping": func(o *IcebergWriteOptions) { o.Mapping = []IcebergMapping{} },
		"lowest and highest limits": func(o *IcebergWriteOptions) {
			o.Limits = &IcebergLimits{MaxRecordBytes: u32(2147483648), MaxRecords: u32(1), TimeoutMs: u32(1000)}
		},
		"supplied empty limits": func(o *IcebergWriteOptions) { o.Limits = &IcebergLimits{} },
		"shortest name":         func(o *IcebergWriteOptions) { o.ConnectionRef = ConnectionRef{Name: "abc"} },
	}
	for name, mutate := range cases {
		options := icebergBase()
		mutate(&options)
		if err := New("lake").IcebergWrite("w", options).Err(); err != nil {
			t.Fatalf("%s: %v", name, err)
		}
	}
	empty := icebergBase()
	empty.Mapping = []IcebergMapping{}
	empty.Limits = &IcebergLimits{}
	node := icebergNode(t, empty)
	if mapping, found := node["mapping"].([]any); !found || len(mapping) != 0 {
		t.Fatalf("a supplied empty mapping must stay: %v", node["mapping"])
	}
	if limits, found := node["limits"].(map[string]any); !found || len(limits) != 0 {
		t.Fatalf("supplied empty limits must stay: %v", node["limits"])
	}
}

func TestIcebergWriteChainsLikeAnyOtherNode(t *testing.T) {
	text, err := New("lake").Step("extract", "orders.extract").IcebergWrite("load", icebergBase()).Step("notify", "notify.done").End("done").JSON()
	if err != nil {
		t.Fatal(err)
	}
	var built struct{ Nodes []map[string]any }
	if err = json.Unmarshal([]byte(text), &built); err != nil {
		t.Fatal(err)
	}
	if built.Nodes[0]["next"] != "load" || built.Nodes[1]["next"] != "notify" {
		t.Fatalf("not linked by the flow: %v", built.Nodes)
	}
}
