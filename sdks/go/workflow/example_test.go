package workflow_test

import (
	"fmt"
	"time"

	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/workflow"
)

// A workflow that charges a card and ships, written in the same way in every SDK: the canonical text
// is byte-identical in TypeScript, Go, Python, C# and Java.
func Example() {
	text, err := workflow.New("order-flow").
		Description("charges and ships").
		Step("charge", "payments.charge",
			workflow.Retry(workflow.MaxAttempts(3), workflow.WithStrategy(workflow.Exponential), workflow.Jitter(true)),
			workflow.Timeout(30*time.Second)).
		Sleep("pause", 2*time.Second).
		Step("ship", "shipping.ship").
		JSON()
	if err != nil {
		panic(err)
	}
	fmt.Println(text)
	// Output:
	// {"apiVersion":"sagat-octopus-orchestrator.io/v1","entrypoint":"charge","kind":"Workflow","metadata":{"description":"charges and ships","name":"order-flow"},"nodes":[{"activity":"payments.charge","id":"charge","next":"pause","retry":{"jitter":true,"maxAttempts":3,"strategy":"exponential"},"timeoutMs":30000,"type":"activity"},{"durationMs":2000,"id":"pause","next":"ship","type":"timer"},{"activity":"shipping.ship","id":"ship","type":"activity"}]}
}

// A saga: the body is compensated in reverse order when it fails; a step names its undo.
func ExampleBuilder_Saga() {
	body := workflow.Flow().
		Step("reserve", "stock.reserve", workflow.CompensateWith("stock.release")).
		Step("charge", "payments.charge", workflow.CompensateWith("payments.refund"), workflow.ReconcileWith("payments.lookup"))
	doc, err := workflow.New("checkout-flow").
		Saga("checkout", body, workflow.OnCancel(workflow.CompensateCompleted)).
		Step("notify", "notifications.send").
		Build()
	fmt.Println(err, doc["entrypoint"])
	// Output: <nil> checkout
}
