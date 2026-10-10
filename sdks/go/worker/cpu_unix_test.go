//go:build unix

package worker_test

import (
	"syscall"
	"time"
)

// cpuTime is the CPU time (user and system) this process has used.
func cpuTime() time.Duration {
	var usage syscall.Rusage
	if err := syscall.Getrusage(syscall.RUSAGE_SELF, &usage); err != nil {
		return 0
	}
	return time.Duration(usage.Utime.Nano() + usage.Stime.Nano())
}
