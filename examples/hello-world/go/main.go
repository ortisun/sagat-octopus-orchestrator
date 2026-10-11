// The hello-world workflow served and started with the Go SDK: a worker that answers the two
// activities of the workflow (templates/hello-world), a run of it, and the end of the run.
//
//	SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 \
//	SAGAT_NAMESPACE=payments SAGAT_TOKEN=... go run . [NAME]
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/signal"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
)

// What the compose step receives: the input of the run.
type Person struct {
	Name string `json:"name"`
}

type Greeting struct {
	Greeting string `json:"greeting"`
}

type Delivery struct {
	Delivered bool   `json:"delivered"`
	Greeting  string `json:"greeting"`
}

func main() {
	if err := run(); err != nil {
		var failure *sagat.Error
		if errors.As(err, &failure) {
			// The code, the trace and what to do about it; never the payload or the token.
			fmt.Fprintf(os.Stderr, "%s (trace %s): %s\n", failure.Code, failure.TraceID, failure.Remediation)
			if failure.Reason == "WAIT_TIMEOUT" {
				fmt.Fprintln(os.Stderr, "the run did not finish in time: is it waiting for a worker? Let this identity serve its activities (sagat worker bind)")
			}
		} else {
			fmt.Fprintln(os.Stderr, err)
		}
		os.Exit(1)
	}
}

func run() error {
	apiURL, grpcURL, namespace, token := setting("SAGAT_SERVER"), setting("SAGAT_GRPC"), setting("SAGAT_NAMESPACE"), setting("SAGAT_TOKEN")
	name := "Ada"
	if len(os.Args) > 1 {
		name = os.Args[1]
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()

	// The worker: it takes the tasks of the two activities and answers them.
	host, err := worker.New(grpcURL, namespace, "hello-world-go", sagat.StaticToken(token))
	if err != nil {
		return err
	}
	if err := host.RegisterActivity("greeting.compose", worker.Typed(func(ctx context.Context, in Person) (Greeting, error) {
		return Greeting{Greeting: "Hello, " + in.Name + "!"}, nil
	})); err != nil {
		return err
	}
	if err := host.RegisterActivity("greeting.deliver", worker.Typed(func(ctx context.Context, in Person) (Delivery, error) {
		// The outputs of the steps that ran before this one, by step id.
		var composed Greeting
		if err := json.Unmarshal(worker.TaskFrom(ctx).Steps["compose"], &composed); err != nil {
			return Delivery{}, worker.Permanent("NO_GREETING", "the compose step gave no greeting")
		}
		// A real delivery is an external effect: send worker.TaskFrom(ctx).IdempotencyKey to the
		// provider, which is the same for every attempt of this step.
		fmt.Printf("delivering %s\n", composed.Greeting)
		return Delivery{Delivered: true, Greeting: composed.Greeting}, nil
	})); err != nil {
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
	fmt.Printf("serving greeting.compose and greeting.deliver at protocol %s\n", host.ProtocolVersion())

	// The client: start the run and wait for it.
	api, err := client.New(apiURL, namespace, sagat.StaticToken(token))
	if err != nil {
		return err
	}
	started, err := api.Start(ctx, "hello-world", map[string]any{"name": name})
	if err != nil {
		return err
	}
	fmt.Printf("run %s started\n", started.RunID)
	done, err := api.WaitForCompletion(ctx, started.RunID, client.WaitTimeout(60*time.Second))
	if err != nil {
		return err
	}
	fmt.Printf("run %s %s\n", done.RunID, done.Status)
	if done.Status != "SUCCEEDED" {
		return fmt.Errorf("the run ended %s", done.Status)
	}
	return nil
}

func setting(name string) string {
	value := os.Getenv(name)
	if value == "" {
		fmt.Fprintf(os.Stderr, "%s is not set (see the README)\n", name)
		os.Exit(2)
	}
	return value
}
