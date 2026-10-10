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
	failures := 0
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
			h.pollFailed(ctx, l, err, &failures)
			continue
		}
		failures = 0
		h.emit(Event{Type: EventPoll, Tasks: len(response.GetTasks())})
		for _, lease := range response.GetTasks() {
			h.dispatch(l, lease)
		}
		if len(response.GetTasks()) == 0 {
			wait := time.Duration(response.GetRetryAfterMs()) * time.Millisecond
			if wait <= 0 {
				wait = time.Second
			}
			h.idle(ctx, boff.Jitter(max(wait, 50*time.Millisecond)))
		}
	}
}

func (h *Host) pollFailed(ctx context.Context, l *link, err error, failures *int) {
	var se *sagat.Error
	if !errors.As(err, &se) {
		h.log.Error("the poll loop failed", "error", typeName(err))
		h.idle(ctx, time.Second)
		return
	}
	if se.Code == sagat.CodeUnauthenticated {
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
		retryAfter = *se.RetryAfter
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
