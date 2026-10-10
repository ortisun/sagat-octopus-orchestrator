package worker_test

import (
	"context"
	"os"
	"os/signal"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
)

type ChargeInput struct {
	OrderID string `json:"orderId"`
	Amount  string `json:"amount"`
}

type Receipt struct {
	ChargeID string `json:"chargeId"`
}

// Register an activity and its compensation, run until interrupted, and stop with a grace period. The
// example is compiled, not run: it needs a platform.
func Example() {
	host, err := worker.New("https://workers.example.com:443", "payments", "payments-worker", sagat.StaticToken("<token>"))
	if err != nil {
		panic(err)
	}
	_ = host.RegisterActivity("payments.charge", worker.Typed(func(ctx context.Context, in ChargeInput) (Receipt, error) {
		task := worker.TaskFrom(ctx)
		// Send task.IdempotencyKey to the provider: the effect happens once however often this attempt is repeated.
		_ = task.IdempotencyKey
		if in.Amount == "" {
			return Receipt{}, worker.Permanent("INVALID_AMOUNT", "the order has no amount")
		}
		return Receipt{ChargeID: "ch_1"}, nil
	}))
	_ = host.RegisterCompensation("payments.refund", worker.Typed(func(ctx context.Context, in ChargeInput) (worker.Void, error) {
		// Refund the effect of TaskFrom(ctx).TargetNode, keyed by TargetIdempotencyKey. It must tolerate
		// an effect that never happened.
		return worker.Void{}, nil
	}))
	if err := host.Start(context.Background()); err != nil {
		panic(err)
	}
	interrupted, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()
	<-interrupted.Done()
	grace, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	_ = host.Stop(grace)
}
