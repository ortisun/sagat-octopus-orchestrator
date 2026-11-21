package worker_test

import (
	"context"
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"

	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
)

// The hand-back of sdks/SPEC.md section 6.2: a lease the host received and will not run is reported
// with FailStep TASK_NOT_STARTED, before any handler of it runs, once, and never for a task whose
// handler started.

func numbered(n int) *pb.TaskLease {
	return lease(func(l *pb.TaskLease) {
		l.TaskId, l.RunId, l.LeaseToken = fmt.Sprintf("run-%d/charge/1", n), fmt.Sprintf("run-%d", n), fmt.Sprintf("LEASE-TOKEN-%d", n)
	})
}

// handedBack is the task ids of the FailStep TASK_NOT_STARTED calls, sorted, after checking that each is
// the whole hand-back: retryable, the fixed message, the lease's fence and the failure's idempotency key.
func handedBack(t *testing.T, f *fakeService) []string {
	t.Helper()
	var ids []string
	for _, c := range f.of("FailStep") {
		r := c.request.(*pb.FailStepRequest)
		if r.GetErrorCode() != "TASK_NOT_STARTED" {
			continue
		}
		if !r.GetRetryable() || r.GetSafeMessage() != "the host did not start this task" ||
			r.GetIdempotencyKey() != "failed-"+r.GetTaskId() || r.GetGeneration() != 1 || r.GetAttempt() != 1 ||
			r.GetLeaseToken() != "LEASE-TOKEN-"+strings.TrimPrefix(r.GetRunId(), "run-") || r.GetStepId() != "charge" {
			t.Errorf("a hand-back that is not the one of the specification: %v", r)
		}
		ids = append(ids, r.GetTaskId())
	}
	sort.Strings(ids)
	return ids
}

func beatsOf(f *fakeService, task string) int {
	n := 0
	for _, c := range f.of("Heartbeat") {
		if c.request.(*pb.HeartbeatRequest).GetTaskId() == task {
			n++
		}
	}
	return n
}

func stop(t *testing.T, h *worker.Host) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	must(t, h.Stop(ctx))
}

func TestAPollAnswerBeyondTheCreditIsHandedBack(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) { f.overfill = true })
	for i := 1; i <= 3; i++ {
		f.lease(numbered(i))
	}
	ran := make(chan string, 3)
	h := newHost(t, f, worker.WithMaxInflight(1))
	must(t, h.RegisterActivity("charge", func(ctx context.Context, _ json.RawMessage) (any, error) {
		ran <- worker.TaskFrom(ctx).TaskID
		return 1, nil
	}))
	start(t, h)
	until(t, "the kept task completes", func() bool { return len(f.of("CompleteStep")) == 1 })
	stop(t, h)
	if got := handedBack(t, f); fmt.Sprint(got) != "[run-2/charge/1 run-3/charge/1]" {
		t.Fatalf("handed back %v, want the two leases beyond the credit", got)
	}
	if first := <-ran; first != "run-1/charge/1" || len(ran) != 0 {
		t.Fatalf("ran %s and %d more: only the first lease may run", first, len(ran))
	}
	if beatsOf(f, "run-2/charge/1")+beatsOf(f, "run-3/charge/1") != 0 {
		t.Fatal("a lease that is handed back is never beaten for")
	}
}

func TestAStreamFrameBeyondTheCreditIsHandedBackAndTheStartedTaskIsNot(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.4"
	f.stream = func(_ *pb.PollTasksRequest, s grpc.ServerStreamingServer[pb.PollTasksResponse]) error {
		if len(f.of("StreamTasks")) > 1 {
			<-s.Context().Done()
			return s.Context().Err()
		}
		return s.Send(&pb.PollTasksResponse{Tasks: []*pb.TaskLease{numbered(1), numbered(2), numbered(3)}})
	}
	release := make(chan struct{})
	h := newHost(t, f, worker.WithMaxInflight(1))
	must(t, h.RegisterActivity("charge", func(context.Context, json.RawMessage) (any, error) { <-release; return 1, nil }))
	start(t, h)
	until(t, "the two hand-backs", func() bool { return len(f.of("FailStep")) == 2 })
	close(release)
	until(t, "the kept task completes", func() bool { return len(f.of("CompleteStep")) == 1 })
	stop(t, h)
	if got := handedBack(t, f); fmt.Sprint(got) != "[run-2/charge/1 run-3/charge/1]" {
		t.Fatalf("handed back %v", got)
	}
	if len(f.of("FailStep")) != 2 {
		t.Fatalf("%d failures: the started task is never handed back", len(f.of("FailStep")))
	}
}

func TestTheRestOfAFrameOverTheInputBudgetIsHandedBack(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.4"
	big := strings.Repeat("x", 5<<20)
	f.stream = func(_ *pb.PollTasksRequest, s grpc.ServerStreamingServer[pb.PollTasksResponse]) error {
		if len(f.of("StreamTasks")) > 1 {
			<-s.Context().Done()
			return s.Context().Err()
		}
		heavy := func(n int) *pb.TaskLease {
			l := numbered(n)
			l.InputJson = `{"run":{"id":"run-1"},"node":"charge","input":"` + big + `"}`
			return l
		}
		return s.Send(&pb.PollTasksResponse{Tasks: []*pb.TaskLease{heavy(1), heavy(2), numbered(3)}})
	}
	h := newHost(t, f, worker.WithMaxInflight(3))
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	until(t, "the kept task completes", func() bool { return len(f.of("CompleteStep")) == 1 })
	until(t, "the hand-backs", func() bool { return len(f.of("FailStep")) == 2 })
	stop(t, h)
	if got := handedBack(t, f); fmt.Sprint(got) != "[run-2/charge/1 run-3/charge/1]" {
		t.Fatalf("handed back %v: the lease over the budget and the rest of the frame", got)
	}
}

func TestAStreamFrameReadWhileStoppingIsHandedBackWhole(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.4"
	f.stream = func(_ *pb.PollTasksRequest, s grpc.ServerStreamingServer[pb.PollTasksResponse]) error {
		if len(f.of("StreamTasks")) > 1 {
			<-s.Context().Done()
			return s.Context().Err()
		}
		return s.Send(&pb.PollTasksResponse{Tasks: []*pb.TaskLease{numbered(1), numbered(2)}})
	}
	h := newHost(t, f, worker.WithMaxInflight(4))
	read := make(chan struct{})
	proceed := make(chan struct{})
	var once sync.Once
	worker.SetFrameReceived(h, func(int) { once.Do(func() { close(read) }); <-proceed })
	ran := make(chan struct{}, 2)
	must(t, h.RegisterActivity("charge", func(context.Context, json.RawMessage) (any, error) { ran <- struct{}{}; return 1, nil }))
	start(t, h)
	<-read
	stopped := make(chan error, 1)
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		stopped <- h.Stop(ctx)
	}()
	until(t, "the stop begins", func() bool { return worker.Stopping(h) })
	close(proceed)
	must(t, <-stopped)
	// Stop returned: the hand-backs were made before the connection closed.
	if got := handedBack(t, f); fmt.Sprint(got) != "[run-1/charge/1 run-2/charge/1]" {
		t.Fatalf("handed back %v, want the whole frame", got)
	}
	if len(ran) != 0 {
		t.Fatal("a host that is stopping starts nothing new")
	}
}

func TestAPollAnswerThatArrivesWhileStoppingIsHandedBackAndTheRunningTaskIsNot(t *testing.T) {
	f := newFakeService(t)
	f.lease(numbered(1))
	h := newHost(t, f, worker.WithMaxInflight(4))
	started := make(chan struct{})
	release := make(chan struct{})
	must(t, h.RegisterActivity("charge", func(ctx context.Context, _ json.RawMessage) (any, error) {
		close(started)
		<-release
		return 1, nil
	}))
	// The second poll answer that carries leases is held in the loop until the stop has begun.
	hold, holding := make(chan struct{}), make(chan struct{})
	var answers atomic.Int32
	worker.SetFrameReceived(h, func(leases int) {
		if leases > 0 && answers.Add(1) == 2 {
			close(holding)
			<-hold
		}
	})
	start(t, h)
	<-started
	f.lease(numbered(2), numbered(3))
	<-holding
	stopped := make(chan error, 1)
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		stopped <- h.Stop(ctx)
	}()
	until(t, "the stop begins", func() bool { return worker.Stopping(h) })
	close(hold)
	until(t, "the hand-backs", func() bool { return len(f.of("FailStep")) == 2 })
	close(release)
	must(t, <-stopped)
	if got := handedBack(t, f); fmt.Sprint(got) != "[run-2/charge/1 run-3/charge/1]" {
		t.Fatalf("handed back %v", got)
	}
	if len(f.of("CompleteStep")) != 1 || len(f.of("FailStep")) != 2 {
		t.Fatal("the running task reports its own result and is never handed back")
	}
}

func TestADuplicateLeaseIsIgnoredNeverHandedBack(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.4"
	f.stream = func(_ *pb.PollTasksRequest, s grpc.ServerStreamingServer[pb.PollTasksResponse]) error {
		if len(f.of("StreamTasks")) > 1 {
			<-s.Context().Done()
			return s.Context().Err()
		}
		// The second is the running task's own lease again; the fourth repeats a lease handed back.
		return s.Send(&pb.PollTasksResponse{Tasks: []*pb.TaskLease{numbered(1), numbered(1), numbered(2), numbered(2)}})
	}
	release := make(chan struct{})
	h := newHost(t, f, worker.WithMaxInflight(1))
	must(t, h.RegisterActivity("charge", func(context.Context, json.RawMessage) (any, error) { <-release; return 1, nil }))
	start(t, h)
	until(t, "the hand-back", func() bool { return len(f.of("FailStep")) >= 1 })
	close(release)
	until(t, "the kept task completes", func() bool { return len(f.of("CompleteStep")) == 1 })
	stop(t, h)
	if got := handedBack(t, f); fmt.Sprint(got) != "[run-2/charge/1]" {
		t.Fatalf("handed back %v: a duplicate is never reported, a lease is handed back once", got)
	}
}

func TestSiblingIterationsThatShareATaskIdAreNotDuplicates(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.5"
	sibling := func(index uint32) *pb.TaskLease {
		l := numbered(1)
		l.Instance = &pb.ExecutionInstanceIdentity{DefinitionNodeId: "charge", InstancePath: []*pb.ScopePathSegment{{ScopeNodeId: "each", IterationIndex: index}}}
		return l
	}
	f.stream = func(_ *pb.PollTasksRequest, s grpc.ServerStreamingServer[pb.PollTasksResponse]) error {
		if len(f.of("StreamTasks")) > 1 {
			<-s.Context().Done()
			return s.Context().Err()
		}
		return s.Send(&pb.PollTasksResponse{Tasks: []*pb.TaskLease{sibling(0), sibling(1)}})
	}
	h := newHost(t, f, worker.WithMaxInflight(2))
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	until(t, "both iterations complete", func() bool { return len(f.of("CompleteStep")) == 2 })
	stop(t, h)
	if len(f.of("FailStep")) != 0 {
		t.Fatal("sibling iterations run, neither is handed back")
	}
}

func TestAHandBackIsRetriedWithTheSameKeyAndEndsQuietlyWhenStale(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) { f.overfill = true })
	f.lease(numbered(1), numbered(2), numbered(3))
	f.fail("FailStep", failure{code: codes.Unavailable}, failure{code: codes.Aborted, reason: "STALE_ATTEMPT"})
	var mu sync.Mutex
	var events []worker.Event
	done := make(chan struct{})
	h := newHost(t, f, worker.WithMaxInflight(1), worker.WithEventHandler(func(e worker.Event) {
		mu.Lock()
		defer mu.Unlock()
		if e.Type == worker.EventFailed || e.Type == worker.EventStale {
			events = append(events, e)
			if len(events) == 2 {
				close(done)
			}
		}
	}))
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	<-done
	stop(t, h)
	keys := map[string]int{}
	for _, c := range f.of("FailStep") {
		keys[c.request.(*pb.FailStepRequest).GetIdempotencyKey()]++
	}
	// Three calls for two leases: one answered UNAVAILABLE and repeated with its key, one stale.
	if len(f.of("FailStep")) != 3 || len(keys) != 2 {
		t.Fatalf("hand-back calls %d with keys %v", len(f.of("FailStep")), keys)
	}
	var failed, stale int
	mu.Lock()
	defer mu.Unlock()
	for _, e := range events {
		switch {
		case e.Type == worker.EventFailed && e.Code == "TASK_NOT_STARTED" && e.Retryable:
			failed++
		case e.Type == worker.EventStale:
			stale++
		}
	}
	if failed != 1 || stale != 1 {
		t.Fatalf("events %v: one hand-back counted as a failure with its code, one stale", events)
	}
}

func TestAFloodOfLeasesIsHandedBackEightAtATime(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) { f.overfill = true })
	const total = 1000
	for i := 1; i <= total; i++ {
		f.lease(numbered(i))
	}
	release := make(chan struct{})
	var inside, peak atomic.Int32
	f.set(func(f *fakeService) {
		f.failGate = func(context.Context) {
			now := inside.Add(1)
			for old := peak.Load(); now > old && !peak.CompareAndSwap(old, now); old = peak.Load() {
			}
			<-release
			inside.Add(-1)
		}
	})
	h := newHost(t, f, worker.WithMaxInflight(1))
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	// Eight hand-backs are held at the server; every other one waits its turn in the host.
	until(t, "eight held and the rest queued", func() bool {
		queued, _ := worker.HandBacks(h)
		return inside.Load() == 8 && queued == total-1-8
	})
	if _, workers := worker.HandBacks(h); workers != 8 || len(f.of("FailStep")) != 8 {
		t.Fatalf("%d workers and %d failures on their way: at most eight at once", workers, len(f.of("FailStep")))
	}
	close(release)
	until(t, "every hand-back", func() bool { return len(f.of("FailStep")) == total-1 })
	stop(t, h)
	if peak.Load() > 8 {
		t.Fatalf("%d hand-backs at once", peak.Load())
	}
	if got := handedBack(t, f); len(got) != total-1 {
		t.Fatalf("%d handed back", len(got))
	}
}

func TestALeaseWhoseHandBackIsOnItsWayIsADuplicate(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) { f.overfill = true })
	f.lease(numbered(1), numbered(2), numbered(3))
	release := make(chan struct{})
	f.set(func(f *fakeService) { f.failGate = func(context.Context) { <-release } })
	var mu sync.Mutex
	var ran []string
	blocked := make(chan struct{})
	h := newHost(t, f, worker.WithMaxInflight(2))
	must(t, h.RegisterActivity("charge", func(ctx context.Context, _ json.RawMessage) (any, error) {
		id := worker.TaskFrom(ctx).TaskID
		mu.Lock()
		ran = append(ran, id)
		mu.Unlock()
		if id == "run-1/charge/1" {
			<-blocked
		}
		return 1, nil
	}))
	// The answer after the one that handed lease 3 back leases it again; the loop has looked at it once
	// the answer after that one is read.
	var withLeases atomic.Int32
	var once sync.Once
	looked := make(chan struct{})
	worker.SetFrameReceived(h, func(leases int) {
		if leases > 0 {
			withLeases.Add(1)
		} else if withLeases.Load() == 2 {
			once.Do(func() { close(looked) })
		}
	})
	start(t, h)
	until(t, "the hand-back of lease 3 held at the server", func() bool { return len(f.of("FailStep")) == 1 })
	until(t, "lease 2 done", func() bool { return len(f.of("CompleteStep")) == 1 })
	f.lease(numbered(3))
	<-looked
	close(release)
	close(blocked)
	until(t, "lease 1 done", func() bool { return len(f.of("CompleteStep")) == 2 })
	stop(t, h)
	mu.Lock()
	defer mu.Unlock()
	sort.Strings(ran)
	if fmt.Sprint(ran) != "[run-1/charge/1 run-2/charge/1]" {
		t.Fatalf("ran %v: a lease whose hand-back is on its way never runs", ran)
	}
	if got := handedBack(t, f); fmt.Sprint(got) != "[run-3/charge/1]" {
		t.Fatalf("handed back %v: once", got)
	}
}

func TestASiblingIterationBeyondTheCreditIsHandedBackWithItsInstance(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.5"
	sibling := func(index uint32) *pb.TaskLease {
		l := numbered(1)
		l.Instance = &pb.ExecutionInstanceIdentity{DefinitionNodeId: "charge", InstancePath: []*pb.ScopePathSegment{{ScopeNodeId: "each", IterationIndex: index}}}
		return l
	}
	f.stream = func(_ *pb.PollTasksRequest, s grpc.ServerStreamingServer[pb.PollTasksResponse]) error {
		if len(f.of("StreamTasks")) > 1 {
			<-s.Context().Done()
			return s.Context().Err()
		}
		return s.Send(&pb.PollTasksResponse{Tasks: []*pb.TaskLease{sibling(0), sibling(1)}})
	}
	h := newHost(t, f, worker.WithMaxInflight(1))
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	until(t, "the kept iteration and the hand-back", func() bool {
		return len(f.of("CompleteStep")) == 1 && len(f.of("FailStep")) == 1
	})
	stop(t, h)
	r := f.of("FailStep")[0].request.(*pb.FailStepRequest)
	if r.GetErrorCode() != "TASK_NOT_STARTED" || r.GetInstance().GetInstancePath()[0].GetIterationIndex() != 1 {
		t.Fatalf("the hand-back of the second iteration carries its own instance: %v", r)
	}
}

func TestAHandBackStillOnItsWayWhenTheGraceEndsIsDroppedAndItsLeaseLapses(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) { f.overfill = true })
	const total = 12
	for i := 1; i <= total; i++ {
		f.lease(numbered(i))
	}
	// The server never answers a hand-back before the stop has ended: eight are held there, the rest queued.
	f.set(func(f *fakeService) {
		f.failGate = func(ctx context.Context) { <-ctx.Done() }
	})
	var mu sync.Mutex
	dropped := map[string]int{}
	h := newHost(t, f, worker.WithMaxInflight(1), worker.WithEventHandler(func(e worker.Event) {
		if e.Type == worker.EventDropped {
			mu.Lock()
			dropped[e.TaskID]++
			mu.Unlock()
		}
	}))
	must(t, h.RegisterActivity("charge", rawOK))
	start(t, h)
	until(t, "eight held and the rest queued", func() bool {
		queued, workers := worker.HandBacks(h)
		return len(f.of("FailStep")) == 8 && workers == 8 && queued == total-1-8
	})
	began := time.Now()
	ctx, cancel := context.WithTimeout(context.Background(), 200*time.Millisecond)
	defer cancel()
	if err := h.Stop(ctx); err == nil {
		t.Fatal("a stop whose grace ended says so")
	}
	if took := time.Since(began); took > 200*time.Millisecond+2*time.Second+time.Second {
		t.Fatalf("stop took %v: the grace and its two seconds bound it", took)
	}
	// Every lease that was not reported ends as dropped, once, and lapses.
	until(t, "every hand-back dropped", func() bool {
		mu.Lock()
		defer mu.Unlock()
		return len(dropped) == total-1
	})
	mu.Lock()
	defer mu.Unlock()
	for id, n := range dropped {
		if n != 1 || id == "run-1/charge/1" {
			t.Fatalf("dropped %v: each hand-back once, never the task that ran", dropped)
		}
	}
	if len(f.of("FailStep")) != 8 {
		t.Fatalf("%d hand-backs reached the server: the queued ones are not sent on a closed connection", len(f.of("FailStep")))
	}
}

func TestAFrameAfterTheCreditIsUsedUpIsReadAndHandedBack(t *testing.T) {
	f := newFakeService(t)
	f.accepted = "1.4"
	f.stream = func(_ *pb.PollTasksRequest, s grpc.ServerStreamingServer[pb.PollTasksResponse]) error {
		if len(f.of("StreamTasks")) > 1 {
			<-s.Context().Done()
			return s.Context().Err()
		}
		if err := s.Send(&pb.PollTasksResponse{Tasks: []*pb.TaskLease{numbered(1)}}); err != nil {
			return err
		}
		return s.Send(&pb.PollTasksResponse{Tasks: []*pb.TaskLease{numbered(2), numbered(3)}})
	}
	release := make(chan struct{})
	h := newHost(t, f, worker.WithMaxInflight(1))
	must(t, h.RegisterActivity("charge", func(context.Context, json.RawMessage) (any, error) { <-release; return 1, nil }))
	start(t, h)
	until(t, "the two hand-backs", func() bool { return len(f.of("FailStep")) == 2 })
	close(release)
	until(t, "the kept task completes", func() bool { return len(f.of("CompleteStep")) == 1 })
	stop(t, h)
	if got := handedBack(t, f); fmt.Sprint(got) != "[run-2/charge/1 run-3/charge/1]" {
		t.Fatalf("handed back %v: the frame after the credit is read and handed back", got)
	}
}
