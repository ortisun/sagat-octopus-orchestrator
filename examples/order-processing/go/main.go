// The order-processing examples serve the same activities against three workflow shapes:
// condition routing, parallel fulfillment, and a reminder timer.
package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/signal"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
)

type orderInput struct {
	OrderID    string `json:"orderId"`
	Amount     int    `json:"amount"`
	FailNotify bool   `json:"failNotify"`
}

type validation struct {
	Accepted bool `json:"accepted"`
}

type reservation struct {
	Reserved bool   `json:"reserved"`
	OrderID  string `json:"orderId"`
}

type quote struct {
	ShippingCents int `json:"shippingCents"`
}

type notification struct {
	Notified bool   `json:"notified"`
	OrderID  string `json:"orderId"`
}

func main() {
	exit := 0
	if err := run(); err != nil {
		exit = 1
		var failure *sagat.Error
		if errors.As(err, &failure) {
			fmt.Fprintf(os.Stderr, "%s (trace %s): %s\n", failure.Code, failure.TraceID, failure.Remediation)
		} else {
			fmt.Fprintln(os.Stderr, err)
		}
	}
	os.Exit(exit)
}

func run() error {
	apiURL := requiredSetting("SAGAT_SERVER")
	grpcURL := requiredSetting("SAGAT_GRPC")
	namespace := requiredSetting("SAGAT_NAMESPACE")
	token := requiredSetting("SAGAT_TOKEN")
	scenario, workflow := selectedScenario()

	input := orderInput{
		OrderID:    "demo-order-001",
		Amount:     100,
		FailNotify: os.Getenv("SAGAT_FAIL_NOTIFY") == "1",
	}
	if os.Getenv("SAGAT_REJECT_ORDER") == "1" {
		input.Amount = -1
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()

	host, err := worker.New(grpcURL, namespace, "order-processing-go", sagat.StaticToken(token))
	if err != nil {
		return err
	}
	if err := registerActivities(host); err != nil {
		return err
	}
	if err := host.Start(ctx); err != nil {
		return err
	}
	defer func() {
		grace, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		_ = host.Stop(grace)
	}()

	api, err := client.New(apiURL, namespace, sagat.StaticToken(token))
	if err != nil {
		return err
	}
	started, err := api.Start(ctx, workflow, input)
	if err != nil {
		return err
	}
	done, err := api.WaitForCompletion(ctx, started.RunID, client.WaitTimeout(60*time.Second))
	if err != nil {
		return err
	}
	fmt.Printf("scenario %s run %s %s\n", scenario, done.RunID, done.Status)
	if done.Status != "SUCCEEDED" {
		return fmt.Errorf("the run ended %s", done.Status)
	}
	return nil
}

func registerActivities(host *worker.Host) error {
	if err := host.RegisterActivity("order.validate", worker.Typed(func(_ context.Context, input orderInput) (validation, error) {
		fmt.Println("activity order.validate")
		return validation{Accepted: input.Amount > 0}, nil
	})); err != nil {
		return err
	}
	if err := host.RegisterActivity("order.reserve", worker.Typed(func(_ context.Context, input orderInput) (reservation, error) {
		fmt.Println("activity order.reserve")
		return reservation{Reserved: true, OrderID: input.OrderID}, nil
	})); err != nil {
		return err
	}
	if err := host.RegisterActivity("order.quote", worker.Typed(func(_ context.Context, _ orderInput) (quote, error) {
		fmt.Println("activity order.quote")
		return quote{ShippingCents: 500}, nil
	})); err != nil {
		return err
	}
	return host.RegisterActivity("order.notify", worker.Typed(func(ctx context.Context, input orderInput) (notification, error) {
		fmt.Println("activity order.notify")
		// Predecessors differ by scenario. If a real notification needs one, read it from
		// worker.TaskFrom(ctx).Steps with a presence check instead of assuming it exists.
		if input.FailNotify {
			return notification{}, worker.Permanent("NOTIFICATION_REJECTED", "the simulated notification was rejected")
		}
		// A real provider call must use task.IdempotencyKey for deduplication and reconcile an
		// unknown outcome before retrying. This example has no external effect.
		return notification{Notified: true, OrderID: input.OrderID}, nil
	}))
}

func selectedScenario() (string, string) {
	scenario := os.Getenv("SAGAT_SCENARIO")
	if scenario == "" {
		scenario = "routing"
	}
	workflows := map[string]string{
		"routing":     "order-routing",
		"fulfillment": "order-fulfillment",
		"reminder":    "order-reminder",
	}
	workflow, ok := workflows[scenario]
	if !ok {
		fmt.Fprintln(os.Stderr, "SAGAT_SCENARIO must be routing, fulfillment, or reminder")
		os.Exit(2)
	}
	return scenario, workflow
}

func requiredSetting(name string) string {
	value := os.Getenv(name)
	if value == "" {
		fmt.Fprintf(os.Stderr, "%s is not set (see the README)\n", name)
		os.Exit(2)
	}
	return value
}
