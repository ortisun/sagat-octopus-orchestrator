package ops_test

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/workflow/ops"
)

func text(t *testing.T, stage map[string]any) string {
	t.Helper()
	var out strings.Builder
	encoder := json.NewEncoder(&out)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(stage); err != nil {
		t.Fatal(err)
	}
	return strings.TrimSuffix(out.String(), "\n")
}

func TestTheFactoryWritesOneStagePerOperatorKeyedByItsName(t *testing.T) {
	cases := []struct {
		operator ops.Operator
		name     string
		want     string
	}{
		{ops.Filter("element.total > 0"), "filter", `{"filter":{"where":"element.total > 0"}}`},
		{ops.Map("element.lines"), "map", `{"map":{"select":"element.lines"}}`},
		{ops.FlatMap("element"), "flatMap", `{"flatMap":{"select":"element"}}`},
		{ops.Skip(0), "skip", `{"skip":{"count":0}}`},
		{ops.Take(1000), "take", `{"take":{"count":1000}}`},
		{ops.DistinctBy("element.sku"), "distinct", `{"distinct":{"by":"element.sku"}}`},
		{ops.Distinct(), "distinct", `{"distinct":{}}`},
		{ops.Window(25), "window", `{"window":{"count":25}}`},
	}
	for _, c := range cases {
		if got := text(t, c.operator.Stage()); got != c.want {
			t.Errorf("got %s, want %s", got, c.want)
		}
		if c.operator.Name() != c.name {
			t.Errorf("name %q, want %q", c.operator.Name(), c.name)
		}
	}
}

func TestTheFactoryOnlyAssemblesAndLeavesRangesToTheServer(t *testing.T) {
	if got := text(t, ops.Window(0).Stage()); got != `{"window":{"count":0}}` {
		t.Errorf("got %s", got)
	}
	if got := text(t, ops.Skip(-1).Stage()); got != `{"skip":{"count":-1}}` {
		t.Errorf("got %s", got)
	}
}

func TestAStageIsAFreshCopy(t *testing.T) {
	operator := ops.Filter("element.ok")
	stage := operator.Stage()
	stage["filter"].(map[string]any)["where"] = "true"
	stage["take"] = nil
	if got := text(t, operator.Stage()); got != `{"filter":{"where":"element.ok"}}` {
		t.Errorf("the operator changed: %s", got)
	}
}

func TestTheZeroOperatorHasNoName(t *testing.T) {
	var zero ops.Operator
	if zero.Name() != "" {
		t.Errorf("name %q", zero.Name())
	}
}
