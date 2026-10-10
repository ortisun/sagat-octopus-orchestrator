// Package backoff holds the waiting rules of the SDK: exponential backoff with jitter and a sleep
// that ends when its context does.
package backoff

import (
	"context"
	"math"
	"math/rand/v2"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
	"github.com/ortisun/sagat-octopus-orchestrator/sdks/go/internal/bound"
)

// Delay is min(MaxDelay, InitialDelay*2^n) scaled by random() in [0.5, 1.0]; n is the attempt that
// just failed, counted from 0. random returns a value in [0, 1).
func Delay(policy sagat.RetryPolicy, n int, random func() float64) time.Duration {
	policy = policy.WithDefaults()
	if random == nil {
		random = rand.Float64
	}
	ceiling := float64(policy.InitialDelay) * math.Pow(2, float64(n))
	if ceiling > float64(policy.MaxDelay) || math.IsInf(ceiling, 0) {
		ceiling = float64(policy.MaxDelay)
	}
	return time.Duration(ceiling * (0.5 + random()*0.5)).Round(time.Millisecond)
}

// After is the wait before the next attempt: the backoff, or what the server asked for when that is
// longer.
func After(policy sagat.RetryPolicy, n int, err *sagat.Error, random func() float64) time.Duration {
	d := Delay(policy, n, random)
	if err != nil && err.RetryAfter != nil && *err.RetryAfter > d {
		return CapServerWait(*err.RetryAfter)
	}
	return d
}

// Jitter scales d by a random factor in [0.8, 1.2).
func Jitter(d time.Duration) time.Duration {
	return time.Duration(float64(d) * (0.8 + rand.Float64()*0.4))
}

// Sleep waits for d, or returns the context's error as soon as the context ends.
func Sleep(ctx context.Context, d time.Duration) error {
	if d <= 0 {
		return ctx.Err()
	}
	timer := time.NewTimer(d)
	defer timer.Stop()
	select {
	case <-timer.C:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// CapServerWait caps a wait that came from the server at bound.MaxServerWait.
func CapServerWait(d time.Duration) time.Duration { return bound.Wait(d) }
