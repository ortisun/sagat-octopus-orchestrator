package worker

import (
	"context"
	"time"
)

// What the tests of package worker_test reach inside a host: a barrier in the receive loop and whether
// Stop has begun. Compiled only with the tests.

// SetFrameReceived installs a function the receive loop calls with the number of leases after it read a
// poll answer or a stream frame and before it looks at it: a test holds the loop there while Stop begins.
func SetFrameReceived(h *Host, hook func(leases int)) { h.frameReceived = hook }

// Stopping reports whether Stop has begun.
func Stopping(h *Host) bool { return h.stopping.Load() }

// HandBacks reports the hand-backs waiting their turn and the workers making them.
func HandBacks(h *Host) (queued, workers int) {
	h.mu.Lock()
	defer h.mu.Unlock()
	return len(h.handBackQueue), h.handBackWorkers
}

// ObservePauses makes observe see every pause of the loop of h, with its length, as the pause begins. It is called
// before Start; the tests read the pacing of the host from it instead of from the wall clock (AGENTS.md Rule 18).
func ObservePauses(h *Host, observe func(time.Duration)) { h.pauses = observe }

// SetStepLogTicker makes ticks the one-second tick of every attempt's step-log sender, called before Start: a test
// ticks when it says, instead of when the wall clock does (AGENTS.md Rule 18). The function returned stops nothing.
func SetStepLogTicker(h *Host, ticks <-chan time.Time) {
	h.stepLogTicker = func() (<-chan time.Time, func()) { return ticks, func() {} }
}

// Reregister registers the running host again, as the loop does when the server says the session is gone: a new
// session id and token, and a new step-log policy for the attempts dispatched after it.
func Reregister(h *Host) error {
	h.lifecycle.Lock()
	defer h.lifecycle.Unlock()
	return h.register(context.Background(), h.current)
}

// ResetStepLogSecrets empties the process's step-log secret set and its warned lines: the set lives for the process,
// and a test that fills it gives the next tests a process whose capture runs.
func ResetStepLogSecrets() {
	processCapture.secrets.wipe()
	processCapture.mu.Lock()
	processCapture.warned = map[string]bool{}
	processCapture.mu.Unlock()
}

// StepLogBatchesStopped reports whether an attempt of h that runs was answered UNIMPLEMENTED: a barrier for a handler
// that goes on logging only once the answer was read.
func StepLogBatchesStopped(h *Host) bool {
	h.mu.Lock()
	defer h.mu.Unlock()
	for capture := range h.captures {
		capture.mu.Lock()
		stopped := capture.batchesStopped
		capture.mu.Unlock()
		if stopped {
			return true
		}
	}
	return false
}
