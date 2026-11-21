package worker_test

import (
	"context"
	"encoding/json"
	"fmt"
	"runtime"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"google.golang.org/grpc/codes"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/worker"
)

// startDefault starts a host with the retry policy a user gets (no fast retries for the test) and stops it
// when the test ends. The pauses of its loop are recorded among the calls of the fake service.
func startDefault(t *testing.T, f *fakeService, opts ...worker.Option) *worker.Host {
	t.Helper()
	h, err := worker.New(f.url(), "payments", "pacing-worker", sagat.StaticToken("tok"), opts...)
	must(t, err)
	must(t, h.RegisterActivity("charge", rawOK))
	f.observePauses(h)
	must(t, h.Start(context.Background()))
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		_ = h.Stop(ctx)
	})
	return h
}

// paced checks that each two polls that follow each other are separated by exactly one pause of the loop, that
// the pause is within [low(k), high(k)] for the k-th pair, and that the second poll came no sooner than the pause
// asked. It reads the length the host chose from the host, and the wall clock only for a lower bound, which a
// loaded machine cannot break (AGENTS.md Rule 18).
func paced(t *testing.T, f *fakeService, polls int, low, high func(k int) time.Duration) {
	t.Helper()
	until(t, fmt.Sprintf("%d polls", polls), func() bool { return len(f.of("PollTasks")) >= polls })
	pauses, gaps := pausesBetween(f.sequence(), "PollTasks")
	for k := 0; k < polls-1; k++ {
		if len(pauses[k]) != 1 {
			t.Errorf("polls %d and %d: %d pauses between them, %v", k+1, k+2, len(pauses[k]), pauses[k])
			continue
		}
		if d := pauses[k][0]; d < low(k) || d > high(k) {
			t.Errorf("polls %d and %d: a pause of %v, outside [%v, %v]", k+1, k+2, d, low(k), high(k))
		} else if gaps[k] < d {
			t.Errorf("polls %d and %d came %v apart: the pause of %v was not waited out", k+1, k+2, gaps[k], d)
		}
	}
}

func exactly(d time.Duration) func(int) time.Duration { return func(int) time.Duration { return d } }

// The host never busy-polls (SPEC section 6.2): whatever the poll answered, the next one is after a wait. Each
// of these checks the pause between polls; a loop that did not wait would make none.
func TestAFailedPollIsFollowedByABackoffAndNotByAnotherPollAtOnce(t *testing.T) {
	for name, code := range map[string]codes.Code{"unavailable": codes.Unavailable, "internal": codes.Internal, "deadline": codes.DeadlineExceeded} {
		t.Run(name, func(t *testing.T) {
			f := newFakeService(t)
			f.set(func(f *fakeService) { f.always["PollTasks"] = failure{code: code} })
			startDefault(t, f)
			// The backoff is 100 to 200 ms, then 200 to 400, then 400 to 800.
			paced(t, f, 4,
				func(k int) time.Duration { return (100 * time.Millisecond) << k },
				func(k int) time.Duration { return (200 * time.Millisecond) << k })
		})
	}
}

func TestAPollRefusedWithARetryAfterWaitsForIt(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) {
		f.always["PollTasks"] = failure{code: codes.ResourceExhausted, reason: "RATE_LIMITED", retryAfter: "400"}
	})
	startDefault(t, f)
	paced(t, f, 3, exactly(400*time.Millisecond), exactly(400*time.Millisecond))
}

func TestAPollRefusedWithNoRetryAfterStillWaits(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) {
		f.always["PollTasks"] = failure{code: codes.ResourceExhausted, reason: "RATE_LIMITED"}
	})
	startDefault(t, f)
	paced(t, f, 4, exactly(100*time.Millisecond), exactly(100*time.Millisecond))
}

func TestAnEmptyPollWaitsForWhatItAskedAndNotLess(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) { f.pollRetryAfterMs = 500 })
	startDefault(t, f)
	// 500 ms with 20 % jitter: 400 to 600.
	paced(t, f, 3, exactly(400*time.Millisecond), exactly(600*time.Millisecond))
}

func TestAHostWithEverySlotBusyPollsNoMoreAndPauses(t *testing.T) {
	f := newFakeService(t)
	release := make(chan struct{})
	var running atomic.Bool
	h, err := worker.New(f.url(), "payments", "full-worker", sagat.StaticToken("tok"), worker.WithMaxInflight(1), worker.WithHeartbeatInterval(time.Minute))
	must(t, err)
	must(t, h.RegisterActivity("charge", func(ctx context.Context, _ json.RawMessage) (any, error) {
		running.Store(true)
		<-release
		return 1, nil
	}))
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		_ = h.Stop(ctx)
	})
	f.lease(lease())
	must(t, h.Start(context.Background()))
	until(t, "the task", func() bool { return running.Load() })
	polls := len(f.of("PollTasks"))
	cpu := cpuTime()
	time.Sleep(800 * time.Millisecond)
	if more := len(f.of("PollTasks")) - polls; more != 0 {
		t.Errorf("%d polls while every slot was busy", more)
	}
	// A loop that did not pause would spin a core for the whole window.
	if used := cpuTime() - cpu; used > 300*time.Millisecond {
		t.Errorf("%v of CPU in 800 ms with every slot busy: the loop does not pause", used)
	}
	close(release)
	until(t, "the report and the poll after it", func() bool { return len(f.of("CompleteStep")) == 1 && len(f.of("PollTasks")) > polls })
}

func TestStopWhileTheServerHoldsAPollReturnsWithinItsTimeout(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) { f.pollDelay = 30 * time.Second })
	h, err := worker.New(f.url(), "payments", "held-worker", sagat.StaticToken("tok"))
	must(t, err)
	must(t, h.RegisterActivity("charge", rawOK))
	must(t, h.Start(context.Background()))
	until(t, "the held poll", func() bool { return len(f.of("PollTasks")) == 1 })
	ctx, cancel := context.WithTimeout(context.Background(), 300*time.Millisecond)
	defer cancel()
	began := time.Now()
	err = h.Stop(ctx)
	if took := time.Since(began); took > 2*time.Second {
		t.Errorf("stop took %v with a poll held by the server and a timeout of 300 ms", took)
	}
	if err != nil {
		t.Logf("stop: %v", err)
	}
	// And the host is gone: nothing of it keeps running.
	until(t, "no goroutine of the host", func() bool { return count("(*Host).run", "(*Host).beat", "(*Host).execute") == 0 })
}

// ---- goroutines

// count is the number of goroutines whose stack mentions any of the patterns.
func count(patterns ...string) int {
	buf := make([]byte, 4<<20)
	buf = buf[:runtime.Stack(buf, true)]
	n := 0
	for _, stack := range strings.Split(string(buf), "\n\n") {
		for _, pattern := range patterns {
			if strings.Contains(stack, pattern) {
				n++
				break
			}
		}
	}
	return n
}

// clientConn counts the goroutines of gRPC *client* connections (the fake service is a server in this very
// process; its goroutines are made by NewServerTransport and are not counted).
func clientConn() int {
	return count("grpc/internal/transport.NewHTTP2Client", "grpcsync.NewCallbackSerializer", "grpc.(*ClientConn)", "grpc.(*addrConn)")
}

// settle waits until the count has not moved for a while: the goroutines of earlier tests wind down.
func settle(f func() int) int {
	last, since := f(), time.Now()
	for time.Since(since) < 400*time.Millisecond {
		time.Sleep(20 * time.Millisecond)
		if now := f(); now != last {
			last, since = now, time.Now()
		}
	}
	return last
}

func TestAStoppedHostClosesItsConnectionAndLeavesNoHeartbeatBehind(t *testing.T) {
	f := newFakeService(t)
	before := settle(clientConn)
	beforeHost := settle(func() int { return count("sdks/go/worker.(*Host)") })
	h := newHost(t, f, worker.WithHeartbeatInterval(10*time.Millisecond), f.clientSide())
	must(t, h.RegisterActivity("slow", func(ctx context.Context, _ json.RawMessage) (any, error) {
		time.Sleep(60 * time.Millisecond)
		return 1, nil
	}))
	f.lease(lease(forActivity("slow", "1")))
	start(t, h)
	until(t, "a connection", func() bool { return clientConn() > before })
	until(t, "the report", func() bool { return len(f.of("CompleteStep")) == 1 })
	// The heartbeat of a task ends with its report, not with the host.
	until(t, "no heartbeat goroutine once the task is reported", func() bool { return count("(*Host).beat") == 0 })
	// A beat on its way when the heartbeat ended may reach the service after it; the host must send none
	// afterwards. The sleep only gives a heartbeat that went on the time to show.
	f.note("heartbeat ended")
	time.Sleep(100 * time.Millisecond)
	if n := f.sentAfter("Heartbeat", "heartbeat ended"); n != 0 {
		t.Errorf("%d heartbeats were sent after the report", n)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	must(t, h.Stop(ctx))
	until(t, "the connection of the host closed", func() bool { return clientConn() <= before })
	until(t, "nothing of the host left", func() bool { return count("sdks/go/worker.(*Host)") <= beforeHost })
	// Started again, it opens a connection of its own, and closes it again.
	start(t, h)
	until(t, "a new connection", func() bool { return clientConn() > before })
	must(t, h.Stop(ctx))
	until(t, "the second connection closed", func() bool { return clientConn() <= before })
}

// A task that will not end: its handler ignores the cancellation. The host is stopped, and what is the host's goes
// with it: the heartbeat of that task does not outlive the host, though the handler's own goroutine (which no
// runtime can kill) may.
func TestTheHeartbeatOfATaskThatWillNotEndStopsWithTheHost(t *testing.T) {
	f := newFakeService(t)
	release := make(chan struct{})
	var started atomic.Bool
	h, err := worker.New(f.url(), "payments", "stubborn-worker", sagat.StaticToken("tok"), worker.WithHeartbeatInterval(10*time.Millisecond), f.clientSide())
	must(t, err)
	must(t, h.RegisterActivity("charge", func(context.Context, json.RawMessage) (any, error) {
		started.Store(true)
		<-release // never looks at its context
		return 1, nil
	}))
	t.Cleanup(func() { close(release) })
	f.lease(lease())
	must(t, h.Start(context.Background()))
	until(t, "the task", func() bool { return started.Load() })
	until(t, "heartbeats", func() bool { return len(f.of("Heartbeat")) >= 3 })
	if count("(*Host).beat") == 0 {
		t.Fatal("there is a heartbeat goroutine while the task runs")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	began := time.Now()
	if err := h.Stop(ctx); err == nil {
		t.Error("the task did not end within the grace: Stop says so")
	}
	if took := time.Since(began); took > 5*time.Second {
		t.Errorf("stop took %v", took)
	}
	until(t, "no heartbeat goroutine of the stopped host", func() bool { return count("(*Host).beat") == 0 })
	// Counted where the host sends, not where the service receives: a beat on its way when the host stopped may
	// still land. The sleep only gives a heartbeat that went on the time to show.
	f.note("stopped")
	time.Sleep(150 * time.Millisecond)
	if more := f.sentAfter("Heartbeat", "stopped"); more != 0 {
		t.Errorf("%d heartbeats after the host was stopped", more)
	}
}
