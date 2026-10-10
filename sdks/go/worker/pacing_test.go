package worker_test

import (
	"context"
	"encoding/json"
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
// when the test ends.
func startDefault(t *testing.T, f *fakeService, opts ...worker.Option) *worker.Host {
	t.Helper()
	h, err := worker.New(f.url(), "payments", "pacing-worker", sagat.StaticToken("tok"), opts...)
	must(t, err)
	must(t, h.RegisterActivity("charge", rawOK))
	must(t, h.Start(context.Background()))
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		_ = h.Stop(ctx)
	})
	return h
}

// The host never busy-polls (SPEC section 6.2): whatever the poll answered, the next one is after a wait. Each
// of these counts the polls in a fixed window; a loop that did not wait would make thousands.
func TestAFailedPollIsFollowedByABackoffAndNotByAnotherPollAtOnce(t *testing.T) {
	for name, code := range map[string]codes.Code{"unavailable": codes.Unavailable, "internal": codes.Internal, "deadline": codes.DeadlineExceeded} {
		t.Run(name, func(t *testing.T) {
			f := newFakeService(t)
			f.set(func(f *fakeService) { f.always["PollTasks"] = failure{code: code} })
			startDefault(t, f)
			time.Sleep(time.Second)
			if polls := len(f.of("PollTasks")); polls < 2 || polls > 6 {
				t.Errorf("%d polls in a second of failures: the backoff is 100 to 200 ms, then 200 to 400, then 400 to 800", polls)
			}
		})
	}
}

func TestAPollRefusedWithARetryAfterWaitsForIt(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) {
		f.always["PollTasks"] = failure{code: codes.ResourceExhausted, reason: "RATE_LIMITED", retryAfter: "400"}
	})
	startDefault(t, f)
	time.Sleep(1100 * time.Millisecond)
	polls := f.of("PollTasks")
	if len(polls) < 2 || len(polls) > 4 {
		t.Fatalf("%d polls in 1.1 s, each refused with a retry-after of 400 ms", len(polls))
	}
	for i := 1; i < len(polls); i++ {
		if gap := polls[i].at.Sub(polls[i-1].at); gap < 350*time.Millisecond {
			t.Errorf("poll %d came %v after the one before: the retry-after of 400 ms was not waited for", i+1, gap)
		}
	}
}

func TestAPollRefusedWithNoRetryAfterStillWaits(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) {
		f.always["PollTasks"] = failure{code: codes.ResourceExhausted, reason: "RATE_LIMITED"}
	})
	startDefault(t, f)
	time.Sleep(time.Second)
	if polls := len(f.of("PollTasks")); polls < 2 || polls > 12 {
		t.Errorf("%d polls in a second of refusals", polls)
	}
}

func TestAnEmptyPollWaitsForWhatItAskedAndNotLess(t *testing.T) {
	f := newFakeService(t)
	f.set(func(f *fakeService) { f.pollRetryAfterMs = 500 })
	startDefault(t, f)
	time.Sleep(1300 * time.Millisecond)
	polls := f.of("PollTasks")
	// 500 ms with 20 % jitter: 400 to 600.
	if len(polls) < 2 || len(polls) > 4 {
		t.Errorf("%d polls in 1.3 s of empty answers asking for 500 ms", len(polls))
	}
	for i := 1; i < len(polls); i++ {
		if gap := polls[i].at.Sub(polls[i-1].at); gap < 380*time.Millisecond {
			t.Errorf("poll %d came %v after the one before", i+1, gap)
		}
	}
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
	h := newHost(t, f, worker.WithHeartbeatInterval(10*time.Millisecond))
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
	beats := len(f.of("Heartbeat"))
	time.Sleep(100 * time.Millisecond)
	if len(f.of("Heartbeat")) != beats {
		t.Error("a heartbeat was made after the report")
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
