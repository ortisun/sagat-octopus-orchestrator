package sagat

import "time"

// RetryPolicy says how many times, and how patiently, a call that is safe to repeat is repeated
// (sdks/SPEC.md, section 5): exponential backoff with full jitter, min(MaxDelay, InitialDelay*2^n)
// scaled by a random factor in [0.5, 1.0].
//
// A zero field keeps its default, so RetryPolicy{MaxAttempts: 1} turns retries off and
// RetryPolicy{InitialDelay: 20 * time.Millisecond} only shortens the first wait.
type RetryPolicy struct {
	// MaxAttempts is the number of attempts in all, the first included; 4 by default.
	MaxAttempts int
	// InitialDelay is the ceiling of the first wait; 200 ms by default.
	InitialDelay time.Duration
	// MaxDelay is the longest wait; 5 s by default.
	MaxDelay time.Duration
}

// DefaultRetry is the policy of an SDK that was given none.
var DefaultRetry = RetryPolicy{MaxAttempts: 4, InitialDelay: 200 * time.Millisecond, MaxDelay: 5 * time.Second}

// WithDefaults fills the zero fields with the defaults.
func (p RetryPolicy) WithDefaults() RetryPolicy {
	if p.MaxAttempts <= 0 {
		p.MaxAttempts = DefaultRetry.MaxAttempts
	}
	if p.InitialDelay <= 0 {
		p.InitialDelay = DefaultRetry.InitialDelay
	}
	if p.MaxDelay <= 0 {
		p.MaxDelay = DefaultRetry.MaxDelay
	}
	return p
}
