package worker

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
