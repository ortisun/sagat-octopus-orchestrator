package client_test

import (
	"context"
	"errors"
	"fmt"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
)

// Start a run and wait for it. The example is compiled, not run: it needs a platform.
func Example() {
	c, err := client.New("https://api.example.com/v1", "payments", sagat.StaticToken("<token>"))
	if err != nil {
		panic(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()

	run, err := c.Start(ctx, "order-flow", map[string]any{"orderId": "o-1", "amount": "19.99"},
		client.WithIdempotencyKey("order-o-1"), client.WithCorrelationID("o-1"))
	if err != nil {
		var e *sagat.Error
		if errors.As(err, &e) {
			fmt.Println(e.Code, e.TraceID, e.Remediation)
		}
		return
	}
	done, err := c.WaitForCompletion(ctx, run.RunID, client.WaitTimeout(30*time.Second))
	if err != nil {
		return
	}
	fmt.Println(done.Status)
}
