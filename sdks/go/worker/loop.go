package worker

import (
	"context"
	"errors"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	boff "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/backoff"
	pb "github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/workerpb"
)

func (h *Host) inflight() int {
	h.mu.Lock()
	defer h.mu.Unlock()
	return len(h.tasks)
}

// idle is the pause between polls. A host that is stopping does not pause: Stop may have run while a
// poll was in flight, with nothing left to wake. A task that finishes wakes the loop, because a
// slot is free.
func (h *Host) idle(ctx context.Context, d time.Duration) {
	if h.stopping.Load() {
		return
	}
	timer := time.NewTimer(d)
	defer timer.Stop()
	select {
	case <-timer.C:
	case <-h.wake:
	case <-ctx.Done():
	}
}

// run is the loop of sdks/SPEC.md section 6.2. It never busy-polls: an empty answer is always
// followed by a wait.
func (h *Host) run(ctx context.Context, l *link) {
	defer close(l.loopDone)
	failures, refused := 0, 0
	for !h.stopping.Load() {
		free := h.cfg.maxInflight - h.inflight()
		if free <= 0 {
			h.idle(ctx, 50*time.Millisecond)
			continue
		}
		sess := h.session.Load()
		response, err := l.rpc.poll(ctx, &pb.PollTasksRequest{
			WorkerId: sess.workerID, SessionToken: sess.token, MaxTasks: int32(min(free, maxPoll)),
		})
		if err != nil {
			if h.stopping.Load() {
				return
			}
			h.pollFailed(ctx, l, err, &failures, &refused)
			continue
		}
		failures, refused = 0, 0
		tasks := response.GetTasks()
		// The host runs at most as many tasks as it asked for: a server that sends more is not obeyed. The
		// leases of the rest lapse and the server leases them again.
		if asked := min(free, maxPoll); len(tasks) > asked {
			h.log.Warn("the server sent more tasks than were asked for: the rest are left to lapse",
				"asked", asked, "received", len(tasks), "ignored", len(tasks)-asked)
			tasks = tasks[:asked]
		}
		h.emit(Event{Type: EventPoll, Tasks: len(tasks)})
		for _, lease := range tasks {
			h.dispatch(l, lease)
		}
		if len(tasks) == 0 {
			h.idle(ctx, boff.Jitter(pollWait(response.GetRetryAfterMs())))
		}
	}
}

func (h *Host) pollFailed(ctx context.Context, l *link, err error, failures, refused *int) {
	var se *sagat.Error
	if !errors.As(err, &se) {
		h.log.Error("the poll loop failed", "error", typeName(err))
		h.idle(ctx, time.Second)
		return
	}
	if se.Code == sagat.CodeUnauthenticated {
		// A server that registers a worker and refuses it at once must not be hammered: the first refusal
		// registers again at once, and each one in a row after it waits for the backoff first. A poll that
		// succeeds starts over.
		if *refused > 0 {
			h.idle(ctx, boff.Delay(h.retry, *refused-1, nil))
			if h.stopping.Load() {
				return
			}
		}
		*refused++
		h.log.Warn("the session is gone: registering again")
		h.emit(Event{Type: EventReconnect})
		registerErr := h.register(ctx, l)
		if registerErr == nil {
			return
		}
		h.log.Error("registering again failed", "code", codeOf(registerErr))
	}
	var retryAfter time.Duration
	if se.RetryAfter != nil {
		retryAfter = boff.CapServerWait(*se.RetryAfter)
	}
	if se.Code == sagat.CodeRateLimited {
		h.idle(ctx, max(retryAfter, 100*time.Millisecond))
		return
	}
	h.emit(Event{Type: EventReconnect})
	policy := h.retry
	policy.MaxDelay = 30 * time.Second
	wait := max(boff.Delay(policy, *failures, nil), retryAfter)
	*failures++
	h.idle(ctx, min(30*time.Second, wait))
}

// pollWait is how long the host waits after an empty poll that asked for retryAfterMs: a second when it
// asked for nothing, at least 50 ms, at most a minute whatever the server says.
func pollWait(retryAfterMs int32) time.Duration {
	wait := time.Duration(retryAfterMs) * time.Millisecond
	if wait <= 0 {
		wait = time.Second
	}
	return boff.CapServerWait(max(wait, 50*time.Millisecond))
}
