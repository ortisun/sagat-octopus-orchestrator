//go:build !unix

package worker_test

import "time"

// cpuTime is not measured where there is no getrusage: the test that uses it then checks nothing about the CPU.
func cpuTime() time.Duration { return 0 }
