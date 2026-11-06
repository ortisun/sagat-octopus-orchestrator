package main

// The steps of feature 018: schedules, business identifiers, the with-start calls, batch jobs and the reset of
// a run, and the references a step may make to what an earlier step produced. The JSON shapes are the ones of
// the TypeScript runner, member by member.

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
	"sort"
	"time"

	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/client"
)

var (
	workflowRef = regexp.MustCompile(`^\{\{workflow:([^}]+)\}\}$`)
	stepRef     = regexp.MustCompile(`^\{\{step:([^:}]+):([^}]+)\}\}$`)
	nowRef      = regexp.MustCompile(`^\{\{now:(-?\d+)\}\}$`)
	hourRef     = regexp.MustCompile(`^\{\{hour:(-?\d+)\}\}$`)
)

// resolveRefs replaces, in every string of a step that is exactly one reference, the reference by its value: the
// namespace id (`{{namespaceId}}`), a workflow id (`{{workflow:<key>}}`), a value an earlier step produced
// (`{{step:<id>:<field>}}`), and instants computed when the step runs (`{{now:<offsetMs>}}`, and
// `{{hour:<offsetHours>}}` for the start of a UTC hour).
func resolveRefs(value any, config object, results map[string]object, now time.Time) (any, error) {
	switch v := value.(type) {
	case string:
		if v == "{{namespaceId}}" {
			return config["namespaceId"], nil
		}
		if m := workflowRef.FindStringSubmatch(v); m != nil {
			workflow, ok := asObject(config["workflows"])[m[1]].(object)
			if !ok {
				return nil, fmt.Errorf("no workflow %s", m[1])
			}
			return workflow["workflowId"], nil
		}
		if m := stepRef.FindStringSubmatch(v); m != nil {
			found, ok := results[m[1]][m[2]]
			if !ok {
				return nil, fmt.Errorf("step %s has no %s", m[1], m[2])
			}
			return found, nil
		}
		if m := nowRef.FindStringSubmatch(v); m != nil {
			var offset int64
			if _, err := fmt.Sscan(m[1], &offset); err != nil {
				return nil, err
			}
			return now.Add(time.Duration(offset) * time.Millisecond).UTC().Format("2006-01-02T15:04:05.000Z"), nil
		}
		if m := hourRef.FindStringSubmatch(v); m != nil {
			var offset int64
			if _, err := fmt.Sscan(m[1], &offset); err != nil {
				return nil, err
			}
			return now.UTC().Truncate(time.Hour).Add(time.Duration(offset) * time.Hour).Format("2006-01-02T15:04:05.000Z"), nil
		}
		return v, nil
	case []any:
		out := make([]any, len(v))
		for i, item := range v {
			resolved, err := resolveRefs(item, config, results, now)
			if err != nil {
				return nil, err
			}
			out[i] = resolved
		}
		return out, nil
	case object:
		out := make(object, len(v))
		for key, item := range v {
			resolved, err := resolveRefs(item, config, results, now)
			if err != nil {
				return nil, err
			}
			out[key] = resolved
		}
		return out, nil
	default:
		return value, nil
	}
}

// into reads a part of a step into the type the SDK takes. Numbers keep their text until the SDK writes them.
func into(value any, target any) error {
	data, err := encode(value)
	if err != nil {
		return err
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	return decoder.Decode(target)
}

// presentInt64 reads a whole number of a step, or zero when it is absent.
func presentInt64(value any) int64 {
	n, ok := value.(interface{ Int64() (int64, error) })
	if !ok {
		return 0
	}
	i, _ := n.Int64()
	return i
}

// delayOf is a start delay the scenario gave in milliseconds, as the SDK's duration type. A number with a
// fraction keeps it: the SDK is the one that turns the duration into whole milliseconds, once.
func delayOf(value any) (time.Duration, error) {
	n, ok := value.(json.Number)
	if !ok {
		return 0, broken("a start delay is a number of milliseconds, got %v", value)
	}
	if ms, err := n.Int64(); err == nil {
		return time.Duration(ms) * time.Millisecond, nil
	}
	ms, err := n.Float64()
	if err != nil {
		return 0, broken("a start delay is a number of milliseconds, got %v", value)
	}
	return time.Duration(ms * float64(time.Millisecond)), nil
}

func instantOfText(value any) (time.Time, error) {
	text, ok := value.(string)
	if !ok {
		return time.Time{}, broken("an instant was expected, got %v", value)
	}
	t, err := time.Parse(time.RFC3339Nano, text)
	if err != nil {
		return time.Time{}, broken("%q is not an instant", text)
	}
	return t, nil
}

func epoch(t time.Time) any {
	if t.IsZero() {
		return nil
	}
	return t.UnixMilli()
}

func nullable[T any](p *T) any {
	if p == nil {
		return nil
	}
	return *p
}

func stringsJSON(values []string) []any {
	out := make([]any, len(values))
	for i, v := range values {
		out[i] = v
	}
	return out
}

func rawJSON(values []json.RawMessage) []any {
	out := make([]any, len(values))
	for i, v := range values {
		out[i] = v
	}
	return out
}

func countsJSON(counts map[string]int64) object {
	out := object{}
	for name, n := range counts {
		out[name] = n
	}
	return out
}

func cursorJSON(next string) any {
	if next == "" {
		return nil
	}
	return "<cursor>"
}

func scheduleJSON(s *client.Schedule) object {
	return object{
		"scheduleId": s.ScheduleID, "name": s.Name, "origin": s.Origin, "state": s.State, "revision": s.Revision, "exhausted": s.Exhausted,
		"action": object{
			"workflowId": orNull(s.Action.WorkflowID), "versionPolicy": orNull(s.Action.VersionPolicy), "versionId": orNull(s.Action.VersionID),
			"inputDigest": orNull(s.Action.InputDigest), "nominalTimeField": orNull(s.Action.NominalTimeField), "runTimeoutMs": nullable(s.Action.RunTimeoutMs),
		},
		"note": orNull(s.Note), "noteBy": orNull(s.NoteBy), "remainingActions": nullable(s.RemainingActions), "nextActionEpochMs": epoch(s.NextActionAt),
		"createdAt": iso(s.CreatedAt), "updatedAt": iso(s.UpdatedAt),
	}
}

func actionJSON(a *client.ScheduleActionRecord) object {
	return object{
		"kind": a.Kind, "origin": orNull(a.Origin), "revision": a.Revision, "outcome": a.Outcome, "reason": orNull(a.Reason), "runId": orNull(a.RunID),
		"backfillId": orNull(a.BackfillID), "nominalTime": iso(a.NominalTime), "actualTime": iso(a.ActualTime), "summaryCount": nullable(a.SummaryCount),
		"summaryLastNominalTime": iso(a.SummaryLastNominalTime), "decidedAt": iso(a.DecidedAt),
	}
}

func descriptionJSON(d *client.ScheduleDescription) object {
	out := scheduleJSON(&d.Schedule)
	next := make([]any, len(d.NextActions))
	for i, n := range d.NextActions {
		next[i] = object{"nominalTimeEpochMs": n.NominalTime.UnixMilli(), "actualTimeEpochMs": n.ActualTime.UnixMilli()}
	}
	recent := make([]any, len(d.RecentActions))
	for i := range d.RecentActions {
		recent[i] = actionJSON(&d.RecentActions[i])
	}
	out["nextActions"], out["recentActions"] = next, recent
	out["openRuns"], out["bufferedCount"], out["counters"] = stringsJSON(d.OpenRuns), d.BufferedCount, countsJSON(d.Counters)
	return out
}

func backfillJSON(b *client.ScheduleBackfill) object {
	return object{
		"backfillId": b.BackfillID, "start": iso(b.Start), "end": iso(b.End), "cursor": iso(b.Cursor), "state": b.State, "overlap": b.Overlap,
		"counts": countsJSON(b.Counts),
	}
}

func batchCountsJSON(c client.BatchCounts) object {
	return object{
		"selected": c.Selected, "succeeded": c.Succeeded, "skipped": c.Skipped, "failed": c.Failed, "cancelled": c.Cancelled, "deferred": c.Deferred,
	}
}

func jobJSON(j *client.BatchJob) object {
	return object{
		"batchId": j.BatchID, "action": j.Action, "state": j.State, "failureReason": orNull(j.FailureReason), "createdBy": j.CreatedBy,
		"hasPayload": j.HasPayload, "maxItems": nullable(j.MaxItems), "itemsPerSecond": nullable(j.ItemsPerSecond), "counts": batchCountsJSON(j.Counts),
		"createdAt": iso(j.CreatedAt), "startedAt": iso(j.StartedAt), "finishedAt": iso(j.FinishedAt), "expiresAt": iso(j.ExpiresAt),
	}
}

func resetJSON(r *client.RunReset) object {
	return object{
		"resetId": r.ResetID, "sourceRunId": r.SourceRunID, "sequence": r.Sequence, "state": r.State, "failureReason": orNull(r.FailureReason),
		"newRunId": orNull(r.NewRunID), "reapplyReport": rawJSON(r.ReapplyReport), "createdAt": iso(r.CreatedAt),
	}
}

func receiptJSON(r *client.UpdateReceipt) any {
	if r == nil {
		return nil
	}
	return object{
		"operation": r.Operation, "updateKey": r.UpdateKey, "status": r.Status, "result": r.Result, "beforeStateRevision": r.BeforeStateRevision,
		"afterStateRevision": r.AfterStateRevision, "runVersion": r.RunVersion, "replayed": r.Replayed,
	}
}

// operations is what the steps of feature 018 need of the runner.
type operations struct {
	client      *client.Client
	config      object
	results     map[string]object
	runOf       func(object) (string, error)
	stepOfRun   func(string) string
	workflowFor func(string) string
}

func (o *operations) namespaceID() string { return asString(o.config["namespaceId"]) }

// resultField is a text an earlier step produced. A step that failed has none, and that is this step's failure.
func (o *operations) resultField(id, field string) (string, error) {
	found, ok := o.results[id][field].(string)
	if !ok {
		return "", fmt.Errorf("step %s has no %s", id, field)
	}
	return found, nil
}

func (o *operations) schedule(step object) (string, error) {
	return o.resultField(asString(step["schedule"]), "scheduleId")
}

// play runs one step of feature 018. handled is false for an op that is not one of them.
func (o *operations) play(ctx context.Context, step object, op string, callOpts []client.CallOption) (result object, handled bool, err error) {
	c, ns := o.client, o.namespaceID()
	handled = true
	switch op {
	case "scheduleCreate", "scheduleUpdate":
		var write client.ScheduleWrite
		if err := into(step["write"], &write); err != nil {
			return nil, true, broken("write: %v", err)
		}
		var saved *client.Schedule
		if op == "scheduleCreate" {
			saved, err = c.Schedules.Create(ctx, ns, write, callOpts...)
		} else {
			var id string
			if id, err = o.schedule(step); err != nil {
				return nil, true, err
			}
			saved, err = c.Schedules.Update(ctx, ns, id, write, presentInt64(step["expectedRevision"]), callOpts...)
		}
		if err != nil {
			return nil, true, err
		}
		return scheduleJSON(saved), true, nil
	case "scheduleGet":
		id, err := o.schedule(step)
		if err != nil {
			return nil, true, err
		}
		opts := callOpts
		if v, ok := step["nextCount"]; ok {
			opts = append(opts, client.WithNextCount(int(presentInt64(v))))
		}
		if v, ok := step["recentCount"]; ok {
			opts = append(opts, client.WithRecentCount(int(presentInt64(v))))
		}
		description, err := c.Schedules.Get(ctx, ns, id, opts...)
		if err != nil {
			return nil, true, err
		}
		return descriptionJSON(description), true, nil
	case "scheduleList":
		given := asObject(step["filter"])
		filter := client.ScheduleListFilter{
			State: asString(given["state"]), Origin: asString(given["origin"]), Name: asString(given["name"]),
			WorkflowID: asString(given["workflowId"]), Limit: asCount(step["limit"]),
		}
		items := []any{}
		read := func(s *client.Schedule) {
			items = append(items, object{"name": s.Name, "state": s.State, "origin": s.Origin, "revision": s.Revision})
		}
		next := ""
		if all, _ := step["all"].(bool); all {
			for s, err := range c.Schedules.ListAll(ctx, ns, filter, callOpts...) {
				if err != nil {
					return nil, true, err
				}
				read(s)
			}
		} else {
			page, err := c.Schedules.List(ctx, ns, filter, callOpts...)
			if err != nil {
				return nil, true, err
			}
			for i := range page.Items {
				read(&page.Items[i])
			}
			next = page.NextCursor
		}
		return object{"items": items, "nextCursor": cursorJSON(next)}, true, nil
	case "scheduleDelete":
		id, err := o.schedule(step)
		if err != nil {
			return nil, true, err
		}
		if err := c.Schedules.Delete(ctx, ns, id, presentInt64(step["expectedRevision"]), callOpts...); err != nil {
			return nil, true, err
		}
		return object{"deleted": true}, true, nil
	case "schedulePause", "scheduleResume":
		id, err := o.schedule(step)
		if err != nil {
			return nil, true, err
		}
		var saved *client.Schedule
		if op == "schedulePause" {
			saved, err = c.Schedules.Pause(ctx, ns, id, asString(step["note"]), callOpts...)
		} else {
			saved, err = c.Schedules.Resume(ctx, ns, id, asString(step["note"]), callOpts...)
		}
		if err != nil {
			return nil, true, err
		}
		return scheduleJSON(saved), true, nil
	case "scheduleTrigger":
		id, err := o.schedule(step)
		if err != nil {
			return nil, true, err
		}
		opts := callOpts
		if v := asString(step["overlap"]); v != "" {
			opts = append(opts, client.WithOverlap(client.OverlapPolicy(v)))
		}
		record, err := c.Schedules.Trigger(ctx, ns, id, opts...)
		if err != nil {
			return nil, true, err
		}
		return actionJSON(record), true, nil
	case "scheduleBackfill":
		id, err := o.schedule(step)
		if err != nil {
			return nil, true, err
		}
		start, err := instantOfText(step["start"])
		if err != nil {
			return nil, true, err
		}
		end, err := instantOfText(step["end"])
		if err != nil {
			return nil, true, err
		}
		opts := callOpts
		if v := asString(step["overlap"]); v != "" {
			opts = append(opts, client.WithOverlap(client.OverlapPolicy(v)))
		}
		backfill, err := c.Schedules.Backfill(ctx, ns, id, start, end, opts...)
		if err != nil {
			return nil, true, err
		}
		return backfillJSON(backfill), true, nil
	case "scheduleCancelBackfill":
		id, err := o.schedule(step)
		if err != nil {
			return nil, true, err
		}
		backfillID, err := o.resultField(asString(step["backfill"]), "backfillId")
		if err != nil {
			return nil, true, err
		}
		backfill, err := c.Schedules.CancelBackfill(ctx, ns, id, backfillID, callOpts...)
		if err != nil {
			return nil, true, err
		}
		return backfillJSON(backfill), true, nil
	case "scheduleBackfills":
		id, err := o.schedule(step)
		if err != nil {
			return nil, true, err
		}
		backfills, err := c.Schedules.ListBackfills(ctx, ns, id, callOpts...)
		if err != nil {
			return nil, true, err
		}
		items := make([]any, len(backfills))
		for i := range backfills {
			items[i] = backfillJSON(&backfills[i])
		}
		return object{"items": items}, true, nil
	case "scheduleActions":
		id, err := o.schedule(step)
		if err != nil {
			return nil, true, err
		}
		page, err := c.Schedules.ListActions(ctx, ns, id, client.ScheduleActionFilter{Outcome: asString(step["outcome"]), Limit: asCount(step["limit"])}, callOpts...)
		if err != nil {
			return nil, true, err
		}
		items := make([]any, len(page.Items))
		for i := range page.Items {
			items[i] = actionJSON(&page.Items[i])
		}
		return object{"items": items, "nextCursor": cursorJSON(page.NextCursor)}, true, nil
	case "awaitSchedule":
		// The scheduler decides the times of a backfill: wait for its counters, never for a delay.
		id, err := o.schedule(step)
		if err != nil {
			return nil, true, err
		}
		counter, atLeast := asString(step["counter"]), int64(1)
		if v, ok := step["atLeast"]; ok {
			atLeast = presentInt64(v)
		}
		return nil, true, awaiting(ctx, "schedule counter "+counter, timeoutOf(step), func() (bool, error) {
			description, err := c.Schedules.Get(ctx, ns, id, client.WithNextCount(0), client.WithRecentCount(0))
			return err == nil && description.Counters[counter] >= atLeast, err
		})
	case "awaitBackfill":
		// A backfill counts the times it found already acted on in its own counts, not in the schedule's counters.
		id, err := o.schedule(step)
		if err != nil {
			return nil, true, err
		}
		backfillID, err := o.resultField(asString(step["backfill"]), "backfillId")
		if err != nil {
			return nil, true, err
		}
		state := asString(step["state"])
		return nil, true, awaiting(ctx, "backfill state "+state, timeoutOf(step), func() (bool, error) {
			backfills, err := c.Schedules.ListBackfills(ctx, ns, id)
			if err != nil {
				return false, err
			}
			for _, b := range backfills {
				if b.BackfillID == backfillID && b.State == state {
					return true, nil
				}
			}
			return false, nil
		})
	case "businessIdGet":
		binding, err := c.BusinessIDs.Get(ctx, ns, asString(step["businessId"]), callOpts...)
		if err != nil {
			return nil, true, err
		}
		return object{
			"businessId": binding.BusinessID, "executionId": binding.ExecutionID, "currentRunId": binding.CurrentRunID, "open": binding.Open,
			"lastStatus": orNull(binding.LastStatus), "workflowId": binding.WorkflowID, "boundAt": iso(binding.BoundAt), "reusableAfter": iso(binding.ReusableAfter),
		}, true, nil
	case "signalWithStart":
		workflowID := asString(step["workflowId"])
		if key := asString(step["workflow"]); key != "" {
			workflowID = o.workflowFor(key)
		}
		opts := callOpts
		if v := asString(step["businessIdReusePolicy"]); v != "" {
			opts = append(opts, client.WithBusinessIDReusePolicy(client.BusinessIDReusePolicy(v)))
		}
		signal := asObject(step["signal"])
		started, err := c.SignalWithStart(ctx, workflowID, asString(step["businessId"]), startInput(step), asString(signal["name"]), payloadOf(signal["payload"]), opts...)
		if err != nil {
			return nil, true, err
		}
		return object{"runId": started.RunID, "started": started.Started, "deliveredTo": stringsJSON(started.DeliveredTo)}, true, nil
	case "updateWithStart":
		update := asObject(step["update"])
		request := client.UpdateRequest{UpdateKey: asString(update["updateKey"]), Args: payloadOrNull(update["args"])}
		if v, ok := update["expectedStateRevision"]; ok {
			revision := presentInt64(v)
			request.ExpectedStateRevision = &revision
		}
		opts := callOpts
		if v := asString(step["businessIdConflictPolicy"]); v != "" {
			opts = append(opts, client.WithBusinessIDConflictPolicy(client.BusinessIDConflictPolicy(v)))
		}
		if v := asString(step["businessIdReusePolicy"]); v != "" {
			opts = append(opts, client.WithBusinessIDReusePolicy(client.BusinessIDReusePolicy(v)))
		}
		if v, ok := step["startDelayMs"]; ok {
			delay, err := delayOf(v)
			if err != nil {
				return nil, true, err
			}
			opts = append(opts, client.WithStartDelay(delay))
		}
		answered, err := c.UpdateWithStart(ctx, o.workflowFor(asString(step["workflow"])), asString(step["businessId"]), startInput(step), asString(update["name"]), request, opts...)
		if err != nil {
			return nil, true, err
		}
		return object{"runId": answered.RunID, "started": answered.Started, "update": object{
			"outcome": answered.Update.Outcome, "reason": orNull(answered.Update.Reason), "receipt": receiptJSON(answered.Update.Receipt),
		}}, true, nil
	case "batchPreview":
		var selector client.BatchSelector
		if err := into(step["selector"], &selector); err != nil {
			return nil, true, broken("selector: %v", err)
		}
		preview, err := c.Batch.Preview(ctx, ns, selector, presentInt64(step["maxItems"]), callOpts...)
		if err != nil {
			return nil, true, err
		}
		// The sample is a set of runs in the order the server chose; the scenario compares which of its own runs it holds.
		steps := make([]string, len(preview.Sample))
		for i, runID := range preview.Sample {
			steps[i] = o.stepOfRun(runID)
		}
		sort.Strings(steps)
		return object{"matched": preview.Matched, "capped": preview.Capped, "sampleSize": len(preview.Sample), "sampleSteps": stringsJSON(steps)}, true, nil
	case "batchCreate":
		var request client.BatchRequest
		if err := into(step["request"], &request); err != nil {
			return nil, true, broken("request: %v", err)
		}
		// What does not change while the job runs (its state and counters do): the job as it was accepted.
		job, err := c.Batch.Create(ctx, ns, request, callOpts...)
		if err != nil {
			return nil, true, err
		}
		return object{
			"batchId": job.BatchID, "action": job.Action, "createdBy": job.CreatedBy, "hasPayload": job.HasPayload,
			"maxItems": nullable(job.MaxItems), "itemsPerSecond": nullable(job.ItemsPerSecond),
		}, true, nil
	case "batchGet", "batchCancel":
		id, err := o.resultField(asString(step["batch"]), "batchId")
		if err != nil {
			return nil, true, err
		}
		var job *client.BatchJob
		if op == "batchGet" {
			job, err = c.Batch.Get(ctx, ns, id, callOpts...)
		} else {
			job, err = c.Batch.Cancel(ctx, ns, id, asString(step["reason"]), callOpts...)
		}
		if err != nil {
			return nil, true, err
		}
		return jobJSON(job), true, nil
	case "batchList":
		page, err := c.Batch.List(ctx, ns, client.BatchListOptions{State: asString(step["state"]), Limit: asCount(step["limit"])}, callOpts...)
		if err != nil {
			return nil, true, err
		}
		items := make([]any, len(page.Items))
		for i := range page.Items {
			items[i] = object{"action": page.Items[i].Action, "state": page.Items[i].State, "counts": batchCountsJSON(page.Items[i].Counts)}
		}
		return object{"items": items, "nextCursor": cursorJSON(page.NextCursor)}, true, nil
	case "batchItems":
		id, err := o.resultField(asString(step["batch"]), "batchId")
		if err != nil {
			return nil, true, err
		}
		page, err := c.Batch.ListItems(ctx, ns, id, client.BatchListOptions{State: asString(step["state"]), Limit: asCount(step["limit"])}, callOpts...)
		if err != nil {
			return nil, true, err
		}
		// The order of the items is the server's; which of the scenario's runs sits at which ordinal is not part of the
		// contract, so the items are named by the step that started the run and sorted by it.
		type named struct {
			step string
			item object
		}
		items := make([]named, len(page.Items))
		for i, item := range page.Items {
			runStep := o.stepOfRun(item.RunID)
			items[i] = named{runStep, object{
				"runStep": runStep, "state": item.State, "reason": orNull(item.Reason), "errorCode": orNull(item.ErrorCode),
				"attempts": item.Attempts, "newRunId": orNull(item.NewRunID),
			}}
		}
		sort.SliceStable(items, func(a, b int) bool { return items[a].step < items[b].step })
		sorted := make([]any, len(items))
		for i, item := range items {
			sorted[i] = item.item
		}
		return object{"items": sorted, "nextCursor": cursorJSON(page.NextCursor)}, true, nil
	case "awaitBatch":
		id, err := o.resultField(asString(step["batch"]), "batchId")
		if err != nil {
			return nil, true, err
		}
		state := asString(step["state"])
		return nil, true, awaiting(ctx, "batch state "+state, timeoutOf(step), func() (bool, error) {
			job, err := c.Batch.Get(ctx, ns, id)
			return err == nil && job.State == state, err
		})
	case "resetPoints":
		id, err := o.runOf(step)
		if err != nil {
			return nil, true, broken("%v", err)
		}
		page, err := c.ResetPoints(ctx, id, client.ResetPointsOptions{Limit: asCount(step["limit"])}, callOpts...)
		if err != nil {
			return nil, true, err
		}
		items := make([]any, len(page.Items))
		for i, p := range page.Items {
			items[i] = object{"sequence": p.Sequence, "kind": p.Kind, "node": orNull(p.Node), "occurredAt": iso(p.OccurredAt)}
		}
		return object{"items": items, "nextCursor": cursorJSON(page.NextCursor)}, true, nil
	case "resetPreview":
		id, err := o.runOf(step)
		if err != nil {
			return nil, true, broken("%v", err)
		}
		plan, err := c.PreviewReset(ctx, id, presentInt64(step["sequence"]), client.ResetReapply(asString(step["reapply"])), callOpts...)
		if err != nil {
			return nil, true, err
		}
		restored := make([]any, len(plan.Restored))
		for i, r := range plan.Restored {
			restored[i] = object{"node": r.Node, "kind": r.Kind}
		}
		reexecuted := make([]any, len(plan.Reexecuted))
		for i, r := range plan.Reexecuted {
			reexecuted[i] = object{"node": r.Node, "completedInSource": r.CompletedInSource, "markers": stringsJSON(r.Markers)}
		}
		return object{
			"sequence": plan.Sequence, "restored": restored, "reexecuted": reexecuted, "reapply": rawJSON(plan.Reapply),
			"requiresAcknowledgement": plan.RequiresAcknowledgement, "prefixEvents": plan.PrefixEvents,
		}, true, nil
	case "reset":
		id, err := o.runOf(step)
		if err != nil {
			return nil, true, broken("%v", err)
		}
		acknowledged, _ := step["acknowledgeReexecution"].(bool)
		made, err := c.Reset(ctx, id, client.ResetRequest{
			Sequence: presentInt64(step["sequence"]), Reason: asString(step["reason"]), Reapply: client.ResetReapply(asString(step["reapply"])),
			SourcePolicy: asString(step["sourcePolicy"]), BusinessID: asString(step["businessId"]), AcknowledgeReexecution: acknowledged,
		}, callOpts...)
		if err != nil {
			return nil, true, err
		}
		return resetJSON(made), true, nil
	case "resets":
		id, err := o.runOf(step)
		if err != nil {
			return nil, true, broken("%v", err)
		}
		resets, err := c.ListResets(ctx, id, callOpts...)
		if err != nil {
			return nil, true, err
		}
		items := make([]any, len(resets))
		for i := range resets {
			items[i] = resetJSON(&resets[i])
		}
		return object{"items": items}, true, nil
	case "getReset":
		id, err := o.runOf(step)
		if err != nil {
			return nil, true, broken("%v", err)
		}
		resetID, err := o.resultField(asString(step["reset"]), "resetId")
		if err != nil {
			return nil, true, err
		}
		got, err := c.GetReset(ctx, id, resetID, callOpts...)
		if err != nil {
			return nil, true, err
		}
		return resetJSON(got), true, nil
	}
	return nil, false, nil
}

func timeoutOf(step object) time.Duration {
	if v, ok := step["timeoutMs"]; ok {
		return asMillis(v)
	}
	return 30 * time.Second
}

// awaiting asks done every 50 ms until it is true; a wait that ends without it is a failure of this step.
func awaiting(ctx context.Context, what string, timeout time.Duration, done func() (bool, error)) error {
	end := time.Now().Add(timeout)
	for {
		ok, err := done()
		if err != nil {
			return err
		}
		if ok {
			return nil
		}
		if time.Now().After(end) {
			return errors.New("timed out waiting for " + what)
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(50 * time.Millisecond):
		}
	}
}

// payloadOrNull is the arguments of an Update as the scenario wrote them; absent is JSON null.
func payloadOrNull(value any) any {
	if value == nil {
		return json.RawMessage("null")
	}
	return raw(value)
}
