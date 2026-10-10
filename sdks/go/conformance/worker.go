package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"sync"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
	"google.golang.org/grpc"
)

// workerLog is the log of one worker: an entry is appended when a handler ends.
type workerLog struct {
	mu      sync.Mutex
	entries []any
}

func (l *workerLog) add(entry object) {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.entries = append(l.entries, entry)
}

func (l *workerLog) len() int {
	l.mu.Lock()
	defer l.mu.Unlock()
	return len(l.entries)
}

func (l *workerLog) snapshot() []any {
	l.mu.Lock()
	defer l.mu.Unlock()
	return append([]any{}, l.entries...)
}

// waitOrCancelled waits ms, or until ctx ends; true if it ended. ms of 0 or less waits for the end.
func waitOrCancelled(ctx context.Context, ms time.Duration) bool {
	if ms <= 0 {
		<-ctx.Done()
		return true
	}
	timer := time.NewTimer(ms)
	defer timer.Stop()
	select {
	case <-timer.C:
		return false
	case <-ctx.Done():
		return true
	}
}

func failureOf(behavior object) *worker.ActivityFailure {
	f := asObject(behavior["fails"])
	return &worker.ActivityFailure{Code: asString(f["code"]), Message: asString(f["message"]), Retryable: f["retryable"] == true}
}

// behave evaluates a behavior of the scenario for one attempt (README, "Worker activities").
func behave(ctx context.Context, behavior object, input json.RawMessage, outcome *string) (result any, verdict worker.Verdict, err error) {
	task := worker.TaskFrom(ctx)
	if until, ok := behavior["failsUntilAttempt"]; ok && task.Attempt < asInt(until) {
		*outcome = "failed"
		return nil, verdict, failureOf(behavior)
	}
	if _, ok := behavior["failsUntilAttempt"]; !ok && behavior["fails"] != nil {
		*outcome = "failed"
		return nil, verdict, failureOf(behavior)
	}
	if text, ok := behavior["throws"]; ok {
		*outcome = "threw"
		return nil, verdict, errors.New(asString(text))
	}
	if text, ok := behavior["outcomeUnknown"]; ok {
		*outcome = "failed"
		return nil, verdict, worker.OutcomeUnknown(asString(text))
	}
	if hang, _ := behavior["hang"].(bool); hang {
		waitOrCancelled(ctx, 0)
		*outcome = "cancelled"
		return nil, verdict, nil
	}
	if ms, ok := behavior["sleepMs"]; ok {
		if waitOrCancelled(ctx, asMillis(ms)) {
			*outcome = "cancelled"
			onCancel := asObject(behavior["onCancel"])
			if raises, _ := onCancel["raises"].(bool); raises {
				return nil, verdict, sagat.NewError(sagat.CodeCancelled, "cancelled")
			}
			if returns, ok := onCancel["returns"]; ok {
				return raw(returns), verdict, nil
			}
			return nil, verdict, nil
		}
	}
	if name, ok := behavior["readsSecret"]; ok {
		secrets, err := task.Secrets(ctx)
		if err != nil {
			return nil, verdict, err
		}
		value, err := secrets.Bytes(asString(name))
		if err != nil {
			return nil, verdict, err
		}
		out := object{}
		for key, item := range asObject(behavior["returns"]) {
			out[key] = item
		}
		out["secretLength"] = len(value)
		clear(value)
		return out, verdict, nil
	}
	if kind, ok := behavior["verdict"]; ok {
		var output any
		if value, present := behavior["output"]; present {
			output = raw(value)
		}
		switch asString(kind) {
		case "applied":
			verdict = worker.Applied(output)
		case "not_applied":
			verdict = worker.NotApplied()
		default:
			verdict = worker.Unknown()
		}
		return nil, verdict, nil
	}
	if echo, _ := behavior["echo"].(bool); echo {
		return object{"input": input, "steps": task.Steps}, verdict, nil
	}
	if echo, _ := behavior["echoInput"].(bool); echo {
		return input, verdict, nil
	}
	if n, ok := behavior["returnsText"]; ok {
		return strings.Repeat("x", asInt(n)), verdict, nil
	}
	if returns, ok := behavior["returns"]; ok {
		return raw(returns), verdict, nil
	}
	return nil, verdict, nil
}

// logEntry is what the log records of an attempt, when its handler ends.
func logEntry(ctx context.Context, name, kind string, input json.RawMessage, outcome string) object {
	task := worker.TaskFrom(ctx)
	var aborted any
	if ctx.Err() != nil {
		aborted = context.Cause(ctx).Error()
	}
	traceID := ""
	if parts := strings.Split(task.Traceparent, "-"); len(parts) > 1 {
		traceID = parts[1]
	}
	return object{
		"activity": name, "kind": kind, "node": task.Node, "attempt": task.Attempt, "input": input,
		"taskKind": string(task.Kind), "targetNode": task.TargetNode, "targetIdempotencyKey": task.TargetIdempotencyKey,
		"idempotencyKey": task.IdempotencyKey, "traceparentPresent": task.Traceparent != "", "traceId": traceID,
		"steps": task.Steps, "deadlineAhead": task.Deadline().After(time.Now()), "abortedReason": aborted, "outcome": outcome,
	}
}

func hostOf(def object, config object, p *plan, log *workerLog) (*worker.Host, error) {
	token := asString(config["workerToken"])
	if asString(def["token"]) == "unbound" {
		token = asString(config["unboundWorkerToken"])
	}
	maxInflight := 4
	if n, ok := def["maxInflight"]; ok {
		maxInflight = asInt(n)
	}
	opts := []worker.Option{
		worker.WithMaxInflight(maxInflight),
		worker.WithDialOptions(grpc.WithChainUnaryInterceptor(p.unary())),
		worker.WithRetry(sagat.RetryPolicy{InitialDelay: 20 * time.Millisecond, MaxDelay: 200 * time.Millisecond}),
	}
	if v := asString(def["protocolVersion"]); v != "" {
		opts = append(opts, worker.WithProtocolVersion(v))
	}
	if ms, ok := def["heartbeatIntervalMs"]; ok {
		opts = append(opts, worker.WithHeartbeatInterval(asMillis(ms)))
	}
	host, err := worker.New(asString(config["grpcUrl"]), asString(config["namespace"]), asString(def["workerName"]), sagat.StaticToken(token), opts...)
	if err != nil {
		return nil, err
	}
	activities := asObject(def["activities"])
	for _, name := range sortedKeys(activities) {
		activity := asObject(activities[name])
		kind := asString(activity["as"])
		if kind == "" {
			kind = "activity"
		}
		behavior := asObject(activity["behavior"])
		name := name
		var registerErr error
		switch kind {
		case "reconciler":
			registerErr = host.RegisterReconciler(name, func(ctx context.Context, input json.RawMessage) (verdict worker.Verdict, err error) {
				outcome := "returned"
				defer func() { log.add(logEntry(ctx, name, kind, input, outcome)) }()
				_, verdict, err = behave(ctx, behavior, input, &outcome)
				return verdict, err
			})
		default:
			handler := func(ctx context.Context, input json.RawMessage) (any, error) {
				outcome := "returned"
				defer func() { log.add(logEntry(ctx, name, kind, input, outcome)) }()
				result, _, err := behave(ctx, behavior, input, &outcome)
				return result, err
			}
			if kind == "compensation" {
				registerErr = host.RegisterCompensation(name, handler)
			} else {
				registerErr = host.RegisterActivity(name, handler)
			}
		}
		if registerErr != nil {
			return nil, fmt.Errorf("registering %s: %w", name, registerErr)
		}
	}
	return host, nil
}
