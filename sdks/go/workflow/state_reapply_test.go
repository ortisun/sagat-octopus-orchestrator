package workflow_test

import (
	"testing"

	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/workflow"
)

func accountState(update workflow.UpdateDefinition) (map[string]any, error) {
	update.ArgsSchema = doc{"type": "integer"}
	update.State = "state"
	update.Result = "newState.balance"
	update.ResultSchema = doc{"type": "integer"}
	update.Disclosure = workflow.DeclaredBusinessResultV1
	document, err := workflow.New("account-state").
		State(workflow.StateDefinition{
			Schema:  doc{"type": "object", "properties": doc{"balance": doc{"type": "integer"}}, "required": []any{"balance"}, "additionalProperties": false},
			Initial: "input.account",
			Updates: map[string]workflow.UpdateDefinition{"balance.credit": update},
		}).
		End("done").Build()
	if err != nil {
		return nil, err
	}
	state := document["state"].(map[string]any)
	return state["updates"].(map[string]any)["balance.credit"].(map[string]any), nil
}

func TestAnUpdateOptsInToKeepItsArgumentsForAResetAndTheDefaultIsNotWritten(t *testing.T) {
	kept, err := accountState(workflow.UpdateDefinition{ReapplyOnReset: true})
	if err != nil {
		t.Fatal(err)
	}
	if kept["reapplyOnReset"] != true {
		t.Errorf("an Update that opts in writes reapplyOnReset: true, got %v", kept)
	}
	// the default is not written: a document that does not opt in is the one made before the field
	plain, err := accountState(workflow.UpdateDefinition{})
	if err != nil {
		t.Fatal(err)
	}
	if _, written := plain["reapplyOnReset"]; written {
		t.Errorf("a default is not part of the document, got %v", plain)
	}
}
