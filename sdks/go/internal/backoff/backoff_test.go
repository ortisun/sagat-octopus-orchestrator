package backoff

import (
	"context"
	"testing"
	"time"

	sagat "github.com/ortisun/sagat-octopus-orchestrator/sdks/go"
)

func TestDelayIsExponentialCappedAndScaledByJitter(t *testing.T) {
	policy := sagat.RetryPolicy{MaxAttempts: 4, InitialDelay: 200 * time.Millisecond, MaxDelay: 5 * time.Second}
	low := func() float64 { return 0 }       // factor 0.5
	high := func() float64 { return 0.9999 } // factor ~1.0
	cases := []struct {
		n         int
		low, high time.Duration
	}{
		{0, 100 * time.Millisecond, 200 * time.Millisecond},
		{1, 200 * time.Millisecond, 400 * time.Millisecond},
		{3, 800 * time.Millisecond, 1600 * time.Millisecond},
		{10, 2500 * time.Millisecond, 5 * time.Second}, // capped
		{2000, 2500 * time.Millisecond, 5 * time.Second},
	}
	for _, c := range cases {
		if got := Delay(policy, c.n, low); got != c.low {
			t.Errorf("n=%d low: %v, want %v", c.n, got, c.low)
		}
		if got := Delay(policy, c.n, high); got != c.high {
			t.Errorf("n=%d high: %v, want %v", c.n, got, c.high)
		}
	}
}

func TestAfterHonoursWhatTheServerAskedForWhenItIsLonger(t *testing.T) {
	policy := sagat.RetryPolicy{InitialDelay: 10 * time.Millisecond, MaxDelay: 20 * time.Millisecond}
	asked := 3 * time.Second
	e := sagat.NewError(sagat.CodeRateLimited, "slow")
	e.RetryAfter = &asked
	if got := After(policy, 0, e, nil); got != asked {
		t.Errorf("%v", got)
	}
	short := time.Millisecond
	e.RetryAfter = &short
	if got := After(policy, 0, e, func() float64 { return 0.5 }); got < 5*time.Millisecond || got > 10*time.Millisecond {
		t.Errorf("the backoff wins when it is longer: %v", got)
	}
	if got := After(policy, 0, nil, func() float64 { return 0 }); got != 5*time.Millisecond {
		t.Errorf("%v", got)
	}
}

func TestSleepEndsWithItsContext(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	go func() { time.Sleep(20 * time.Millisecond); cancel() }()
	began := time.Now()
	if err := Sleep(ctx, time.Minute); err == nil || time.Since(began) > 5*time.Second {
		t.Errorf("%v after %v", err, time.Since(began))
	}
	if err := Sleep(context.Background(), 5*time.Millisecond); err != nil {
		t.Error(err)
	}
	for i := 0; i < 100; i++ {
		if d := Jitter(time.Second); d < 800*time.Millisecond || d >= 1200*time.Millisecond {
			t.Fatalf("jitter out of range: %v", d)
		}
	}
}
