package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"sort"
	"strings"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
)

func asError(err error, target **sagat.Error) bool { return errors.As(err, target) }

func iso(t time.Time) any {
	if t.IsZero() {
		return nil
	}
	return t.UTC().Format("2006-01-02T15:04:05.000Z")
}

func orNull(s string) any {
	if s == "" {
		return nil
	}
	return s
}

func runJSON(run *client.Run) object {
	var parent any
	if run.Parent != nil {
		parent = object{"runId": run.Parent.RunID, "node": run.Parent.Node, "cancelPolicy": run.Parent.CancelPolicy}
	}
	observed := object{
		"runId": run.RunID, "workflowId": run.WorkflowID, "versionId": run.VersionID, "status": string(run.Status), "version": run.Version,
		"paused": run.Paused, "startedBy": orNull(run.StartedBy), "createdAt": iso(run.CreatedAt), "updatedAt": iso(run.UpdatedAt),
		"finishedAt": iso(run.FinishedAt), "deadline": iso(run.Deadline), "traceId": orNull(run.TraceID),
		"correlationId": orNull(run.CorrelationID), "depth": run.Depth, "parent": parent,
	}
	// Present only for a run in a chain, so the observations of every other run stay as they were.
	if run.Chain != nil {
		observed["chain"] = object{
			"chainId": run.Chain.ChainID, "segmentIndex": run.Chain.SegmentIndex,
			"completionKind": orNull(string(run.Chain.CompletionKind)), "successorRunId": orNull(run.Chain.SuccessorRunID),
		}
	}
	// Present only for a run started again from an ended one.
	if run.RerunOf != "" {
		observed["rerunOf"] = run.RerunOf
	}
	return observed
}

func chainJSON(chain *client.ChainInspection) object {
	segments := make([]any, 0, len(chain.Segments))
	for _, item := range chain.Segments {
		segments = append(segments, object{"index": item.Index, "runId": item.RunID, "predecessorRunId": orNull(item.PredecessorRunID), "successorRunId": orNull(item.SuccessorRunID)})
	}
	return object{"chainId": chain.ChainID, "currentRunId": chain.CurrentRunID, "revision": chain.Revision, "cancelRequested": chain.CancelRequested, "finalOutcome": orNull(chain.FinalOutcome), "segments": segments}
}

func errorJSON(err error) object {
	var se *sagat.Error
	if !errors.As(err, &se) {
		return object{"code": "UNEXPECTED", "message": fmt.Sprintf("%T", err)}
	}
	var status any
	switch {
	case se.HTTPStatus != 0:
		status = se.HTTPStatus
	case se.GRPCStatus != "":
		status = se.GRPCStatus
	}
	var retryAfter any
	if se.RetryAfter != nil {
		retryAfter = se.RetryAfter.Milliseconds()
	}
	return object{
		"code": string(se.Code), "status": status, "retryable": se.Retryable, "traceId": se.TraceID, "reason": orNull(se.Reason),
		"retryAfterMs": retryAfter, "remediation": se.Remediation, "message": se.Message,
	}
}

type hostEntry struct {
	def  object
	host *worker.Host
	log  *workerLog
}

func run(scenario, config object) (object, error) {
	ctx := context.Background()
	unique := asString(config["unique"])
	var faults []fault
	if list := scenario["faults"]; list != nil {
		if err := json.Unmarshal(raw(list), &faults); err != nil {
			return nil, fmt.Errorf("faults: %w", err)
		}
	}
	p := newPlan(faults)
	apiURL, err := url.Parse(asString(config["apiUrl"]))
	if err != nil {
		return nil, err
	}
	httpClient := &http.Client{Transport: &transport{plan: p, inner: http.DefaultTransport.(*http.Transport).Clone(), apiPath: strings.TrimRight(apiURL.Path, "/")}}
	clientFor := func(token string) (*client.Client, error) {
		return client.New(asString(config["apiUrl"]), asString(config["namespace"]), sagat.StaticToken(token),
			client.WithHTTPClient(httpClient),
			client.WithRetry(sagat.RetryPolicy{InitialDelay: 20 * time.Millisecond, MaxDelay: 200 * time.Millisecond}))
	}
	clients := map[string]*client.Client{}
	for name, key := range map[string]string{
		"default": "clientToken", "bad": "badToken", "other": "otherTenantToken",
		// Members of the tenant with one role each, the worker identity, and an operator the scenario revokes.
		"worker": "workerToken", "operator": "operatorToken", "viewer": "viewerToken", "runner": "runnerToken", "revoked": "revokedToken",
	} {
		if clients[name], err = clientFor(asString(config[key])); err != nil {
			return nil, err
		}
	}

	hosts := map[string]*hostEntry{}
	for key, def := range asObject(scenario["workers"]) {
		hosts[key] = &hostEntry{def: asObject(def), log: &workerLog{}}
	}
	startWorker := func(key string) (object, error) {
		entry, ok := hosts[key]
		if !ok {
			return nil, fmt.Errorf("no worker %s", key)
		}
		host, err := hostOf(entry.def, config, p, entry.log)
		if err != nil {
			return nil, err
		}
		entry.host = host
		if err := host.Start(ctx); err != nil {
			return nil, err
		}
		return object{"protocolVersion": host.ProtocolVersion()}, nil
	}
	stopWorker := func(entry *hostEntry, timeout time.Duration) {
		if entry.host == nil {
			return
		}
		stopCtx, cancel := context.WithTimeout(ctx, timeout)
		defer cancel()
		_ = entry.host.Stop(stopCtx)
	}

	results := map[string]object{}
	runOf := func(step object) (string, error) {
		if page := asString(step["page"]); page != "" {
			// A segment of a page of listChainRuns, by position.
			items := asList(results[page]["items"])
			index := asCount(step["index"])
			if index < 0 || index >= len(items) || asString(asObject(items[index])["runId"]) == "" {
				return "", fmt.Errorf("step %s has no segment %d", page, index)
			}
			return asString(asObject(items[index])["runId"]), nil
		}
		// A rerun answers "newRunId" and no "runId": the step then names the run it started.
		named := results[asString(step["run"])]
		id, _ := named["runId"].(string)
		if id == "" {
			id, _ = named["newRunId"].(string)
		}
		if id == "" {
			return "", fmt.Errorf("step %s has no run", asString(step["run"]))
		}
		return id, nil
	}
	approvalOf := func(step object) (string, error) {
		if id := asString(step["approvalId"]); id != "" {
			return id, nil
		}
		items := asList(results[asString(step["approval"])]["items"])
		if len(items) == 0 || asString(asObject(items[0])["approvalId"]) == "" {
			return "", fmt.Errorf("step %s has no approval", asString(step["approval"]))
		}
		return asString(asObject(items[0])["approvalId"]), nil
	}

	for _, key := range sortedHostKeys(hosts) {
		entry := hosts[key]
		if autostart, present := entry.def["autostart"]; present && autostart == false {
			continue
		}
		if _, err := startWorker(key); err != nil {
			return nil, fmt.Errorf("worker %s did not start: %w", key, err)
		}
	}

	observations := []any{}
	for _, item := range asList(scenario["steps"]) {
		step := asObject(item)
		id, op := asString(step["id"]), asString(step["op"])
		tokenName := asString(step["token"])
		if tokenName == "" {
			tokenName = "default"
		}
		c, ok := clients[tokenName]
		if !ok {
			return nil, fmt.Errorf("step %s: unknown token %q", id, tokenName)
		}
		observation := object{"id": id, "op": op}
		result, err := playStep(ctx, step, op, c, config, unique, p, hosts, startWorker, stopWorker, runOf, approvalOf, results)
		if err != nil {
			var unexpected *runnerError
			if errors.As(err, &unexpected) {
				return nil, fmt.Errorf("step %s: %w", id, err)
			}
			observation["ok"] = false
			observation["error"] = errorJSON(err)
		} else {
			observation["ok"] = true
			if result != nil {
				observation["result"] = result
				results[id] = result
			}
		}
		observations = append(observations, observation)
	}

	logs := object{}
	measurements := object{}
	for key, entry := range hosts {
		stopWorker(entry, 2*time.Second)
		logs[key] = entry.log.snapshot()
		measurements[key] = entry.log.measurements()
	}
	return object{
		"scenario": scenario["name"], "language": "go", "sdkVersion": sagat.Version,
		"observations": observations, "workerLog": logs, "workerMeasurements": measurements, "faultsApplied": p.applied(),
	}, nil
}

func sortedHostKeys(hosts map[string]*hostEntry) []string {
	keys := make([]string, 0, len(hosts))
	for key := range hosts {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}

// runnerError is a mistake of the runner or of the scenario, not an outcome to observe: the run ends.
type runnerError struct{ message string }

func (e *runnerError) Error() string { return e.message }

func broken(format string, args ...any) error { return &runnerError{fmt.Sprintf(format, args...)} }

func playStep(ctx context.Context, step object, op string, c *client.Client, config object, unique string, p *plan,
	hosts map[string]*hostEntry, startWorker func(string) (object, error), stopWorker func(*hostEntry, time.Duration),
	runOf func(object) (string, error), approvalOf func(object) (string, error), results map[string]object) (object, error) {
	workflowOf := func(key string) (name, id string) {
		w := asObject(asObject(config["workflows"])[key])
		return asString(w["name"]), asString(w["workflowId"])
	}
	chainRef := func(value any) (string, error) {
		text := asString(value)
		if text == "{{namespaceId}}" {
			return asString(config["namespaceId"]), nil
		}
		field := ""
		ref := ""
		if strings.HasPrefix(text, "{{chainId:") && strings.HasSuffix(text, "}}") {
			field, ref = "runId", strings.TrimSuffix(strings.TrimPrefix(text, "{{chainId:"), "}}")
		} else if strings.HasPrefix(text, "{{currentRunId:") && strings.HasSuffix(text, "}}") {
			field, ref = "currentRunId", strings.TrimSuffix(strings.TrimPrefix(text, "{{currentRunId:"), "}}")
		}
		if field == "" {
			return text, nil
		}
		resolved := asString(results[ref][field])
		if resolved == "" {
			return "", broken("step %s has no %s", ref, field)
		}
		return resolved, nil
	}
	var callOpts []client.CallOption
	if key := asString(step["idempotencyKey"]); key != "" {
		callOpts = append(callOpts, client.WithIdempotencyKey(key))
	}
	switch op {
	case "start":
		_, workflowID := workflowOf(asString(step["workflow"]))
		opts := callOpts
		if v := asString(step["correlationId"]); v != "" {
			opts = append(opts, client.WithCorrelationID(v))
		}
		if v := asString(step["traceparent"]); v != "" {
			opts = append(opts, client.WithTraceparent(v))
		}
		if v, ok := step["deadlineInMs"]; ok {
			opts = append(opts, client.WithDeadlineIn(asMillis(v)))
		}
		run, err := c.Start(ctx, workflowID, startInput(step), opts...)
		if err != nil {
			return nil, err
		}
		return runJSON(run), nil
	case "startWithBlob":
		_, workflowID := workflowOf(asString(step["workflow"]))
		opts := callOpts
		if v := asString(step["correlationId"]); v != "" {
			opts = append(opts, client.WithCorrelationID(v))
		}
		if v := asString(step["traceparent"]); v != "" {
			opts = append(opts, client.WithTraceparent(v))
		}
		if v, ok := step["deadlineInMs"]; ok {
			opts = append(opts, client.WithDeadlineIn(asMillis(v)))
		}
		var body []byte
		if text, ok := step["inputBlobText"].(string); ok {
			body = []byte(text)
		} else {
			body = bytes.Repeat([]byte("i"), asInt(step["inputBlobBytes"]))
		}
		digest := sha256.Sum256(body)
		size := int64(len(body))
		descriptor, err := c.UploadRunInput(ctx, bytes.NewReader(body), client.UploadRunInputOptions{
			ExpectedSize: &size, ExpectedSHA256: hex.EncodeToString(digest[:]),
		})
		if err != nil {
			return nil, err
		}
		run, err := c.StartWithBlob(ctx, workflowID, descriptor, opts...)
		if err != nil {
			return nil, err
		}
		return runJSON(run), nil
	case "workflowByName":
		name, _ := workflowOf(asString(step["workflow"]))
		run, err := c.Start(ctx, name, startInput(step), callOpts...)
		if err != nil {
			return nil, err
		}
		return runJSON(run), nil
	case "get":
		id, err := runOf(step)
		if err != nil {
			return nil, broken("%v", err)
		}
		run, err := c.Get(ctx, id)
		if err != nil {
			return nil, err
		}
		return runJSON(run), nil
	case "list":
		options := client.ListOptions{Status: client.RunStatus(asString(step["status"]))}
		if v, ok := step["limit"]; ok {
			options.Limit = asInt(v)
		}
		ids := []any{}
		if all, _ := step["all"].(bool); all {
			for run, err := range c.ListAll(ctx, options) {
				if err != nil {
					return nil, err
				}
				ids = append(ids, run.RunID)
			}
		} else {
			// "waitForCount": the runs of a chain are made by the scheduler, so a scenario that needs a hand-off
			// to have happened waits for it by the count of runs in a status.
			for attempt := 0; ; attempt++ {
				page, err := c.List(ctx, options)
				if err != nil {
					return nil, err
				}
				ids = ids[:0]
				for _, run := range page.Items {
					ids = append(ids, run.RunID)
				}
				if len(ids) >= asInt(step["waitForCount"]) || attempt >= 100 {
					break
				}
				time.Sleep(50 * time.Millisecond)
			}
		}
		// In the order the server gave (newest first): the SDK keeps it, also across pages. "countOnly" answers
		// the number alone, for runs whose ids are derived from their parent's and so sort anywhere.
		if only, _ := step["countOnly"].(bool); only {
			return object{"count": len(ids)}, nil
		}
		return object{"items": ids}, nil
	case "listApprovals":
		id, err := runOf(step)
		if err != nil {
			return nil, broken("%v", err)
		}
		var listOpts []client.CallOption
		if v := asString(step["traceparent"]); v != "" {
			listOpts = append(listOpts, client.WithTraceparent(v))
		}
		page, err := c.ListApprovals(ctx, id, client.ApprovalListOptions{Cursor: asString(step["cursor"]), Limit: asInt(step["limit"])}, listOpts...)
		if err != nil {
			return nil, err
		}
		for attempt := 0; step["waitForItem"] == true && len(page.Items) == 0 && attempt < 100; attempt++ {
			select {
			case <-ctx.Done():
				return nil, ctx.Err()
			case <-time.After(50 * time.Millisecond):
			}
			page, err = c.ListApprovals(ctx, id, client.ApprovalListOptions{Cursor: asString(step["cursor"]), Limit: asInt(step["limit"])}, listOpts...)
			if err != nil {
				return nil, err
			}
		}
		items := make([]any, 0, len(page.Items))
		for _, item := range page.Items {
			path := make([]any, 0, len(item.InstancePath))
			for _, scope := range item.InstancePath {
				path = append(path, object{"scopeNodeId": scope.ScopeNodeID, "iterationIndex": scope.IterationIndex})
			}
			items = append(items, object{"approvalId": item.ApprovalID, "runId": item.RunID, "nodeId": item.NodeID,
				"instancePath": path, "status": string(item.Status), "deadline": iso(item.Deadline), "settledAt": iso(item.SettledAt)})
		}
		return object{"items": items, "nextCursor": orNull(page.NextCursor)}, nil
	case "cancel", "operate":
		id, err := runOf(step)
		if err != nil {
			return nil, broken("%v", err)
		}
		opts := callOpts
		if v, ok := step["expectedRunVersion"]; ok {
			n, _ := v.(json.Number).Int64()
			opts = append(opts, client.WithExpectedRunVersion(n))
		}
		var result *client.OperationResult
		if op == "cancel" {
			result, err = c.Cancel(ctx, id, asString(step["reason"]), opts...)
		} else {
			if v := asString(step["stepId"]); v != "" {
				opts = append(opts, client.WithStepID(v))
			}
			if v := asString(step["outcome"]); v != "" {
				opts = append(opts, client.WithOutcome(v))
			}
			if v := asString(step["errorCode"]); v != "" {
				opts = append(opts, client.WithErrorCode(v))
			}
			result, err = c.Operate(ctx, id, client.Action(asString(step["action"])), asString(step["reason"]), opts...)
		}
		if err != nil {
			return nil, err
		}
		observed := object{"accepted": result.Accepted, "runStatus": orNull(string(result.RunStatus))}
		// "newRunId" only for the action that starts a run, so the other operations observe as they did.
		if result.NewRunID != "" {
			observed["newRunId"] = result.NewRunID
		}
		return observed, nil
	case "decideApproval":
		approvalID, err := approvalOf(step)
		if err != nil {
			return nil, broken("%v", err)
		}
		request := client.ApprovalDecisionRequest{
			DecisionKey: asString(step["decisionKey"]), Decision: client.ApprovalDecision(asString(step["decision"])), ReasonCode: asString(step["reasonCode"]),
		}
		opts := []client.CallOption{}
		if value := asString(step["traceparent"]); value != "" {
			opts = append(opts, client.WithTraceparent(value))
		}
		receipt, err := c.DecideApproval(ctx, approvalID, request, opts...)
		if err != nil {
			return nil, err
		}
		return object{
			"receiptId": receipt.ReceiptID, "approvalId": receipt.ApprovalID, "decisionKey": receipt.DecisionKey,
			"decision": string(receipt.Decision), "reasonCode": orNull(receipt.ReasonCode), "actor": receipt.Actor,
			"decidedAt": iso(receipt.DecidedAt), "outcomeSequence": receipt.OutcomeSequence, "replayed": receipt.Replayed,
		}, nil
	case "query":
		id, err := runOf(step)
		if err != nil {
			return nil, broken("%v", err)
		}
		opts := callOpts
		if value := asString(step["traceparent"]); value != "" {
			opts = append(opts, client.WithTraceparent(value))
		}
		result, err := c.Query(ctx, id, asString(step["name"]), payloadOf(step["args"]), opts...)
		if err != nil {
			return nil, err
		}
		return object{"runVersion": result.RunVersion, "stateRevision": result.StateRevision, "result": result.Result}, nil
	case "queryChain":
		query := client.ChainQueryRequest{Args: payloadOf(step["args"])}
		if value := asString(step["expectedRunId"]); value != "" {
			expectedRunID, err := chainRef(value)
			if err != nil {
				return nil, broken("%v", err)
			}
			query.ExpectedRunID = expectedRunID
		}
		namespaceID, err := chainRef(step["namespaceId"])
		if err != nil {
			return nil, broken("%v", err)
		}
		chainID, err := chainRef(step["chainId"])
		if err != nil {
			return nil, broken("%v", err)
		}
		opts := callOpts
		if value := asString(step["traceparent"]); value != "" {
			opts = append(opts, client.WithTraceparent(value))
		}
		result, err := c.QueryChain(ctx, namespaceID, chainID, asString(step["name"]), query, opts...)
		if err != nil {
			return nil, err
		}
		return object{"runId": result.RunID, "runVersion": result.RunVersion, "stateRevision": result.StateRevision, "result": result.Result}, nil
	case "update":
		id, err := runOf(step)
		if err != nil {
			return nil, broken("%v", err)
		}
		request := client.UpdateRequest{UpdateKey: asString(step["updateKey"]), Args: payloadOf(step["args"])}
		if value, ok := step["expectedStateRevision"]; ok {
			revision := int64(asInt(value))
			request.ExpectedStateRevision = &revision
		}
		opts := callOpts
		if value := asString(step["traceparent"]); value != "" {
			opts = append(opts, client.WithTraceparent(value))
		}
		result, err := c.Update(ctx, id, asString(step["name"]), request, opts...)
		if err != nil {
			return nil, err
		}
		return object{
			"operation": result.Operation, "updateKey": result.UpdateKey, "status": result.Status, "result": result.Result,
			"beforeStateRevision": result.BeforeStateRevision, "afterStateRevision": result.AfterStateRevision,
			"runVersion": result.RunVersion, "replayed": result.Replayed,
		}, nil
	case "updateChain":
		namespaceID, err := chainRef(step["namespaceId"])
		if err != nil {
			return nil, broken("%v", err)
		}
		chainID, err := chainRef(step["chainId"])
		if err != nil {
			return nil, broken("%v", err)
		}
		request := client.ChainUpdateRequest{UpdateKey: asString(step["updateKey"]), Args: payloadOf(step["args"])}
		if value := asString(step["expectedRunId"]); value != "" {
			request.ExpectedRunID, err = chainRef(value)
			if err != nil {
				return nil, broken("%v", err)
			}
		}
		if value, ok := step["expectedStateRevision"]; ok {
			revision := int64(asInt(value))
			request.ExpectedStateRevision = &revision
		}
		opts := callOpts
		if value := asString(step["traceparent"]); value != "" {
			opts = append(opts, client.WithTraceparent(value))
		}
		result, err := c.UpdateChain(ctx, namespaceID, chainID, asString(step["name"]), request, opts...)
		if err != nil {
			return nil, err
		}
		return object{
			"runId": result.RunID, "operation": result.Operation, "updateKey": result.UpdateKey,
			"status": result.Status, "result": result.Result,
			"beforeStateRevision": result.BeforeStateRevision, "afterStateRevision": result.AfterStateRevision,
			"runVersion": result.RunVersion, "replayed": result.Replayed,
		}, nil
	case "listChainRuns":
		namespaceID, err := chainRef(step["namespaceId"])
		if err != nil {
			return nil, err
		}
		chainID, err := chainRef(step["chainId"])
		if err != nil {
			return nil, err
		}
		options := client.ChainRunListOptions{}
		options.Limit = asCount(step["limit"])
		options.Cursor = asString(step["cursor"])
		if strings.HasPrefix(options.Cursor, "{{nextCursor:") && strings.HasSuffix(options.Cursor, "}}") {
			ref := strings.TrimSuffix(strings.TrimPrefix(options.Cursor, "{{nextCursor:"), "}}")
			options.Cursor = asString(results[ref]["nextCursor"])
			if options.Cursor == "" {
				return nil, broken("step %s has no next page", ref)
			}
		}
		opts := callOpts
		if value := asString(step["traceparent"]); value != "" {
			opts = append(opts, client.WithTraceparent(value))
		}
		page, err := c.ListChainRuns(ctx, namespaceID, chainID, options, opts...)
		if err != nil {
			return nil, err
		}
		items := make([]any, 0, len(page.Items))
		for _, item := range page.Items {
			items = append(items, object{
				"index": item.Index, "runId": item.RunID, "status": string(item.Status),
				"completionKind": orNull(string(item.CompletionKind)), "predecessorRunId": orNull(item.PredecessorRunID),
				"successorRunId": orNull(item.SuccessorRunID), "createdAt": iso(item.CreatedAt), "finishedAt": iso(item.FinishedAt),
			})
		}
		return object{"items": items, "nextCursor": orNull(page.NextCursor)}, nil
	case "inspectChain":
		namespaceID, err := chainRef(step["namespaceId"])
		if err != nil {
			return nil, err
		}
		chainID, err := chainRef(step["chainId"])
		if err != nil {
			return nil, err
		}
		opts := callOpts
		if value := asString(step["traceparent"]); value != "" {
			opts = append(opts, client.WithTraceparent(value))
		}
		result, err := c.InspectChain(ctx, namespaceID, chainID, opts...)
		if err != nil {
			return nil, err
		}
		if wait := asString(step["waitForHeadChange"]); wait != "" {
			previous, err := chainRef(wait)
			if err != nil {
				return nil, err
			}
			for attempt := 0; result.CurrentRunID == previous && attempt < 100; attempt++ {
				time.Sleep(50 * time.Millisecond)
				result, err = c.InspectChain(ctx, namespaceID, chainID, opts...)
				if err != nil {
					return nil, err
				}
			}
		}
		return chainJSON(result), nil
	case "signalChain":
		namespaceID, err := chainRef(step["namespaceId"])
		if err != nil {
			return nil, err
		}
		chainID, err := chainRef(step["chainId"])
		if err != nil {
			return nil, err
		}
		expectedRunID, err := chainRef(step["expectedRunId"])
		if err != nil {
			return nil, err
		}
		opts := callOpts
		if value := asString(step["traceparent"]); value != "" {
			opts = append(opts, client.WithTraceparent(value))
		}
		result, err := c.SignalChain(ctx, namespaceID, chainID, asString(step["name"]), payloadOf(step["payload"]), client.ChainCommand{ExpectedRunID: expectedRunID}, opts...)
		if err != nil {
			return nil, err
		}
		delivered := make([]any, len(result.DeliveredTo))
		for i, value := range result.DeliveredTo {
			delivered[i] = value
		}
		return object{"deliveredTo": delivered}, nil
	case "cancelChain":
		namespaceID, err := chainRef(step["namespaceId"])
		if err != nil {
			return nil, err
		}
		chainID, err := chainRef(step["chainId"])
		if err != nil {
			return nil, err
		}
		expectedRunID, err := chainRef(step["expectedRunId"])
		if err != nil {
			return nil, err
		}
		opts := callOpts
		if value := asString(step["traceparent"]); value != "" {
			opts = append(opts, client.WithTraceparent(value))
		}
		err = c.CancelChain(ctx, namespaceID, chainID, asString(step["reason"]), client.ChainCommand{ExpectedRunID: expectedRunID}, opts...)
		if err != nil {
			return nil, err
		}
		return object{"accepted": true}, nil
	case "signal":
		id, err := runOf(step)
		if err != nil {
			return nil, broken("%v", err)
		}
		result, err := c.Signal(ctx, id, asString(step["name"]), payloadOf(step["payload"]), callOpts...)
		if err != nil {
			return nil, err
		}
		delivered := make([]any, len(result.DeliveredTo))
		for i, name := range result.DeliveredTo {
			delivered[i] = name
		}
		return object{"deliveredTo": delivered}, nil
	case "publishEvent":
		receipt, err := c.PublishEvent(ctx, client.Event{
			ID: asString(step["eventId"]), Type: asString(step["type"]), Data: payloadOf(step["data"]), CorrelationKey: asString(step["correlationKey"]),
		})
		if err != nil {
			return nil, err
		}
		return object{"eventId": receipt.EventID, "status": string(receipt.Status)}, nil
	case "waitForCompletion":
		id, err := runOf(step)
		if err != nil {
			return nil, broken("%v", err)
		}
		timeout := 60 * time.Second
		if v, ok := step["timeoutMs"]; ok {
			timeout = asMillis(v)
		}
		run, err := c.WaitForCompletion(ctx, id, client.WaitTimeout(timeout), client.PollInterval(100*time.Millisecond))
		if err != nil {
			return nil, err
		}
		return runJSON(run), nil
	case "waitForChainCompletion":
		namespaceID, err := chainRef(step["namespaceId"])
		if err != nil {
			return nil, err
		}
		chainID, err := chainRef(step["chainId"])
		if err != nil {
			return nil, err
		}
		timeout := 60 * time.Second
		if v, ok := step["timeoutMs"]; ok {
			timeout = asMillis(v)
		}
		chain, err := c.WaitForChainCompletion(ctx, namespaceID, chainID, client.WaitTimeout(timeout), client.PollInterval(100*time.Millisecond))
		if err != nil {
			return nil, err
		}
		return chainJSON(chain), nil
	case "events":
		id, err := runOf(step)
		if err != nil {
			return nil, broken("%v", err)
		}
		page, err := c.Events(ctx, id, client.EventsOptions{Limit: 200})
		if err != nil {
			return nil, err
		}
		events := make([]any, len(page.Items))
		for i, e := range page.Items {
			events[i] = object{"type": rawOrNull(e.Raw["eventType"]), "stepId": rawOrNull(e.Raw["stepId"]), "attempt": rawOrNull(e.Raw["attempt"])}
		}
		return object{"events": events}, nil
	case "sleep":
		time.Sleep(asMillis(step["ms"]))
		return nil, nil
	case "control":
		body := object{"action": step["action"]}
		if step["run"] != nil {
			id, err := runOf(step)
			if err != nil {
				return nil, broken("%v", err)
			}
			body["run"] = id
		}
		if step["node"] != nil {
			body["node"] = step["node"]
		}
		return nil, control(ctx, asString(config["controlUrl"]), body)
	case "startWorker":
		return startWorkerStep(startWorker, asString(step["worker"]))
	case "stopWorker":
		entry, ok := hosts[asString(step["worker"])]
		if !ok {
			return nil, broken("no worker %s", asString(step["worker"]))
		}
		timeout := 5 * time.Second
		if v, ok := step["timeoutMs"]; ok {
			timeout = asMillis(v)
		}
		stopWorker(entry, timeout)
		return nil, nil
	case "restartWorker":
		// Stop, then start, on the same host object: the host is restartable (SPEC section 6.8).
		entry, ok := hosts[asString(step["worker"])]
		if !ok || entry.host == nil {
			return nil, broken("worker %s was never started", asString(step["worker"]))
		}
		timeout := 5 * time.Second
		if v, ok := step["timeoutMs"]; ok {
			timeout = asMillis(v)
		}
		stopWorker(entry, timeout)
		if err := entry.host.Start(ctx); err != nil {
			var se *sagat.Error
			if errors.As(err, &se) {
				return nil, err
			}
			return nil, broken("%v", err)
		}
		return object{"protocolVersion": entry.host.ProtocolVersion()}, nil
	case "probeWorker":
		return probeWorker(ctx, config, unique, asString(step["protocolVersion"]))
	case "awaitJournal":
		id, err := runOf(step)
		if err != nil {
			return nil, broken("%v", err)
		}
		timeout := 20 * time.Second
		if v, ok := step["timeoutMs"]; ok {
			timeout = asMillis(v)
		}
		end := time.Now().Add(timeout)
		for {
			page, err := c.Events(ctx, id, client.EventsOptions{Limit: 200})
			if err != nil {
				return nil, err
			}
			for _, e := range page.Items {
				if e.Type == asString(step["eventType"]) && (step["stepId"] == nil || e.StepID == asString(step["stepId"])) {
					return nil, nil
				}
			}
			if time.Now().After(end) {
				return nil, broken("the journal of %s has no %s", id, asString(step["eventType"]))
			}
			time.Sleep(25 * time.Millisecond)
		}
	case "awaitWorkerLog":
		entry, ok := hosts[asString(step["worker"])]
		if !ok {
			return nil, broken("no worker %s", asString(step["worker"]))
		}
		timeout := 20 * time.Second
		if v, ok := step["timeoutMs"]; ok {
			timeout = asMillis(v)
		}
		count := asInt(step["count"])
		end := time.Now().Add(timeout)
		for entry.log.len() < count && time.Now().Before(end) {
			time.Sleep(20 * time.Millisecond)
		}
		if entry.log.len() < count {
			return nil, broken("the worker log has %d entries, not %d", entry.log.len(), count)
		}
		return nil, nil
	default:
		return nil, broken("unknown step op %q", op)
	}
}

// startWorkerStep starts a worker; a refusal of the platform is an observation, a mistake of the
// runner is not.
func startWorkerStep(start func(string) (object, error), key string) (object, error) {
	result, err := start(key)
	if err != nil {
		var se *sagat.Error
		if errors.As(err, &se) {
			return nil, err
		}
		return nil, broken("%v", err)
	}
	return result, nil
}

func startInput(step object) any {
	if input, ok := step["input"]; ok {
		return raw(input)
	}
	return json.RawMessage("{}")
}

func payloadOf(value any) any {
	if value == nil {
		return json.RawMessage("{}")
	}
	return raw(value)
}

func rawOrNull(m json.RawMessage) any {
	if len(m) == 0 {
		return nil
	}
	return m
}

func control(ctx context.Context, controlURL string, body object) error {
	payload, err := encode(body)
	if err != nil {
		return broken("%v", err)
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, controlURL+"/control", bytes.NewReader(payload))
	if err != nil {
		return broken("%v", err)
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := (&http.Client{Timeout: 30 * time.Second}).Do(req)
	if err != nil {
		return broken("control %s: %v", asString(body["action"]), err)
	}
	defer resp.Body.Close()
	_, _ = io.Copy(io.Discard, resp.Body)
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		return broken("control %s answered %d", asString(body["action"]), resp.StatusCode)
	}
	return nil
}

// probeWorker registers a throw-away host with an offered protocol version.
func probeWorker(ctx context.Context, config object, unique, version string) (object, error) {
	host, err := worker.New(asString(config["grpcUrl"]), asString(config["namespace"]), "probe-"+unique, sagat.StaticToken(asString(config["workerToken"])),
		worker.WithProtocolVersion(version), worker.WithRetry(sagat.RetryPolicy{MaxAttempts: 1}))
	if err != nil {
		return nil, broken("%v", err)
	}
	if err := host.RegisterActivity("conformance.probe", func(context.Context, json.RawMessage) (any, error) { return nil, nil }); err != nil {
		return nil, broken("%v", err)
	}
	if err := host.Start(ctx); err != nil {
		return nil, err
	}
	accepted := host.ProtocolVersion()
	stopCtx, cancel := context.WithTimeout(ctx, 500*time.Millisecond)
	defer cancel()
	_ = host.Stop(stopCtx)
	return object{"accepted": accepted}, nil
}
