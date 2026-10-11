// The hello-world workflow served and started with the Go SDK: a worker that answers the two
// activities of the workflow (templates/hello-world), a run of it, and the end of the run.
//
//	SAGAT_API_URL=http://127.0.0.1:8080/v1 SAGAT_GRPC_URL=http://127.0.0.1:19000 \
//	SAGAT_NAMESPACE=payments SAGAT_TOKEN=... go run . [NAME]
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

// What a step receives: the input of the run and the outputs of the steps that ran before it.
type Task struct {
	Input struct {
		Name string `json:"name"`
	} `json:"input"`
	Steps map[string]struct {
		Output struct {
			Greeting string `json:"greeting"`
		} `json:"output"`
	} `json:"steps"`
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
		} else {
			fmt.Fprintln(os.Stderr, err)
		}
		os.Exit(1)
	}
}

func run() error {
	apiURL, grpcURL, namespace, token := setting("SAGAT_API_URL"), setting("SAGAT_GRPC_URL"), setting("SAGAT_NAMESPACE"), setting("SAGAT_TOKEN")
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
	if err := host.RegisterActivity("greeting.compose", worker.Typed(func(ctx context.Context, in Task) (Greeting, error) {
		return Greeting{Greeting: "Hello, " + in.Input.Name + "!"}, nil
	})); err != nil {
		return err
	}
	if err := host.RegisterActivity("greeting.deliver", worker.Typed(func(ctx context.Context, in Task) (Delivery, error) {
		// A real delivery is an external effect: send worker.TaskFrom(ctx).IdempotencyKey to the
		// provider, which is the same for every attempt of this step.
		return Delivery{Delivered: true, Greeting: in.Steps["compose"].Output.Greeting}, nil
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
